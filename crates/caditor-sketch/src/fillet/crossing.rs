use std::f64::consts::{PI, TAU};

use caditor_geometry::Point2;

use super::{Corner, FilletError, TOLERANCE, centers, whole};
use crate::{
    constraint::Constraint,
    curve::{ArcGeometry, EllipseGeometry, direction_angle},
    entity::Entity,
    id::EntityId,
    intersect::{self, Carrier, Shape},
    sketch::{Sketch, SketchError},
    trim::{keeps_length, keeps_sweep, same_ellipse},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    pub curve: EntityId,
    pub near: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossingPlan(Planned);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Planned {
    Existing(Corner),
    Join {
        curves: [EntityId; 2],
        courses: [Course; 2],
        changes: [Change; 2],
        corner: Point2,
    },
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
    Ellipse {
        ellipse: EllipseGeometry,
        ends: [EntityId; 2],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Neighbour {
    turn: f64,
    at: Point2,
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
            Self::Ellipse { .. } => None,
        }
    }

    fn extent(&self) -> f64 {
        match *self {
            Self::Line { start, end, .. } => start.abs().max(end.abs()).max_element(),
            Self::Arc { arc, .. } => arc.center.abs().max_element() + arc.radius,
            Self::Circle {
                center_at, radius, ..
            } => center_at.abs().max_element() + radius,
            Self::Ellipse { ellipse, .. } => {
                ellipse.center.abs().max_element()
                    + ellipse.major_radius().max(ellipse.minor_radius)
            }
        }
    }

    fn has_length(&self, tolerance: f64) -> bool {
        match *self {
            Self::Line { start, end, .. } => start.distance(end) > tolerance,
            Self::Arc { arc, .. } => arc.radius > tolerance,
            Self::Circle { radius, .. } => radius > tolerance,
            Self::Ellipse { ellipse, .. } => {
                ellipse.major_radius() > tolerance && ellipse.minor_radius > tolerance
            }
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
            Self::Ellipse { ellipse, .. } => point.distance(ellipse.closest_point(point)),
        }
    }

    fn longer_side(&self, corner: Point2, others: &[Point2]) -> Point2 {
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
                arc.point_at(arc.start_angle + middle_of_longer_part(turned, arc.sweep))
            }
            Self::Ellipse { ellipse, .. } => {
                let turned = (ellipse.parameter_of(corner) - ellipse.start).rem_euclid(TAU);
                ellipse.point_at(ellipse.start + middle_of_longer_part(turned, ellipse.sweep))
            }
            Self::Circle { .. } => {
                let [one, other] = self.sides(corner, others);
                if one.distance(corner) >= other.distance(corner) {
                    one
                } else {
                    other
                }
            }
        }
    }

    fn sides(&self, corner: Point2, others: &[Point2]) -> [Point2; 2] {
        let (center_at, radius) = match *self {
            Self::Circle {
                center_at, radius, ..
            } => (center_at, radius),
            Self::Arc { arc, .. } => (arc.center, arc.radius),
            Self::Line { start, end, .. } => return [start, end],
            Self::Ellipse { ellipse, .. } => {
                return [
                    ellipse.point_at(ellipse.start),
                    ellipse.point_at(ellipse.end()),
                ];
            }
        };
        let circle = ArcGeometry::full_circle(center_at, radius);
        let from = direction_angle(corner - center_at);
        let [ahead, behind] = neighbours(center_at, corner, others)
            .map_or([PI, PI], |found| found.map(|neighbour| neighbour.turn));
        [
            circle.point_at(from + ahead / 2.0),
            circle.point_at(from + behind + (TAU - behind) / 2.0),
        ]
    }

    fn change(&self, corner: Point2, near: Point2, others: &[Point2], tolerance: f64) -> Change {
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
                Turning {
                    at: turned_to(corner),
                    picked: turned_to(near),
                    sweep: arc.sweep,
                    slack: tolerance / arc.radius,
                }
                .change(ends)
            }
            Self::Ellipse { ellipse, ends, .. } => {
                let turned_to =
                    |point: Point2| (ellipse.parameter_of(point) - ellipse.start).rem_euclid(TAU);
                Turning {
                    at: turned_to(corner),
                    picked: turned_to(near),
                    sweep: ellipse.sweep,
                    slack: tolerance / ellipse.major_radius().max(ellipse.minor_radius),
                }
                .change(ends)
            }
            Self::Circle { center_at, .. } => {
                let Some([ahead, behind]) = neighbours(center_at, corner, others) else {
                    return Change::Opened {
                        start_at_corner: true,
                        far: corner,
                    };
                };
                let from = direction_angle(corner - center_at);
                let picked = (direction_angle(near - center_at) - from).rem_euclid(TAU);
                let forward = picked <= ahead.turn
                    || (picked < behind.turn && picked - ahead.turn <= behind.turn - picked);
                Change::Opened {
                    start_at_corner: forward,
                    far: if forward { ahead.at } else { behind.at },
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Turning {
    at: f64,
    picked: f64,
    sweep: f64,
    slack: f64,
}

impl Turning {
    fn change(self, [first, last]: [EntityId; 2]) -> Change {
        let Self {
            at,
            picked,
            sweep,
            slack,
        } = self;
        if at <= slack || at >= TAU - slack {
            return Change::Kept(first);
        }
        if (at - sweep).abs() <= slack {
            return Change::Kept(last);
        }
        if at > sweep {
            return if at - sweep <= TAU - at {
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
        let picked = if picked <= sweep {
            picked
        } else if picked - sweep < TAU - picked {
            sweep
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
}

fn middle_of_longer_part(turned: f64, sweep: f64) -> f64 {
    if turned >= sweep {
        sweep / 2.0
    } else if turned >= sweep - turned {
        turned / 2.0
    } else {
        (turned + sweep) / 2.0
    }
}

fn neighbours(center: Point2, corner: Point2, others: &[Point2]) -> Option<[Neighbour; 2]> {
    let from = direction_angle(corner - center);
    let around: Vec<Neighbour> = others
        .iter()
        .map(|at| Neighbour {
            turn: (direction_angle(*at - center) - from).rem_euclid(TAU),
            at: *at,
        })
        .collect();
    let ahead = around
        .iter()
        .copied()
        .min_by(|a, b| a.turn.total_cmp(&b.turn))?;
    let behind = around
        .iter()
        .copied()
        .max_by(|a, b| a.turn.total_cmp(&b.turn))?;
    Some([ahead, behind])
}

impl Sketch {
    pub fn crossing_picks(
        &self,
        first: EntityId,
        second: EntityId,
    ) -> Result<[Pick; 2], FilletError> {
        let courses = [self.course(first)?, self.course(second)?];
        let tolerance = tolerance_of(&courses);
        let crossings = crossings_of(&courses, tolerance);
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
        let others = other_crossings(&crossings, corner, tolerance);
        let [one, other] = courses;
        let nears = match courses {
            [Course::Circle { .. }, Course::Circle { .. }] => [
                outside(&one, &other, corner, &others),
                outside(&other, &one, corner, &others),
            ],
            [Course::Circle { .. }, _] => {
                let leaving = other.longer_side(corner, &others);
                [widest(&one, corner, &others, leaving), leaving]
            }
            [_, Course::Circle { .. }] => {
                let leaving = one.longer_side(corner, &others);
                [leaving, widest(&other, corner, &others, leaving)]
            }
            _ => [
                one.longer_side(corner, &others),
                other.longer_side(corner, &others),
            ],
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
        let plan = self.crossing_plan(picks)?;
        self.join_planned(&plan)
    }

    pub fn join_planned(&mut self, plan: &CrossingPlan) -> Result<Corner, FilletError> {
        let (curves, courses, changes, corner) = match plan.0 {
            Planned::Existing(existing) => return Ok(existing),
            Planned::Join {
                curves,
                courses,
                changes,
                corner,
            } => (curves, courses, changes, corner),
        };
        let mut working = self.clone();
        let shared = working
            .join_curves(curves, &courses, &changes, corner)
            .map_err(FilletError::Edit)?;
        let joined = working.corner_at(shared)?;
        *self = working;
        Ok(joined)
    }

    pub fn crossing_plan(&self, picks: [Pick; 2]) -> Result<CrossingPlan, FilletError> {
        let [first, second] = picks;
        if first.curve == second.curve {
            return Err(FilletError::SameCurve {
                entity: first.curve,
                label: self.entity_label(first.curve),
            });
        }
        let courses = [self.course(first.curve)?, self.course(second.curve)?];
        let tolerance = tolerance_of(&courses);
        let crossings = crossings_of(&courses, tolerance);
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
            return Ok(CrossingPlan(Planned::Existing(existing)));
        }
        let others = other_crossings(&crossings, corner, tolerance);
        let changes = [
            courses[0].change(corner, first.near, &others, tolerance),
            courses[1].change(corner, second.near, &others, tolerance),
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
        Ok(CrossingPlan(Planned::Join {
            curves: [first.curve, second.curve],
            courses,
            changes,
            corner,
        }))
    }

    fn join_curves(
        &mut self,
        curves: [EntityId; 2],
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
        for ((curve, course), change) in curves.into_iter().zip(courses).zip(changes) {
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
                    self.restructure(curve, line, &[], keeps_length)?;
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
                    self.restructure(curve, arc, &[], keeps_sweep)?;
                    self.drop_if_unused(moved)?;
                }
                (
                    Course::Ellipse {
                        ends: [start, end], ..
                    },
                    Change::Moved { end: moved, .. },
                ) => {
                    let Some(&Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius,
                        ..
                    }) = self.entity(curve)
                    else {
                        return Err(SketchError::NoSuchEntity(curve));
                    };
                    let [kept_start, kept_end] = if moved == start {
                        [shared, end]
                    } else {
                        [start, shared]
                    };
                    let arc = Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius,
                        start: kept_start,
                        end: kept_end,
                    };
                    self.drop_length_dimensions(start, end)?;
                    self.restructure(curve, arc, &[], keeps_sweep)?;
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
                    self.restructure(curve, arc, &[], |_| true)?;
                    far_ends.push((far_end, curve));
                }
                (_, Change::Moved { .. } | Change::Reached { .. } | Change::Opened { .. }) => {
                    return Err(SketchError::NoSuchEntity(curve));
                }
            }
        }
        for (far_end, curve) in far_ends {
            let other = curves
                .into_iter()
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
            Entity::EllipticalArc { start, end, .. } => Course::Ellipse {
                ellipse: self.ellipse(curve).ok_or_else(not_line_or_arc)?,
                ends: [start, end],
            },
            Entity::Point(_) | Entity::Spline { .. } | Entity::Ellipse { .. } => {
                return Err(not_line_or_arc());
            }
        };
        let tolerance = TOLERANCE * course.extent().max(1.0);
        if course.has_length(tolerance) {
            Ok(course)
        } else {
            Err(not_line_or_arc())
        }
    }

    fn never_meet(&self, first: EntityId, second: EntityId) -> FilletError {
        FilletError::NeverMeet {
            first: self.entity_label(first),
            second: self.entity_label(second),
        }
    }
}

fn crossings_of(courses: &[Course; 2], tolerance: f64) -> Vec<Point2> {
    match courses {
        [
            Course::Ellipse { ellipse: one, .. },
            Course::Ellipse { ellipse: other, .. },
        ] => {
            let (one, other) = (whole(one), whole(other));
            if same_ellipse(&one, &other, tolerance) {
                Vec::new()
            } else {
                intersect::ellipse_ellipse(&one, &other, tolerance)
            }
        }
        [Course::Ellipse { ellipse, .. }, other] | [other, Course::Ellipse { ellipse, .. }] => {
            other.carrier().map_or_else(Vec::new, |carrier| {
                intersect::crossings(carrier, &Shape::Ellipse(whole(ellipse)), tolerance)
            })
        }
        [one, other] => match (one.carrier(), other.carrier()) {
            (Some(one), Some(other)) => centers(one, other, tolerance),
            _ => Vec::new(),
        },
    }
}

fn outside(circle: &Course, other: &Course, corner: Point2, others: &[Point2]) -> Point2 {
    let center = match *other {
        Course::Circle { center_at, .. } => center_at,
        Course::Arc { arc, .. } => arc.center,
        Course::Ellipse { ellipse, .. } => ellipse.center,
        Course::Line { start, .. } => start,
    };
    let [one, two] = circle.sides(corner, others);
    if one.distance(center) >= two.distance(center) {
        one
    } else {
        two
    }
}

fn widest(circle: &Course, corner: Point2, others: &[Point2], toward: Point2) -> Point2 {
    let leaving = (toward - corner).normalize_or_zero();
    let [one, two] = circle.sides(corner, others);
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

fn other_crossings(crossings: &[Point2], corner: Point2, tolerance: f64) -> Vec<Point2> {
    crossings
        .iter()
        .copied()
        .filter(|crossing| crossing.distance(corner) > tolerance)
        .collect()
}
