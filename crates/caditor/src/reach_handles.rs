use caditor_document::{
    Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, SolidFeature, SolidStart,
    Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};

use crate::{
    model::Model,
    move_manipulator::{
        ABOUT_AXIS, ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIGHLIGHTED, HIT_POINTS, Handle,
        Reach, segment_distance, step_for,
    },
    solid_tools,
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

fn length(model: &Model, expression: &Expression) -> Option<f64> {
    let parameters = model.parameters();
    expression
        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
        .ok()
}

fn reaches(model: &Model, extent: &ExtrudeExtent) -> [Option<(Reach, f64, f64)>; 2] {
    let end_length = |end: &ExtrudeEnd| end.distance().and_then(|value| length(model, value));
    match extent {
        ExtrudeExtent::OneSide { end, reversed } => {
            let sign = if *reversed { -1.0 } else { 1.0 };
            [
                end_length(end).map(|value| (Reach::Only, sign, value)),
                None,
            ]
        }
        ExtrudeExtent::Symmetric { distance } => [
            length(model, distance).map(|value| (Reach::Symmetric, 1.0, value / 2.0)),
            None,
        ],
        ExtrudeExtent::TwoSides { forward, backward } => [
            end_length(forward).map(|value| (Reach::Forward, 1.0, value)),
            end_length(backward).map(|value| (Reach::Backward, -1.0, value)),
        ],
    }
}

fn distance_of(model: &Model, extent: &ExtrudeExtent, reach: Reach) -> Option<f64> {
    let end_length = |end: &ExtrudeEnd| end.distance().and_then(|value| length(model, value));
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only) => end_length(end),
        (ExtrudeExtent::Symmetric { distance }, Reach::Symmetric) => length(model, distance),
        (ExtrudeExtent::TwoSides { forward, .. }, Reach::Forward) => end_length(forward),
        (ExtrudeExtent::TwoSides { backward, .. }, Reach::Backward) => end_length(backward),
        _ => None,
    }
}

fn with_distance(extent: &ExtrudeExtent, reach: Reach, distance: Expression) -> ExtrudeExtent {
    let mut changed = extent.clone();
    match (&mut changed, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only)
        | (ExtrudeExtent::TwoSides { forward: end, .. }, Reach::Forward)
        | (ExtrudeExtent::TwoSides { backward: end, .. }, Reach::Backward) => {
            *end = ExtrudeEnd::Distance(distance);
        }
        (ExtrudeExtent::Symmetric { distance: total }, Reach::Symmetric) => *total = distance,
        _ => {}
    }
    changed
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
        let start = match &extrude.start {
            None => 0.0,
            Some(SolidStart::Distance(distance)) => length(model, distance)?,
            Some(SolidStart::Plane(_)) => return None,
        };
        let document = model.document();
        let plane = crate::scene::sketch_plane(document, model.evaluation(), extrude.sketch)?;
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
        let arrows = reaches(model, &extrude.extent).map(arrow);
        arrows.iter().any(Option::is_some).then_some(Self {
            feature,
            arrows,
            forward,
        })
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

    pub fn add_to(&self, batch: &mut Batch, highlighted: Option<Handle>) {
        for arrow in self.arrows() {
            let (from, tip) = arrow.segment();
            let colour = if highlighted == Some(Handle::Reach(arrow.reach)) {
                HIGHLIGHTED
            } else {
                ABOUT_AXIS
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
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(at) = ray.closest_along_line(self.end, self.direction) else {
            return false;
        };
        let pulled = at - self.grabbed;
        let grown = match self.reach {
            Reach::Symmetric => 2.0 * pulled,
            Reach::Only | Reach::Forward | Reach::Backward => pulled,
        };
        let wanted = self.from + grown;
        let rounded = if free {
            wanted
        } else {
            (wanted / self.step).round() * self.step
        };
        let distance = rounded.max(self.step);
        let changed = distance != self.distance;
        self.distance = distance;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.distance != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let distance = model.units().length.measured(self.distance);
        let extrude = Extrude {
            extent: with_distance(&self.start.extent, self.reach, distance),
            ..self.start.clone()
        };
        solid_tools::edit(model.document(), feature, SolidFeature::Extrude(extrude))
    }

    pub fn readout(&self, units: Units) -> String {
        format!(
            "{} {}",
            self.reach.caption(),
            units.readout_text(self.distance)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_distance_replaces_only_its_own_end() {
        let ten = Expression::number(10.0);
        let two = ExtrudeExtent::two_sides(ten.clone(), ten.clone());

        let changed = with_distance(&two, Reach::Backward, Expression::number(4.0));
        let unchanged = with_distance(&two, Reach::Only, Expression::number(4.0));

        assert_eq!(
            changed,
            ExtrudeExtent::two_sides(ten.clone(), Expression::number(4.0))
        );
        assert_eq!(unchanged, two);
    }
}
