use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Move, MoveAxis, Pivot, Transaction, TurnCentre,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{
    Plane, Point3, Ray, Rotation3, Vector2, Vector3, rotation_from_turns, turns_about_axes,
};
use caditor_render::{Batch, Color, Fill, Layer, Line, Stroke, View};

use crate::{
    manipulator::{self, Held},
    model::Model,
    move_panel,
    scene_palette::ScenePalette,
    selection::Axis,
    solid_panel,
    turn_handles::TurnEnd,
    units::Units,
};

pub const ARROW_POINTS: f64 = 96.0;
pub const GAP_POINTS: f64 = 14.0;
pub const HEAD_POINTS: f64 = 18.0;
pub const HEAD_HALF_POINTS: f64 = 7.0;
pub const SHAFT_WIDTH: f32 = 3.0;
const SQUARE_FROM: f64 = 0.3;
const SQUARE_TO: f64 = 0.5;
const SQUARE_ALPHA: f32 = 0.45;
pub const HIT_POINTS: f64 = 8.0;
pub const END_ON: f64 = 0.97;
const EDGE_ON: f64 = 0.2;
pub const STEP_POINTS: f64 = 4.0;
const RING_REACH: f64 = 1.25;
const RING_SEGMENTS: usize = 72;
const TURN_STEP_DEGREES: f64 = 5.0;
const UNCHANGED_DEGREES: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Along(MoveAxis),
    Across(MoveAxis),
    Turn(MoveAxis),
    TurnAbout,
    Reach(Reach),
    Place(PlaceGrip),
    Length,
    Revolve(TurnEnd),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceGrip {
    AlongX,
    AlongY,
    OnPlane,
}

impl PlaceGrip {
    pub fn words(self, noun: &str) -> String {
        match self {
            Self::AlongX => format!("Drag to move {noun} along its Position X"),
            Self::AlongY => format!("Drag to move {noun} along its Position Y"),
            Self::OnPlane => format!("Drag to move {noun} where it stands"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Only,
    Symmetric,
    Forward,
    Backward,
}

impl Reach {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Only | Self::Symmetric => solid_panel::DISTANCE,
            Self::Forward => solid_panel::FORWARD_DISTANCE,
            Self::Backward => solid_panel::BACKWARD_DISTANCE,
        }
    }

    pub fn field_caption(self) -> &'static str {
        match self {
            Self::Symmetric => solid_panel::TOTAL_DISTANCE,
            Self::Only | Self::Forward | Self::Backward => self.caption(),
        }
    }
}

impl Handle {
    const ALL: [Self; 6] = [
        Self::Across(MoveAxis::X),
        Self::Across(MoveAxis::Y),
        Self::Across(MoveAxis::Z),
        Self::Along(MoveAxis::X),
        Self::Along(MoveAxis::Y),
        Self::Along(MoveAxis::Z),
    ];

    fn moves(self) -> Vec<MoveAxis> {
        match self {
            Self::Along(axis) => vec![axis],
            Self::Across(normal) => MoveAxis::ALL
                .into_iter()
                .filter(|axis| *axis != normal)
                .collect(),
            Self::Turn(_)
            | Self::TurnAbout
            | Self::Reach(_)
            | Self::Place(_)
            | Self::Length
            | Self::Revolve(_) => Vec::new(),
        }
    }

    pub fn words(self) -> String {
        match self {
            Self::Along(axis) => format!("Drag to move the body along {}", axis.name()),
            Self::Turn(axis) => format!(
                "Drag to turn the body about {} through its centre",
                axis.name()
            ),
            Self::TurnAbout => "Drag to turn the body about its axis".to_owned(),
            Self::Reach(reach) => format!(
                "Drag to change the extrusion's {}",
                reach.caption().to_lowercase()
            ),
            Self::Across(_) => {
                let names: Vec<&str> = self.moves().iter().map(|axis| axis.name()).collect();
                format!("Drag to move the body in the {} plane", names.concat())
            }
            Self::Place(grip) => grip.words("it"),
            Self::Length => "Drag to change the distance".to_owned(),
            Self::Revolve(end) => end.words(),
        }
    }
}

