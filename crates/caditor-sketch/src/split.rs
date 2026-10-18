use std::f64::consts::TAU;

use crate::{
    constraint::Constraint,
    curve::direction_angle,
    entity::Entity,
    id::{ConstraintId, EntityId},
    intersect,
    sketch::{Sketch, SketchError},
    trim::{keeps_length, keeps_sweep},
};

const TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SplitError {
    #[error(
        "{label} cannot be split; only lines, arcs, elliptical arcs and splines drawn from control \
         points can"
    )]
    NotSplittable { entity: EntityId, label: String },
    #[error(
        "{label} passes through its points, so it cannot be split without changing its shape; \
         trim it, or draw it from control points instead"
    )]
    FitSpline { entity: EntityId, label: String },
    #[error("{label} is closed, so one point does not split it; trim it open first")]
    ClosedSpline { entity: EntityId, label: String },
    #[error("{label} is not a point")]
    NotAPoint { entity: EntityId, label: String },
    #[error("{point} does not lie on {curve} between its ends")]
    NotOnCurve { point: String, curve: String },
    #[error("{label} follows the geometry it was projected from, so it cannot be split")]
    Projected { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

impl Sketch {
    pub fn check_split(&self, curve: EntityId, point: EntityId) -> Result<(), SplitError> {
        let not_splittable = || SplitError::NotSplittable {
            entity: curve,
            label: self.entity_label(curve),
        };
        if self.is_projected(curve) {
            return Err(SplitError::Projected {
                entity: curve,
                label: self.entity_label(curve),
            });
        }
        let at = match self.entity(point) {
            Some(Entity::Point(at)) => *at,
            _ => {
                return Err(SplitError::NotAPoint {
                    entity: point,
                    label: self.entity_label(point),
                });
            }
        };
        let not_on = || SplitError::NotOnCurve {
            point: self.entity_label(point),
            curve: self.entity_label(curve),
        };
        let entity = self.entity(curve).ok_or_else(not_splittable)?;
        if entity.points().contains(&point) {
            return Err(not_on());
        }
        let tolerance = TOLERANCE * at.abs().max_element().max(1.0);
        let inside = match entity {
            Entity::Line { .. } => {
                let (start, end) = self.line_endpoints(curve).ok_or_else(not_splittable)?;
                let along = end - start;
                let length = along.length();
                let direction = along.try_normalize().ok_or_else(not_on)?;
                let distance = (at - start).perp_dot(direction).abs();
                let reach = (at - start).dot(direction);
                distance <= tolerance && reach > tolerance && reach < length - tolerance
            }
            Entity::Arc { .. } => {
                let arc = self.arc(curve).ok_or_else(not_splittable)?;
                let off = (at.distance(arc.center) - arc.radius).abs();
                let turned = (direction_angle(at - arc.center) - arc.start_angle).rem_euclid(TAU);
                let slack = if arc.radius > 0.0 {
                    tolerance / arc.radius
                } else {
                    0.0
                };
                off <= tolerance && turned > slack && turned < arc.sweep - slack
            }
            Entity::EllipticalArc { .. } => {
                let ellipse = self.ellipse(curve).ok_or_else(not_splittable)?;
                let reach = ellipse.major_radius().max(ellipse.minor_radius);
                let off = intersect::ellipse_level(&ellipse, at).abs();
                let turned = (ellipse.parameter_of(at) - ellipse.start).rem_euclid(TAU);
                let slack = if reach > 0.0 { tolerance / reach } else { 0.0 };
                off <= tolerance && turned > slack && turned < ellipse.sweep - slack
            }
            Entity::Spline { kind, .. } => {
                if kind.passes_its_points() {
                    return Err(SplitError::FitSpline {
                        entity: curve,
                        label: self.entity_label(curve),
                    });
                }
                if kind.is_closed() {
                    return Err(SplitError::ClosedSpline {
                        entity: curve,
                        label: self.entity_label(curve),
                    });
                }
                self.spline_split_is_inside(curve, at, tolerance)
                    .map_err(SplitError::Edit)?
            }
            Entity::Point(_) | Entity::Circle { .. } | Entity::Ellipse { .. } => {
                return Err(not_splittable());
            }
        };
        if inside { Ok(()) } else { Err(not_on()) }
    }

    pub fn split_at(&mut self, curve: EntityId, point: EntityId) -> Result<EntityId, SplitError> {
        self.check_split(curve, point)?;
        let mut working = self.clone();
        let piece = working
            .split_curve(curve, point)
            .map_err(SplitError::Edit)?;
        *self = working;
        Ok(piece)
    }

    pub(crate) fn lies_on_major_axis(&self, curve: EntityId, point: EntityId) -> bool {
        let (Some(ellipse), Some(at)) = (self.ellipse(curve), self.point(point)) else {
            return false;
        };
        let across = (at - ellipse.center).dot(ellipse.axis().perp());
        across.abs() <= TOLERANCE * ellipse.major_radius().max(ellipse.minor_radius).max(1.0)
    }

    fn split_curve(&mut self, curve: EntityId, point: EntityId) -> Result<EntityId, SketchError> {
        let on_curve: Vec<ConstraintId> = self
            .constraints_using(point)
            .into_iter()
            .filter(|id| {
                matches!(
                    self.constraint(*id),
                    Some(Constraint::Coincident(a, b))
                        if (*a == point && *b == curve) || (*a == curve && *b == point)
                )
            })
            .collect();
        let at_middle = self.constraints_using(point).into_iter().any(|id| {
            matches!(
                self.constraint(id),
                Some(Constraint::Midpoint { point: middle, curve: of }) if *middle == point && *of == curve
            )
        });
        for id in on_curve {
            self.remove_constraint(id)?;
        }
        let entity = self
            .entity(curve)
            .cloned()
            .ok_or(SketchError::NoSuchEntity(curve))?;
        match entity {
            Entity::Line { start, end } => {
                let moved = self.far_constraints(curve, start, end);
                let moved_ids: Vec<ConstraintId> = moved.iter().map(|(id, _)| *id).collect();
                let directions: Vec<Constraint> = self
                    .constraints_using(curve)
                    .into_iter()
                    .filter_map(|id| match self.constraint(id)? {
                        Constraint::Horizontal(_) => Some(Constraint::Horizontal(curve)),
                        Constraint::Vertical(_) => Some(Constraint::Vertical(curve)),
                        _ => None,
                    })
                    .collect();
                let far_midpoints = self.restructure_leaving(
                    curve,
                    Entity::Line { start, end: point },
                    &moved_ids,
                    keeps_length,
                )?;
                let piece = self.add_piece(curve, Entity::Line { start: point, end })?;
                self.move_constraints(moved, curve, piece)?;
                self.settle_midpoints(&[piece], far_midpoints)?;
                if directions.is_empty() {
                    self.add_constraint(Constraint::Collinear(curve, piece))?;
                } else {
                    for direction in directions {
                        self.add_constraint(direction.with_entity_replaced(curve, piece))?;
                    }
                }
                if at_middle {
                    self.add_constraint(Constraint::Equal(curve, piece))?;
                }
                Ok(piece)
            }
            Entity::Arc { center, start, end } => {
                let moved = self.far_constraints(curve, start, end);
                let moved_ids: Vec<ConstraintId> = moved.iter().map(|(id, _)| *id).collect();
                let far_midpoints = self.restructure_leaving(
                    curve,
                    Entity::Arc {
                        center,
                        start,
                        end: point,
                    },
                    &moved_ids,
                    keeps_sweep,
                )?;
                let piece = self.add_piece(
                    curve,
                    Entity::Arc {
                        center,
                        start: point,
                        end,
                    },
                )?;
                self.move_constraints(moved, curve, piece)?;
                self.settle_midpoints(&[piece], far_midpoints)?;
                Ok(piece)
            }
            Entity::EllipticalArc {
                center,
                major,
                minor_radius,
                start,
                end,
            } => {
                let on_axis = self.lies_on_major_axis(curve, point);
                let moved = self.far_constraints(curve, start, end);
                let moved_ids: Vec<ConstraintId> = moved.iter().map(|(id, _)| *id).collect();
                self.restructure(
                    curve,
                    Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius,
                        start,
                        end: point,
                    },
                    &moved_ids,
                    keeps_sweep,
                )?;
                let piece = self.add_piece(
                    curve,
                    Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius,
                        start: point,
                        end,
                    },
                )?;
                self.move_constraints(moved, curve, piece)?;
                if on_axis {
                    self.add_constraint(Constraint::Equal(curve, piece))?;
                }
                Ok(piece)
            }
            Entity::Spline { .. } => self.split_spline(curve, point),
            Entity::Point(_) | Entity::Circle { .. } | Entity::Ellipse { .. } => {
                Err(SketchError::WrongKind {
                    entity: curve,
                    found: self.entity_label(curve),
                    needed: "a line, an arc, an elliptical arc or a spline",
                })
            }
        }
    }
}

#[cfg(test)]
mod spline_tests;
#[cfg(test)]
mod tests;
