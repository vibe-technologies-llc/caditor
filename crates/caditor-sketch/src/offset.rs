use std::{
    collections::BTreeSet,
    f64::consts::{PI, TAU},
};

use caditor_expression::Expression;
use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{ArcGeometry, Faceting, direction_angle},
    entity::Entity,
    id::EntityId,
    intersect::{self, Carrier, Shape},
    sketch::{Sketch, SketchError},
};

const TOLERANCE: f64 = 1e-7;
const SMOOTH_ANGLE: f64 = 1e-6;
const COLLAPSE_FRACTION: f64 = 1e-6;
const SIDE_SEGMENT_ANGLE: f64 = PI / 90.0;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum OffsetError {
    #[error("select the lines, arcs or circles to offset")]
    NothingSelected,
    #[error("{label} cannot be offset; only lines, arcs and circles can")]
    NotOffsettable { entity: EntityId, label: String },
    #[error(
        "{label} cannot be offset: the curve at one distance from an ellipse is no ellipse, so \
         nothing would keep it there; offset lines, arcs and circles"
    )]
    EllipseNotOffsettable { entity: EntityId, label: String },
    #[error("{label} has no length to offset")]
    NoLength { entity: EntityId, label: String },
    #[error("three or more of the selected curves meet at one point; offset one chain at a time")]
    Branches,
    #[error("the selected curves form {count} separate chains; offset one chain at a time")]
    SeveralChains { count: usize },
    #[error("the distance must be greater than zero")]
    NotPositive,
    #[error("offsetting {label} this far would shrink it to nothing")]
    Collapses { entity: EntityId, label: String },
    #[error("offsetting this far leaves nothing of {label}")]
    UsedUp { entity: EntityId, label: String },
    #[error("the offsets of {first} and {second} would not meet at their corner")]
    Apart { first: String, second: String },
    #[error("the offset would cross itself at this distance")]
    CrossesItself,
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn sign(self) -> f64 {
        match self {
            Self::Left => 1.0,
            Self::Right => -1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Course {
    Line {
        start: Point2,
        end: Point2,
    },
    Arc {
        center: Point2,
        radius: f64,
        from: f64,
        sweep: f64,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
}

impl Course {
    fn start(&self) -> Point2 {
        match *self {
            Self::Line { start, .. } => start,
            Self::Arc {
                center,
                radius,
                from,
                ..
            } => center + Vector2::from_angle(from) * radius,
            Self::Circle { center, radius } => center + Vector2::X * radius,
        }
    }

    fn end(&self) -> Point2 {
        match *self {
            Self::Line { end, .. } => end,
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => center + Vector2::from_angle(from + sweep) * radius,
            Self::Circle { .. } => self.start(),
        }
    }

    fn start_tangent(&self) -> Vector2 {
        match *self {
            Self::Line { start, end } => (end - start).normalize_or_zero(),
            Self::Arc { from, sweep, .. } => Vector2::from_angle(from).perp() * sweep.signum(),
            Self::Circle { .. } => Vector2::Y,
        }
    }

    fn end_tangent(&self) -> Vector2 {
        match *self {
            Self::Line { .. } | Self::Circle { .. } => self.start_tangent(),
            Self::Arc { from, sweep, .. } => {
                Vector2::from_angle(from + sweep).perp() * sweep.signum()
            }
        }
    }

    fn center(&self) -> Option<Point2> {
        match *self {
            Self::Line { .. } => None,
            Self::Arc { center, .. } | Self::Circle { center, .. } => Some(center),
        }
    }

    fn is_line(&self) -> bool {
        matches!(self, Self::Line { .. })
    }

    fn offset(&self, by: f64) -> Option<Self> {
        let shrunk = |radius: f64, towards_center: f64| {
            let offset = radius - towards_center;
            (offset > radius * COLLAPSE_FRACTION).then_some(offset)
        };
        match *self {
            Self::Line { start, end } => {
                let normal = self.start_tangent().perp() * by;
                Some(Self::Line {
                    start: start + normal,
                    end: end + normal,
                })
            }
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => Some(Self::Arc {
                center,
                radius: shrunk(radius, by * sweep.signum())?,
                from,
                sweep,
            }),
            Self::Circle { center, radius } => Some(Self::Circle {
                center,
                radius: shrunk(radius, by)?,
            }),
        }
    }

    fn with_start(self, point: Point2) -> Self {
        match self {
            Self::Line { end, .. } => Self::Line { start: point, end },
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let moved = from + wrapped(direction_angle(point - center) - from);
                Self::Arc {
                    center,
                    radius,
                    from: moved,
                    sweep: from + sweep - moved,
                }
            }
            Self::Circle { .. } => self,
        }
    }

    fn with_end(self, point: Point2) -> Self {
        match self {
            Self::Line { start, .. } => Self::Line { start, end: point },
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let end = from + sweep;
                let moved = end + wrapped(direction_angle(point - center) - end);
                Self::Arc {
                    center,
                    radius,
                    from,
                    sweep: moved - from,
                }
            }
            Self::Circle { .. } => self,
        }
    }

    fn keeps_course_of(&self, untrimmed: &Self, tolerance: f64) -> bool {
        match (*self, *untrimmed) {
            (Self::Line { start, end }, Self::Line { .. }) => {
                (end - start).dot(untrimmed.start_tangent()) > tolerance
            }
            (Self::Arc { radius, sweep, .. }, Self::Arc { sweep: whole, .. }) => {
                sweep * whole.signum() * radius > tolerance
            }
            _ => true,
        }
    }

    fn carrier(&self) -> Option<Carrier> {
        match *self {
            Self::Line { start, end } => Some(Carrier::Line {
                through: start,
                direction: (end - start).try_normalize()?,
            }),
            Self::Arc { center, radius, .. } | Self::Circle { center, radius } => {
                Some(Carrier::Circle { center, radius })
            }
        }
    }

    fn arc_geometry(&self) -> Option<ArcGeometry> {
        match *self {
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => Some(if sweep >= 0.0 {
                ArcGeometry {
                    center,
                    radius,
                    start_angle: from,
                    sweep,
                }
            } else {
                ArcGeometry {
                    center,
                    radius,
                    start_angle: from + sweep,
                    sweep: -sweep,
                }
            }),
            Self::Line { .. } | Self::Circle { .. } => None,
        }
    }

    fn shape(&self) -> Shape {
        match *self {
            Self::Line { start, end } => Shape::Segment { start, end },
            Self::Circle { center, radius } => Shape::Circle { center, radius },
            Self::Arc { center, radius, .. } => match self.arc_geometry() {
                Some(arc) => Shape::Arc(arc),
                None => Shape::Circle { center, radius },
            },
        }
    }

    fn contains(&self, point: Point2, tolerance: f64) -> bool {
        match *self {
            Self::Line { start, end } => {
                let edge = end - start;
                let length = edge.length();
                if length <= 0.0 {
                    return false;
                }
                let along = (point - start).dot(edge) / (length * length);
                let slack = tolerance / length;
                along >= -slack && along <= 1.0 + slack
            }
            Self::Arc { .. } => self
                .arc_geometry()
                .is_some_and(|arc| intersect::on_arc(&arc, point, tolerance)),
            Self::Circle { .. } => true,
        }
    }

    fn closest(&self, point: Point2) -> Point2 {
        match *self {
            Self::Line { start, end } => intersect::closest_on_segment(start, end, point),
            Self::Circle { center, radius } => {
                center + (point - center).try_normalize().unwrap_or(Vector2::X) * radius
            }
            Self::Arc { center, radius, .. } => {
                let on_circle =
                    center + (point - center).try_normalize().unwrap_or(Vector2::X) * radius;
                if self.contains(on_circle, 0.0) {
                    return on_circle;
                }
                let (start, end) = (self.start(), self.end());
                if start.distance(point) <= end.distance(point) {
                    start
                } else {
                    end
                }
            }
        }
    }

    fn tangent_at(&self, point: Point2) -> Vector2 {
        match *self {
            Self::Line { .. } => self.start_tangent(),
            Self::Arc { center, sweep, .. } => {
                (point - center).normalize_or_zero().perp() * sweep.signum()
            }
            Self::Circle { center, .. } => (point - center).normalize_or_zero().perp(),
        }
    }

    fn points(&self, max_segment_angle: f64) -> Vec<Point2> {
        match *self {
            Self::Line { start, end } => vec![start, end],
            Self::Circle { center, radius } => {
                ArcGeometry::full_circle(center, radius).polyline(max_segment_angle)
            }
            Self::Arc {
                center,
                radius,
                from,
                sweep,
            } => {
                let segments = (sweep.abs() / max_segment_angle.max(1e-3)).ceil().max(1.0) as usize;
                (0..=segments)
                    .map(|index| {
                        let angle = from + sweep * index as f64 / segments as f64;
                        center + Vector2::from_angle(angle) * radius
                    })
                    .collect()
            }
        }
    }

    fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        match *self {
            Self::Line { start, end } => vec![start, end],
            Self::Circle { center, radius } => {
                ArcGeometry::full_circle(center, radius).faceted(faceting)
            }
            Self::Arc { .. } => self
                .arc_geometry()
                .map(|arc| arc.faceted(faceting))
                .unwrap_or_default(),
        }
    }

    fn bounds(&self) -> (Point2, Point2) {
        match *self {
            Self::Line { start, end } => (start.min(end), start.max(end)),
            Self::Arc { center, radius, .. } | Self::Circle { center, radius } => (
                center - Vector2::splat(radius),
                center + Vector2::splat(radius),
            ),
        }
    }

    fn extent(&self) -> f64 {
        let (low, high) = self.bounds();
        low.abs().max(high.abs()).max_element().max(1.0)
    }
}