fn axis_colour(palette: &ScenePalette, axis: MoveAxis) -> Color {
    palette.axis(match axis {
        MoveAxis::X => Axis::X,
        MoveAxis::Y => Axis::Y,
        MoveAxis::Z => Axis::Z,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Turning {
    Fixed,
    Axes,
    About(Vector3),
}

impl Turning {
    fn handles(self) -> Vec<Handle> {
        match self {
            Self::Fixed => Vec::new(),
            Self::Axes => MoveAxis::ALL.into_iter().map(Handle::Turn).collect(),
            Self::About(_) => vec![Handle::TurnAbout],
        }
    }

    fn about(self) -> Vector3 {
        match self {
            Self::About(direction) => direction,
            Self::Fixed | Self::Axes => Vector3::Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveHandles {
    pub feature: FeatureId,
    origin: Point3,
    reach: f64,
    forward: Vector3,
    turning: Turning,
    frame: Plane,
}

fn direction(frame: &Plane, axis: MoveAxis) -> Vector3 {
    axis.direction_in(Some(frame))
}

impl MoveHandles {
    pub fn of(
        model: &Model,
        open: Option<FeatureId>,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let feature = open?;
        let FeatureKind::Move(movement) = &model.document().feature(feature)?.kind else {
            return None;
        };
        let body = if movement.copy {
            feature
        } else {
            movement.body
        };
        let parameters = model.parameters();
        let frame = model.move_frame(movement)?;
        let shift = || {
            MoveAxis::ALL
                .into_iter()
                .try_fold(Vector3::ZERO, |shift, axis| {
                    let along = axis
                        .of(&movement.offset)
                        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
                        .ok()?;
                    Some(shift + direction(&frame, axis) * along)
                })
        };
        let (centre, turning) = match (&movement.about, model.move_pivot(feature, movement)) {
            (TurnCentre::Body, Some(pivot)) => (pivot.point() + shift()?, Turning::Axes),
            (TurnCentre::Axis(_), Some(Pivot::Axis(axis))) => {
                let before = model
                    .evaluation()
                    .body_seen_by(feature, movement.body)?
                    .bounding_box()?
                    .center();
                let along = (before - axis.origin()).dot(axis.direction());
                let foot = axis.origin() + axis.direction() * along;
                (foot + shift()?, Turning::About(axis.direction()))
            }
            _ => (
                model.evaluation().body(body)?.bounding_box()?.center(),
                Turning::Fixed,
            ),
        };
        let origin = match model.draft_placement() {
            Some((moved, placement)) if moved == body => placement.apply_point(centre),
            Some(_) | None => centre,
        };
        let depth = view.view_depth(origin);
        if !(depth.is_finite() && depth > 0.0) {
            return None;
        }
        let reach = ARROW_POINTS * pixels_per_point * view.units_per_pixel_at(depth);
        (reach.is_finite() && reach > 0.0).then_some(Self {
            feature,
            origin,
            reach,
            forward: view.forward(),
            turning,
            frame,
        })
    }

    fn per_point(&self) -> f64 {
        self.reach / ARROW_POINTS
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point())
    }

    fn direction(&self, axis: MoveAxis) -> Vector3 {
        direction(&self.frame, axis)
    }

    fn arrow(&self, axis: MoveAxis) -> Option<(Point3, Point3)> {
        let direction = self.direction(axis);
        (direction.dot(self.forward).abs() < END_ON).then(|| {
            (
                self.origin + direction * GAP_POINTS * self.per_point(),
                self.origin + direction * self.reach,
            )
        })
    }

    fn square(&self, normal: MoveAxis) -> Option<[Point3; 4]> {
        if self.direction(normal).dot(self.forward).abs() < EDGE_ON {
            return None;
        }
        let [first, second] = <[MoveAxis; 2]>::try_from(Handle::Across(normal).moves()).ok()?;
        let (first, second) = (self.direction(first), self.direction(second));
        let corner =
            |a: f64, b: f64| self.origin + first * a * self.reach + second * b * self.reach;
        Some([
            corner(SQUARE_FROM, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_FROM),
            corner(SQUARE_TO, SQUARE_TO),
            corner(SQUARE_FROM, SQUARE_TO),
        ])
    }

    fn ring(&self, handle: Handle) -> Option<Vec<Point3>> {
        let (normal, first, second) = match (handle, self.turning) {
            (Handle::Turn(axis), Turning::Axes) => {
                let [normal, first, second] = match axis {
                    MoveAxis::X => [MoveAxis::X, MoveAxis::Y, MoveAxis::Z],
                    MoveAxis::Y => [MoveAxis::Y, MoveAxis::Z, MoveAxis::X],
                    MoveAxis::Z => [MoveAxis::Z, MoveAxis::X, MoveAxis::Y],
                }
                .map(|axis| self.direction(axis));
                (normal, first, second)
            }
            (Handle::TurnAbout, Turning::About(direction)) => {
                let first = direction.any_orthonormal_vector();
                (direction, first, direction.cross(first))
            }
            _ => return None,
        };
        if normal.dot(self.forward).abs() < EDGE_ON {
            return None;
        }
        let radius = RING_REACH * self.reach;
        Some(
            (0..=RING_SEGMENTS)
                .map(|step| {
                    let angle = std::f64::consts::TAU * step as f64 / RING_SEGMENTS as f64;
                    self.origin + (first * angle.cos() + second * angle.sin()) * radius
                })
                .collect(),
        )
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let document = model.document();
        let FeatureKind::Move(movement) = &document.feature(self.feature)?.kind else {
            return None;
        };
        let held = |caption: &str, expression: &Expression| {
            Held::of(document, self.feature, caption, expression).driven()
        };
        match handle {
            Handle::Along(_) | Handle::Across(_) => handle.moves().into_iter().find_map(|axis| {
                held(
                    &move_panel::distance_caption(axis),
                    axis.of(&movement.offset),
                )
            }),
            Handle::Turn(turned) => {
                let alone = evaluated(model, &movement.turn, Dimension::ANGLE)
                    .is_some_and(|from| adds_alone(from, turned));
                MoveAxis::ALL
                    .into_iter()
                    .filter(|axis| !alone || *axis == turned)
                    .find_map(|axis| held(&move_panel::turn_caption(axis), axis.of(&movement.turn)))
            }
            Handle::TurnAbout => held(move_panel::ANGLE, &movement.about.axis_turn()?.angle),
            Handle::Reach(_) | Handle::Place(_) | Handle::Length | Handle::Revolve(_) => None,
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let inside = Handle::ALL.into_iter().find(|handle| match handle {
            Handle::Across(normal) => self.square(*normal).is_some_and(|corners| {
                let projected: Option<Vec<Vector2>> =
                    corners.iter().map(|corner| view.project(*corner)).collect();
                projected.is_some_and(|projected| within(&projected, cursor))
            }),
            Handle::Along(_)
            | Handle::Turn(_)
            | Handle::TurnAbout
            | Handle::Reach(_)
            | Handle::Place(_)
            | Handle::Length
            | Handle::Revolve(_) => false,
        });
        let reach = HIT_POINTS * pixels_per_point;
        let arrows = MoveAxis::ALL.into_iter().filter_map(|axis| {
            let (from, to) = self.arrow(axis)?;
            let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
            Some((distance, Handle::Along(axis)))
        });
        let rings = self.turning.handles().into_iter().filter_map(|handle| {
            let points: Option<Vec<Vector2>> = self
                .ring(handle)?
                .into_iter()
                .map(|point| view.project(point))
                .collect();
            let distance = points?
                .windows(2)
                .filter_map(|pair| match pair {
                    [from, to] => Some(segment_distance(*from, *to, cursor)),
                    _ => None,
                })
                .fold(f64::INFINITY, f64::min);
            Some((distance, handle))
        });
        let nearest = arrows
            .chain(rings)
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle);
        nearest.or(inside)
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        match handle {
            Handle::Along(axis) => {
                let (from, tip) = self.arrow(axis)?;
                Some(from.lerp(tip, 0.6) + self.direction(axis) * along)
            }
            Handle::Across(normal) => {
                let corners = self.square(normal)?;
                Some(corners[0].lerp(corners[2], 0.5))
            }
            Handle::Turn(_) | Handle::TurnAbout => {
                let ring = self.ring(handle)?;
                let index = (along.max(0.0) as usize).min(ring.len() - 1);
                ring.get(index).copied()
            }
            Handle::Reach(_) | Handle::Place(_) | Handle::Length | Handle::Revolve(_) => None,
        }
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        let manipulator = self;
        let colour = |handle: Handle, axis: MoveAxis| {
            if highlighted == Some(handle) {
                palette.handle_highlighted
            } else {
                axis_colour(palette, axis)
            }
        };
        for normal in MoveAxis::ALL {
            if let Some(corners) = manipulator.square(normal) {
                let handle = Handle::Across(normal);
                batch.fills.push(Fill::convex(
                    &corners,
                    colour(handle, normal).with_alpha(SQUARE_ALPHA),
                    Layer::Front,
                    None,
                ));
            }
        }
        for handle in manipulator.turning.handles() {
            let Some(ring) = manipulator.ring(handle) else {
                continue;
            };
            let colour = match handle {
                Handle::Turn(axis) => colour(handle, axis),
                _ if highlighted == Some(handle) => palette.handle_highlighted,
                _ => palette.handle,
            };
            for pair in ring.windows(2) {
                if let [start, end] = pair {
                    batch.lines.push(Line {
                        start: *start,
                        end: *end,
                        color: colour,
                        width: SHAFT_WIDTH,
                        layer: Layer::Front,
                        pick: None,
                        stroke: Stroke::Solid,
                    });
                }
            }
        }
        for axis in MoveAxis::ALL {
            let Some((from, tip)) = manipulator.arrow(axis) else {
                continue;
            };
            let handle = Handle::Along(axis);
            let colour = colour(handle, axis);
            let arrow = Arrow {
                from,
                tip,
                direction: manipulator.direction(axis),
                per_point: manipulator.per_point(),
                forward: manipulator.forward,
            };
            arrow.add_to(batch, colour);
        }
    }
}

pub struct Arrow {
    pub from: Point3,
    pub tip: Point3,
    pub direction: Vector3,
    pub per_point: f64,
    pub forward: Vector3,
}

impl Arrow {
    pub fn add_to(&self, batch: &mut Batch, colour: Color) {
        let base = self.tip - self.direction * HEAD_POINTS * self.per_point;
        batch.lines.push(Line {
            start: self.from,
            end: base,
            color: colour,
            width: SHAFT_WIDTH,
            layer: Layer::Front,
            pick: None,
            stroke: Stroke::Solid,
        });
        let Some(side) = self.direction.cross(self.forward).try_normalize() else {
            return;
        };
        let half = side * HEAD_HALF_POINTS * self.per_point;
        batch.fills.push(Fill::convex(
            &[self.tip, base + half, base - half],
            colour,
            Layer::Front,
            None,
        ));
    }
}

pub fn step_for(per_point: f64) -> f64 {
    nice_step(per_point * STEP_POINTS)
}

pub fn within(polygon: &[Vector2], point: Vector2) -> bool {
    let turns: Vec<f64> = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| (*b - *a).perp_dot(point - *a))
        .collect();
    turns.iter().all(|turn| *turn >= 0.0) || turns.iter().all(|turn| *turn <= 0.0)
}

pub fn segment_distance(from: Vector2, to: Vector2, point: Vector2) -> f64 {
    let along = to - from;
    let length_squared = along.length_squared();
    if length_squared == 0.0 {
        return from.distance(point);
    }
    let fraction = ((point - from).dot(along) / length_squared).clamp(0.0, 1.0);
    (from + along * fraction).distance(point)
}

fn nice_step(at_least: f64) -> f64 {
    if !(at_least.is_finite() && at_least > 0.0) {
        return 1.0;
    }
    let decade = 10.0_f64.powf(at_least.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|factor| factor * decade)
        .find(|step| *step >= at_least)
        .unwrap_or(10.0 * decade)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MoveDrag {
    pub feature: FeatureId,
    pub handle: Handle,
    start: Move,
    from: [f64; 3],
    origin: Point3,
    grabbed: Point3,
    step: f64,
    offset: [f64; 3],
    from_turns: [f64; 3],
    turns: [f64; 3],
    about: Vector3,
    from_angle: f64,
    angle: f64,
    frame: Plane,
}

impl MoveDrag {
    pub fn begin(
        model: &Model,
        manipulator: &MoveHandles,
        handle: Handle,
        ray: Ray,
    ) -> Option<Self> {
        let FeatureKind::Move(start) = &model.document().feature(manipulator.feature)?.kind else {
            return None;
        };
        let parameters = model.parameters();
        let from = evaluated(model, &start.offset, Dimension::LENGTH)?;
        let from_turns = evaluated(model, &start.turn, Dimension::ANGLE)?;
        let from_angle = match start.about.axis_turn() {
            Some(turn) => turn
                .angle
                .evaluate_as(Dimension::ANGLE, &|id| parameters.value(id))
                .ok()?,
            None => 0.0,
        };
        let about = manipulator.turning.about();
        let grabbed = point_on(handle, &manipulator.frame, manipulator.origin, about, ray)?;
        Some(Self {
            feature: manipulator.feature,
            handle,
            start: start.clone(),
            from,
            origin: manipulator.origin,
            grabbed,
            step: manipulator.step(),
            offset: from,
            from_turns,
            turns: from_turns,
            about,
            from_angle,
            angle: from_angle,
            frame: manipulator.frame,
        })
    }

    fn swept(&self, direction: Vector3, at: Point3, free: bool) -> f64 {
        let (grabbed, now) = (self.grabbed - self.origin, at - self.origin);
        let angle = grabbed
            .cross(now)
            .dot(direction)
            .atan2(grabbed.dot(now))
            .to_degrees();
        if free {
            angle
        } else {
            (angle / TURN_STEP_DEGREES).round() * TURN_STEP_DEGREES
        }
    }

    pub fn follow(&mut self, ray: Ray, free: bool) -> bool {
        let Some(at) = point_on(self.handle, &self.frame, self.origin, self.about, ray) else {
            return false;
        };
        if self.handle == Handle::TurnAbout {
            let angle = self.from_angle + self.swept(self.about, at, free);
            let changed = angle != self.angle;
            self.angle = angle;
            return changed;
        }
        if let Handle::Turn(axis) = self.handle {
            let angle = self.swept(direction(&self.frame, axis), at, free);
            let turns = turned(self.from_turns, axis, angle);
            let changed = turns != self.turns;
            self.turns = turns;
            return changed;
        }
        let moved = at - self.grabbed;
        let mut offset = self.from;
        for axis in self.handle.moves() {
            let along = moved.dot(direction(&self.frame, axis));
            let rounded = if free {
                along
            } else {
                (along / self.step).round() * self.step
            };
            if let Some(value) = offset.get_mut(axis.index()) {
                *value += rounded;
            }
        }
        let changed = offset != self.offset;
        self.offset = offset;
        changed
    }

    pub fn has_moved(&self) -> bool {
        self.offset != self.from || self.turns != self.from_turns || self.angle != self.from_angle
    }

    fn changes(&self, document: &Document, units: Units) -> (Move, Vec<Edit>) {
        let held = |caption: &str, expression: &Expression| {
            Held::of(document, self.feature, caption, expression)
        };
        let mut movement = self.start.clone();
        let mut named = Vec::new();
        for axis in self.handle.moves() {
            if let Some(value) = self.offset.get(axis.index()) {
                let slot = axis.of_mut(&mut movement.offset);
                held(&move_panel::distance_caption(axis), slot).set(
                    slot,
                    units.length.measured(*value),
                    &mut named,
                );
            }
        }
        for axis in MoveAxis::ALL {
            let (now, before) = (*axis.of(&self.turns), *axis.of(&self.from_turns));
            if (now - before).abs() > UNCHANGED_DEGREES {
                let slot = axis.of_mut(&mut movement.turn);
                held(&move_panel::turn_caption(axis), slot).set(
                    slot,
                    units.angle.measured(now),
                    &mut named,
                );
            }
        }
        if let TurnCentre::Axis(turn) = &mut movement.about
            && (self.angle - self.from_angle).abs() > UNCHANGED_DEGREES
        {
            held(move_panel::ANGLE, &turn.angle).set(
                &mut turn.angle,
                units.angle.measured(self.angle),
                &mut named,
            );
        }
        (movement, named)
    }

    pub fn transaction(&self, model: &Model) -> Option<Transaction> {
        let (movement, named) = self.changes(model.document(), model.units());
        manipulator::keeping_names(
            model.document(),
            self.feature,
            FeatureKind::Move(movement),
            named,
        )
    }

    pub fn readout(&self, units: Units) -> String {
        if self.handle == Handle::TurnAbout {
            let turned = self.angle - self.from_angle;
            let shown = if turned >= 0.0 { "+" } else { "" };
            return format!(
                "Turn about its axis {shown}{}",
                units.angle.readout_text(turned)
            );
        }
        if let Handle::Turn(axis) = self.handle {
            let start = rotation_from_turns(self.from_turns.map(f64::to_radians));
            let now = rotation_from_turns(self.turns.map(f64::to_radians));
            let (about, angle) = (now * start.inverse()).to_axis_angle();
            let sign = if about.dot(axis.direction()) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            let turned = sign * angle.to_degrees();
            let shown = if turned >= 0.0 { "+" } else { "" };
            return format!(
                "Turn about {} {shown}{}",
                axis.name(),
                units.angle.readout_text(turned)
            );
        }
        self.handle
            .moves()
            .into_iter()
            .filter_map(|axis| {
                let index = axis.index();
                let moved = self.offset.get(index)? - self.from.get(index)?;
                let sign = if moved >= 0.0 { "+" } else { "" };
                Some(format!(
                    "{} {sign}{}",
                    axis.name(),
                    units.readout_text(moved)
                ))
            })
            .collect::<Vec<_>>()
            .join("   ")
    }
}

fn evaluated(model: &Model, values: &[Expression; 3], dimension: Dimension) -> Option<[f64; 3]> {
    let parameters = model.parameters();
    let evaluated: Option<Vec<f64>> = values
        .iter()
        .map(|value| {
            value
                .evaluate_as(dimension, &|id| parameters.value(id))
                .ok()
        })
        .collect();
    <[f64; 3]>::try_from(evaluated?).ok()
}

fn adds_alone(from: [f64; 3], axis: MoveAxis) -> bool {
    let applied_after: &[MoveAxis] = match axis {
        MoveAxis::X => &[MoveAxis::Y, MoveAxis::Z],
        MoveAxis::Y => &[MoveAxis::Z],
        MoveAxis::Z => &[],
    };
    applied_after.iter().all(|later| *later.of(&from) == 0.0)
}

fn turned(from: [f64; 3], axis: MoveAxis, degrees: f64) -> [f64; 3] {
    if adds_alone(from, axis) {
        let mut turns = from;
        *axis.of_mut(&mut turns) += degrees;
        return turns;
    }
    let start = rotation_from_turns(from.map(f64::to_radians));
    let turn = Rotation3::from_axis_angle(axis.direction(), degrees.to_radians());
    turns_about_axes(turn * start).map(f64::to_degrees)
}

fn point_on(
    handle: Handle,
    frame: &Plane,
    origin: Point3,
    about: Vector3,
    ray: Ray,
) -> Option<Point3> {
    match handle {
        Handle::TurnAbout => {
            let plane = Plane::new(origin, about)?;
            Some(ray.at(ray.intersect_plane(&plane)?))
        }
        Handle::Along(axis) => {
            let along = ray.closest_along_line(origin, direction(frame, axis))?;
            Some(origin + direction(frame, axis) * along)
        }
        Handle::Turn(axis) | Handle::Across(axis) => {
            let plane = Plane::new(origin, direction(frame, axis))?;
            Some(ray.at(ray.intersect_plane(&plane)?))
        }
        Handle::Reach(_) | Handle::Place(_) | Handle::Length | Handle::Revolve(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use caditor_render::Viewpoint;

    use super::*;

    fn looking_down_at_the_origin() -> View {
        let viewpoint =
            Viewpoint::looking_from(Vector3::new(1.0, -1.0, 1.0), Point3::ZERO, 200.0).unwrap();
        View::new(viewpoint, 800.0, 600.0)
    }

    fn manipulator(view: &View) -> MoveHandles {
        let depth = view.view_depth(Point3::ZERO);
        MoveHandles {
            feature: FeatureId::from_raw(3),
            origin: Point3::ZERO,
            reach: ARROW_POINTS * view.units_per_pixel_at(depth),
            forward: view.forward(),
            turning: Turning::Axes,
            frame: Plane::XY,
        }
    }

    #[test]
    fn the_arrows_and_squares_are_hit_where_they_are_drawn() {
        let view = looking_down_at_the_origin();
        let manipulator = manipulator(&view);

        for axis in MoveAxis::ALL {
            let (from, tip) = manipulator.arrow(axis).unwrap();
            let middle = view.project(from.lerp(tip, 0.5)).unwrap();
            assert_eq!(
                manipulator.hit(&view, middle, 1.0),
                Some(Handle::Along(axis))
            );
        }
        let square = manipulator.square(MoveAxis::Z).unwrap();
        let centre = view.project(square[0].lerp(square[2], 0.5)).unwrap();
        assert_eq!(
            manipulator.hit(&view, centre, 1.0),
            Some(Handle::Across(MoveAxis::Z))
        );
        let away = view.project(Point3::new(-200.0, 200.0, 0.0)).unwrap();
        assert_eq!(manipulator.hit(&view, away, 1.0), None);
    }

    #[test]
    fn an_arrow_pointing_at_the_eye_is_left_out() {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap();
        let view = View::new(viewpoint, 800.0, 600.0);
        let manipulator = manipulator(&view);

        assert_eq!(manipulator.arrow(MoveAxis::Z), None);
        assert!(manipulator.arrow(MoveAxis::X).is_some());
        assert!(manipulator.square(MoveAxis::Z).is_some());
        assert_eq!(manipulator.square(MoveAxis::X), None);
    }

    #[test]
    fn steps_are_one_two_or_five_of_a_power_of_ten() {
        assert_eq!(nice_step(0.3), 0.5);
        assert_eq!(nice_step(1.0), 1.0);
        assert_eq!(nice_step(1.2), 2.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert!((nice_step(0.013) - 0.02).abs() < 1e-12);
    }

    #[test]
    fn a_point_on_a_handle_follows_the_ray_along_its_axis_or_plane() {
        let ray = Ray::new(Point3::new(7.0, 3.0, 50.0), Vector3::NEG_Z).unwrap();

        let along = point_on(
            Handle::Along(MoveAxis::X),
            &Plane::XY,
            Point3::ZERO,
            Vector3::Z,
            ray,
        )
        .unwrap();
        assert!(along.distance(Point3::new(7.0, 0.0, 0.0)) < 1e-9);
        let across = point_on(
            Handle::Across(MoveAxis::Z),
            &Plane::XY,
            Point3::ZERO,
            Vector3::Z,
            ray,
        )
        .unwrap();
        assert!(across.distance(Point3::new(7.0, 3.0, 0.0)) < 1e-9);
    }

    #[test]
    fn the_words_name_the_axis_or_plane() {
        assert_eq!(
            Handle::Along(MoveAxis::Y).words(),
            "Drag to move the body along Y"
        );
        assert_eq!(
            Handle::Across(MoveAxis::Y).words(),
            "Drag to move the body in the XZ plane"
        );
    }

    #[test]
    fn a_turn_adds_to_its_own_field_when_none_is_applied_after_it_and_composes_otherwise() {
        assert_eq!(
            turned([10.0, 20.0, 30.0], MoveAxis::Z, 15.0),
            [10.0, 20.0, 45.0]
        );
        assert_eq!(
            turned([10.0, 20.0, 0.0], MoveAxis::Y, -5.0),
            [10.0, 15.0, 0.0]
        );

        let from = [10.0, 20.0, 30.0];
        let composed = turned(from, MoveAxis::X, 25.0);

        let expected = Rotation3::from_axis_angle(Vector3::X, 25.0_f64.to_radians())
            * rotation_from_turns(from.map(f64::to_radians));
        let found = rotation_from_turns(composed.map(f64::to_radians));
        let point = Vector3::new(1.0, -2.0, 3.0);
        assert!((found * point).distance(expected * point) < 1e-9);
    }
}
