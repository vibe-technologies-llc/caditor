use caditor_geometry::Point2;

use super::{Change, Course, CrossingPlan, Pick, Planned, other_crossings};
use crate::{
    constraint::Constraint,
    curve::BSpline,
    entity::Entity,
    fillet::{Corner, FilletError, TOLERANCE, whole},
    id::EntityId,
    intersect::{Shape, closest_spline_parameter},
    sketch::{Sketch, SketchError},
    trim::spline::SplineCarrier,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Spliced {
    curves: [EntityId; 2],
    corner: Point2,
    parts: [Part; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Plain(Course, Change),
    Spline(SplineChange),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SplineChange {
    Kept(EntityId),
    Trim {
        replaced: EntityId,
    },
    Extend {
        replaced: EntityId,
        at_end: bool,
        reached: f64,
    },
}

enum Joined {
    Plain(Course),
    Spline(Box<Drawn>),
}

struct Drawn {
    carrier: SplineCarrier,
    spline: BSpline,
}

impl Joined {
    fn shape(&self, reach: f64) -> Option<Shape> {
        match self {
            Self::Plain(Course::Line { start, end, .. }) => {
                let direction = (*end - *start).try_normalize()?;
                Some(Shape::Segment {
                    start: *start - direction * reach,
                    end: *start + direction * reach,
                })
            }
            Self::Plain(Course::Arc { arc, .. }) => Some(Shape::Circle {
                center: arc.center,
                radius: arc.radius,
            }),
            Self::Plain(Course::Circle {
                center_at, radius, ..
            }) => Some(Shape::Circle {
                center: *center_at,
                radius: *radius,
            }),
            Self::Plain(Course::Ellipse { ellipse, .. }) => Some(Shape::Ellipse(whole(ellipse))),
            Self::Spline(drawn) => Some(Shape::Spline(drawn.carrier.extended().clone())),
        }
    }

    fn extent(&self) -> f64 {
        match self {
            Self::Plain(course) => course.extent(),
            Self::Spline(_) => 1.0,
        }
    }

    fn distance_to(&self, point: Point2) -> f64 {
        match self {
            Self::Plain(course) => course.distance_to(point),
            Self::Spline(drawn) => drawn
                .spline
                .point_at(closest_spline_parameter(&drawn.spline, point))
                .distance(point),
        }
    }
}

impl Sketch {
    pub(super) fn is_spline(&self, curve: EntityId) -> bool {
        matches!(self.entity(curve), Some(Entity::Spline { .. }))
    }

    fn spliced_curve(&self, curve: EntityId, reach: f64) -> Result<Joined, FilletError> {
        if !self.is_spline(curve) {
            return self.course(curve).map(Joined::Plain);
        }
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
        let spline = self.spline(curve).ok_or_else(not_line_or_arc)?;
        let carrier = self
            .spline_carrier(curve, reach)
            .ok_or_else(not_line_or_arc)?;
        Ok(Joined::Spline(Box::new(Drawn { carrier, spline })))
    }

    fn joined_pair(&self, curves: [EntityId; 2]) -> Result<[Joined; 2], FilletError> {
        let [first, second] = curves;
        let reach = |curve: EntityId| match self.is_spline(curve) {
            true => 1.0,
            false => self.course(curve).map_or(1.0, |course| course.extent()),
        };
        let pair = [
            self.spliced_curve(first, reach(second))?,
            self.spliced_curve(second, reach(first))?,
        ];
        for (index, other) in [(0, 1), (1, 0)] {
            if let (Some(Joined::Plain(Course::Circle { .. })), Some(Joined::Spline(_))) =
                (pair.get(index), pair.get(other))
            {
                let circle = curves.get(index).copied().unwrap_or(first);
                let spline = curves.get(other).copied().unwrap_or(second);
                return Err(FilletError::CircleToSpline {
                    circle: self.entity_label(circle),
                    spline: self.entity_label(spline),
                });
            }
        }
        Ok(pair)
    }

    fn crossings_of_joined(&self, pair: &[Joined; 2]) -> Vec<Point2> {
        let [one, other] = pair;
        let reach = 4.0 * one.extent().max(other.extent()).max(1.0);
        let tolerance = TOLERANCE * reach;
        let (carrier, shape) = match (one, other) {
            (Joined::Spline(drawn), joined) | (joined, Joined::Spline(drawn)) => {
                (&drawn.carrier, joined.shape(reach))
            }
            _ => return Vec::new(),
        };
        shape
            .map(|shape| carrier.crossings(&shape, tolerance))
            .unwrap_or_default()
    }

    pub(super) fn spliced_plan(&self, picks: [Pick; 2]) -> Result<CrossingPlan, FilletError> {
        let [first, second] = picks;
        let pair = self.joined_pair([first.curve, second.curve])?;
        let crossings = self.crossings_of_joined(&pair);
        let reach = pair.iter().map(Joined::extent).fold(1.0, f64::max);
        let tolerance = TOLERANCE * reach;
        let corner = crossings
            .iter()
            .copied()
            .min_by(|a, b| {
                let distance = |at: Point2| at.distance(first.near) + at.distance(second.near);
                distance(*a).total_cmp(&distance(*b))
            })
            .ok_or_else(|| self.never_meet(first.curve, second.curve))?;
        if let Ok(existing) = self.corner_between(first.curve, second.curve)
            && existing.position.distance(corner) <= tolerance
        {
            return Ok(CrossingPlan(Planned::Existing(existing)));
        }
        let others = other_crossings(&crossings, corner, tolerance);
        let mut parts = Vec::with_capacity(2);
        for (pick, joined) in picks.iter().zip(&pair) {
            parts.push(match joined {
                Joined::Plain(course) => {
                    let change = course.change(corner, pick.near, &others, tolerance);
                    Part::Plain(*course, self.settled(pick.curve, change)?)
                }
                Joined::Spline(drawn) => {
                    Part::Spline(self.spline_change(pick, drawn, corner, tolerance)?)
                }
            });
        }
        let [one, other] = parts[..] else {
            return Err(self.never_meet(first.curve, second.curve));
        };
        Ok(CrossingPlan(Planned::Spliced(Box::new(Spliced {
            curves: [first.curve, second.curve],
            corner,
            parts: [one, other],
        }))))
    }

    fn spline_change(
        &self,
        pick: &Pick,
        drawn: &Drawn,
        corner: Point2,
        tolerance: f64,
    ) -> Result<SplineChange, FilletError> {
        let (first, last) = drawn.carrier.ends();
        let at_end = |end: EntityId| {
            self.point(end)
                .is_some_and(|point| point.distance(corner) <= tolerance)
        };
        if at_end(first) {
            return Ok(SplineChange::Kept(first));
        }
        if at_end(last) {
            return Ok(SplineChange::Kept(last));
        }
        let share = drawn.carrier.share_at(corner);
        let extend = |replaced: EntityId, at_end: bool| -> Result<SplineChange, FilletError> {
            self.check_free_end(pick.curve, replaced)
                .map_err(FilletError::Held)?;
            Ok(SplineChange::Extend {
                replaced,
                at_end,
                reached: drawn.carrier.knot_of(share),
            })
        };
        if share <= 0.0 {
            return extend(first, false);
        }
        if share >= 1.0 {
            return extend(last, true);
        }
        let picked = closest_spline_parameter(&drawn.spline, pick.near);
        Ok(SplineChange::Trim {
            replaced: if picked < share { last } else { first },
        })
    }

    pub(super) fn spliced_picks(
        &self,
        first: EntityId,
        second: EntityId,
    ) -> Result<[Pick; 2], FilletError> {
        let pair = self.joined_pair([first, second])?;
        let crossings = self.crossings_of_joined(&pair);
        let corner = crossings
            .iter()
            .copied()
            .min_by(|a, b| {
                let reach = |at: Point2| {
                    pair.iter()
                        .map(|joined| joined.distance_to(at))
                        .sum::<f64>()
                };
                reach(*a).total_cmp(&reach(*b))
            })
            .ok_or_else(|| self.never_meet(first, second))?;
        let tolerance = TOLERANCE * pair.iter().map(Joined::extent).fold(1.0, f64::max);
        let others = other_crossings(&crossings, corner, tolerance);
        let near = |joined: &Joined| match joined {
            Joined::Plain(course) => course.longer_side(corner, &others),
            Joined::Spline(drawn) => {
                let (start, end) = (drawn.spline.point_at(0.0), drawn.spline.point_at(1.0));
                if start.distance(corner) >= end.distance(corner) {
                    start
                } else {
                    end
                }
            }
        };
        let [one, other] = &pair;
        Ok([
            Pick {
                curve: first,
                near: near(one),
            },
            Pick {
                curve: second,
                near: near(other),
            },
        ])
    }

    pub(super) fn join_spliced(&mut self, spliced: &Spliced) -> Result<Corner, FilletError> {
        let mut working = self.clone();
        let shared = working.splice(spliced).map_err(FilletError::Edit)?;
        let joined = working.corner_at(shared)?;
        *self = working;
        Ok(joined)
    }

    fn splice(&mut self, spliced: &Spliced) -> Result<EntityId, SketchError> {
        let kept: Vec<EntityId> = spliced
            .parts
            .iter()
            .filter_map(|part| match *part {
                Part::Plain(_, Change::Kept(end)) | Part::Spline(SplineChange::Kept(end)) => {
                    Some(end)
                }
                _ => None,
            })
            .collect();
        let shared = match kept.first() {
            Some(end) => *end,
            None => self.add_point(spliced.corner),
        };
        if let [one, other] = kept[..]
            && one != other
        {
            self.add_constraint(Constraint::Coincident(one, other))?;
        }
        let mut far_ends = Vec::new();
        for (curve, part) in spliced.curves.into_iter().zip(&spliced.parts) {
            match *part {
                Part::Plain(course, change) => {
                    self.reshape_plain(curve, course, change, shared, &mut far_ends)?;
                }
                Part::Spline(SplineChange::Kept(_)) => {}
                Part::Spline(SplineChange::Trim { replaced }) => {
                    self.cut_spline_end(curve, replaced, shared)?;
                    self.drop_if_unused(replaced)?;
                }
                Part::Spline(SplineChange::Extend {
                    replaced,
                    at_end,
                    reached,
                }) => self.extend_spline_to(curve, replaced, at_end, reached, shared)?,
            }
        }
        Ok(shared)
    }
}