fn wrapped(angle: f64) -> f64 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Element {
    curve: EntityId,
    end: EntityId,
    center: Option<EntityId>,
    course: Course,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    elements: Vec<Element>,
    labels: Vec<String>,
    closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Made {
    Offset(usize),
    Corner(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joint {
    Sharp,
    Tangent,
    Concentric,
}

impl Joint {
    fn carries_distance(self) -> bool {
        matches!(self, Self::Tangent | Self::Concentric)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Part {
    made: Made,
    course: Course,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    parts: Vec<Part>,
    joints: Vec<Joint>,
    closed: bool,
}

impl Outline {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        self.parts
            .iter()
            .map(|part| part.course.faceted(faceting))
            .collect()
    }

    pub fn curve_count(&self) -> usize {
        self.parts.len()
    }
}

impl Chain {
    pub fn curves(&self) -> Vec<EntityId> {
        self.elements.iter().map(|element| element.curve).collect()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn distance_to(&self, point: Point2) -> f64 {
        self.elements
            .iter()
            .map(|element| element.course.closest(point).distance(point))
            .fold(f64::INFINITY, f64::min)
    }

    pub fn default_side(&self) -> Side {
        if self.closed && self.signed_area() > 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    }

    pub fn side_of(&self, point: Point2) -> Side {
        if self.closed {
            let inside = self.winds_around(point);
            return if inside == (self.signed_area() > 0.0) {
                Side::Left
            } else {
                Side::Right
            };
        }
        let nearest = self
            .elements
            .iter()
            .enumerate()
            .map(|(index, element)| {
                let on = element.course.closest(point);
                (index, on, on.distance(point))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((index, on, _)) = nearest else {
            return Side::Left;
        };
        let Some(element) = self.elements.get(index) else {
            return Side::Left;
        };
        let course = element.course;
        let tolerance = TOLERANCE * course.extent();
        let normal = if on.distance(course.end()) <= tolerance
            && let Some(next) = self.elements.get(index + 1)
        {
            course.end_tangent().perp() + next.course.start_tangent().perp()
        } else if on.distance(course.start()) <= tolerance
            && let Some(previous) = index
                .checked_sub(1)
                .and_then(|previous| self.elements.get(previous))
        {
            course.start_tangent().perp() + previous.course.end_tangent().perp()
        } else {
            course.tangent_at(on).perp()
        };
        if (point - on).dot(normal) >= 0.0 {
            Side::Left
        } else {
            Side::Right
        }
    }

    fn outline_points(&self) -> Vec<Point2> {
        let mut points = Vec::new();
        for element in &self.elements {
            let mut course = element.course.points(SIDE_SEGMENT_ANGLE);
            if !points.is_empty() && !course.is_empty() {
                course.remove(0);
            }
            points.extend(course);
        }
        points
    }

    fn signed_area(&self) -> f64 {
        let points = self.outline_points();
        let following = points.iter().skip(1).chain(points.first());
        points
            .iter()
            .zip(following)
            .map(|(a, b)| a.perp_dot(*b))
            .sum::<f64>()
            / 2.0
    }

    fn winds_around(&self, point: Point2) -> bool {
        let points = self.outline_points();
        let following = points.iter().skip(1).chain(points.first());
        let mut inside = false;
        for (a, b) in points.iter().zip(following) {
            if (a.y > point.y) != (b.y > point.y) {
                let crossing = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if point.x < crossing {
                    inside = !inside;
                }
            }
        }
        inside
    }

    fn label(&self, index: usize) -> String {
        self.labels.get(index).cloned().unwrap_or_default()
    }

    fn scale(&self) -> f64 {
        self.elements
            .iter()
            .map(|element| element.course.extent())
            .fold(1.0, f64::max)
    }

    pub fn outline(&self, side: Side, distance: f64) -> Result<Outline, OffsetError> {
        if !(distance.is_finite() && distance > 0.0) {
            return Err(OffsetError::NotPositive);
        }
        let by = side.sign() * distance;
        let tolerance = TOLERANCE * self.scale().max(distance);
        let untrimmed: Vec<Course> = self
            .elements
            .iter()
            .enumerate()
            .map(|(index, element)| {
                element
                    .course
                    .offset(by)
                    .ok_or_else(|| OffsetError::Collapses {
                        entity: element.curve,
                        label: self.label(index),
                    })
            })
            .collect::<Result<_, _>>()?;
        let mut courses = untrimmed.clone();
        let count = courses.len();
        let joint_count = if self.closed && count > 1 {
            count
        } else {
            count.saturating_sub(1)
        };
        let mut junctions: Vec<Junction> = Vec::with_capacity(joint_count);
        for index in 0..joint_count {
            let next = (index + 1) % count;
            let junction = self.join(&mut courses, index, next, by, tolerance)?;
            junctions.push(junction);
        }
        for (index, (course, whole)) in courses.iter().zip(&untrimmed).enumerate() {
            if !course.keeps_course_of(whole, tolerance) {
                return Err(OffsetError::UsedUp {
                    entity: self
                        .elements
                        .get(index)
                        .map_or(EntityId::ORIGIN, |e| e.curve),
                    label: self.label(index),
                });
            }
        }
        let mut parts = Vec::with_capacity(count + joint_count);
        let mut joints = Vec::with_capacity(count + joint_count);
        for (index, course) in courses.iter().enumerate() {
            parts.push(Part {
                made: Made::Offset(index),
                course: *course,
            });
            match junctions.get(index) {
                Some(Junction::Round(corner)) => {
                    joints.push(Joint::Tangent);
                    parts.push(Part {
                        made: Made::Corner(index),
                        course: *corner,
                    });
                    joints.push(Joint::Tangent);
                }
                Some(Junction::Meet(joint)) => joints.push(*joint),
                None => {}
            }
        }
        let outline = Outline {
            parts,
            joints,
            closed: self.closed && count > 1,
        };
        if outline.crosses_itself(tolerance) {
            return Err(OffsetError::CrossesItself);
        }
        Ok(outline)
    }

    fn join(
        &self,
        courses: &mut [Course],
        index: usize,
        next: usize,
        by: f64,
        tolerance: f64,
    ) -> Result<Junction, OffsetError> {
        let (Some(element), Some(following)) = (self.elements.get(index), self.elements.get(next))
        else {
            return Ok(Junction::Meet(Joint::Sharp));
        };
        let (Some(before), Some(after)) = (courses.get(index).copied(), courses.get(next).copied())
        else {
            return Ok(Junction::Meet(Joint::Sharp));
        };
        let leaving = element.course.end_tangent();
        let entering = following.course.start_tangent();
        let turn = leaving.perp_dot(entering);
        let along = leaving.dot(entering);
        let corner = element.course.end();
        let meet = |courses: &mut [Course], point: Point2| {
            if let Some(course) = courses.get_mut(index) {
                *course = course.with_end(point);
            }
            if let Some(course) = courses.get_mut(next) {
                *course = course.with_start(point);
            }
        };
        if turn.abs() <= SMOOTH_ANGLE && along > 0.0 {
            meet(courses, before.end().midpoint(after.start()));
            let concentric = match (element.course.center(), following.course.center()) {
                (Some(a), Some(b)) => a.distance(b) <= tolerance,
                _ => false,
            };
            return Ok(Junction::Meet(if concentric {
                Joint::Concentric
            } else {
                Joint::Tangent
            }));
        }
        let convex = turn.abs() <= SMOOTH_ANGLE || by * turn < 0.0;
        if before.is_line() && after.is_line() && turn.abs() > SMOOTH_ANGLE {
            let crossing = before.start()
                + leaving * ((after.start() - before.start()).perp_dot(entering) / turn);
            meet(courses, crossing);
            return Ok(Junction::Meet(Joint::Sharp));
        }
        if convex {
            let from = direction_angle(before.end() - corner);
            let to = direction_angle(after.start() - corner);
            let sweep = if by > 0.0 {
                -(from - to).rem_euclid(TAU)
            } else {
                (to - from).rem_euclid(TAU)
            };
            return Ok(Junction::Round(Course::Arc {
                center: corner,
                radius: by.abs(),
                from,
                sweep,
            }));
        }
        let crossing =
            meeting_point(&before, &after, tolerance).ok_or_else(|| OffsetError::Apart {
                first: self.label(index),
                second: self.label(next),
            })?;
        meet(courses, crossing);
        Ok(Junction::Meet(Joint::Sharp))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Junction {
    Meet(Joint),
    Round(Course),
}

fn meeting_point(before: &Course, after: &Course, tolerance: f64) -> Option<Point2> {
    let aim = before.end().midpoint(after.start());
    let candidates = match (before.carrier()?, after.carrier()?) {
        (line @ Carrier::Line { .. }, Carrier::Circle { center, radius })
        | (Carrier::Circle { center, radius }, line @ Carrier::Line { .. }) => {
            intersect::crossings(line, &Shape::Circle { center, radius }, tolerance)
        }
        (circle @ Carrier::Circle { .. }, Carrier::Circle { center, radius }) => {
            intersect::crossings(circle, &Shape::Circle { center, radius }, tolerance)
        }
        (Carrier::Line { .. }, Carrier::Line { .. }) => Vec::new(),
    };
    candidates
        .into_iter()
        .min_by(|a, b| a.distance(aim).total_cmp(&b.distance(aim)))
}

impl Outline {
    fn adjacent(&self, first: usize, second: usize) -> bool {
        let count = self.parts.len();
        let (low, high) = (first.min(second), first.max(second));
        high - low == 1 || (self.closed && low == 0 && high + 1 == count)
    }

    fn crosses_itself(&self, tolerance: f64) -> bool {
        let mut order: Vec<(usize, (Point2, Point2))> = self
            .parts
            .iter()
            .map(|part| part.course.bounds())
            .enumerate()
            .collect();
        order.sort_by(|a, b| a.1.0.x.total_cmp(&b.1.0.x));
        for (position, (first, (low, high))) in order.iter().enumerate() {
            for (second, (other_low, other_high)) in order.iter().skip(position + 1) {
                if other_low.x > high.x + tolerance {
                    break;
                }
                let overlapping =
                    other_low.y <= high.y + tolerance && other_high.y >= low.y - tolerance;
                if !overlapping || self.adjacent(*first, *second) {
                    continue;
                }
                let (Some(a), Some(b)) = (self.parts.get(*first), self.parts.get(*second)) else {
                    continue;
                };
                if touches(&a.course, &b.course, tolerance) {
                    return true;
                }
            }
        }
        false
    }
}

fn touches(a: &Course, b: &Course, tolerance: f64) -> bool {
    let Some(carrier) = a.carrier() else {
        return false;
    };
    intersect::crossings(carrier, &b.shape(), tolerance)
        .into_iter()
        .any(|point| a.contains(point, tolerance))
}

#[derive(Debug, Clone, Copy)]
struct End {
    position: Point2,
    element: usize,
}

impl Sketch {
    pub fn offset_chain(&self, curves: &[EntityId]) -> Result<Chain, OffsetError> {
        let picked: BTreeSet<EntityId> = curves
            .iter()
            .copied()
            .filter(|id| {
                !id.is_reference()
                    && self
                        .entity(*id)
                        .is_some_and(|entity| !matches!(entity, Entity::Point(_)))
            })
            .collect();
        if picked.is_empty() {
            return Err(OffsetError::NothingSelected);
        }
        let elements: Vec<Element> = picked
            .iter()
            .map(|curve| self.offset_element(*curve))
            .collect::<Result<_, _>>()?;
        let circles = elements
            .iter()
            .filter(|element| matches!(element.course, Course::Circle { .. }))
            .count();
        if circles > 0 && elements.len() > 1 {
            let others = elements.len() - circles;
            let count = circles + usize::from(others > 0);
            return Err(OffsetError::SeveralChains {
                count: count.max(2),
            });
        }
        if circles == 1 {
            return Ok(self.chain_of(elements, true));
        }
        let ends = element_ends(&elements);
        let links = linked_ends(&ends, scale_of(&elements))?;
        self.walk(elements, &links)
    }

    pub fn offset_chain_through(&self, curve: EntityId) -> Vec<EntityId> {
        let Ok(start) = self.offset_element(curve) else {
            return Vec::new();
        };
        if matches!(start.course, Course::Circle { .. }) {
            return vec![curve];
        }
        let candidates: Vec<Element> = self
            .entities()
            .filter(|(id, entity)| {
                *id != curve && matches!(entity, Entity::Line { .. } | Entity::Arc { .. })
            })
            .filter_map(|(id, _)| self.offset_element(id).ok())
            .collect();
        let elements: Vec<Element> = std::iter::once(start).chain(candidates).collect();
        let ends = element_ends(&elements);
        let partners = all_links(&ends, scale_of(&elements));
        let mut taken = BTreeSet::from([0_usize]);
        let mut pending = vec![0_usize];
        while let Some(element) = pending.pop() {
            for at_start in [true, false] {
                let slot = 2 * element + usize::from(!at_start);
                let Some(partners) = partners.get(slot) else {
                    continue;
                };
                if let [only] = partners.as_slice()
                    && let Some(end) = ends.get(*only)
                    && taken.insert(end.element)
                {
                    pending.push(end.element);
                }
            }
        }
        taken
            .into_iter()
            .filter_map(|index| elements.get(index).map(|element| element.curve))
            .collect()
    }

    pub fn offset(
        &mut self,
        curves: &[EntityId],
        side: Side,
        distance: f64,
        value: Expression,
    ) -> Result<Vec<EntityId>, OffsetError> {
        let chain = self.offset_chain(curves)?;
        let outline = chain.outline(side, distance)?;
        let mut working = self.clone();
        let added = working
            .add_offset(&chain, &outline, value)
            .map_err(OffsetError::Edit)?;
        *self = working;
        Ok(added)
    }

    fn offset_element(&self, curve: EntityId) -> Result<Element, OffsetError> {
        let label = || self.entity_label(curve);
        let no_length = || OffsetError::NoLength {
            entity: curve,
            label: label(),
        };
        let entity = self.entity(curve).ok_or_else(no_length)?;
        let (end, center, course) = match *entity {
            Entity::Line { end, .. } => {
                let (start_at, end_at) = self.line_endpoints(curve).ok_or_else(no_length)?;
                if start_at.distance(end_at) <= TOLERANCE * start_at.abs().max_element().max(1.0) {
                    return Err(no_length());
                }
                (
                    end,
                    None,
                    Course::Line {
                        start: start_at,
                        end: end_at,
                    },
                )
            }
            Entity::Arc { center, end, .. } => {
                let arc = self.arc(curve).ok_or_else(no_length)?;
                if arc.radius <= 0.0 {
                    return Err(no_length());
                }
                (
                    end,
                    Some(center),
                    Course::Arc {
                        center: arc.center,
                        radius: arc.radius,
                        from: arc.start_angle,
                        sweep: arc.sweep,
                    },
                )
            }
            Entity::Circle { center, .. } => {
                let (at, radius) = self.circle(curve).ok_or_else(no_length)?;
                (center, Some(center), Course::Circle { center: at, radius })
            }
            Entity::Spline { .. } => {
                return Err(OffsetError::NotOffsettable {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => {
                return Err(OffsetError::EllipseNotOffsettable {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Point(_) => return Err(no_length()),
        };
        Ok(Element {
            curve,
            end,
            center,
            course,
        })
    }

    fn chain_of(&self, elements: Vec<Element>, closed: bool) -> Chain {
        let labels = elements
            .iter()
            .map(|element| self.entity_label(element.curve))
            .collect();
        Chain {
            elements,
            labels,
            closed,
        }
    }

    fn walk(&self, elements: Vec<Element>, links: &[Option<usize>]) -> Result<Chain, OffsetError> {
        let count = elements.len();
        let partner = |element: usize, at_start: bool| {
            links
                .get(2 * element + usize::from(!at_start))
                .copied()
                .flatten()
        };
        let components = components(count, links);
        if components > 1 {
            return Err(OffsetError::SeveralChains { count: components });
        }
        let open_end = (0..count)
            .flat_map(|element| [(element, true), (element, false)])
            .find(|(element, at_start)| partner(*element, *at_start).is_none());
        let (mut element, mut entered_at_start) = open_end.unwrap_or((0, true));
        let mut ordered = Vec::with_capacity(count);
        let mut seen = BTreeSet::new();
        while seen.insert(element) {
            let Some(original) = elements.get(element) else {
                break;
            };
            ordered.push(self.oriented(*original, !entered_at_start));
            let exit = 2 * element + usize::from(entered_at_start);
            let Some(next) = links.get(exit).copied().flatten() else {
                break;
            };
            element = next / 2;
            entered_at_start = next % 2 == 0;
        }
        let closed = open_end.is_none();
        Ok(self.chain_of(ordered, closed))
    }

    fn oriented(&self, element: Element, reversed: bool) -> Element {
        if !reversed {
            return element;
        }
        let start = match self.entity(element.curve) {
            Some(Entity::Line { start, .. } | Entity::Arc { start, .. }) => *start,
            _ => element.end,
        };
        let course = match element.course {
            Course::Line { start, end } => Course::Line {
                start: end,
                end: start,
            },
            Course::Arc {
                center,
                radius,
                from,
                sweep,
            } => Course::Arc {
                center,
                radius,
                from: from + sweep,
                sweep: -sweep,
            },
            circle @ Course::Circle { .. } => circle,
        };
        Element {
            end: start,
            course,
            ..element
        }
    }

    fn add_offset(
        &mut self,
        chain: &Chain,
        outline: &Outline,
        value: Expression,
    ) -> Result<Vec<EntityId>, SketchError> {
        let mut made = Vec::with_capacity(outline.parts.len());
        for part in &outline.parts {
            made.push(self.add_offset_part(chain, part)?);
        }
        for (index, (part, piece)) in outline.parts.iter().zip(&made).enumerate() {
            let Made::Offset(element) = part.made else {
                continue;
            };
            let Some(original) = chain.elements.get(element) else {
                continue;
            };
            let carried = index
                .checked_sub(1)
                .and_then(|previous| outline.joints.get(previous))
                .is_some_and(|joint| joint.carries_distance());
            if carried {
                if original.course.is_line() {
                    self.add_constraint(Constraint::Parallel(original.curve, piece.curve))?;
                }
            } else {
                self.hold_offset(original, piece, value.clone())?;
            }
        }
        let count = made.len();
        let closing = outline.closed.then(|| count.saturating_sub(1));
        for (index, joint) in outline.joints.iter().enumerate() {
            if Some(index) == closing {
                continue;
            }
            self.join_offset_parts(&made, index, (index + 1) % count, *joint)?;
        }
        if let Some(index) = closing
            && let Some(joint) = outline.joints.get(index)
        {
            self.join_offset_parts(&made, index, 0, *joint)?;
        }
        Ok(made.iter().map(|piece| piece.curve).collect())
    }

    fn add_offset_part(&mut self, chain: &Chain, part: &Part) -> Result<Piece, SketchError> {
        let (center, construction) = match part.made {
            Made::Offset(index) => {
                let element = chain.elements.get(index);
                (
                    element.and_then(|element| element.center),
                    element.is_some_and(|element| self.is_construction(element.curve)),
                )
            }
            Made::Corner(index) => {
                let element = chain.elements.get(index);
                let next = chain.elements.get((index + 1) % chain.elements.len());
                let construction = [element, next]
                    .into_iter()
                    .flatten()
                    .all(|element| self.is_construction(element.curve));
                (element.map(|element| element.end), construction)
            }
        };
        let piece = match part.course {
            Course::Line { start, end } => {
                let start = self.add_point(start);
                let end = self.add_point(end);
                let curve = self.add_entity(Entity::Line { start, end })?;
                Piece {
                    curve,
                    first: Some(start),
                    last: Some(end),
                }
            }
            Course::Arc { sweep, .. } => {
                let center = center.ok_or(SketchError::MissingEntity(EntityId::ORIGIN))?;
                let first = self.add_point(part.course.start());
                let last = self.add_point(part.course.end());
                let (start, end) = if sweep >= 0.0 {
                    (first, last)
                } else {
                    (last, first)
                };
                let curve = self.add_entity(Entity::Arc { center, start, end })?;
                Piece {
                    curve,
                    first: Some(first),
                    last: Some(last),
                }
            }
            Course::Circle { radius, .. } => {
                let center = center.ok_or(SketchError::MissingEntity(EntityId::ORIGIN))?;
                let curve = self.add_entity(Entity::Circle { center, radius })?;
                Piece {
                    curve,
                    first: None,
                    last: None,
                }
            }
        };
        if construction {
            self.set_construction(piece.curve, true)?;
        }
        Ok(piece)
    }

    fn add_entity(&mut self, entity: Entity) -> Result<EntityId, SketchError> {
        let id = EntityId::from_raw(self.next_id());
        self.insert_entity(id, entity)?;
        Ok(id)
    }

    fn hold_offset(
        &mut self,
        original: &Element,
        piece: &Piece,
        value: Expression,
    ) -> Result<(), SketchError> {
        match original.course {
            Course::Line { .. } => {
                self.add_constraint(Constraint::Distance {
                    from: original.curve,
                    to: piece.curve,
                    value,
                })?;
            }
            Course::Arc { .. } => {
                let start = match self.entity(piece.curve) {
                    Some(Entity::Arc { start, .. }) => *start,
                    _ => return Err(SketchError::NoSuchEntity(piece.curve)),
                };
                self.add_constraint(Constraint::Distance {
                    from: start,
                    to: original.curve,
                    value,
                })?;
            }
            Course::Circle { .. } => {
                let (center, radius) = self
                    .circle(piece.curve)
                    .ok_or(SketchError::NoSuchEntity(piece.curve))?;
                let center_point = self
                    .center_of(piece.curve)
                    .ok_or(SketchError::NoSuchEntity(piece.curve))?;
                let rim = self.add_point(center + Vector2::X * radius);
                self.add_constraint(Constraint::Coincident(rim, piece.curve))?;
                self.add_constraint(Constraint::HorizontalPoints(rim, center_point))?;
                self.add_constraint(Constraint::Distance {
                    from: rim,
                    to: original.curve,
                    value,
                })?;
            }
        }
        Ok(())
    }

    fn join_offset_parts(
        &mut self,
        made: &[Piece],
        index: usize,
        next: usize,
        joint: Joint,
    ) -> Result<(), SketchError> {
        let (Some(before), Some(after)) = (made.get(index), made.get(next)) else {
            return Ok(());
        };
        if joint == Joint::Tangent {
            self.add_constraint(Constraint::Tangent(before.curve, after.curve))?;
        }
        if let (Some(end), Some(start)) = (before.last, after.first) {
            self.add_constraint(Constraint::Coincident(end, start))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Piece {
    curve: EntityId,
    first: Option<EntityId>,
    last: Option<EntityId>,
}

fn scale_of(elements: &[Element]) -> f64 {
    elements
        .iter()
        .map(|element| element.course.extent())
        .fold(1.0, f64::max)
}

fn element_ends(elements: &[Element]) -> Vec<End> {
    elements
        .iter()
        .enumerate()
        .flat_map(|(element, entry)| {
            [
                End {
                    position: entry.course.start(),
                    element,
                },
                End {
                    position: entry.course.end(),
                    element,
                },
            ]
        })
        .collect()
}

fn all_links(ends: &[End], scale: f64) -> Vec<Vec<usize>> {
    let tolerance = TOLERANCE * scale;
    let mut order: Vec<usize> = (0..ends.len()).collect();
    order.sort_by(|a, b| {
        let x = |index: &usize| ends.get(*index).map_or(0.0, |end| end.position.x);
        x(a).total_cmp(&x(b))
    });
    let mut partners = vec![Vec::new(); ends.len()];
    for (position, first) in order.iter().enumerate() {
        let Some(a) = ends.get(*first) else {
            continue;
        };
        for second in order.iter().skip(position + 1) {
            let Some(b) = ends.get(*second) else {
                continue;
            };
            if b.position.x - a.position.x > tolerance {
                break;
            }
            if a.element != b.element && a.position.distance(b.position) <= tolerance {
                if let Some(list) = partners.get_mut(*first) {
                    list.push(*second);
                }
                if let Some(list) = partners.get_mut(*second) {
                    list.push(*first);
                }
            }
        }
    }
    for list in &mut partners {
        list.sort_unstable();
    }
    partners
}

fn linked_ends(ends: &[End], scale: f64) -> Result<Vec<Option<usize>>, OffsetError> {
    all_links(ends, scale)
        .into_iter()
        .map(|partners| match partners.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some(*only)),
            _ => Err(OffsetError::Branches),
        })
        .collect()
}

fn components(count: usize, links: &[Option<usize>]) -> usize {
    let mut seen = vec![false; count];
    let mut components = 0;
    for first in 0..count {
        if seen.get(first).copied().unwrap_or(true) {
            continue;
        }
        components += 1;
        let mut pending = vec![first];
        while let Some(element) = pending.pop() {
            match seen.get_mut(element) {
                Some(visited) if !*visited => *visited = true,
                _ => continue,
            }
            for slot in [2 * element, 2 * element + 1] {
                if let Some(Some(partner)) = links.get(slot) {
                    pending.push(partner / 2);
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests;
