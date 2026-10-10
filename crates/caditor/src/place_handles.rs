use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Hole, Primitive, Transaction, displayed_plane,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, Fill, Layer, View};
use caditor_sketch::Entity;

use crate::{
    feature_fields::POSITION_CAPTIONS,
    handle_snap::Snap,
    hole_on_curve, hole_tools,
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, Handle, HandleSegment, PlaceGrip,
        segment_distance, step_for, within,
    },
    scene,
    scene_palette::ScenePalette,
    units::Units,
};

const ARROW_SHARE: f64 = 0.7;
const SQUARE_HALF_POINTS: f64 = 9.0;
const SQUARE_ALPHA: f32 = 0.55;
const EDGE_ON: f64 = 0.2;
const ARROWS: [PlaceGrip; 2] = [PlaceGrip::AlongX, PlaceGrip::AlongY];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    Hole,
    Curved,
    Primitive,
}

impl Subject {
    fn noun(self) -> &'static str {
        match self {
            Self::Hole | Self::Curved => "the hole",
            Self::Primitive => "the shape",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaceHandles {
    pub feature: FeatureId,
    subject: Subject,
    plane: Plane,
    at: Point2,
    per_point: f64,
    forward: Vector3,
}

fn committed_hole(document: &Document, feature: FeatureId) -> Option<&Hole> {
    document.feature(feature)?.kind.hole()
}

fn committed_primitive(document: &Document, feature: FeatureId) -> Option<&Primitive> {
    document.feature(feature)?.kind.primitive()
}

fn previewed_point(model: &Model, feature: FeatureId, hole: &Hole) -> Option<Point2> {
    model
        .draft_transaction(feature)?
        .edits()
        .iter()
        .find_map(|edit| match edit {
            Edit::SetSketchEntity {
                feature: sketch,
                entity: Entity::Point(at),
                ..
            } if *sketch == hole.sketch => Some(*at),
            _ => None,
        })
}

fn primitive_at(model: &Model, feature: FeatureId, primitive: &Primitive) -> Option<Point2> {
    let parameters = model.shown_parameters(feature);
    let [x, y] = primitive.at.each_ref().map(|expression| {
        expression
            .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
            .ok()
    });
    Some(Point2::new(x?, y?))
}

fn shown_primitive(model: &Model, feature: FeatureId) -> Option<&Primitive> {
    match model.draft_kind(feature) {
        Some(FeatureKind::Primitive(primitive)) => Some(primitive),
        _ => committed_primitive(model.document(), feature),
    }
}

fn held(document: &Document, feature: FeatureId, at: &[Expression; 2], index: usize) -> Held {
    match (POSITION_CAPTIONS.get(index), at.get(index)) {
        (Some(caption), Some(expression)) => Held::of(document, feature, caption, expression),
        _ => Held::Typed,
    }
}

fn axes(grip: PlaceGrip) -> &'static [usize] {
    match grip {
        PlaceGrip::AlongX => &[0],
        PlaceGrip::AlongY => &[1],
        PlaceGrip::OnPlane => &[0, 1],
    }
}

impl PlaceHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let document = model.document();
        let (subject, plane, at) = if let Some(hole) = committed_hole(document, feature) {
            if hole_on_curve::mount(document, hole).is_some() {
                let evaluation = model
                    .draft_evaluation_of(feature)
                    .unwrap_or_else(|| model.evaluation());
                let plane = scene::sketch_plane(document, evaluation, hole.sketch)?;
                return Self::placed(
                    view,
                    pixels_per_point,
                    feature,
                    Subject::Curved,
                    plane,
                    Point2::ZERO,
                );
            }
            let lone = hole_tools::lone_point(document, hole)?;
            let plane = scene::sketch_plane(document, model.evaluation(), hole.sketch)?;
            let at = previewed_point(model, feature, hole).unwrap_or(lone.at);
            (Subject::Hole, plane, at)
        } else {
            let primitive = shown_primitive(model, feature)?;
            let plane = displayed_plane(model.evaluation(), feature, &primitive.plane)?;
            let at = primitive_at(model, feature, primitive)?;
            (Subject::Primitive, plane, at)
        };
        Self::placed(view, pixels_per_point, feature, subject, plane, at)
    }

    fn placed(
        view: &View,
        pixels_per_point: f64,
        feature: FeatureId,
        subject: Subject,
        plane: Plane,
        at: Point2,
    ) -> Option<Self> {
        let centre = plane.to_world(at);
        let depth = view.view_depth(centre);
        let per_point = pixels_per_point * view.units_per_pixel_at(depth);
        let shown = depth.is_finite() && depth > 0.0 && per_point.is_finite() && per_point > 0.0;
        shown.then_some(Self {
            feature,
            subject,
            plane,
            at,
            per_point,
            forward: view.forward(),
        })
    }

    fn centre(&self) -> Point3 {
        self.plane.to_world(self.at)
    }

    fn direction(&self, grip: PlaceGrip) -> Option<Vector3> {
        match grip {
            PlaceGrip::AlongX => Some(self.plane.x_axis()),
            PlaceGrip::AlongY => Some(self.plane.y_axis()),
            PlaceGrip::OnPlane => None,
        }
    }

    fn arrow(&self, grip: PlaceGrip) -> Option<(Point3, Point3, Vector3)> {
        let direction = self.direction(grip)?;
        (direction.dot(self.forward).abs() < END_ON).then(|| {
            let centre = self.centre();
            (
                centre + direction * GAP_POINTS * self.per_point,
                centre + direction * ARROW_POINTS * ARROW_SHARE * self.per_point,
                direction,
            )
        })
    }

    fn square(&self) -> Option<[Point3; 4]> {
        if self.plane.normal().dot(self.forward).abs() < EDGE_ON {
            return None;
        }
        let half = SQUARE_HALF_POINTS * self.per_point;
        let (x, y) = (self.plane.x_axis() * half, self.plane.y_axis() * half);
        let centre = self.centre();
        Some([
            centre - x - y,
            centre + x - y,
            centre + x + y,
            centre - x + y,
        ])
    }

    pub fn segments(&self) -> Vec<HandleSegment> {
        let arrows = ARROWS
            .into_iter()
            .filter_map(|grip| self.arrow(grip))
            .map(|(from, tip, _)| (from, tip));
        let edges = self.square().into_iter().flat_map(|corners| {
            (0..corners.len()).filter_map(move |index| {
                Some((
                    *corners.get(index)?,
                    *corners.get((index + 1) % corners.len())?,
                ))
            })
        });
        arrows.chain(edges).collect()
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point)
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Place(grip) = handle else {
            return None;
        };
        let document = model.document();
        let primitive = match self.subject {
            Subject::Hole | Subject::Curved => return None,
            Subject::Primitive => committed_primitive(document, self.feature)?,
        };
        axes(grip)
            .iter()
            .find_map(|index| held(document, self.feature, &primitive.at, *index).driven())
    }

    pub fn words(&self, handle: Handle) -> String {
        match handle {
            Handle::Place(PlaceGrip::OnPlane)
                if matches!(self.subject, Subject::Hole | Subject::Curved) =>
            {
                "Drag to move the hole on its face".to_owned()
            }
            Handle::Place(grip) => grip.words(self.subject.noun()),
            _ => handle.words(),
        }
    }

    pub fn typing(&self, grip: PlaceGrip) -> Option<(String, usize)> {
        let [x, y] = POSITION_CAPTIONS;
        match (self.subject, grip) {
            (Subject::Curved, _) => None,
            (Subject::Hole | Subject::Primitive, PlaceGrip::AlongX) => Some((x.to_owned(), 1)),
            (Subject::Hole | Subject::Primitive, PlaceGrip::AlongY) => Some((y.to_owned(), 1)),
            (Subject::Hole | Subject::Primitive, PlaceGrip::OnPlane) => {
                Some((format!("{x}, {y}"), 2))
            }
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        let arrow = ARROWS
            .into_iter()
            .filter_map(|grip| {
                let (from, to, _) = self.arrow(grip)?;
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Place(grip)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle);
        let inside = self.square().is_some_and(|corners| {
            let projected: Option<Vec<Vector2>> =
                corners.iter().map(|corner| view.project(*corner)).collect();
            projected.is_some_and(|projected| within(&projected, cursor))
        });
        arrow.or_else(|| inside.then_some(Handle::Place(PlaceGrip::OnPlane)))
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        let Handle::Place(grip) = handle else {
            return None;
        };
        match self.arrow(grip) {
            Some((from, tip, direction)) => Some(from.lerp(tip, 0.6) + direction * along),
            None => Some(self.centre() + self.plane.x_axis() * along),
        }
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        let colour = |grip: PlaceGrip| {
            if highlighted == Some(Handle::Place(grip)) {
                palette.handle_highlighted
            } else {
                palette.handle
            }
        };
        if let Some(corners) = self.square() {
            batch.fills.push(Fill::convex(
                &corners,
                colour(PlaceGrip::OnPlane).with_alpha(SQUARE_ALPHA),
                Layer::Front,
                None,
            ));
        }
        for grip in ARROWS {
            let Some((from, tip, direction)) = self.arrow(grip) else {
                continue;
            };
            Arrow {
                from,
                tip,
                direction,
                per_point: self.per_point,
                forward: self.forward,
            }
            .add_to(batch, colour(grip));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Start {
    Hole(Hole),
    Curved(Hole),
    Primitive(Primitive),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceDrag {
    start: Start,
    grip: PlaceGrip,
    plane: Plane,
    grabbed: Point2,
    step: f64,
    from: Point2,
    at: Point2,
    snapped: Option<Snap>,
}

impl PlaceDrag {
    pub fn begin(model: &Model, handles: &PlaceHandles, grip: PlaceGrip, ray: Ray) -> Option<Self> {
        let document = model.document();
        let (start, from) = match handles.subject {
            Subject::Hole => {
                let hole = committed_hole(document, handles.feature)?;
                let from = hole_tools::lone_point(document, hole)?.at;
                (Start::Hole(hole.clone()), from)
            }
            Subject::Curved => (
                Start::Curved(committed_hole(document, handles.feature)?.clone()),
                Point2::ZERO,
            ),
            Subject::Primitive => {
                let primitive = committed_primitive(document, handles.feature)?;
                let from = primitive_at(model, handles.feature, primitive)?;
                (Start::Primitive(primitive.clone()), from)
            }
        };
        let plane = handles.plane;
        let grabbed = plane.to_local(ray.at(ray.intersect_plane(&plane)?));
        Some(Self {
            start,
            grip,
            plane,
            grabbed,
            step: handles.step(),
            from,
            at: from,
            snapped: None,
        })
    }

    fn kept(&self, wanted: Point2) -> Point2 {
        match self.grip {
            PlaceGrip::AlongX => Point2::new(wanted.x, self.from.y),
            PlaceGrip::AlongY => Point2::new(self.from.x, wanted.y),
            PlaceGrip::OnPlane => wanted,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        if let Some((landed, snap)) = snap
            .filter(|_| !free)
            .and_then(|snap| Some((snap.on_plane(&self.plane)?, snap)))
        {
            let placed = self.kept(landed);
            let changed = placed != self.at || self.snapped != Some(snap);
            self.at = placed;
            self.snapped = Some(snap);
            return changed;
        }
        let unsnapped = self.snapped.take().is_some();
        let Some(at) = ray
            .intersect_plane(&self.plane)
            .map(|distance| self.plane.to_local(ray.at(distance)))
        else {
            return unsnapped;
        };
        let pulled = at - self.grabbed;
        let pulled = match self.grip {
            PlaceGrip::AlongX => Vector2::new(pulled.x, 0.0),
            PlaceGrip::AlongY => Vector2::new(0.0, pulled.y),
            PlaceGrip::OnPlane => pulled,
        };
        let wanted = self.from + pulled;
        let placed = if free {
            wanted
        } else {
            self.kept((wanted / self.step).round() * self.step)
        };
        let changed = placed != self.at || unsnapped;
        self.at = placed;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.at != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let document = model.document();
        match &self.start {
            Start::Hole(hole) => hole_tools::moved(document, feature, hole, self.at).ok(),
            Start::Curved(hole) => {
                hole_on_curve::moved_to(model, feature, hole, self.plane.to_world(self.at)).ok()
            }
            Start::Primitive(primitive) => {
                let unit = model.units().length;
                let mut moved = primitive.clone();
                let mut named = Vec::new();
                for (index, (from, to)) in [(self.from.x, self.at.x), (self.from.y, self.at.y)]
                    .into_iter()
                    .enumerate()
                {
                    if from == to {
                        continue;
                    }
                    let keeper = held(document, feature, &primitive.at, index);
                    if let Some(slot) = moved.at.get_mut(index) {
                        keeper.set(slot, unit.measured(to), &mut named);
                    }
                }
                manipulator::keeping_names(document, feature, FeatureKind::Primitive(moved), named)
            }
        }
    }

    pub fn readout(&self, units: Units) -> String {
        let [x, y] = POSITION_CAPTIONS;
        let shown = format!(
            "{x} {}, {y} {}",
            units.readout_text(self.at.x),
            units.readout_text(self.at.y)
        );
        match self.snapped {
            Some(snap) => format!("{shown} {}", snap.kind.words()),
            None => shown,
        }
    }
}

fn axes_typed(grip: PlaceGrip, values: &[Expression]) -> [Option<&Expression>; 2] {
    match grip {
        PlaceGrip::AlongX => [values.first(), None],
        PlaceGrip::AlongY => [None, values.first()],
        PlaceGrip::OnPlane => [values.first(), values.get(1)],
    }
}

pub fn typed(
    model: &Model,
    feature: FeatureId,
    grip: PlaceGrip,
    values: &[Expression],
) -> Option<Transaction> {
    let document = model.document();
    let typed = axes_typed(grip, values);
    if let Some(hole) = committed_hole(document, feature) {
        if hole_on_curve::mount(document, hole).is_some() {
            return None;
        }
        let from = hole_tools::lone_point(document, hole)?.at;
        let parameters = model.parameters();
        let [typed_x, typed_y] = typed;
        let [x, y] = [(typed_x, from.x), (typed_y, from.y)].map(|(value, kept)| {
            value.map_or(Some(kept), |value| {
                value
                    .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
                    .ok()
            })
        });
        return hole_tools::moved(document, feature, hole, Point2::new(x?, y?)).ok();
    }
    let primitive = committed_primitive(document, feature)?;
    let mut moved = primitive.clone();
    let mut named = Vec::new();
    for (index, value) in typed.into_iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        let keeper = held(document, feature, &primitive.at, index);
        if let Some(slot) = moved.at.get_mut(index) {
            keeper.set(slot, value.clone(), &mut named);
        }
    }
    manipulator::keeping_names(document, feature, FeatureKind::Primitive(moved), named)
}
