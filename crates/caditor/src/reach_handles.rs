use caditor_document::{
    Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, ParameterValues, SolidFeature,
    SolidStart, Transaction, displayed_start_offset,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};

use crate::{
    handle_snap::Snap,
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, Handle, Reach, segment_distance,
        step_for,
    },
    scene_palette::ScenePalette,
    units::Units,
};

#[cfg(test)]
const GRIP_FRACTION: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ReachArrow {
    reach: Reach,
    end: Point3,
    direction: Vector3,
    per_point: f64,
}

impl ReachArrow {
    fn segment(&self) -> (Point3, Point3) {
        (
            self.end + self.direction * GAP_POINTS * self.per_point,
            self.end + self.direction * ARROW_POINTS * self.per_point,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReachHandles {
    pub feature: FeatureId,
    arrows: [Option<ReachArrow>; 2],
    forward: Vector3,
}

fn shown_extrude(model: &Model, feature: FeatureId) -> Option<&Extrude> {
    let kind = model
        .draft_kind(feature)
        .or_else(|| Some(&model.document().feature(feature)?.kind))?;
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => Some(extrude),
        _ => None,
    }
}

fn committed_extrude(model: &Model, feature: FeatureId) -> Option<&Extrude> {
    match &model.document().feature(feature)?.kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => Some(extrude),
        _ => None,
    }
}

fn length(parameters: &ParameterValues, expression: &Expression) -> Option<f64> {
    expression
        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
        .ok()
}

pub fn start_offset(
    model: &Model,
    feature: FeatureId,
    sketch_plane: &Plane,
    start: Option<&SolidStart>,
) -> Option<f64> {
    match start {
        None => Some(0.0),
        Some(SolidStart::Distance(distance)) => length(model.shown_parameters(feature), distance),
        Some(SolidStart::Plane(target)) => {
            displayed_start_offset(model.evaluation(), feature, sketch_plane, target)
        }
    }
}

fn reaches(parameters: &ParameterValues, extent: &ExtrudeExtent) -> [Option<(Reach, f64, f64)>; 2] {
    let end_length = |end: &ExtrudeEnd| end.distance().and_then(|value| length(parameters, value));
    match extent {
        ExtrudeExtent::OneSide { end, reversed } => {
            let sign = if *reversed { -1.0 } else { 1.0 };
            [
                end_length(end).map(|value| (Reach::Only, sign, value)),
                None,
            ]
        }
        ExtrudeExtent::Symmetric { distance } => [
            length(parameters, distance).map(|value| (Reach::Symmetric, 1.0, value / 2.0)),
            None,
        ],
        ExtrudeExtent::TwoSides { forward, backward } => [
            end_length(forward).map(|value| (Reach::Forward, 1.0, value)),
            end_length(backward).map(|value| (Reach::Backward, -1.0, value)),
        ],
    }
}

fn distance_of(model: &Model, extent: &ExtrudeExtent, reach: Reach) -> Option<f64> {
    let parameters = model.parameters();
    let end_length = |end: &ExtrudeEnd| end.distance().and_then(|value| length(parameters, value));
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only) => end_length(end),
        (ExtrudeExtent::Symmetric { distance }, Reach::Symmetric) => length(parameters, distance),
        (ExtrudeExtent::TwoSides { forward, .. }, Reach::Forward) => end_length(forward),
        (ExtrudeExtent::TwoSides { backward, .. }, Reach::Backward) => end_length(backward),
        _ => None,
    }
}

fn slot_mut(extent: &mut ExtrudeExtent, reach: Reach) -> Option<&mut Expression> {
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only)
        | (ExtrudeExtent::TwoSides { forward: end, .. }, Reach::Forward)
        | (ExtrudeExtent::TwoSides { backward: end, .. }, Reach::Backward) => match end {
            ExtrudeEnd::Distance(distance) => Some(distance),
            _ => None,
        },
        (ExtrudeExtent::Symmetric { distance }, Reach::Symmetric) => Some(distance),
        _ => None,
    }
}

fn held(model: &Model, feature: FeatureId, extrude: &Extrude, reach: Reach) -> Option<Held> {
    let mut extent = extrude.extent.clone();
    let slot = slot_mut(&mut extent, reach)?;
    Some(Held::of(
        model.document(),
        feature,
        reach.field_caption(),
        slot,
    ))
}

