use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::{Constraint, MAX_LENGTH},
    curve::{BSpline, Faceting},
    entity::Entity,
    id::EntityId,
    sketch::{Sketch, SketchError},
    solve::joined_ends,
};

const TOLERANCE: f64 = 1e-9;
const TANGENT_LEG: f64 = 1.0 / 3.0;
const CURVATURE_LEG: f64 = 1.0 / 5.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Continuity {
    #[default]
    Tangent,
    Curvature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendEnd {
    pub curve: EntityId,
    pub point: EntityId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlendCurve {
    pub control_points: Vec<Point2>,
}

impl BlendCurve {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        BSpline::clamped(self.control_points.clone())
            .map(|spline| spline.faceted(faceting))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BlendError {
    #[error("{label} has no end to blend from; choose the end of a line, arc or spline")]
    NoEnds { entity: EntityId, label: String },
    #[error("that point is not an end of {label}")]
    NotAnEnd { curve: EntityId, label: String },
    #[error("both ends are on {label}; a blend joins the ends of two different curves")]
    SameCurve { curve: EntityId, label: String },
    #[error("the ends of {first} and {second} already meet, so there is no gap to blend")]
    EndsMeet { first: String, second: String },
    #[error("{label} has no length at that end, so there is no direction to blend along")]
    NoDirection { curve: EntityId, label: String },
    #[error(
        "{first} and {second} already meet elsewhere, so the blend could not keep its two ends apart"
    )]
    AlreadyJoined { first: String, second: String },
    #[error("the blend reaches further than {} m from the origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Leaving {
    at: Point2,
    direction: Vector2,
    curvature: f64,
}

impl Sketch {
    pub fn ends_of_curve(&self, curve: EntityId) -> Option<[EntityId; 2]> {
        match self.entity(curve)? {
            Entity::Line { start, end } | Entity::Arc { start, end, .. } => Some([*start, *end]),
            Entity::Spline { control_points } => {
                Some([*control_points.first()?, *control_points.last()?])
            }
            Entity::Point(_) | Entity::Circle { .. } => None,
        }
    }

    pub fn blend_curve(
        &self,
        first: BlendEnd,
        second: BlendEnd,
        continuity: Continuity,
    ) -> Result<BlendCurve, BlendError> {
        if first.curve == second.curve {
            return Err(BlendError::SameCurve {
                curve: first.curve,
                label: self.entity_label(first.curve),
            });
        }
        let (from, to) = (self.leaving(first)?, self.leaving(second)?);
        let span = from.at.distance(to.at);
        let extent = from.at.abs().max(to.at.abs()).max_element().max(1.0);
        if span <= TOLERANCE * extent {
            return Err(BlendError::EndsMeet {
                first: self.entity_label(first.curve),
                second: self.entity_label(second.curve),
            });
        }
        let control_points = match continuity {
            Continuity::Tangent => {
                let leg = span * TANGENT_LEG;
                vec![
                    from.at,
                    from.at + from.direction * leg,
                    to.at + to.direction * leg,
                    to.at,
                ]
            }
            Continuity::Curvature => {
                let bend = unit_bend();
                let [start, after, bent] = curving_legs(from, span, bend);
                let [end, before, turned] = curving_legs(to, span, bend);
                vec![start, after, bent, turned, before, end]
            }
        };
        if control_points
            .iter()
            .any(|point| !point.is_finite() || point.abs().max_element() > MAX_LENGTH)
        {
            return Err(BlendError::OutOfReach);
        }
        Ok(BlendCurve { control_points })
    }

    pub fn blend(
        &mut self,
        first: BlendEnd,
        second: BlendEnd,
        continuity: Continuity,
    ) -> Result<EntityId, BlendError> {
        let curve = self.blend_curve(first, second, continuity)?;
        let mut working = self.clone();
        let made = working.add_spline(&curve.control_points);
        let [start, end] = working
            .ends_of_curve(made)
            .ok_or(BlendError::Edit(SketchError::TooFewControlPoints))?;
        let edit = |result: Result<_, SketchError>| result.map_err(BlendError::Edit);
        edit(working.add_constraint(Constraint::Coincident(start, first.point)))?;
        edit(working.add_constraint(Constraint::Coincident(end, second.point)))?;
        for (own, picked) in [(start, first), (end, second)] {
            match joined_ends(&working, made, picked.curve) {
                Some((joined, theirs))
                    if joined == own && theirs.is_none_or(|theirs| theirs == picked.point) => {}
                _ => {
                    return Err(BlendError::AlreadyJoined {
                        first: self.entity_label(first.curve),
                        second: self.entity_label(second.curve),
                    });
                }
            }
        }
        for picked in [first, second] {
            edit(working.add_constraint(Constraint::Tangent(made, picked.curve)))?;
        }
        if continuity == Continuity::Curvature {
            for picked in [first, second] {
                edit(working.add_constraint(Constraint::Curvature(made, picked.curve)))?;
            }
        }
        *self = working;
        Ok(made)
    }

    fn leaving(&self, end: BlendEnd) -> Result<Leaving, BlendError> {
        let label = || self.entity_label(end.curve);
        let Some([start, finish]) = self.ends_of_curve(end.curve) else {
            return Err(BlendError::NoEnds {
                entity: end.curve,
                label: label(),
            });
        };
        let at_start = if end.point == start {
            true
        } else if end.point == finish {
            false
        } else {
            return Err(BlendError::NotAnEnd {
                curve: end.curve,
                label: label(),
            });
        };
        let missing = || BlendError::Edit(SketchError::MissingEntity(end.curve));
        let (at, along, curvature) = match self.entity(end.curve) {
            Some(Entity::Line { .. }) => {
                let (first, last) = self.line_endpoints(end.curve).ok_or_else(missing)?;
                if at_start {
                    (first, first - last, 0.0)
                } else {
                    (last, last - first, 0.0)
                }
            }
            Some(Entity::Arc { .. }) => {
                let arc = self.arc(end.curve).ok_or_else(missing)?;
                let angle = if at_start {
                    arc.start_angle
                } else {
                    arc.end_angle()
                };
                let counter_clockwise = Vector2::new(-angle.sin(), angle.cos());
                let curvature = 1.0 / arc.radius;
                if at_start {
                    (arc.point_at(angle), -counter_clockwise, -curvature)
                } else {
                    (arc.point_at(angle), counter_clockwise, curvature)
                }
            }
            Some(Entity::Spline { .. }) => {
                let spline = self.spline(end.curve).ok_or_else(missing)?;
                let parameter = if at_start { 0.0 } else { 1.0 };
                let [tangent, bend] = spline.derivatives(parameter);
                let speed = tangent.length();
                let curvature = tangent.perp_dot(bend) / (speed * speed * speed);
                let at = spline.point_at(parameter);
                if at_start {
                    (at, -tangent, -curvature)
                } else {
                    (at, tangent, curvature)
                }
            }
            Some(Entity::Point(_) | Entity::Circle { .. }) | None => return Err(missing()),
        };
        let length = along.length();
        let scale = at.abs().max_element().max(1.0);
        if !(length.is_finite() && length > TOLERANCE * scale) {
            return Err(BlendError::NoDirection {
                curve: end.curve,
                label: label(),
            });
        }
        Ok(Leaving {
            at,
            direction: along / length,
            curvature: if curvature.is_finite() {
                curvature
            } else {
                0.0
            },
        })
    }
}

fn unit_bend() -> f64 {
    let template = [
        Point2::new(0.0, 0.0),
        Point2::new(1.0, 0.0),
        Point2::new(2.0, 1.0),
        Point2::new(3.0, 1.0),
        Point2::new(4.0, 1.0),
        Point2::new(5.0, 1.0),
    ];
    BSpline::clamped(template.to_vec()).map_or(1.0, |spline| {
        let [tangent, bend] = spline.derivatives(0.0);
        tangent.perp_dot(bend) / tangent.length().powi(3)
    })
}

fn curving_legs(leaving: Leaving, span: f64, unit_bend: f64) -> [Point2; 3] {
    let tight = unit_bend / leaving.curvature.abs();
    let leg = (span * CURVATURE_LEG).min(tight);
    let offset = leaving.curvature * leg * leg / unit_bend;
    let along = leaving.direction;
    [
        leaving.at,
        leaving.at + along * leg,
        leaving.at + along * (2.0 * leg) + along.perp() * offset,
    ]
}

#[cfg(test)]
mod tests;
