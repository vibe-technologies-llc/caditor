use std::f64::consts::{PI, TAU};

use caditor_expression::Expression;
use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{ArcGeometry, Faceting, direction_angle},
    entity::Entity,
    id::EntityId,
    intersect::{self, Carrier, Shape},
    sketch::{Sketch, SketchError},
    trim::keeps_length,
};

const TOLERANCE: f64 = 1e-7;
const SMOOTH_ANGLE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FilletError {
    #[error("{label} is not where two lines or arcs meet")]
    NotACorner { point: EntityId, label: String },
    #[error("only {curve} ends at {label}, so there is no corner to round")]
    OneCurve {
        point: EntityId,
        label: String,
        curve: String,
    },
    #[error("{count} curves meet at {label}; a fillet rounds the corner between two")]
    TooManyCurves {
        point: EntityId,
        label: String,
        count: usize,
    },
    #[error("{first} and {second} do not meet at their ends")]
    NotJoined { first: String, second: String },
    #[error("{label} cannot be filleted; only lines and arcs can")]
    NotLineOrArc { entity: EntityId, label: String },
    #[error("{first} and {second} meet without a corner to round")]
    NoCorner { first: String, second: String },
    #[error("the radius must be greater than zero")]
    NotPositive,
    #[error("the radius is too large for {label}")]
    TooLarge { entity: EntityId, label: String },
    #[error("no arc of this radius touches both {first} and {second} near their corner")]
    NoFit { first: String, second: String },
    #[error("{label} follows the geometry it was projected from, so its corner cannot be rounded")]
    Projected { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    pub point: EntityId,
    pub position: Point2,
    pub curves: [EntityId; 2],
    ends: [EntityId; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rounding {
    pub center: Point2,
    pub radius: f64,
    pub touches: [Point2; 2],
    arc: ArcGeometry,
    starts_on_first: bool,
}

impl Rounding {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        self.arc.faceted(faceting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Leg {
    Line {
        far: Point2,
    },
    Arc {
        arc: ArcGeometry,
        corner_at_start: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Side {
    curve: EntityId,
    leg: Leg,
    leaving: Vector2,
}

impl Side {
    fn carrier_toward(&self, corner: Point2, inside: Vector2, radius: f64) -> Option<Carrier> {
        match self.leg {
            Leg::Line { .. } => Some(Carrier::Line {
                through: corner + inside * radius,
                direction: self.leaving,
            }),
            Leg::Arc { arc, .. } => {
                let toward_center = inside.dot(arc.center - corner) > 0.0;
                let offset = if toward_center {
                    arc.radius - radius
                } else {
                    arc.radius + radius
                };
                (offset > 0.0).then_some(Carrier::Circle {
                    center: arc.center,
                    radius: offset,
                })
            }
        }
    }

    fn touch(&self, corner: Point2, center: Point2) -> Point2 {
        match self.leg {
            Leg::Line { .. } => corner + self.leaving * (center - corner).dot(self.leaving),
            Leg::Arc { arc, .. } => {
                arc.center
                    + (center - arc.center).try_normalize().unwrap_or(Vector2::X) * arc.radius
            }
        }
    }

    fn reaches(&self, corner: Point2, touch: Point2, tolerance: f64) -> bool {
        match self.leg {
            Leg::Line { far } => {
                let along = (touch - corner).dot(self.leaving);
                along > tolerance && along < (far - corner).dot(self.leaving) - tolerance
            }
            Leg::Arc {
                arc,
                corner_at_start,
            } => {
                let at = direction_angle(touch - arc.center);
                let from = direction_angle(corner - arc.center);
                let turned = if corner_at_start {
                    (at - from).rem_euclid(TAU)
                } else {
                    (from - at).rem_euclid(TAU)
                };
                let slack = if arc.radius > 0.0 {
                    tolerance / arc.radius
                } else {
                    0.0
                };
                turned > slack && turned < arc.sweep - slack
            }
        }
    }
}

impl Sketch {
    pub fn fillet_corners(&self) -> Vec<Corner> {
        let mut ends = self.curve_ends();
        ends.sort_by(|a, b| a.2.x.total_cmp(&b.2.x));
        let mut taken = vec![false; ends.len()];
        let mut corners = Vec::new();
        for (index, (_, _, position)) in ends.iter().enumerate() {
            if taken.get(index).copied().unwrap_or(true) {
                continue;
            }
            let tolerance = tolerance_at(*position);
            let near: Vec<usize> = (index..ends.len())
                .take_while(|other| {
                    ends.get(*other)
                        .is_some_and(|(_, _, at)| at.x - position.x <= tolerance)
                })
                .filter(|other| {
                    ends.get(*other)
                        .is_some_and(|(_, _, at)| at.distance(*position) <= tolerance)
                })
                .collect();
            for other in &near {
                if let Some(flag) = taken.get_mut(*other) {
                    *flag = true;
                }
            }
            let here: Vec<&(EntityId, EntityId, Point2)> =
                near.iter().filter_map(|other| ends.get(*other)).collect();
            let (Some(first), Some(second), 2) = (here.first(), here.get(1), here.len()) else {
                continue;
            };
            let spline = [first.0, second.0]
                .iter()
                .any(|curve| matches!(self.entity(*curve), Some(Entity::Spline { .. })));
            if first.0 == second.0 || spline {
                continue;
            }
            let ([first, second], point) = if first.0 < second.0 {
                ([first, second], first.1)
            } else {
                ([second, first], second.1)
            };
            let corner = Corner {
                point,
                position: *position,
                curves: [first.0, second.0],
                ends: [first.1, second.1],
            };
            if self.corner_sides(&corner).is_ok() {
                corners.push(corner);
            }
        }
        corners.sort_by_key(|corner| corner.curves);
        corners
    }

    pub fn corner_at(&self, point: EntityId) -> Result<Corner, FilletError> {
        let label = || self.entity_label(point);
        let position = self.point(point).ok_or_else(|| FilletError::NotACorner {
            point,
            label: label(),
        })?;
        let tolerance = tolerance_at(position);
        let mut here: Vec<(EntityId, EntityId)> = self
            .curve_ends()
            .into_iter()
            .filter(|(_, _, at)| at.distance(position) <= tolerance)
            .map(|(curve, end, _)| (curve, end))
            .collect();
        here.sort_by_key(|(curve, end)| (*end != point, *curve));
        let curves: std::collections::BTreeSet<EntityId> =
            here.iter().map(|(curve, _)| *curve).collect();
        let (first, second) = match (here.as_slice(), curves.len()) {
            ([], _) => {
                return Err(FilletError::NotACorner {
                    point,
                    label: label(),
                });
            }
            ([(curve, _)], _) => {
                return Err(FilletError::OneCurve {
                    point,
                    label: label(),
                    curve: self.entity_label(*curve),
                });
            }
            ([first, second], 2) => (*first, *second),
            (_, count) => {
                return Err(FilletError::TooManyCurves {
                    point,
                    label: label(),
                    count: count.max(3),
                });
            }
        };
        for (curve, _) in [first, second] {
            if matches!(self.entity(curve), Some(Entity::Spline { .. })) {
                return Err(FilletError::NotLineOrArc {
                    entity: curve,
                    label: self.entity_label(curve),
                });
            }
            if self.is_projected(curve) {
                return Err(FilletError::Projected {
                    entity: curve,
                    label: self.entity_label(curve),
                });
            }
        }
        Ok(Corner {
            point: first.1,
            position,
            curves: [first.0, second.0],
            ends: [first.1, second.1],
        })
    }

    pub fn corner_between(&self, first: EntityId, second: EntityId) -> Result<Corner, FilletError> {
        let not_joined = || FilletError::NotJoined {
            first: self.entity_label(first),
            second: self.entity_label(second),
        };
        for curve in [first, second] {
            if !matches!(
                self.entity(curve),
                Some(Entity::Line { .. } | Entity::Arc { .. })
            ) {
                return Err(FilletError::NotLineOrArc {
                    entity: curve,
                    label: self.entity_label(curve),
                });
            }
        }
        let ends = self.curve_ends();
        let of = |curve: EntityId| {
            ends.iter()
                .filter(move |(owner, _, _)| *owner == curve)
                .map(|(_, point, position)| (*point, *position))
        };
        let shared = of(first).find(|(_, position)| {
            of(second).any(|(_, other)| other.distance(*position) <= tolerance_at(*position))
        });
        let (point, _) = shared.ok_or_else(not_joined)?;
        let corner = self.corner_at(point)?;
        if corner.curves.contains(&first) && corner.curves.contains(&second) {
            Ok(corner)
        } else {
            Err(not_joined())
        }
    }

    pub fn rounding(&self, corner: &Corner, radius: f64) -> Result<Rounding, FilletError> {
        if !(radius.is_finite() && radius > 0.0) {
            return Err(FilletError::NotPositive);
        }
        let [first, second] = self.corner_sides(corner)?;
        let at = corner.position;
        let turn = first.leaving.perp_dot(second.leaving);
        let first_inside = first.leaving.perp() * turn.signum();
        let second_inside = second.leaving.perp() * -turn.signum();
        let too_large = |side: &Side| FilletError::TooLarge {
            entity: side.curve,
            label: self.entity_label(side.curve),
        };
        let no_fit = || FilletError::NoFit {
            first: self.entity_label(first.curve),
            second: self.entity_label(second.curve),
        };
        let first_carrier = first
            .carrier_toward(at, first_inside, radius)
            .ok_or_else(|| too_large(&first))?;
        let second_carrier = second
            .carrier_toward(at, second_inside, radius)
            .ok_or_else(|| too_large(&second))?;
        let tolerance = TOLERANCE * at.abs().max_element().max(radius).max(1.0);
        let center = centers(first_carrier, second_carrier, tolerance)
            .into_iter()
            .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
            .ok_or_else(no_fit)?;
        let touches = [first.touch(at, center), second.touch(at, center)];
        for (side, touch) in [first, second].iter().zip(touches) {
            if !side.reaches(at, touch, tolerance) {
                return Err(too_large(side));
            }
        }
        let [a, b] = touches;
        let from = direction_angle(a - center);
        let sweep = (direction_angle(b - center) - from).rem_euclid(TAU);
        let (arc, starts_on_first) = if sweep <= PI {
            (
                ArcGeometry {
                    center,
                    radius,
                    start_angle: from,
                    sweep,
                },
                true,
            )
        } else {
            (
                ArcGeometry {
                    center,
                    radius,
                    start_angle: direction_angle(b - center),
                    sweep: TAU - sweep,
                },
                false,
            )
        };
        Ok(Rounding {
            center,
            radius,
            touches,
            arc,
            starts_on_first,
        })
    }

    pub fn radius_through(&self, corner: &Corner, point: Point2) -> Option<f64> {
        let [first, second] = self.corner_sides(corner).ok()?;
        let half = first.leaving.angle_to(second.leaving).abs() / 2.0;
        let sine = half.sin();
        if !(sine > 0.0 && sine < 1.0) {
            return None;
        }
        let radius = point.distance(corner.position) * sine / (1.0 - sine);
        (radius.is_finite() && radius > 0.0).then_some(radius)
    }

    pub fn fillet(
        &mut self,
        corner: &Corner,
        radius: f64,
        value: Expression,
    ) -> Result<EntityId, FilletError> {
        let current = self.corner_at(corner.point)?;
        if current.curves != corner.curves {
            return Err(FilletError::NotJoined {
                first: self.entity_label(corner.curves[0]),
                second: self.entity_label(corner.curves[1]),
            });
        }
        let rounding = self.rounding(&current, radius)?;
        let mut working = self.clone();
        let arc = working
            .round_corner(&current, &rounding, value)
            .map_err(FilletError::Edit)?;
        *self = working;
        Ok(arc)
    }

    fn curve_ends(&self) -> Vec<(EntityId, EntityId, Point2)> {
        self.entities()
            .flat_map(|(curve, entity)| {
                let ends = match entity {
                    Entity::Line { start, end } | Entity::Arc { start, end, .. } => {
                        vec![*start, *end]
                    }
                    Entity::Spline { control_points } => control_points
                        .first()
                        .into_iter()
                        .chain(control_points.last())
                        .copied()
                        .collect(),
                    Entity::Point(_) | Entity::Circle { .. } => Vec::new(),
                };
                ends.into_iter()
                    .filter_map(move |end| Some((curve, end, self.point(end)?)))
            })
            .collect()
    }

    fn corner_sides(&self, corner: &Corner) -> Result<[Side; 2], FilletError> {
        let side = |curve: EntityId, end: EntityId| -> Result<Side, FilletError> {
            let not_line_or_arc = || FilletError::NotLineOrArc {
                entity: curve,
                label: self.entity_label(curve),
            };
            match self.entity(curve).ok_or_else(not_line_or_arc)? {
                Entity::Line { start, end: last } => {
                    let far = if *start == end { *last } else { *start };
                    let far = self.point(far).ok_or_else(not_line_or_arc)?;
                    let leaving = (far - corner.position)
                        .try_normalize()
                        .ok_or_else(not_line_or_arc)?;
                    Ok(Side {
                        curve,
                        leg: Leg::Line { far },
                        leaving,
                    })
                }
                Entity::Arc { start, .. } => {
                    let arc = self.arc(curve).ok_or_else(not_line_or_arc)?;
                    let corner_at_start = *start == end;
                    let radial = (corner.position - arc.center)
                        .try_normalize()
                        .ok_or_else(not_line_or_arc)?;
                    let leaving = if corner_at_start {
                        radial.perp()
                    } else {
                        -radial.perp()
                    };
                    Ok(Side {
                        curve,
                        leg: Leg::Arc {
                            arc,
                            corner_at_start,
                        },
                        leaving,
                    })
                }
                Entity::Point(_) | Entity::Circle { .. } | Entity::Spline { .. } => {
                    Err(not_line_or_arc())
                }
            }
        };
        let first = side(corner.curves[0], corner.ends[0])?;
        let second = side(corner.curves[1], corner.ends[1])?;
        if first.leaving.perp_dot(second.leaving).abs() <= SMOOTH_ANGLE {
            return Err(FilletError::NoCorner {
                first: self.entity_label(first.curve),
                second: self.entity_label(second.curve),
            });
        }
        Ok([first, second])
    }

    fn round_corner(
        &mut self,
        corner: &Corner,
        rounding: &Rounding,
        value: Expression,
    ) -> Result<EntityId, SketchError> {
        let kept = corner.point;
        let construction = corner
            .curves
            .iter()
            .all(|curve| self.is_construction(*curve));
        let mut shortened = Vec::with_capacity(2);
        for ((curve, end), touch) in corner.curves.iter().zip(corner.ends).zip(rounding.touches) {
            let moved = self.add_point(touch);
            let entity = self
                .entity(*curve)
                .cloned()
                .ok_or(SketchError::NoSuchEntity(*curve))?;
            let is_line = matches!(entity, Entity::Line { .. });
            let reshaped = match entity {
                Entity::Line { start, end: last } if start == end => Entity::Line {
                    start: moved,
                    end: last,
                },
                Entity::Line { start, .. } => Entity::Line { start, end: moved },
                Entity::Arc {
                    center,
                    start,
                    end: last,
                } if start == end => Entity::Arc {
                    center,
                    start: moved,
                    end: last,
                },
                Entity::Arc { center, start, .. } => Entity::Arc {
                    center,
                    start,
                    end: moved,
                },
                other => other,
            };
            self.restructure(*curve, reshaped, &[], |constraint| {
                !is_line || keeps_length(constraint)
            })?;
            shortened.push(moved);
        }
        for end in corner.ends {
            if end != kept {
                self.merge_point(end, kept)?;
            }
        }
        let center = self.add_point(rounding.center);
        let first_end = self.add_point(rounding.arc.point_at(rounding.arc.start_angle));
        let last_end = self.add_point(rounding.arc.point_at(rounding.arc.end_angle()));
        let arc = EntityId::from_raw(self.next_id());
        self.insert_entity(
            arc,
            Entity::Arc {
                center,
                start: first_end,
                end: last_end,
            },
        )?;
        if construction {
            self.set_construction(arc, true)?;
        }
        let (on_first, on_second) = if rounding.starts_on_first {
            (first_end, last_end)
        } else {
            (last_end, first_end)
        };
        if let [first_touch, second_touch] = shortened.as_slice() {
            self.add_constraint(Constraint::Coincident(*first_touch, on_first))?;
            self.add_constraint(Constraint::Coincident(*second_touch, on_second))?;
        }
        for curve in corner.curves {
            self.add_constraint(Constraint::Tangent(curve, arc))?;
        }
        self.add_constraint(Constraint::Radius { entity: arc, value })?;
        for curve in corner.curves {
            self.add_constraint(Constraint::Coincident(kept, curve))?;
        }
        Ok(arc)
    }

    fn merge_point(&mut self, from: EntityId, into: EntityId) -> Result<(), SketchError> {
        for id in self.constraints_using(from) {
            let constraint = self.remove_constraint(id)?;
            let moved = constraint.with_entity_replaced(from, into);
            let duplicate = self.constraints().any(|(_, existing)| *existing == moved);
            if !duplicate && self.check_constraint(&moved).is_ok() {
                self.insert_constraint(id, moved)?;
            }
        }
        if self.entities_using(from).is_empty() {
            self.remove_unused_entity(from)?;
        }
        Ok(())
    }
}

fn tolerance_at(position: Point2) -> f64 {
    TOLERANCE * position.abs().max_element().max(1.0)
}

fn centers(first: Carrier, second: Carrier, tolerance: f64) -> Vec<Point2> {
    match (first, second) {
        (
            Carrier::Line { through, direction },
            Carrier::Line {
                through: other,
                direction: other_direction,
            },
        ) => {
            let turn = direction.perp_dot(other_direction);
            if turn.abs() <= SMOOTH_ANGLE {
                return Vec::new();
            }
            vec![through + direction * ((other - through).perp_dot(other_direction) / turn)]
        }
        (line @ Carrier::Line { .. }, Carrier::Circle { center, radius })
        | (Carrier::Circle { center, radius }, line @ Carrier::Line { .. }) => {
            intersect::crossings(line, &Shape::Circle { center, radius }, tolerance)
        }
        (circle @ Carrier::Circle { .. }, Carrier::Circle { center, radius }) => {
            intersect::crossings(circle, &Shape::Circle { center, radius }, tolerance)
        }
    }
}

#[cfg(test)]
mod tests;