impl ReachHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let extrude = shown_extrude(model, feature)?;
        if extrude.direction.is_some() {
            return None;
        }
        let parameters = model.shown_parameters(feature);
        let document = model.document();
        let plane = crate::scene::sketch_plane(document, model.evaluation(), extrude.sketch)?;
        let start = start_offset(model, feature, &plane, extrude.start.as_ref())?;
        let bounds = model.sketch_bounds(document.feature(extrude.sketch)?)?;
        let normal = plane.normal();
        let base = plane.to_world(plane.to_local(bounds.center())) + normal * start;
        let forward = view.forward();
        let arrow = |found: Option<(Reach, f64, f64)>| {
            let (reach, sign, distance) = found?;
            let direction = normal * sign;
            let end = base + direction * distance;
            let depth = view.view_depth(end);
            let per_point = pixels_per_point * view.units_per_pixel_at(depth);
            let shown = depth.is_finite()
                && depth > 0.0
                && per_point.is_finite()
                && per_point > 0.0
                && direction.dot(forward).abs() < END_ON;
            shown.then_some(ReachArrow {
                reach,
                end,
                direction,
                per_point,
            })
        };
        let arrows = reaches(parameters, &extrude.extent).map(arrow);
        arrows.iter().any(Option::is_some).then_some(Self {
            feature,
            arrows,
            forward,
        })
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Reach(reach) = handle else {
            return None;
        };
        held(
            model,
            self.feature,
            committed_extrude(model, self.feature)?,
            reach,
        )?
        .driven()
    }

    fn arrows(&self) -> impl Iterator<Item = &ReachArrow> {
        self.arrows.iter().flatten()
    }

    fn arrow(&self, reach: Reach) -> Option<&ReachArrow> {
        self.arrows().find(|arrow| arrow.reach == reach)
    }

    pub fn step(&self) -> f64 {
        self.arrows()
            .next()
            .map_or(1.0, |arrow| step_for(arrow.per_point))
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        self.arrows()
            .filter_map(|arrow| {
                let (from, to) = arrow.segment();
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Reach(arrow.reach)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle)
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        let Handle::Reach(reach) = handle else {
            return None;
        };
        let arrow = self.arrow(reach)?;
        let (from, tip) = arrow.segment();
        Some(from.lerp(tip, GRIP_FRACTION) + arrow.direction * along)
    }

    #[cfg(test)]
    pub fn foot(&self, handle: Handle) -> Option<Point3> {
        let Handle::Reach(reach) = handle else {
            return None;
        };
        Some(self.arrow(reach)?.end)
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        for arrow in self.arrows() {
            let (from, tip) = arrow.segment();
            let colour = if highlighted == Some(Handle::Reach(arrow.reach)) {
                palette.handle_highlighted
            } else {
                palette.handle
            };
            Arrow {
                from,
                tip,
                direction: arrow.direction,
                per_point: arrow.per_point,
                forward: self.forward,
            }
            .add_to(batch, colour);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReachDrag {
    start: Extrude,
    reach: Reach,
    end: Point3,
    direction: Vector3,
    grabbed: f64,
    step: f64,
    from: f64,
    distance: f64,
    snapped: Option<Snap>,
}

impl ReachDrag {
    pub fn begin(model: &Model, handles: &ReachHandles, reach: Reach, ray: Ray) -> Option<Self> {
        let start = committed_extrude(model, handles.feature)?;
        let arrow = handles.arrow(reach)?;
        let from = distance_of(model, &start.extent, reach)?;
        let grabbed = ray.closest_along_line(arrow.end, arrow.direction)?;
        Some(Self {
            start: start.clone(),
            reach,
            end: arrow.end,
            direction: arrow.direction,
            grabbed,
            step: handles.step(),
            from,
            distance: from,
            snapped: None,
        })
    }

    fn grown(&self, pulled: f64) -> f64 {
        match self.reach {
            Reach::Symmetric => 2.0 * pulled,
            Reach::Only | Reach::Forward | Reach::Backward => pulled,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        let pulled = ray
            .closest_along_line(self.end, self.direction)
            .map(|at| self.from + self.grown(at - self.grabbed));
        let snapped = snap.filter(|_| !free).and_then(|snap| {
            let distance = self.from + self.grown(snap.along(self.end, self.direction)?);
            snap.holds(distance, pulled, self.step)
                .then_some((distance, snap))
        });
        let (distance, snap) = match snapped {
            Some((distance, snap)) => (distance, Some(snap)),
            None => {
                let Some(wanted) = pulled else {
                    return false;
                };
                let rounded = if free {
                    wanted
                } else {
                    (wanted / self.step).round() * self.step
                };
                (rounded, None)
            }
        };
        let distance = distance.max(self.step);
        let changed = distance != self.distance || snap != self.snapped;
        self.distance = distance;
        self.snapped = snap;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.distance != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let distance = model.units().length.measured(self.distance);
        let held = held(model, feature, &self.start, self.reach)?;
        let mut extrude = self.start.clone();
        let mut named = Vec::new();
        held.set(
            slot_mut(&mut extrude.extent, self.reach)?,
            distance,
            &mut named,
        );
        let kind = FeatureKind::Solid(SolidFeature::Extrude(extrude));
        manipulator::keeping_names(model.document(), feature, kind, named)
    }

    pub fn readout(&self, units: Units) -> String {
        let shown = format!(
            "{} {}",
            self.reach.caption(),
            units.readout_text(self.distance)
        );
        match self.snapped {
            Some(snap) => format!("{shown} {}", snap.kind.words()),
            None => shown,
        }
    }
}

pub fn typed(
    model: &Model,
    feature: FeatureId,
    reach: Reach,
    value: Expression,
) -> Option<Transaction> {
    let start = committed_extrude(model, feature)?;
    let held = held(model, feature, start, reach)?;
    let mut extrude = start.clone();
    let mut named = Vec::new();
    held.set(slot_mut(&mut extrude.extent, reach)?, value, &mut named);
    let kind = FeatureKind::Solid(SolidFeature::Extrude(extrude));
    manipulator::keeping_names(model.document(), feature, kind, named)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_distance_replaces_only_its_own_end() {
        let ten = Expression::number(10.0);
        let two = ExtrudeExtent::two_sides(ten.clone(), ten.clone());

        let mut changed = two.clone();
        let mut unchanged = two.clone();
        if let Some(slot) = slot_mut(&mut changed, Reach::Backward) {
            *slot = Expression::number(4.0);
        }

        assert!(slot_mut(&mut unchanged, Reach::Only).is_none());
        assert_eq!(
            changed,
            ExtrudeExtent::two_sides(ten.clone(), Expression::number(4.0))
        );
        assert_eq!(unchanged, two);
    }
}
