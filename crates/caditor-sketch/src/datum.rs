use std::cmp::Ordering;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::Point2;

use crate::{
    constraint::Constraint,
    entity::Entity,
    id::EntityId,
    inference::{InferenceError, Kept, Tolerance},
    sketch::Sketch,
};

impl Sketch {
    pub fn datum_dimensions<F>(
        &self,
        datum: EntityId,
        tolerance: Tolerance,
        value_of: &F,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Kept, InferenceError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let Some(origin) = self.point(datum) else {
            return Err(InferenceError::NotAPoint {
                entity: datum,
                label: self.entity_label(datum),
            });
        };
        let mut candidates = Vec::new();
        if datum != EntityId::ORIGIN && !self.is_fixed(datum) {
            candidates.extend(offsets(
                EntityId::ORIGIN,
                Point2::ZERO,
                datum,
                origin,
                tolerance,
            ));
        }
        candidates.extend(self.sizes());
        let mut points: Vec<(f64, EntityId, Point2)> = self
            .entities()
            .filter_map(|(id, entity)| match *entity {
                Entity::Point(at) if id != datum && !self.is_fixed(id) => {
                    Some((at.distance(origin), id, at))
                }
                _ => None,
            })
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (_, point, at) in points {
            candidates.extend(offsets(datum, origin, point, at, tolerance));
        }
        self.keep_holding(vec![candidates], tolerance, value_of, cancelled)
    }

    fn sizes(&self) -> Vec<Constraint> {
        self.entities()
            .filter(|(id, _)| !self.is_fixed(*id))
            .filter_map(|(id, entity)| match *entity {
                Entity::Circle { radius, .. } => Some(Constraint::Diameter {
                    entity: id,
                    value: millimetres(2.0 * radius),
                }),
                Entity::Arc { .. } => self.circle(id).map(|(_, radius)| Constraint::Radius {
                    entity: id,
                    value: millimetres(radius),
                }),
                Entity::Ellipse { minor_radius, .. }
                | Entity::EllipticalArc { minor_radius, .. } => Some(Constraint::MinorRadius {
                    ellipse: id,
                    value: millimetres(minor_radius),
                }),
                _ => None,
            })
            .collect()
    }
}

fn offsets(
    from: EntityId,
    origin: Point2,
    to: EntityId,
    at: Point2,
    tolerance: Tolerance,
) -> Vec<Constraint> {
    let offset = at - origin;
    let across = match offset.x.abs().partial_cmp(&tolerance.distance) {
        Some(Ordering::Greater) => Constraint::HorizontalDistance {
            from,
            to,
            value: millimetres(offset.x.abs()),
        },
        _ => Constraint::VerticalPoints(from, to),
    };
    let up = match offset.y.abs().partial_cmp(&tolerance.distance) {
        Some(Ordering::Greater) => Constraint::VerticalDistance {
            from,
            to,
            value: millimetres(offset.y.abs()),
        },
        _ => Constraint::HorizontalPoints(from, to),
    };
    vec![across, up]
}

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

#[cfg(test)]
mod tests;
