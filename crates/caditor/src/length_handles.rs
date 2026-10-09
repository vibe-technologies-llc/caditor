use caditor_document::{
    Datum, DatumPlane, Document, Evaluation, FeatureId, FeatureKind, Hole, HoleDepth, OffsetFace,
    Transaction, face_plane, hole_centres,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};

use crate::{
    hole_tools,
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{
        ABOUT_AXIS, ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIGHLIGHTED, HIT_POINTS, Handle,
        segment_distance, step_for,
    },
    scene,
    units::Units,
};

const ARROW_SHARE: f64 = 0.8;
pub const HOLE_DEPTH: &str = "Depth";
pub const OFFSET_DISTANCE: &str = "Distance";
pub const PLANE_OFFSET: &str = "Offset";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    HoleDepth,
    OffsetDistance,
    PlaneOffset,
}

impl Measured {
    fn caption(self) -> &'static str {
        match self {
            Self::HoleDepth => HOLE_DEPTH,
            Self::OffsetDistance => OFFSET_DISTANCE,
            Self::PlaneOffset => PLANE_OFFSET,
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Self::HoleDepth => "Drag to change the hole's depth",
            Self::OffsetDistance => "Drag to change how far the faces move",
            Self::PlaneOffset => "Drag to change the plane's offset",
        }
    }

    fn slot(self, kind: &mut FeatureKind) -> Option<&mut Expression> {
        match (self, kind) {
            (
                Self::HoleDepth,
                FeatureKind::Hole(Hole {
                    depth: HoleDepth::Blind(depth),
                    ..
                }),
            ) => Some(depth),
            (Self::OffsetDistance, FeatureKind::OffsetFace(OffsetFace { distance, .. })) => {
                Some(distance)
            }
            (Self::PlaneOffset, FeatureKind::Datum(Datum::Plane(DatumPlane { offset, .. }))) => {
                Some(offset)
            }
            _ => None,
        }
    }

    fn accepts(self, value: f64, step: f64) -> f64 {
        match self {
            Self::HoleDepth => value.max(step),
            Self::OffsetDistance if value == 0.0 => step,
            Self::OffsetDistance | Self::PlaneOffset => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LengthHandles {
    pub feature: FeatureId,
    pub measured: Measured,
    base: Point3,
    direction: Vector3,
    value: f64,
    per_point: f64,
    forward: Vector3,
}

fn length_of(model: &Model, feature: FeatureId, expression: &Expression) -> Option<f64> {
    let parameters = model.shown_parameters(feature);
    expression
        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
        .ok()
}

fn shown_kind(model: &Model, feature: FeatureId) -> Option<&FeatureKind> {
    model
        .draft_kind(feature)
        .or_else(|| Some(&model.document().feature(feature)?.kind))
}

fn hole_arrow(model: &Model, hole: &Hole) -> Option<(Point3, Vector3)> {
    let document = model.document();
    let sketch = document.feature(hole.sketch)?;
    let displayed = model.displayed_sketch(sketch)?;
    let (_, centre) = hole_centres(&displayed).into_iter().next()?;
    let plane = scene::sketch_plane(document, model.evaluation(), hole.sketch)?;
    let down = if hole.reversed {
        plane.normal()
    } else {
        -plane.normal()
    };
    Some((plane.to_world(centre), down))
}

fn offset_arrow(
    evaluation: &Evaluation,
    feature: FeatureId,
    offset: &OffsetFace,
) -> Option<(Point3, Vector3)> {
    let before = evaluation.body_before(feature)?;
    let solid = &before.solid()?.solid;
    let face = offset.faces.first()?.resolve(solid).ok()?;
    let plane = face_plane(solid, face)?;
    let middle = hole_tools::middle_of(solid, face, &plane)?;
    Some((plane.to_world(middle), plane.normal()))
}

fn plane_arrow(model: &Model, feature: FeatureId) -> Option<(Point3, Vector3)> {
    let evaluation = model
        .draft_evaluation_of(feature)
        .unwrap_or_else(|| model.evaluation());
    let plane = evaluation
        .feature(feature)?
        .result
        .as_deref()?
        .datum()?
        .plane()?;
    Some((plane.origin(), plane.normal()))
}

impl LengthHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let kind = shown_kind(model, feature)?;
        let (measured, end, direction, value) = match kind {
            FeatureKind::Hole(hole) => {
                let HoleDepth::Blind(depth) = &hole.depth else {
                    return None;
                };
                let value = length_of(model, feature, depth)?;
                let (top, down) = hole_arrow(model, hole)?;
                (Measured::HoleDepth, top + down * value, down, value)
            }
            FeatureKind::OffsetFace(offset) => {
                let value = length_of(model, feature, &offset.distance)?;
                let (middle, normal) = offset_arrow(model.evaluation(), feature, offset)?;
                (
                    Measured::OffsetDistance,
                    middle + normal * value,
                    normal,
                    value,
                )
            }
            FeatureKind::Datum(Datum::Plane(plane)) => {
                let value = length_of(model, feature, &plane.offset)?;
                let (origin, normal) = plane_arrow(model, feature)?;
                (Measured::PlaneOffset, origin, normal, value)
            }
            _ => return None,
        };
        let forward = view.forward();
        let depth = view.view_depth(end);
        let per_point = pixels_per_point * view.units_per_pixel_at(depth);
        let shown = depth.is_finite()
            && depth > 0.0
            && per_point.is_finite()
            && per_point > 0.0
            && direction.dot(forward).abs() < END_ON;
        shown.then_some(Self {
            feature,
            measured,
            base: end - direction * value,
            direction,
            value,
            per_point,
            forward,
        })
    }

    fn end(&self) -> Point3 {
        self.base + self.direction * self.value
    }

    fn segment(&self) -> (Point3, Point3) {
        let end = self.end();
        (
            end + self.direction * GAP_POINTS * self.per_point,
            end + self.direction * ARROW_POINTS * ARROW_SHARE * self.per_point,
        )
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point)
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        if handle != Handle::Length {
            return None;
        }
        let document = model.document();
        let mut kind = document.feature(self.feature)?.kind.clone();
        let slot = self.measured.slot(&mut kind)?;
        Held::of(document, self.feature, self.measured.caption(), slot).driven()
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let (from, to) = self.segment();
        let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
        (distance <= HIT_POINTS * pixels_per_point).then_some(Handle::Length)
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        if handle != Handle::Length {
            return None;
        }
        let (from, tip) = self.segment();
        Some(from.lerp(tip, 0.6) + self.direction * along)
    }

    pub fn add_to(&self, batch: &mut Batch, highlighted: Option<Handle>) {
        let (from, tip) = self.segment();
        let colour = if highlighted == Some(Handle::Length) {
            HIGHLIGHTED
        } else {
            ABOUT_AXIS
        };
        Arrow {
            from,
            tip,
            direction: self.direction,
            per_point: self.per_point,
            forward: self.forward,
        }
        .add_to(batch, colour);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LengthDrag {
    start: FeatureKind,
    measured: Measured,
    end: Point3,
    direction: Vector3,
    grabbed: f64,
    step: f64,
    from: f64,
    value: f64,
}

impl LengthDrag {
    pub fn begin(model: &Model, handles: &LengthHandles, ray: Ray) -> Option<Self> {
        let start = model.document().feature(handles.feature)?.kind.clone();
        let mut probe = start.clone();
        let from = length_of(model, handles.feature, handles.measured.slot(&mut probe)?)?;
        let end = handles.base + handles.direction * from;
        let grabbed = ray.closest_along_line(end, handles.direction)?;
        Some(Self {
            start,
            measured: handles.measured,
            end,
            direction: handles.direction,
            grabbed,
            step: handles.step(),
            from,
            value: from,
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(at) = ray.closest_along_line(self.end, self.direction) else {
            return false;
        };
        let wanted = self.from + at - self.grabbed;
        let rounded = if free {
            wanted
        } else {
            (wanted / self.step).round() * self.step
        };
        let value = self.measured.accepts(rounded, self.step);
        let changed = value != self.value;
        self.value = value;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.value != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let document: &Document = model.document();
        let caption = self.measured.caption();
        let mut kind = self.start.clone();
        let held = {
            let mut probe = self.start.clone();
            Held::of(document, feature, caption, self.measured.slot(&mut probe)?)
        };
        let mut named = Vec::new();
        held.set(
            self.measured.slot(&mut kind)?,
            model.units().length.measured(self.value),
            &mut named,
        );
        manipulator::keeping_names(document, feature, kind, named)
    }

    pub fn readout(&self, units: Units) -> String {
        format!(
            "{} {}",
            self.measured.caption(),
            units.readout_text(self.value)
        )
    }
}
