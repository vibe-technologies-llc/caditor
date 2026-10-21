use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Hole, Primitive, Transaction, displayed_plane,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, Color, Fill, Layer, View};
use caditor_sketch::Entity;

use crate::{
    feature_fields::POSITION_CAPTIONS,
    handle_snap::Snap,
    hole_on_curve, hole_placement, hole_tools,
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

pub fn square_around(
    plane: &Plane,
    centre: Point3,
    per_point: f64,
    forward: Vector3,
) -> Option<[Point3; 4]> {
    if plane.normal().dot(forward).abs() < EDGE_ON {
        return None;
    }
    let half = SQUARE_HALF_POINTS * per_point;
    let (x, y) = (plane.x_axis() * half, plane.y_axis() * half);
    Some([
        centre - x - y,
        centre + x - y,
        centre + x + y,
        centre - x + y,
    ])
}

pub fn square_edges(corners: [Point3; 4]) -> impl Iterator<Item = HandleSegment> {
    (0..corners.len()).filter_map(move |index| {
        Some((
            *corners.get(index)?,
            *corners.get((index + 1) % corners.len())?,
        ))
    })
}

pub fn square_holds(corners: &[Point3; 4], view: &View, cursor: Vector2) -> bool {
    let projected: Option<Vec<Vector2>> =
        corners.iter().map(|corner| view.project(*corner)).collect();
    projected.is_some_and(|projected| within(&projected, cursor))
}

pub fn draw_square(batch: &mut Batch, corners: &[Point3; 4], colour: Color) {
    batch.fills.push(Fill::convex(
        corners,
        colour.with_alpha(SQUARE_ALPHA),
        Layer::Front,
        None,
    ));
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
        let shown = view.is_drawn_at(depth) && per_point.is_finite() && per_point > 0.0;
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
        square_around(&self.plane, self.centre(), self.per_point, self.forward)
    }

    pub fn segments(&self) -> Vec<HandleSegment> {
        let arrows = ARROWS
            .into_iter()
            .filter_map(|grip| self.arrow(grip))
            .map(|(from, tip, _)| (from, tip));
        let edges = self.square().into_iter().flat_map(square_edges);
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
        let inside = self
            .square()
            .is_some_and(|corners| square_holds(&corners, view, cursor));
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
            draw_square(batch, &corners, colour(PlaceGrip::OnPlane));
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub direction: Vector2,
    pub value: f64,
    pub least: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slide {
    plane: Plane,
    grabbed: Point2,
    step: f64,
    from: Point2,
    at: Point2,
    line: Option<Line>,
    snapped: Option<Snap>,
}

impl Slide {
    pub fn begin(
        plane: Plane,
        ray: Ray,
        step: f64,
        from: Point2,
        line: Option<Line>,
    ) -> Option<Self> {
        let grabbed = plane.to_local(ray.at(ray.intersect_plane(&plane)?));
        Some(Self {
            plane,
            grabbed,
            step,
            from,
            at: from,
            line,
            snapped: None,
        })
    }

    pub fn plane(&self) -> &Plane {
        &self.plane
    }

    pub fn from(&self) -> Point2 {
        self.from
    }

    pub fn at(&self) -> Point2 {
        self.at
    }

    pub fn snapped(&self) -> Option<Snap> {
        self.snapped
    }

    pub fn value(&self) -> Option<f64> {
        self.line
            .map(|line| line.value + (self.at - self.from).dot(line.direction))
    }

    fn kept(&self, wanted: Point2) -> Point2 {
        match self.line {
            Some(line) => self.from + line.direction * (wanted - self.from).dot(line.direction),
            None => wanted,
        }
    }

    fn rounded(&self, wanted: Point2) -> Point2 {
        match self.line {
            Some(line) => {
                let value = line.value + (wanted - self.from).dot(line.direction);
                let rounded = (value / self.step).round() * self.step;
                self.from + line.direction * (rounded - line.value)
            }
            None => (wanted / self.step).round() * self.step,
        }
    }

    fn bounded(&self, placed: Point2) -> Point2 {
        match self.line {
            Some(Line {
                direction,
                value,
                least: Some(least),
            }) if value + (placed - self.from).dot(direction) < least => {
                self.from + direction * (least - value)
            }
            _ => placed,
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        if let Some((landed, snap)) = snap
            .filter(|_| !free)
            .and_then(|snap| Some((snap.on_plane(&self.plane)?, snap)))
        {
            let placed = self.bounded(self.kept(landed));
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
        let wanted = self.kept(self.from + (at - self.grabbed));
        let placed = if free { wanted } else { self.rounded(wanted) };
        let placed = self.bounded(placed);
        let changed = placed != self.at || unsnapped;
        self.at = placed;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.at != self.from
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
    slide: Slide,
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
        let line = match grip {
            PlaceGrip::AlongX => Some(Line {
                direction: Vector2::X,
                value: from.x,
                least: None,
            }),
            PlaceGrip::AlongY => Some(Line {
                direction: Vector2::Y,
                value: from.y,
                least: None,
            }),
            PlaceGrip::OnPlane => None,
        };
        let slide = Slide::begin(handles.plane, ray, handles.step(), from, line)?;
        Some(Self { start, slide })
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        self.slide.follow(ray, free, snap)
    }

    pub fn has_moved(&self) -> bool {
        self.slide.has_moved()
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let document = model.document();
        let (from, at) = (self.slide.from(), self.slide.at());
        match &self.start {
            Start::Hole(hole) => hole_tools::moved(document, feature, hole, at).ok(),
            Start::Curved(hole) => {
                hole_on_curve::moved_to(model, feature, hole, self.slide.plane().to_world(at)).ok()
            }
            Start::Primitive(primitive) => {
                let unit = model.units().length;
                let mut moved = primitive.clone();
                let mut named = Vec::new();
                for (index, (from, to)) in [(from.x, at.x), (from.y, at.y)].into_iter().enumerate()
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
        let at = self.slide.at();
        let shown = format!(
            "{x} {}, {y} {}",
            units.readout_text(at.x),
            units.readout_text(at.y)
        );
        match self.slide.snapped() {
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
        let point = hole_tools::lone_point(document, hole)?.entity;
        return hole_placement::positioned(model, feature, hole, point, typed).ok();
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
