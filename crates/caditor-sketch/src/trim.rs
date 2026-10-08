use std::{collections::BTreeSet, f64::consts::TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{ArcGeometry, Faceting, direction_angle},
    entity::Entity,
    id::{ConstraintId, EntityId, Reference},
    intersect::{self, Carrier, Shape},
    sketch::{Sketch, SketchError},
};

const TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TrimError {
    #[error("the curve to trim no longer exists")]
    NoSuchCurve(EntityId),
    #[error("{label} is not a line, circle or arc, so it cannot be trimmed")]
    NotACurve { entity: EntityId, label: String },
    #[error("{label} cannot be trimmed; only lines, circles and arcs can")]
    Spline { entity: EntityId, label: String },
    #[error("{label} has no length to trim")]
    NoLength { entity: EntityId, label: String },
    #[error("{label} is built into the sketch, so it cannot be trimmed")]
    Reference { entity: EntityId, label: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be trimmed")]
    Projected { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExtendError {
    #[error("the curve to extend no longer exists")]
    NoSuchCurve(EntityId),
    #[error("{label} is not a line or an arc, so it has no end to extend")]
    NotACurve { entity: EntityId, label: String },
    #[error("{label} is closed, so it has no end to extend")]
    Closed { entity: EntityId, label: String },
    #[error("{label} cannot be extended; only lines and arcs can")]
    Spline { entity: EntityId, label: String },
    #[error("{label} has no length to extend")]
    NoLength { entity: EntityId, label: String },
    #[error("{label} is built into the sketch, so it cannot be extended")]
    Reference { entity: EntityId, label: String },
    #[error("nothing lies beyond this end of {label} to extend it to")]
    NothingToReach { entity: EntityId, label: String },
    #[error("this end of {label} is joined to {other}, so it cannot move")]
    Joined {
        entity: EntityId,
        label: String,
        other: String,
    },
    #[error("this end of {label} is fixed, so it cannot move")]
    Fixed { entity: EntityId, label: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be extended")]
    Projected { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cut {
    pub position: Point2,
    pub cutter: EntityId,
    pub point: Option<EntityId>,
    pub(crate) parameter: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub curve: EntityId,
    pub start: Option<Cut>,
    pub end: Option<Cut>,
    course: Course,
    from: f64,
    to: f64,
}

impl Piece {
    pub fn is_whole(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }

    pub fn cutters(&self) -> Vec<EntityId> {
        let mut cutters: Vec<EntityId> = [self.start, self.end]
            .into_iter()
            .flatten()
            .map(|cut| cut.cutter)
            .collect();
        cutters.dedup();
        cutters
    }

    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        self.course.faceted(self.from, self.to, faceting)
    }

    pub fn middle(&self) -> Point2 {
        self.course.at((self.from + self.to) / 2.0)
    }

    pub fn contains(&self, point: Point2) -> bool {
        let parameter = self.course.parameter(point);
        let slack = self.course.slack(self.course.extent() * TOLERANCE);
        let within = |value: f64| value >= self.from - slack && value <= self.to + slack;
        within(parameter) || (self.course.is_closed() && within(parameter + TAU))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trimmed {
    Deleted,
    Shortened,
    Opened,
    Split { piece: EntityId },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    pub curve: EntityId,
    pub end: EntityId,
    pub from: Point2,
    pub to: Point2,
    pub target: EntityId,
    pub target_point: Option<EntityId>,
    reach: Reach,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reach {
    Straight,
    Around(ArcGeometry),
}

impl Extension {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        match self.reach {
            Reach::Straight => vec![self.from, self.to],
            Reach::Around(arc) => arc.faceted(faceting),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Course {
    Line { start: Point2, end: Point2 },
    Circle { center: Point2, radius: f64 },
    Arc(ArcGeometry),
}

impl Course {
    fn carrier(&self) -> Option<Carrier> {
        match *self {
            Self::Line { start, end } => Some(Carrier::Line {
                through: start,
                direction: (end - start).try_normalize()?,
            }),
            Self::Circle { center, radius } => Some(Carrier::Circle { center, radius }),
            Self::Arc(arc) => Some(Carrier::Circle {
                center: arc.center,
                radius: arc.radius,
            }),
        }
    }

    fn shape(&self) -> Shape {
        match *self {
            Self::Line { start, end } => Shape::Segment { start, end },
            Self::Circle { center, radius } => Shape::Circle { center, radius },
            Self::Arc(arc) => Shape::Arc(arc),
        }
    }

    fn extent(&self) -> f64 {
        self.shape().extent().max(1.0)
    }

    fn is_closed(&self) -> bool {
        matches!(self, Self::Circle { .. })
    }

    fn span(&self) -> f64 {
        match self {
            Self::Line { .. } => 1.0,
            Self::Circle { .. } => TAU,
            Self::Arc(arc) => arc.sweep,
        }
    }

    fn parameter(&self, point: Point2) -> f64 {
        match *self {
            Self::Line { start, end } => {
                let edge = end - start;
                let length_squared = edge.length_squared();
                if length_squared > 0.0 {
                    (point - start).dot(edge) / length_squared
                } else {
                    0.0
                }
            }
            Self::Circle { center, .. } => direction_angle(point - center).rem_euclid(TAU),
            Self::Arc(arc) => {
                let offset =
                    (direction_angle(point - arc.center) - arc.start_angle).rem_euclid(TAU);
                if offset <= arc.sweep {
                    offset
                } else if offset - arc.sweep < TAU - offset {
                    arc.sweep
                } else {
                    0.0
                }
            }
        }
    }

    fn at(&self, parameter: f64) -> Point2 {
        match *self {
            Self::Line { start, end } => start + (end - start) * parameter,
            Self::Circle { center, radius } => center + Vector2::from_angle(parameter) * radius,
            Self::Arc(arc) => arc.point_at(arc.start_angle + parameter),
        }
    }

    fn slack(&self, tolerance: f64) -> f64 {
        let size = match *self {
            Self::Line { start, end } => start.distance(end),
            Self::Circle { radius, .. } => radius,
            Self::Arc(arc) => arc.radius,
        };
        if size > 0.0 { tolerance / size } else { 0.0 }
    }

    fn faceted(&self, from: f64, to: f64, faceting: Faceting) -> Vec<Point2> {
        match *self {
            Self::Line { .. } => vec![self.at(from), self.at(to)],
            Self::Circle { center, radius } => ArcGeometry {
                center,
                radius,
                start_angle: from,
                sweep: to - from,
            }
            .faceted(faceting),
            Self::Arc(arc) => ArcGeometry {
                start_angle: arc.start_angle + from,
                sweep: to - from,
                ..arc
            }
            .faceted(faceting),
        }
    }
}

enum Joint {
    Point(EntityId),
    Curve(EntityId),
}

enum Cutter {
    Shape(Shape),
    Axis(Vector2),
}

impl Cutter {
    fn extent(&self) -> f64 {
        match self {
            Self::Shape(shape) => shape.extent(),
            Self::Axis(_) => 0.0,
        }
    }

    fn crossings(&self, carrier: Carrier, tolerance: f64) -> Vec<Point2> {
        match self {
            Self::Shape(shape) => intersect::crossings(carrier, shape, tolerance),
            Self::Axis(direction) => {
                intersect::line_crossings(carrier, Point2::ZERO, *direction, tolerance)
            }
        }
    }

    fn cut_positions(&self, course: &Course, tolerance: f64) -> Vec<Point2> {
        let Some(carrier) = course.carrier() else {
            return Vec::new();
        };
        match self {
            Self::Shape(shape) => overlap_ends(course, shape, tolerance)
                .unwrap_or_else(|| intersect::crossings(carrier, shape, tolerance)),
            Self::Axis(_) => self.crossings(carrier, tolerance),
        }
    }

    fn end_point_near(
        &self,
        sketch: &Sketch,
        cutter: EntityId,
        position: Point2,
        tolerance: f64,
    ) -> Option<EntityId> {
        match self {
            Self::Shape(shape) => end_point_near(sketch, cutter, shape, position, tolerance),
            Self::Axis(_) => None,
        }
    }
}

impl Sketch {
    pub fn closest_on_curve(&self, curve: EntityId, to: Point2) -> Option<Point2> {
        Some(self.shape_of(curve)?.closest(to))
    }

    pub fn spline_crossings(&self, spline: EntityId, other: EntityId) -> Vec<Point2> {
        let Some(shape @ Shape::Spline(_)) = self.shape_of(spline) else {
            return Vec::new();
        };
        let tolerance = TOLERANCE * shape.extent().max(1.0);
        let axis = |direction: Vector2| {
            intersect::crossings(
                Carrier::Line {
                    through: Point2::ZERO,
                    direction,
                },
                &shape,
                tolerance,
            )
        };
        match other.reference() {
            Some(Reference::HorizontalAxis) => return axis(Vector2::X),
            Some(Reference::VerticalAxis) => return axis(Vector2::Y),
            Some(Reference::Origin) => return Vec::new(),
            None => {}
        }
        match self.shape_of(other) {
            Some(Shape::Segment { start, end }) => self.segment_crossings(spline, start, end),
            Some(Shape::Circle { center, radius }) => {
                intersect::crossings(Carrier::Circle { center, radius }, &shape, tolerance)
            }
            Some(Shape::Arc(arc)) => intersect::crossings(
                Carrier::Circle {
                    center: arc.center,
                    radius: arc.radius,
                },
                &shape,
                tolerance,
            )
            .into_iter()
            .filter(|point| intersect::on_arc(&arc, *point, tolerance))
            .collect(),
            Some(Shape::Spline(other)) => {
                let Shape::Spline(own) = &shape else {
                    return Vec::new();
                };
                intersect::spline_spline(own, &other, tolerance)
            }
            None => Vec::new(),
        }
    }

    pub fn circle_crossings(&self, curve: EntityId, center: Point2, radius: f64) -> Vec<Point2> {
        let Some(shape) = self.shape_of(curve) else {
            return Vec::new();
        };
        let tolerance = TOLERANCE * shape.extent().max(1.0);
        intersect::crossings(Carrier::Circle { center, radius }, &shape, tolerance)
    }

    pub fn segment_crossings(&self, curve: EntityId, from: Point2, to: Point2) -> Vec<Point2> {
        let (Some(shape), Some(direction)) = (self.shape_of(curve), (to - from).try_normalize())
        else {
            return Vec::new();
        };
        let length = from.distance(to);
        let carrier = Carrier::Line {
            through: from,
            direction,
        };
        let tolerance = TOLERANCE * shape.extent().max(1.0);
        intersect::crossings(carrier, &shape, tolerance)
            .into_iter()
            .filter(|point| {
                let along = (*point - from).dot(direction);
                (0.0..=length).contains(&along)
            })
            .collect()
    }

    pub fn trim_pieces(&self, curve: EntityId) -> Result<Vec<Piece>, TrimError> {
        let course = self.trim_course(curve)?;
        let cuts = self.cuts(curve, &course);
        let piece = |start: Option<Cut>, end: Option<Cut>, from: f64, to: f64| Piece {
            curve,
            start,
            end,
            course,
            from,
            to,
        };
        if course.is_closed() {
            if cuts.len() < 2 {
                return Ok(vec![piece(None, None, 0.0, TAU)]);
            }
            let following = cuts.iter().skip(1).chain(cuts.first());
            return Ok(cuts
                .iter()
                .zip(following)
                .map(|(start, end)| {
                    let to = if end.parameter > start.parameter {
                        end.parameter
                    } else {
                        end.parameter + TAU
                    };
                    piece(Some(*start), Some(*end), start.parameter, to)
                })
                .collect());
        }
        let starts = std::iter::once(None).chain(cuts.iter().copied().map(Some));
        let ends = cuts.iter().copied().map(Some).chain(std::iter::once(None));
        let span = course.span();
        Ok(starts
            .zip(ends)
            .map(|(start, end)| {
                let from = start.map_or(0.0, |cut| cut.parameter);
                let to = end.map_or(span, |cut| cut.parameter);
                piece(start, end, from, to)
            })
            .collect())
    }

    pub fn trim_piece(&self, curve: EntityId, near: Point2) -> Result<Piece, TrimError> {
        let pieces = self.trim_pieces(curve)?;
        let course = self.trim_course(curve)?;
        let parameter = match course {
            Course::Line { .. } => course.parameter(near).clamp(0.0, 1.0),
            Course::Circle { .. } | Course::Arc(_) => course.parameter(near),
        };
        let inside = |piece: &Piece| {
            (parameter >= piece.from && parameter <= piece.to)
                || (course.is_closed() && parameter + TAU <= piece.to)
        };
        pieces
            .iter()
            .find(|piece| inside(piece))
            .or(pieces.last())
            .cloned()
            .ok_or(TrimError::NoSuchCurve(curve))
    }

    pub(crate) fn open_cuts(&self, curve: EntityId) -> Result<Option<Vec<Cut>>, TrimError> {
        let course = self.trim_course(curve)?;
        Ok((!course.is_closed()).then(|| self.cuts(curve, &course)))
    }

    pub fn trim(&mut self, curve: EntityId, near: Point2) -> Result<Trimmed, TrimError> {
        let piece = self.trim_piece(curve, near)?;
        let mut working = self.clone();
        let trimmed = working.remove_piece(&piece).map_err(TrimError::Edit)?;
        *self = working;
        Ok(trimmed)
    }

    pub fn extension(&self, curve: EntityId, near: Point2) -> Result<Extension, ExtendError> {
        let label = || self.entity_label(curve);
        if curve.is_reference() {
            return Err(ExtendError::Reference {
                entity: curve,
                label: label(),
            });
        }
        let entity = self.entity(curve).ok_or(ExtendError::NoSuchCurve(curve))?;
        let no_length = || ExtendError::NoLength {
            entity: curve,
            label: label(),
        };
        if self.is_projected(curve) {
            return Err(ExtendError::Projected {
                entity: curve,
                label: label(),
            });
        }
        let (end, from, direction, reach) = match *entity {
            Entity::Point(_) => {
                return Err(ExtendError::NotACurve {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Circle { .. } => {
                return Err(ExtendError::Closed {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Spline { .. } => {
                return Err(ExtendError::Spline {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Line { start, end } => {
                let (start_at, end_at) = self.line_endpoints(curve).ok_or_else(no_length)?;
                let course = Course::Line {
                    start: start_at,
                    end: end_at,
                };
                let at_end = course.parameter(near) >= 0.5;
                let (moving, from, direction) = if at_end {
                    (end, end_at, end_at - start_at)
                } else {
                    (start, start_at, start_at - end_at)
                };
                let direction = direction.try_normalize().ok_or_else(no_length)?;
                (moving, from, direction, None)
            }
            Entity::Arc { start, end, .. } => {
                let arc = self.arc(curve).ok_or_else(no_length)?;
                if arc.radius <= 0.0 {
                    return Err(no_length());
                }
                let at_end = Course::Arc(arc).parameter(near) > arc.sweep / 2.0;
                let (moving, from) = if at_end {
                    (end, arc.point_at(arc.end_angle()))
                } else {
                    (start, arc.point_at(arc.start_angle))
                };
                (moving, from, Vector2::ZERO, Some((arc, at_end)))
            }
        };
        self.check_free_end(curve, end)?;
        let reached = match reach {
            None => self.reach_straight(curve, from, direction),
            Some((arc, at_end)) => self.reach_around(curve, arc, at_end),
        };
        let (to, target, reach) = reached.ok_or_else(|| ExtendError::NothingToReach {
            entity: curve,
            label: label(),
        })?;
        let target_point = self.end_point_at(target, to);
        Ok(Extension {
            curve,
            end,
            from,
            to,
            target,
            target_point,
            reach,
        })
    }

    pub fn extend(&mut self, curve: EntityId, near: Point2) -> Result<Extension, ExtendError> {
        let extension = self.extension(curve, near)?;
        let mut working = self.clone();
        working
            .apply_extension(&extension)
            .map_err(ExtendError::Edit)?;
        *self = working;
        Ok(extension)
    }

    fn shape_of(&self, curve: EntityId) -> Option<Shape> {
        match self.entity(curve)? {
            Entity::Point(_) => None,
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve)?;
                Some(Shape::Segment { start, end })
            }
            Entity::Circle { .. } => {
                let (center, radius) = self.circle(curve)?;
                Some(Shape::Circle { center, radius })
            }
            Entity::Arc { .. } => self.arc(curve).map(Shape::Arc),
            Entity::Spline { .. } => self.spline(curve).map(Shape::Spline),
        }
    }

    fn cutters(&self, excluding: EntityId) -> impl Iterator<Item = (EntityId, Cutter)> + '_ {
        let shapes = self
            .entities()
            .filter(move |(id, _)| *id != excluding)
            .filter_map(|(id, _)| Some((id, Cutter::Shape(self.shape_of(id)?))));
        let axes = [
            (EntityId::HORIZONTAL_AXIS, Cutter::Axis(Vector2::X)),
            (EntityId::VERTICAL_AXIS, Cutter::Axis(Vector2::Y)),
        ];
        shapes.chain(axes)
    }

    fn trim_course(&self, curve: EntityId) -> Result<Course, TrimError> {
        let label = || self.entity_label(curve);
        if curve.is_reference() {
            return Err(TrimError::Reference {
                entity: curve,
                label: label(),
            });
        }
        let no_length = || TrimError::NoLength {
            entity: curve,
            label: label(),
        };
        let entity = self.entity(curve).ok_or(TrimError::NoSuchCurve(curve))?;
        if self.is_projected(curve) {
            return Err(TrimError::Projected {
                entity: curve,
                label: label(),
            });
        }
        let course = match entity {
            Entity::Point(_) => {
                return Err(TrimError::NotACurve {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Spline { .. } => {
                return Err(TrimError::Spline {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve).ok_or_else(no_length)?;
                Course::Line { start, end }
            }
            Entity::Circle { .. } => {
                let (center, radius) = self.circle(curve).ok_or_else(no_length)?;
                Course::Circle { center, radius }
            }
            Entity::Arc { .. } => Course::Arc(self.arc(curve).ok_or_else(no_length)?),
        };
        if course.slack(1.0) == 0.0 || course.carrier().is_none() {
            return Err(no_length());
        }
        Ok(course)
    }

    fn cuts(&self, curve: EntityId, course: &Course) -> Vec<Cut> {
        let span = course.span();
        let mut cuts: Vec<Cut> = Vec::new();
        for (cutter, shape) in self.cutters(curve) {
            let tolerance = TOLERANCE * course.extent().max(shape.extent());
            let slack = course.slack(tolerance);
            for position in shape.cut_positions(course, tolerance) {
                let parameter = course.parameter(position);
                let interior =
                    course.is_closed() || (parameter > slack && parameter < span - slack);
                if interior {
                    cuts.push(Cut {
                        position: course.at(parameter),
                        cutter,
                        point: shape.end_point_near(self, cutter, position, tolerance),
                        parameter,
                    });
                }
            }
        }
        cuts.sort_by(|a, b| a.parameter.total_cmp(&b.parameter));
        let slack = course.slack(TOLERANCE * course.extent());
        let mut merged: Vec<Cut> = Vec::new();
        for cut in cuts {
            match merged.last_mut() {
                Some(last) if cut.parameter - last.parameter <= slack => {
                    if last.point.is_none() && cut.point.is_some() {
                        *last = cut;
                    }
                }
                _ => merged.push(cut),
            }
        }
        let wraps = course.is_closed()
            && merged.len() > 1
            && matches!(
                (merged.first(), merged.last()),
                (Some(first), Some(last)) if first.parameter + TAU - last.parameter <= slack
            );
        if wraps {
            merged.pop();
        }
        merged
    }

    fn remove_piece(&mut self, piece: &Piece) -> Result<Trimmed, SketchError> {
        let curve = piece.curve;
        let entity = self
            .entity(curve)
            .cloned()
            .ok_or(SketchError::NoSuchEntity(curve))?;
        Ok(match (entity, piece.start, piece.end) {
            (Entity::Line { start, end }, None, Some(cut)) => {
                let kept = self.add_point(cut.position);
                self.drop_length_dimensions(start, end)?;
                self.restructure(curve, Entity::Line { start: kept, end }, &[], keeps_length)?;
                self.join(kept, &cut, curve)?;
                self.drop_if_unused(start)?;
                Trimmed::Shortened
            }
            (Entity::Line { start, end }, Some(cut), None) => {
                let kept = self.add_point(cut.position);
                self.drop_length_dimensions(start, end)?;
                self.restructure(curve, Entity::Line { start, end: kept }, &[], keeps_length)?;
                self.join(kept, &cut, curve)?;
                self.drop_if_unused(end)?;
                Trimmed::Shortened
            }
            (Entity::Line { start, end }, Some(first), Some(second)) => {
                let near_end = self.add_point(first.position);
                let far_start = self.add_point(second.position);
                let moved = self.far_constraints(curve, start, end);
                let moved_ids: Vec<ConstraintId> = moved.iter().map(|(id, _, _)| *id).collect();
                let directions: Vec<Constraint> = self
                    .constraints_using(curve)
                    .into_iter()
                    .filter_map(|id| match self.constraint(id)? {
                        Constraint::Horizontal(_) => Some(Constraint::Horizontal(curve)),
                        Constraint::Vertical(_) => Some(Constraint::Vertical(curve)),
                        _ => None,
                    })
                    .collect();
                self.restructure(
                    curve,
                    Entity::Line {
                        start,
                        end: near_end,
                    },
                    &moved_ids,
                    keeps_length,
                )?;
                let split = self.add_piece(
                    curve,
                    Entity::Line {
                        start: far_start,
                        end,
                    },
                )?;
                self.move_constraints(moved, curve, split)?;
                if directions.is_empty() {
                    self.add_constraint(Constraint::Collinear(curve, split))?;
                } else {
                    for direction in directions {
                        self.add_constraint(direction.with_entity_replaced(curve, split))?;
                    }
                    self.add_constraint(Constraint::Coincident(far_start, curve))?;
                }
                self.join(near_end, &first, curve)?;
                self.join(far_start, &second, curve)?;
                Trimmed::Split { piece: split }
            }
            (Entity::Circle { center, .. }, Some(first), Some(second)) => {
                let start = self.add_point(second.position);
                let end = self.add_point(first.position);
                self.restructure(curve, Entity::Arc { center, start, end }, &[], |_| true)?;
                self.join(start, &second, curve)?;
                self.join(end, &first, curve)?;
                Trimmed::Opened
            }
            (Entity::Arc { center, start, end }, None, Some(cut)) => {
                let kept = self.add_point(cut.position);
                let arc = Entity::Arc {
                    center,
                    start: kept,
                    end,
                };
                self.restructure(curve, arc, &[], keeps_sweep)?;
                self.join(kept, &cut, curve)?;
                self.drop_if_unused(start)?;
                Trimmed::Shortened
            }
            (Entity::Arc { center, start, end }, Some(cut), None) => {
                let kept = self.add_point(cut.position);
                let arc = Entity::Arc {
                    center,
                    start,
                    end: kept,
                };
                self.restructure(curve, arc, &[], keeps_sweep)?;
                self.join(kept, &cut, curve)?;
                self.drop_if_unused(end)?;
                Trimmed::Shortened
            }
            (Entity::Arc { center, start, end }, Some(first), Some(second)) => {
                let near_end = self.add_point(first.position);
                let far_start = self.add_point(second.position);
                let center_at = self.point(center).ok_or(SketchError::NotAPoint(center))?;
                let moved = self.far_constraints(curve, start, end);
                let moved_ids: Vec<ConstraintId> = moved.iter().map(|(id, _, _)| *id).collect();
                let arc = Entity::Arc {
                    center,
                    start,
                    end: near_end,
                };
                self.restructure(curve, arc, &moved_ids, keeps_sweep)?;
                let split_center = self.add_point(center_at);
                let split = self.add_piece(
                    curve,
                    Entity::Arc {
                        center: split_center,
                        start: far_start,
                        end,
                    },
                )?;
                self.move_constraints(moved, curve, split)?;
                self.add_constraint(Constraint::Concentric(curve, split))?;
                self.add_constraint(Constraint::Equal(curve, split))?;
                self.join(near_end, &first, curve)?;
                self.join(far_start, &second, curve)?;
                Trimmed::Split { piece: split }
            }
            (Entity::Line { .. } | Entity::Circle { .. } | Entity::Arc { .. }, None, None)
            | (Entity::Circle { .. }, _, _) => {
                self.delete_curve(curve)?;
                Trimmed::Deleted
            }
            (Entity::Point(_) | Entity::Spline { .. }, _, _) => {
                return Err(SketchError::WrongKind {
                    entity: curve,
                    found: self.entity_label(curve),
                    needed: "a line, a circle or an arc",
                });
            }
        })
    }

    pub(crate) fn restructure(
        &mut self,
        curve: EntityId,
        entity: Entity,
        moved: &[ConstraintId],
        keeps: impl Fn(&Constraint) -> bool,
    ) -> Result<(), SketchError> {
        let construction = self.is_construction(curve);
        let mut kept = Vec::new();
        for id in self.constraints_using(curve) {
            let inactive = !self.is_active(id);
            let constraint = self.remove_constraint(id)?;
            if !moved.contains(&id) && keeps(&constraint) {
                kept.push((id, constraint, inactive));
            }
        }
        self.remove_unused_entity(curve)?;
        self.insert_entity(curve, entity)?;
        if construction {
            self.set_construction(curve, true)?;
        }
        for (id, constraint, inactive) in kept {
            self.insert_constraint(id, constraint)?;
            if inactive {
                self.set_active(id, false)?;
            }
        }
        Ok(())
    }

    fn drop_length_dimensions(&mut self, a: EntityId, b: EntityId) -> Result<(), SketchError> {
        let at_a = self.joined_points(a);
        let at_b = self.joined_points(b);
        let between: BTreeSet<ConstraintId> = at_a
            .iter()
            .flat_map(|point| self.constraints_using(*point))
            .filter(|id| match self.constraint(*id) {
                Some(
                    Constraint::Distance { from, to, .. }
                    | Constraint::HorizontalDistance { from, to, .. }
                    | Constraint::VerticalDistance { from, to, .. },
                ) => {
                    (at_a.contains(from) && at_b.contains(to))
                        || (at_b.contains(from) && at_a.contains(to))
                }
                _ => false,
            })
            .collect();
        for id in between {
            self.remove_constraint(id)?;
        }
        Ok(())
    }

    fn joined_points(&self, point: EntityId) -> BTreeSet<EntityId> {
        let mut joined = BTreeSet::from([point]);
        let mut pending = vec![point];
        while let Some(current) = pending.pop() {
            for id in self.constraints_using(current) {
                let Some(Constraint::Coincident(first, second)) = self.constraint(id) else {
                    continue;
                };
                for other in [*first, *second] {
                    let is_point = matches!(self.entity(other), Some(Entity::Point(_)));
                    if is_point && joined.insert(other) {
                        pending.push(other);
                    }
                }
            }
        }
        joined
    }

    pub(crate) fn add_piece(
        &mut self,
        curve: EntityId,
        entity: Entity,
    ) -> Result<EntityId, SketchError> {
        let piece = EntityId::from_raw(self.next_id());
        self.insert_entity(piece, entity)?;
        if self.is_construction(curve) {
            self.set_construction(piece, true)?;
        }
        Ok(piece)
    }

    pub(crate) fn move_constraints(
        &mut self,
        moved: Vec<(ConstraintId, Constraint, bool)>,
        from: EntityId,
        to: EntityId,
    ) -> Result<(), SketchError> {
        for (_, constraint, inactive) in moved {
            let id = self.add_constraint(constraint.with_entity_replaced(from, to))?;
            if inactive {
                self.set_active(id, false)?;
            }
        }
        Ok(())
    }

    pub(crate) fn far_constraints(
        &self,
        curve: EntityId,
        near: EntityId,
        far: EntityId,
    ) -> Vec<(ConstraintId, Constraint, bool)> {
        self.constraints_using(curve)
            .into_iter()
            .filter_map(|id| Some((id, self.constraint(id)?.clone(), !self.is_active(id))))
            .filter(|(_, constraint, _)| {
                let other = match constraint {
                    Constraint::Tangent(a, b)
                    | Constraint::Curvature(a, b)
                    | Constraint::Parallel(a, b)
                    | Constraint::Perpendicular(a, b)
                    | Constraint::Angle { from: a, to: b, .. } => {
                        if *a == curve {
                            *b
                        } else {
                            *a
                        }
                    }
                    _ => return false,
                };
                self.joined(far, other) && !self.joined(near, other)
            })
            .collect()
    }

    fn joined(&self, point: EntityId, other: EntityId) -> bool {
        let points = self.entity(other).map(Entity::points).unwrap_or_default();
        if points.contains(&point) {
            return true;
        }
        let touches = |candidate: EntityId| candidate == other || points.contains(&candidate);
        self.constraints_using(point)
            .into_iter()
            .any(|id| match self.constraint(id) {
                Some(Constraint::Coincident(a, b)) => {
                    (*a == point && touches(*b)) || (*b == point && touches(*a))
                }
                _ => false,
            })
    }

    fn join(&mut self, end: EntityId, cut: &Cut, trimmed: EntityId) -> Result<(), SketchError> {
        let joint = match cut.point {
            Some(point) if self.entity(point).is_some() => Joint::Point(point),
            _ => Joint::Curve(cut.cutter),
        };
        self.attach(end, joint, trimmed)
    }

    fn attach(&mut self, end: EntityId, joint: Joint, curve: EntityId) -> Result<(), SketchError> {
        match joint {
            Joint::Point(point) => {
                let implied: Vec<ConstraintId> = self
                    .constraints_using(point)
                    .into_iter()
                    .filter(|id| {
                        matches!(
                            self.constraint(*id),
                            Some(Constraint::Coincident(a, b))
                                if (*a == point && *b == curve) || (*b == point && *a == curve)
                        )
                    })
                    .collect();
                for id in implied {
                    self.remove_constraint(id)?;
                }
                self.add_constraint(Constraint::Coincident(end, point))?;
            }
            Joint::Curve(cutter) => {
                self.add_constraint(Constraint::Coincident(end, cutter))?;
            }
        }
        Ok(())
    }

    fn drop_if_unused(&mut self, point: EntityId) -> Result<(), SketchError> {
        if self.entity(point).is_none() || !self.entities_using(point).is_empty() {
            return Ok(());
        }
        for id in self.constraints_using(point) {
            self.remove_constraint(id)?;
        }
        self.remove_unused_entity(point)?;
        Ok(())
    }

    fn delete_curve(&mut self, curve: EntityId) -> Result<(), SketchError> {
        for id in self.constraints_using(curve) {
            self.remove_constraint(id)?;
        }
        let removed = self.remove_unused_entity(curve)?;
        let mut points = removed.points();
        points.dedup();
        for point in points {
            self.drop_if_unused(point)?;
        }
        Ok(())
    }

    fn check_free_end(&self, curve: EntityId, end: EntityId) -> Result<(), ExtendError> {
        let label = || self.entity_label(curve);
        if let Some(other) = self
            .entities_using(end)
            .into_iter()
            .find(|user| *user != curve)
        {
            return Err(ExtendError::Joined {
                entity: curve,
                label: label(),
                other: self.entity_label(other),
            });
        }
        for id in self.constraints_using(end) {
            match self.constraint(id) {
                Some(Constraint::Fix { .. }) => {
                    return Err(ExtendError::Fixed {
                        entity: curve,
                        label: label(),
                    });
                }
                Some(Constraint::Coincident(a, b)) => {
                    let other = if *a == end { *b } else { *a };
                    let other_is_point = other == EntityId::ORIGIN
                        || matches!(self.entity(other), Some(Entity::Point(_)));
                    if other_is_point {
                        let owner = self
                            .entities()
                            .find(|(_, entity)| entity.points().contains(&other))
                            .map_or(other, |(owner, _)| owner);
                        return Err(ExtendError::Joined {
                            entity: curve,
                            label: label(),
                            other: self.entity_label(owner),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn reach_straight(
        &self,
        curve: EntityId,
        from: Point2,
        direction: Vector2,
    ) -> Option<(Point2, EntityId, Reach)> {
        let carrier = Carrier::Line {
            through: from,
            direction,
        };
        let reach = from.x.abs().max(from.y.abs()).max(1.0);
        self.cutters(curve)
            .flat_map(|(target, shape)| {
                let tolerance = TOLERANCE * reach.max(shape.extent());
                shape
                    .crossings(carrier, tolerance)
                    .into_iter()
                    .map(move |point| (point, target, tolerance))
            })
            .map(|(point, target, tolerance)| ((point - from).dot(direction), target, tolerance))
            .filter(|(along, _, tolerance)| *along > *tolerance)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(along, target, _)| (from + direction * along, target, Reach::Straight))
    }

    fn reach_around(
        &self,
        curve: EntityId,
        arc: ArcGeometry,
        at_end: bool,
    ) -> Option<(Point2, EntityId, Reach)> {
        let carrier = Carrier::Circle {
            center: arc.center,
            radius: arc.radius,
        };
        let course = Course::Arc(arc);
        let room = TAU - arc.sweep;
        self.cutters(curve)
            .flat_map(|(target, shape)| {
                let tolerance = TOLERANCE * course.extent().max(shape.extent());
                shape
                    .crossings(carrier, tolerance)
                    .into_iter()
                    .map(move |point| (point, target, course.slack(tolerance)))
            })
            .filter_map(|(point, target, slack)| {
                let angle = direction_angle(point - arc.center);
                let turn = if at_end {
                    (angle - arc.end_angle()).rem_euclid(TAU)
                } else {
                    (arc.start_angle - angle).rem_euclid(TAU)
                };
                (turn > slack && turn < room - slack).then_some((turn, target))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(turn, target)| {
                let (start_angle, angle) = if at_end {
                    (arc.end_angle(), arc.end_angle() + turn)
                } else {
                    (arc.start_angle - turn, arc.start_angle - turn)
                };
                let reached = ArcGeometry {
                    start_angle,
                    sweep: turn,
                    ..arc
                };
                (arc.point_at(angle), target, Reach::Around(reached))
            })
    }

    fn end_point_at(&self, curve: EntityId, position: Point2) -> Option<EntityId> {
        let shape = self.shape_of(curve)?;
        let tolerance = TOLERANCE * shape.extent().max(1.0);
        end_point_near(self, curve, &shape, position, tolerance)
    }

    fn apply_extension(&mut self, extension: &Extension) -> Result<(), SketchError> {
        let curve = extension.curve;
        for id in self.constraints_using(extension.end) {
            self.remove_constraint(id)?;
        }
        let keeps: fn(&Constraint) -> bool = match self.entity(curve) {
            Some(Entity::Line { .. }) => keeps_length,
            _ => keeps_sweep,
        };
        let changed: Vec<ConstraintId> = self
            .constraints_using(curve)
            .into_iter()
            .filter(|id| self.constraint(*id).is_some_and(|c| !keeps(c)))
            .collect();
        for id in changed {
            self.remove_constraint(id)?;
        }
        self.replace_entity(extension.end, Entity::Point(extension.to))?;
        let joint = match extension.target_point {
            Some(point) => Joint::Point(point),
            None => Joint::Curve(extension.target),
        };
        self.attach(extension.end, joint, curve)
    }
}

pub(crate) fn keeps_length(constraint: &Constraint) -> bool {
    !matches!(
        constraint,
        Constraint::Midpoint { .. } | Constraint::Equal(..)
    )
}

pub(crate) fn keeps_sweep(constraint: &Constraint) -> bool {
    !matches!(
        constraint,
        Constraint::Midpoint { .. }
            | Constraint::ArcLength { .. }
            | Constraint::Sweep { .. }
            | Constraint::Angle { .. }
    )
}

fn overlap_ends(course: &Course, cutter: &Shape, tolerance: f64) -> Option<Vec<Point2>> {
    match (course, cutter) {
        (
            Course::Line { start, end },
            Shape::Segment {
                start: from,
                end: to,
            },
        ) => {
            let direction = (*end - *start).try_normalize()?;
            let on_line = |point: &Point2| direction.perp_dot(*point - *start).abs() <= tolerance;
            (on_line(from) && on_line(to)).then(|| vec![*from, *to])
        }
        (
            Course::Circle { center, radius },
            Shape::Circle {
                center: other,
                radius: size,
            },
        ) => same_circle(*center, *radius, *other, *size, tolerance).then(Vec::new),
        (Course::Arc(arc), Shape::Circle { center, radius }) => {
            same_circle(arc.center, arc.radius, *center, *radius, tolerance).then(Vec::new)
        }
        (Course::Circle { center, radius }, Shape::Arc(arc)) => {
            same_circle(*center, *radius, arc.center, arc.radius, tolerance).then(|| arc_ends(arc))
        }
        (Course::Arc(own), Shape::Arc(arc)) => {
            same_circle(own.center, own.radius, arc.center, arc.radius, tolerance)
                .then(|| arc_ends(arc))
        }
        _ => None,
    }
}

fn arc_ends(arc: &ArcGeometry) -> Vec<Point2> {
    vec![arc.point_at(arc.start_angle), arc.point_at(arc.end_angle())]
}

fn same_circle(center: Point2, radius: f64, other: Point2, size: f64, tolerance: f64) -> bool {
    center.distance(other) <= tolerance && (radius - size).abs() <= tolerance
}

fn end_point_near(
    sketch: &Sketch,
    curve: EntityId,
    shape: &Shape,
    position: Point2,
    tolerance: f64,
) -> Option<EntityId> {
    let (start_at, end_at) = shape.ends()?;
    let (start, end) = match sketch.entity(curve)? {
        Entity::Line { start, end } | Entity::Arc { start, end, .. } => (*start, *end),
        Entity::Spline { control_points } => (*control_points.first()?, *control_points.last()?),
        Entity::Point(_) | Entity::Circle { .. } => return None,
    };
    if start_at.distance(position) <= tolerance {
        Some(start)
    } else if end_at.distance(position) <= tolerance {
        Some(end)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
