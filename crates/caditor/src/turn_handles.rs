use caditor_document::{
    FeatureId, FeatureKind, Revolve, RevolveAxis, RevolveExtent, SolidFeature, Transaction,
    displayed_axis,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};
use caditor_sketch::Reference;

use crate::{
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{ARROW_POINTS, Arrow, GAP_POINTS, HIT_POINTS, Handle, segment_distance},
    reach_handles::start_offset,
    scene,
    scene_palette::ScenePalette,
    units::Units,
};

const ARROW_SHARE: f64 = 0.6;
const STEP_DEGREES: f64 = 5.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const SMALLEST_RADIUS: f64 = 1e-6;
const ARC_STEP_DEGREES: f64 = 5.0;
const MAX_ARC_STEPS: f64 = 72.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    Only,
    Symmetric,
    Forward,
    Backward,
}

impl TurnEnd {
    fn caption(self) -> &'static str {
        match self {
            Self::Only => "Angle",
            Self::Symmetric => "Total angle",
            Self::Forward => "Forward",
            Self::Backward => "Backward",
        }
    }

    pub fn words(self) -> String {
        format!(
            "Drag to change the revolve's {}",
            self.caption().to_lowercase()
        )
    }

    fn slot(self, extent: &mut RevolveExtent) -> Option<&mut Expression> {
        match (self, extent) {
            (Self::Only, RevolveExtent::OneSide { angle, .. })
            | (Self::Symmetric, RevolveExtent::Symmetric { angle })
            | (Self::Forward, RevolveExtent::TwoSides { forward: angle, .. })
            | (
                Self::Backward,
                RevolveExtent::TwoSides {
                    backward: angle, ..
                },
            ) => Some(angle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Swing {
    foot: Point3,
    radial: Vector3,
    normal: Vector3,
    radius: f64,
}

impl Swing {
    fn at(&self, degrees: f64) -> Point3 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        self.foot + (self.radial * cos + self.normal * sin) * self.radius
    }

    fn tangent(&self, degrees: f64) -> Vector3 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        self.normal * cos - self.radial * sin
    }

    fn degrees_of(&self, ray: Ray) -> Option<f64> {
        let axis = self.radial.cross(self.normal);
        let plane = caditor_geometry::Plane::new(self.foot, axis)?;
        let hit = ray.at(ray.intersect_plane(&plane)?) - self.foot;
        Some(
            hit.dot(self.normal)
                .atan2(hit.dot(self.radial))
                .to_degrees(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Grip {
    end: TurnEnd,
    degrees: f64,
    sign: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnHandles {
    pub feature: FeatureId,
    swing: Swing,
    grips: [Option<Grip>; 2],
    per_point: f64,
    forward: Vector3,
}

fn shown_revolve(model: &Model, feature: FeatureId) -> Option<&Revolve> {
    let kind = model
        .draft_kind(feature)
        .or_else(|| Some(&model.document().feature(feature)?.kind))?;
    match kind {
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => Some(revolve),
        _ => None,
    }
}

fn committed_revolve(model: &Model, feature: FeatureId) -> Option<&Revolve> {
    match &model.document().feature(feature)?.kind {
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => Some(revolve),
        _ => None,
    }
}

fn degrees(model: &Model, feature: FeatureId, expression: &Expression) -> Option<f64> {
    let parameters = model.shown_parameters(feature);
    expression
        .evaluate_as(Dimension::ANGLE, &|id| parameters.value(id))
        .ok()
}

fn axis_line(model: &Model, feature: FeatureId, revolve: &Revolve) -> Option<(Point3, Vector3)> {
    let document = model.document();
    let plane = scene::sketch_plane(document, model.evaluation(), revolve.sketch)?;
    match &revolve.axis {
        RevolveAxis::Sketch(line) => {
            if let Some(reference) = line.reference() {
                let along = match reference {
                    Reference::HorizontalAxis => plane.x_axis(),
                    Reference::VerticalAxis => plane.y_axis(),
                    Reference::Origin => return None,
                };
                return Some((plane.origin(), along));
            }
            let sketch = document.feature(revolve.sketch)?;
            let displayed = model.displayed_sketch(sketch)?;
            let (start, end) = displayed.line_endpoints(*line)?;
            let (start, end) = (plane.to_world(start), plane.to_world(end));
            Some((start, (end - start).try_normalize()?))
        }
        RevolveAxis::Model(axis) => {
            let ray = displayed_axis(model.evaluation(), feature, axis)?;
            Some((ray.origin(), ray.direction()))
        }
    }
}

fn swing(model: &Model, feature: FeatureId, revolve: &Revolve) -> Option<Swing> {
    let document = model.document();
    let plane = scene::sketch_plane(document, model.evaluation(), revolve.sketch)?;
    let bounds = model.sketch_bounds(document.feature(revolve.sketch)?)?;
    let middle = plane.to_world(plane.to_local(bounds.center()));
    let (origin, along) = axis_line(model, feature, revolve)?;
    let foot = origin + along * (middle - origin).dot(along);
    let offset = middle - foot;
    let radius = offset.length();
    let started = plane.normal() * start_offset(model, feature, &plane, revolve.start.as_ref())?;
    (radius > SMALLEST_RADIUS).then_some(Swing {
        foot: foot + started,
        radial: offset / radius,
        normal: plane.normal(),
        radius,
    })
}

fn grips(model: &Model, feature: FeatureId, extent: &RevolveExtent) -> [Option<Grip>; 2] {
    let grip = |end, expression: &Expression, sign: f64, share: f64| {
        degrees(model, feature, expression).map(|value| Grip {
            end,
            degrees: sign * value * share,
            sign,
        })
    };
    match extent {
        RevolveExtent::OneSide { angle, reversed } => [
            grip(
                TurnEnd::Only,
                angle,
                if *reversed { -1.0 } else { 1.0 },
                1.0,
            ),
            None,
        ],
        RevolveExtent::Symmetric { angle } => [grip(TurnEnd::Symmetric, angle, 1.0, 0.5), None],
        RevolveExtent::TwoSides { forward, backward } => [
            grip(TurnEnd::Forward, forward, 1.0, 1.0),
            grip(TurnEnd::Backward, backward, -1.0, 1.0),
        ],
        RevolveExtent::Full | RevolveExtent::UpTo { .. } => [None, None],
    }
}

pub fn turn_arcs(model: &Model, feature: FeatureId) -> Vec<(TurnEnd, Vec<Point3>)> {
    let Some(revolve) = committed_revolve(model, feature) else {
        return Vec::new();
    };
    let Some(swing) = swing(model, feature, revolve) else {
        return Vec::new();
    };
    grips(model, feature, &revolve.extent)
        .into_iter()
        .flatten()
        .map(|grip| {
            let from = match grip.end {
                TurnEnd::Symmetric => -grip.degrees,
                TurnEnd::Only | TurnEnd::Forward | TurnEnd::Backward => 0.0,
            };
            let span = grip.degrees - from;
            let steps = (span.abs() / ARC_STEP_DEGREES)
                .ceil()
                .clamp(1.0, MAX_ARC_STEPS);
            let points = (0..=steps as usize)
                .map(|step| swing.at(from + span * step as f64 / steps))
                .collect();
            (grip.end, points)
        })
        .collect()
}

impl TurnHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let revolve = shown_revolve(model, feature)?;
        let swing = swing(model, feature, revolve)?;
        let grips = grips(model, feature, &revolve.extent);
        let depth = view.view_depth(swing.foot + swing.radial * swing.radius);
        let per_point = pixels_per_point * view.units_per_pixel_at(depth);
        let shown = depth.is_finite()
            && depth > 0.0
            && per_point.is_finite()
            && per_point > 0.0
            && grips.iter().any(Option::is_some);
        shown.then_some(Self {
            feature,
            swing,
            grips,
            per_point,
            forward: view.forward(),
        })
    }

    fn grip(&self, end: TurnEnd) -> Option<Grip> {
        self.grips
            .iter()
            .flatten()
            .find(|grip| grip.end == end)
            .copied()
    }

    fn segment(&self, grip: Grip) -> (Point3, Point3, Vector3) {
        let at = self.swing.at(grip.degrees);
        let direction = self.swing.tangent(grip.degrees) * grip.sign;
        (
            at + direction * GAP_POINTS * self.per_point,
            at + direction * ARROW_POINTS * ARROW_SHARE * self.per_point,
            direction,
        )
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        STEP_DEGREES
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Revolve(end) = handle else {
            return None;
        };
        let revolve = committed_revolve(model, self.feature)?;
        let mut extent = revolve.extent.clone();
        let slot = end.slot(&mut extent)?;
        Held::of(model.document(), self.feature, end.caption(), slot).driven()
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        self.grips
            .iter()
            .flatten()
            .filter_map(|grip| {
                let (from, to, _) = self.segment(*grip);
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Revolve(grip.end)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle)
    }

    #[cfg(test)]
    pub fn grip_at(&self, handle: Handle, turned: f64) -> Option<Point3> {
        let Handle::Revolve(end) = handle else {
            return None;
        };
        let grip = self.grip(end)?;
        let (from, tip, _) = self.segment(grip);
        let offset = from.lerp(tip, 0.6) - self.swing.foot;
        let axis = self.swing.radial.cross(self.swing.normal);
        let (x, y, z) = (
            offset.dot(self.swing.radial),
            offset.dot(self.swing.normal),
            offset.dot(axis),
        );
        let (sin, cos) = (grip.sign * turned).to_radians().sin_cos();
        Some(
            self.swing.foot
                + self.swing.radial * (x * cos - y * sin)
                + self.swing.normal * (x * sin + y * cos)
                + axis * z,
        )
    }

    #[cfg(test)]
    pub fn foot(&self, handle: Handle) -> Option<Point3> {
        let Handle::Revolve(end) = handle else {
            return None;
        };
        Some(self.swing.at(self.grip(end)?.degrees))
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        for grip in self.grips.iter().flatten() {
            let (from, tip, direction) = self.segment(*grip);
            let colour = if highlighted == Some(Handle::Revolve(grip.end)) {
                palette.handle_highlighted
            } else {
                palette.handle
            };
            Arrow {
                from,
                tip,
                direction,
                per_point: self.per_point,
                forward: self.forward,
            }
            .add_to(batch, colour);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnDrag {
    start: Revolve,
    end: TurnEnd,
    swing: Swing,
    sign: f64,
    share: f64,
    limit: f64,
    last: f64,
    swept: f64,
    from: f64,
    value: f64,
}

impl TurnDrag {
    pub fn begin(model: &Model, handles: &TurnHandles, end: TurnEnd, ray: Ray) -> Option<Self> {
        let start = committed_revolve(model, handles.feature)?.clone();
        let grip = handles.grip(end)?;
        let mut extent = start.extent.clone();
        let from = degrees(model, handles.feature, end.slot(&mut extent)?)?;
        let other = match (&start.extent, end) {
            (RevolveExtent::TwoSides { backward, .. }, TurnEnd::Forward) => {
                degrees(model, handles.feature, backward)?
            }
            (RevolveExtent::TwoSides { forward, .. }, TurnEnd::Backward) => {
                degrees(model, handles.feature, forward)?
            }
            _ => 0.0,
        };
        let last = handles.swing.degrees_of(ray)?;
        Some(Self {
            start,
            end,
            swing: handles.swing,
            sign: grip.sign,
            share: if end == TurnEnd::Symmetric { 2.0 } else { 1.0 },
            limit: FULL_TURN_DEGREES - other,
            last,
            swept: 0.0,
            from,
            value: from,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(now) = self.swing.degrees_of(ray) else {
            return false;
        };
        let turned = (now - self.last + 180.0).rem_euclid(360.0) - 180.0;
        self.last = now;
        self.swept += turned;
        let wanted = self.from + self.sign * self.share * self.swept;
        let rounded = if free {
            wanted
        } else {
            (wanted / STEP_DEGREES).round() * STEP_DEGREES
        };
        let value = rounded.clamp(STEP_DEGREES.min(self.limit), self.limit);
        let changed = value != self.value;
        self.value = value;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.value != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let document = model.document();
        let mut probe = self.start.extent.clone();
        let held = Held::of(
            document,
            feature,
            self.end.caption(),
            self.end.slot(&mut probe)?,
        );
        let mut revolve = self.start.clone();
        let mut named = Vec::new();
        held.set(
            self.end.slot(&mut revolve.extent)?,
            model.units().angle.measured(self.value),
            &mut named,
        );
        manipulator::keeping_names(
            document,
            feature,
            FeatureKind::Solid(SolidFeature::Revolve(revolve)),
            named,
        )
    }

    pub fn readout(&self, units: Units) -> String {
        format!(
            "{} {}",
            self.end.caption(),
            units.angle.readout_text(self.value)
        )
    }
}
