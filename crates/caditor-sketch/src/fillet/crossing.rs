use std::f64::consts::{PI, TAU};

use caditor_geometry::Point2;

use super::{Corner, FilletError, TOLERANCE, centers};
use crate::{
    constraint::Constraint,
    curve::{ArcGeometry, direction_angle},
    entity::Entity,
    id::EntityId,
    intersect::Carrier,
    sketch::{Sketch, SketchError},
    trim::{keeps_length, keeps_sweep},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub curve: EntityId,
    pub near: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Course {
    Line {
        start: Point2,
        end: Point2,
        ends: [EntityId; 2],
    },
    Arc {
        arc: ArcGeometry,
        center: EntityId,
        ends: [EntityId; 2],
    },
    Circle {
        center_at: Point2,
        radius: f64,
        center: EntityId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Change {
    Kept(EntityId),
    Moved { end: EntityId, beyond: bool },
    Reached { nearer: EntityId, other: EntityId },
    Opened { start_at_corner: bool, far: Point2 },
}

impl Course {
    fn carrier(&self) -> Option<Carrier> {
        match *self {
            Self::Line { start, end, .. } => Some(Carrier::Line {
                through: start,
                direction: (end - start).try_normalize()?,
            }),
            Self::Arc { arc, .. } => Some(Carrier::Circle {
                center: arc.center,
                radius: arc.radius,
            }),
            Self::Circle {
                center_at, radius, ..
            } => Some(Carrier::Circle {
                center: center_at,
                radius,
            }),
        }
    }

    fn extent(&self) -> f64 {
        match *self {
            Self::Line { start, end, .. } => start.abs().max(end.abs()).max_element(),
            Self::Arc { arc, .. } => arc.center.abs().max_element() + arc.radius,
            Self::Circle {
                center_at, radius, ..
            } => center_at.abs().max_element() + radius,
        }
    }

    fn distance_to(&self, point: Point2) -> f64 {
        match *self {
            Self::Line { start, end, .. } => {
                let span = end - start;
                let share = ((point - start).dot(span) / span.length_squared()).clamp(0.0, 1.0);
                point.distance(start + span * share)
            }
            Self::Arc { arc, .. } => {
                let turned =
                    (direction_angle(point - arc.center) - arc.start_angle).rem_euclid(TAU);
                if turned <= arc.sweep {
                    (point.distance(arc.center) - arc.radius).abs()
                } else {
                    point
                        .distance(arc.point_at(arc.start_angle))
                        .min(point.distance(arc.point_at(arc.end_angle())))
                }
            }
            Self::Circle {
                center_at, radius, ..
            } => (point.distance(center_at) - radius).abs(),
        }
    }

    fn longer_side(&self, corner: Point2, far: Option<Point2>) -> Point2 {
        match *self {
            Self::Line { start, end, .. } => {
                if start.distance(corner) >= end.distance(corner) {
                    start
                } else {
                    end
                }
            }
            Self::Arc { arc, .. } => {
                let turned =
                    (direction_angle(corner - arc.center) - arc.start_angle).rem_euclid(TAU);
                let middle = if turned >= arc.sweep {
                    arc.sweep / 2.0
                } else if turned >= arc.sweep - turned {
                    turned / 2.0
                } else {
                    (turned + arc.sweep) / 2.0
                };
                arc.point_at(arc.start_angle + middle)
            }
            Self::Circle { .. } => {
                let [one, other] = self.sides(corner, far);
                if one.distance(corner) >= other.distance(corner) {
                    one
                } else {
                    other
                }
            }
        }
    }

    fn sides(&self, corner: Point2, far: Option<Point2>) -> [Point2; 2] {
        let (center_at, radius) = match *self {
            Self::Circle {
                center_at, radius, ..
            } => (center_at, radius),
            Self::Arc { arc, .. } => (arc.center, arc.radius),
            Self::Line { start, end, .. } => return [start, end],
        };
        let circle = ArcGeometry::full_circle(center_at, radius);
        let from = direction_angle(corner - center_at);
        let gap = far.map_or(PI, |far| {
            (direction_angle(far - center_at) - from).rem_euclid(TAU)
        });
        [
            circle.point_at(from + gap / 2.0),
            circle.point_at(from + gap + (TAU - gap) / 2.0),
        ]
    }

    fn change(&self, corner: Point2, near: Point2, far: Option<Point2>, tolerance: f64) -> Change {
        match *self {
            Self::Line { start, end, ends } => {
                let span = end - start;
                let length = span.length();
                let along = |point: Point2| (point - start).dot(span) / (length * length);
                let (at, picked) = (along(corner), along(near).clamp(0.0, 1.0));
                let slack = tolerance / length;
                match ends {
                    [first, _] if at.abs() <= slack => Change::Kept(first),
                    [_, last] if (at - 1.0).abs() <= slack => Change::Kept(last),
                    [first, _] if picked > at => Change::Moved {
                        end: first,
                        beyond: at < 0.0,
                    },
                    [_, last] => Change::Moved {
                        end: last,
                        beyond: at > 1.0,
                    },
                }
            }
            Self::Arc { arc, ends, .. } => {
                let turned_to = |point: Point2| {
                    (direction_angle(point - arc.center) - arc.start_angle).rem_euclid(TAU)
                };
                let at = turned_to(corner);
                let slack = tolerance / arc.radius;
                let [first, last] = ends;
                if at <= slack || at >= TAU - slack {
                    return Change::Kept(first);
                }
                if (at - arc.sweep).abs() <= slack {
                    return Change::Kept(last);
                }
                if at > arc.sweep {
                    return if at - arc.sweep <= TAU - at {
                        Change::Reached {
                            nearer: last,
                            other: first,
                        }
                    } else {
                        Change::Reached {
                            nearer: first,
                            other: last,
                        }
                    };
                }
                let picked = turned_to(near);
                let picked = if picked <= arc.sweep {
                    picked
                } else if picked - arc.sweep < TAU - picked {
                    arc.sweep
                } else {
                    0.0
                };
                if picked < at {
                    Change::Moved {
                        end: last,
                        beyond: false,
                    }
                } else {
                    Change::Moved {
                        end: first,
                        beyond: false,
                    }
                }
            }
            Self::Circle { center_at, .. } => {
                let far = far.unwrap_or(corner);
                let from = direction_angle(corner - center_at);
                let gap = (direction_angle(far - center_at) - from).rem_euclid(TAU);
                let picked = (direction_angle(near - center_at) - from).rem_euclid(TAU);
                Change::Opened {
                    start_at_corner: picked <= gap,
                    far,
                }
            }
        }
    }
}

impl Sketch {
    pub fn crossing_picks(
        &self,
        first: EntityId,
        second: EntityId,
    ) -> Result<[Pick; 2], FilletError> {
        let courses = [self.course(first)?, self.course(second)?];
        let tolerance = tolerance_of(&courses);
        let crossings = self.crossings_of(&courses, [first, second], tolerance)?;
        let corner = crossings
            .iter()
            .copied()
            .min_by(|a, b| {
                let reach = |at: Point2| {
                    courses
                        .iter()
                        .map(|course| course.distance_to(at))
                        .sum::<f64>()
                };
                reach(*a).total_cmp(&reach(*b))
            })
            .ok_or_else(|| self.never_meet(first, second))?;
        let far = other_crossing(&crossings, corner, tolerance);
        let [one, other] = courses;
        let nears = match courses {
            [Course::Circle { .. }, Course::Circle { .. }] => [
                outside(&one, &other, corner, far),
                outside(&other, &one, corner, far),
            ],
            [Course::Circle { .. }, _] => {
                let leaving = other.longer_side(corner, far);
                [widest(&one, corner, far, leaving), leaving]
            }
            [_, Course::Circle { .. }] => {
                let leaving = one.longer_side(corner, far);
                [leaving, widest(&other, corner, far, leaving)]
            }
            _ => [one.longer_side(corner, far), other.longer_side(corner, far)],
        };
        let [near_first, near_second] = nears;
        Ok([
            Pick {
                curve: first,
                near: near_first,
            },
            Pick {
                curve: second,
                near: near_second,
            },
        ])
    }

    pub fn join_at_crossing(&mut self, picks: [Pick; 2]) -> Result<Corner, FilletError> {
        let [first, second] = picks;
        if first.curve == second.curve {
            return Err(FilletError::SameCurve {
                entity: first.curve,
                label: self.entity_label(first.curve),
            });
        }
        let courses = [self.course(first.curve)?, self.course(second.curve)?];
        let tolerance = tolerance_of(&courses);
        let crossings = self.crossings_of(&courses, [first.curve, second.curve], tolerance)?;
        let corner = crossings
            .iter()
            .copied()
            .min_by(|a, b| {
                let reach = |at: Point2| at.distance(first.near) + at.distance(second.near);
                reach(*a).total_cmp(&reach(*b))
            })
            .ok_or_else(|| self.never_meet(first.curve, second.curve))?;
        if let Ok(existing) = self.corner_between(first.curve, second.curve)
            && existing.position.distance(corner) <= tolerance
        {
            return Ok(existing);
        }
        let far = other_crossing(&crossings, corner, tolerance);
        let changes = [
            courses[0].change(corner, first.near, far, tolerance),
            courses[1].change(corner, second.near, far, tolerance),
        ];
        if changes
            .iter()
            .any(|change| matches!(change, Change::Opened { far: at, .. } if at.distance(corner) <= tolerance))
        {
            return Err(FilletError::NoCorner {
                first: self.entity_label(first.curve),
                second: self.entity_label(second.curve),
            });
        }
        let mut changes = changes;
        for (pick, change) in picks.iter().zip(changes.iter_mut()) {
            match *change {
                Change::Moved { end, beyond: true } => {
                    self.check_free_end(pick.curve, end)
                        .map_err(FilletError::Held)?;
                }
                Change::Reached { nearer, other } => {
                    let end = match self.check_free_end(pick.curve, nearer) {
                        Ok(()) => nearer,
                        Err(held) => {
                            self.check_free_end(pick.curve, other)
                                .map_err(|_| FilletError::Held(held))?;
                            other
                        }
                    };
                    *change = Change::Moved { end, beyond: true };
                }
                Change::Kept(_) | Change::Moved { .. } | Change::Opened { .. } => {}
            }
        }
        let mut working = self.clone();
        let shared = working
            .join_curves(&picks, &courses, &changes, corner)
            .map_err(FilletError::Edit)?;
        let joined = working.corner_at(shared)?;
        *self = working;
        Ok(joined)
    }

    fn join_curves(
        &mut self,
        picks: &[Pick; 2],
        courses: &[Course; 2],
        changes: &[Change; 2],
        corner: Point2,
    ) -> Result<EntityId, SketchError> {
        let kept: Vec<EntityId> = changes
            .iter()
            .filter_map(|change| match change {
                Change::Kept(end) => Some(*end),
                _ => None,
            })
            .collect();
        let shared = match kept.first() {
            Some(end) => *end,
            None => self.add_point(corner),
        };
        if let [one, other] = kept[..]
            && one != other
        {
            self.add_constraint(Constraint::Coincident(one, other))?;
        }
        let mut far_ends = Vec::new();
        for ((pick, course), change) in picks.iter().zip(courses).zip(changes) {
            match (*course, *change) {
                (_, Change::Kept(_)) => {}
                (
                    Course::Line {
                        ends: [start, end], ..
                    },
                    Change::Moved { end: moved, .. },
                ) => {
                    let line = if moved == start {
                        Entity::Line { start: shared, end }
                    } else {
                        Entity::Line { start, end: shared }
                    };
                    self.drop_length_dimensions(start, end)?;
                    self.restructure(pick.curve, line, &[], keeps_length)?;
                    self.drop_if_unused(moved)?;
                }
                (
                    Course::Arc {
                        center,
                        ends: [start, end],
                        ..
                    },
                    Change::Moved { end: moved, .. },
                ) => {
                    let arc = if moved == start {
                        Entity::Arc {
                            center,
                            start: shared,
                            end,
                        }
                    } else {
                        Entity::Arc {
                            center,
                            start,
                            end: shared,
                        }
                    };
                    self.drop_length_dimensions(start, end)?;
                    self.restructure(pick.curve, arc, &[], keeps_sweep)?;
                    self.drop_if_unused(moved)?;
                }
                (
                    Course::Circle { center, .. },
                    Change::Opened {
                        start_at_corner,
                        far,
                    },
                ) => {
                    let far_end = self.add_point(far);
                    let arc = if start_at_corner {
                        Entity::Arc {
                            center,
                            start: shared,
                            end: far_end,
                        }
                    } else {
                        Entity::Arc {
                            center,
                            start: far_end,
                            end: shared,
                        }
                    };
                    self.restructure(pick.curve, arc, &[], |_| true)?;
                    far_ends.push((far_end, pick.curve));
                }
                (_, Change::Moved { .. } | Change::Reached { .. } | Change::Opened { .. }) => {
                    return Err(SketchError::NoSuchEntity(pick.curve));
                }
            }
        }
        for (far_end, curve) in far_ends {
            let other = picks
                .iter()
                .map(|pick| pick.curve)
                .find(|other| *other != curve)
                .ok_or(SketchError::NoSuchEntity(curve))?;
            self.add_constraint(Constraint::Coincident(far_end, other))?;
        }
        Ok(shared)
    }

    fn course(&self, curve: EntityId) -> Result<Course, FilletError> {
        let label = || self.entity_label(curve);
        if self.is_projected(curve) {
            return Err(FilletError::Projected {
                entity: curve,
                label: label(),
            });
        }
        let not_line_or_arc = || FilletError::NotLineOrArc {
            entity: curve,
            label: label(),
        };
        let course = match *self.entity(curve).ok_or_else(not_line_or_arc)? {
            Entity::Line { start, end } => Course::Line {
                start: self.point(start).ok_or_else(not_line_or_arc)?,
                end: self.point(end).ok_or_else(not_line_or_arc)?,
                ends: [start, end],
            },
            Entity::Arc { center, start, end } => Course::Arc {
                arc: self.arc(curve).ok_or_else(not_line_or_arc)?,
                center,
                ends: [start, end],
            },
            Entity::Circle { center, radius } => Course::Circle {
                center_at: self.point(center).ok_or_else(not_line_or_arc)?,
                radius,
                center,
            },
            Entity::EllipticalArc { .. } => {
                return Err(FilletError::NotCrossable {
                    entity: curve,
                    label: label(),
                });
            }
            Entity::Point(_) | Entity::Spline { .. } | Entity::Ellipse { .. } => {
                return Err(not_line_or_arc());
            }
        };
        let tolerance = TOLERANCE * course.extent().max(1.0);
        let has_length = match course {
            Course::Line { start, end, .. } => start.distance(end) > tolerance,
            Course::Arc { arc, .. } => arc.radius > tolerance,
            Course::Circle { radius, .. } => radius > tolerance,
        };
        if has_length {
            Ok(course)
        } else {
            Err(not_line_or_arc())
        }
    }

    fn crossings_of(
        &self,
        courses: &[Course; 2],
        curves: [EntityId; 2],
        tolerance: f64,
    ) -> Result<Vec<Point2>, FilletError> {
        let [first, second] = curves;
        match (courses[0].carrier(), courses[1].carrier()) {
            (Some(one), Some(other)) => Ok(centers(one, other, tolerance)),
            _ => Err(self.never_meet(first, second)),
        }
    }

    fn never_meet(&self, first: EntityId, second: EntityId) -> FilletError {
        FilletError::NeverMeet {
            first: self.entity_label(first),
            second: self.entity_label(second),
        }
    }
}

fn outside(circle: &Course, other: &Course, corner: Point2, far: Option<Point2>) -> Point2 {
    let center = match *other {
        Course::Circle { center_at, .. } => center_at,
        Course::Arc { arc, .. } => arc.center,
        Course::Line { start, .. } => start,
    };
    let [one, two] = circle.sides(corner, far);
    if one.distance(center) >= two.distance(center) {
        one
    } else {
        two
    }
}

fn widest(circle: &Course, corner: Point2, far: Option<Point2>, toward: Point2) -> Point2 {
    let leaving = (toward - corner).normalize_or_zero();
    let [one, two] = circle.sides(corner, far);
    let opening = |side: Point2| (side - corner).normalize_or_zero().dot(leaving);
    if opening(one) <= opening(two) {
        one
    } else {
        two
    }
}

fn tolerance_of(courses: &[Course; 2]) -> f64 {
    let extent = courses.iter().map(Course::extent).fold(1.0, f64::max);
    TOLERANCE * extent
}

fn other_crossing(crossings: &[Point2], corner: Point2, tolerance: f64) -> Option<Point2> {
    crossings
        .iter()
        .copied()
        .find(|crossing| crossing.distance(corner) > tolerance)
}
