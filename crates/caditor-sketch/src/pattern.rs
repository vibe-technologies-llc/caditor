use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_expression::{BinaryOperator, Expression, Function, Unit};
use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::{Constraint, MAX_LENGTH},
    curve::Faceting,
    entity::Entity,
    id::{EntityId, Reference},
    sketch::{Sketch, SketchError},
};

pub const MAX_PATTERN_INSTANCES: usize = 200;

const TOLERANCE: f64 = 1e-7;
const AXIS_TOLERANCE: f64 = 1e-9;
const FULL_TURN_DEGREES: f64 = 360.0;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatternError {
    #[error("select the geometry to pattern")]
    NothingSelected,
    #[error("a pattern needs a count of at least 2, counting the original")]
    CountTooSmall { count: usize },
    #[error(
        "a pattern makes at most {most} instances, counting the original, and {count} were asked for"
    )]
    TooManyInstances { count: usize, most: usize },
    #[error("the spacing cannot be zero")]
    ZeroSpacing,
    #[error("a value is too large to use")]
    NotFinite,
    #[error("the two directions run along one line, so their copies would overlap")]
    ParallelDirections,
    #[error("{label} is not a point, so nothing can be patterned about it")]
    NotAPoint { entity: EntityId, label: String },
    #[error("everything selected lies on the centre, so turning it makes no copies")]
    NothingToTurn,
    #[error(
        "the angle must be more than 0° and less than 360°; leave it out to space the copies over a full turn"
    )]
    SpreadOutsideTurn,
    #[error("the pattern reaches further than {} m from the origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternValue {
    pub expression: Expression,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternRow {
    pub count: usize,
    pub spacing: PatternValue,
    pub angle: PatternValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RectangularPattern {
    pub first: PatternRow,
    pub second: Option<PatternRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Spread {
    FullTurn,
    Total(PatternValue),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CircularPattern {
    pub count: usize,
    pub spread: Spread,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatternImage {
    pub curves: Vec<Vec<Point2>>,
    pub points: Vec<Point2>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    Shift(Vector2),
    Turn { centre: Point2, radians: f64 },
}

impl Motion {
    fn apply(self, point: Point2) -> Point2 {
        match self {
            Self::Shift(offset) => point + offset,
            Self::Turn { centre, radians } => {
                centre + Vector2::from_angle(radians).rotate(point - centre)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Shift { row: PatternRow, vector: Vector2 },
    Turn(Expression),
}

#[derive(Debug, Clone, PartialEq)]
struct Instance {
    motion: Motion,
    from: Option<usize>,
    step: Step,
}

#[derive(Debug, Clone, PartialEq)]
struct Chosen {
    curves: Vec<EntityId>,
    lone: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
struct Plan {
    curves: Vec<EntityId>,
    points: Vec<EntityId>,
    shared: Vec<EntityId>,
    instances: Vec<Instance>,
}

impl Plan {
    fn positions_stay_in_reach(&self, sketch: &Sketch) -> Result<(), PatternError> {
        let reaches =
            |position: Point2| position.is_finite() && position.abs().max_element() <= MAX_LENGTH;
        let originals: Vec<Point2> = self
            .points
            .iter()
            .filter_map(|point| sketch.point(*point))
            .collect();
        let inside = self.instances.iter().all(|instance| {
            originals
                .iter()
                .all(|at| reaches(instance.motion.apply(*at)))
        });
        if inside {
            Ok(())
        } else {
            Err(PatternError::OutOfReach)
        }
    }
}

impl Sketch {
    pub fn rectangular_image(
        &self,
        items: &[EntityId],
        pattern: &RectangularPattern,
        faceting: Faceting,
    ) -> Result<PatternImage, PatternError> {
        let plan = self.rectangular_plan(items, pattern)?;
        Ok(self.pattern_image(&plan, faceting))
    }

    pub fn circular_image(
        &self,
        items: &[EntityId],
        centre: EntityId,
        pattern: &CircularPattern,
        faceting: Faceting,
    ) -> Result<PatternImage, PatternError> {
        let plan = self.circular_plan(items, centre, pattern)?;
        Ok(self.pattern_image(&plan, faceting))
    }

    pub fn rectangular_pattern(
        &mut self,
        items: &[EntityId],
        pattern: &RectangularPattern,
    ) -> Result<Vec<EntityId>, PatternError> {
        let plan = self.rectangular_plan(items, pattern)?;
        let mut working = self.clone();
        let copies = working
            .add_pattern(&plan, None)
            .map_err(PatternError::Edit)?;
        *self = working;
        Ok(copies)
    }

    pub fn circular_pattern(
        &mut self,
        items: &[EntityId],
        centre: EntityId,
        pattern: &CircularPattern,
    ) -> Result<Vec<EntityId>, PatternError> {
        let plan = self.circular_plan(items, centre, pattern)?;
        let mut working = self.clone();
        let copies = working
            .add_pattern(&plan, Some(centre))
            .map_err(PatternError::Edit)?;
        *self = working;
        Ok(copies)
    }

    fn pattern_image(&self, plan: &Plan, faceting: Faceting) -> PatternImage {
        let lone: Vec<Point2> = plan
            .points
            .iter()
            .filter(|point| !self.is_used_by(**point, &plan.curves))
            .filter_map(|point| self.point(*point))
            .collect();
        let outlines: Vec<Vec<Point2>> = plan
            .curves
            .iter()
            .filter_map(|curve| self.faceted(*curve, faceting))
            .collect();
        let mut image = PatternImage::default();
        for instance in &plan.instances {
            let moved = |points: &[Point2]| -> Vec<Point2> {
                points.iter().map(|at| instance.motion.apply(*at)).collect()
            };
            image
                .curves
                .extend(outlines.iter().map(|outline| moved(outline)));
            image.points.extend(moved(&lone));
        }
        image
    }

    fn is_used_by(&self, point: EntityId, curves: &[EntityId]) -> bool {
        curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .any(|entity| entity.points().contains(&point))
    }

    fn rectangular_plan(
        &self,
        items: &[EntityId],
        pattern: &RectangularPattern,
    ) -> Result<Plan, PatternError> {
        let chosen = self.pattern_items(items, None)?;
        let instances = rectangular_instances(pattern)?;
        let plan = Plan {
            points: self.points_of(&chosen.curves, &chosen.lone),
            curves: chosen.curves,
            shared: Vec::new(),
            instances,
        };
        plan.positions_stay_in_reach(self)?;
        Ok(plan)
    }

    fn circular_plan(
        &self,
        items: &[EntityId],
        centre: EntityId,
        pattern: &CircularPattern,
    ) -> Result<Plan, PatternError> {
        let centre_at = self.centre_position(centre)?;
        let chosen = self.pattern_items(items, Some(centre))?;
        let instances = circular_instances(centre_at, pattern)?;
        let reach = chosen
            .curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .flat_map(Entity::points)
            .chain(chosen.lone.iter().copied())
            .filter_map(|point| self.point(point))
            .map(|position| (position - centre_at).abs().max_element())
            .fold(1.0, f64::max);
        let at_centre = |point: EntityId| {
            self.point(point)
                .is_some_and(|position| position.distance(centre_at) <= TOLERANCE * reach)
        };
        let curves: Vec<EntityId> = chosen
            .curves
            .into_iter()
            .filter(|curve| !self.is_centred_circle(*curve, &at_centre))
            .collect();
        let (shared, points): (Vec<EntityId>, Vec<EntityId>) = self
            .points_of(&curves, &chosen.lone)
            .into_iter()
            .partition(|point| at_centre(*point));
        if points.is_empty() {
            return Err(PatternError::NothingToTurn);
        }
        let plan = Plan {
            curves,
            points,
            shared,
            instances,
        };
        plan.positions_stay_in_reach(self)?;
        Ok(plan)
    }

    fn centre_position(&self, centre: EntityId) -> Result<Point2, PatternError> {
        let not_a_point = || PatternError::NotAPoint {
            entity: centre,
            label: self.entity_label(centre),
        };
        match centre.reference() {
            Some(Reference::Origin) => Ok(Point2::ZERO),
            Some(Reference::HorizontalAxis | Reference::VerticalAxis) => Err(not_a_point()),
            None => match self.entity(centre) {
                Some(Entity::Point(position)) => Ok(*position),
                _ => Err(not_a_point()),
            },
        }
    }

    fn is_centred_circle(&self, curve: EntityId, at_centre: &impl Fn(EntityId) -> bool) -> bool {
        matches!(self.entity(curve), Some(Entity::Circle { center, .. }) if at_centre(*center))
    }

    fn pattern_items(
        &self,
        items: &[EntityId],
        centre: Option<EntityId>,
    ) -> Result<Chosen, PatternError> {
        let chosen: BTreeSet<EntityId> = items
            .iter()
            .copied()
            .filter(|item| !item.is_reference() && Some(*item) != centre)
            .filter(|item| self.entity(*item).is_some())
            .collect();
        if chosen.is_empty() {
            return Err(PatternError::NothingSelected);
        }
        let (lone, curves) = chosen
            .into_iter()
            .partition(|item| matches!(self.entity(*item), Some(Entity::Point(_))));
        Ok(Chosen { curves, lone })
    }

    fn points_of(&self, curves: &[EntityId], lone: &[EntityId]) -> Vec<EntityId> {
        let points: BTreeSet<EntityId> = curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .flat_map(Entity::points)
            .chain(lone.iter().copied())
            .collect();
        points.into_iter().collect()
    }

    fn add_pattern(
        &mut self,
        plan: &Plan,
        centre: Option<EntityId>,
    ) -> Result<Vec<EntityId>, SketchError> {
        let mut copies = Vec::new();
        let mut maps: Vec<BTreeMap<EntityId, EntityId>> = Vec::with_capacity(plan.instances.len());
        let mut equal = Vec::new();
        for instance in &plan.instances {
            let mut map: BTreeMap<EntityId, EntityId> =
                plan.shared.iter().map(|point| (*point, *point)).collect();
            for point in &plan.points {
                let position = self.point(*point).ok_or(SketchError::NotAPoint(*point))?;
                let copy = self.add_point(instance.motion.apply(position));
                map.insert(*point, copy);
            }
            let image_of = |point: EntityId| map.get(&point).copied().unwrap_or(point);
            for curve in &plan.curves {
                let entity = self
                    .entity(*curve)
                    .cloned()
                    .ok_or(SketchError::NoSuchEntity(*curve))?;
                let image = match entity {
                    Entity::Line { start, end } => Entity::Line {
                        start: image_of(start),
                        end: image_of(end),
                    },
                    Entity::Arc { center, start, end } => Entity::Arc {
                        center: image_of(center),
                        start: image_of(start),
                        end: image_of(end),
                    },
                    Entity::Circle { center, radius } => Entity::Circle {
                        center: image_of(center),
                        radius,
                    },
                    Entity::Spline { control_points } => Entity::Spline {
                        control_points: control_points.into_iter().map(image_of).collect(),
                    },
                    Entity::Point(_) => continue,
                };
                let round = matches!(image, Entity::Circle { .. });
                let copy = EntityId::from_raw(self.next_id());
                self.insert_entity(copy, image)?;
                if self.is_construction(*curve) {
                    self.set_construction(copy, true)?;
                }
                if round {
                    equal.push((*curve, copy));
                }
                copies.push(copy);
            }
            let lone = plan
                .points
                .iter()
                .filter(|point| !self.is_used_by(**point, &plan.curves))
                .filter_map(|point| map.get(point).copied());
            copies.extend(lone);
            maps.push(map);
        }
        for (original, copy) in equal {
            self.add_constraint(Constraint::Equal(original, copy))?;
        }
        match centre {
            None => self.tie_shifts(plan, &maps)?,
            Some(centre) => self.tie_turns(plan, &maps, centre)?,
        }
        Ok(copies)
    }

    fn tie_shifts(
        &mut self,
        plan: &Plan,
        maps: &[BTreeMap<EntityId, EntityId>],
    ) -> Result<(), SketchError> {
        for (instance, map) in plan.instances.iter().zip(maps) {
            let Step::Shift { row, vector } = &instance.step else {
                continue;
            };
            let previous = instance.from.and_then(|index| maps.get(index));
            for point in &plan.points {
                let (Some(&earlier), Some(&later)) = (
                    previous.map_or(Some(point), |map| map.get(point)),
                    map.get(point),
                ) else {
                    continue;
                };
                for constraint in shift_constraints(row, *vector, earlier, later) {
                    self.add_constraint(constraint)?;
                }
            }
        }
        Ok(())
    }

    fn tie_turns(
        &mut self,
        plan: &Plan,
        maps: &[BTreeMap<EntityId, EntityId>],
        centre: EntityId,
    ) -> Result<(), SketchError> {
        let hub = self.hub_for(centre)?;
        for point in &plan.shared {
            if *point != centre && !self.held_on(*point, centre) {
                self.add_constraint(Constraint::Coincident(*point, centre))?;
            }
        }
        let mut rays: BTreeMap<EntityId, EntityId> = BTreeMap::new();
        let mut ray_to = |sketch: &mut Self, point: EntityId| -> Result<EntityId, SketchError> {
            if let Some(ray) = rays.get(&point) {
                return Ok(*ray);
            }
            let ray = EntityId::from_raw(sketch.next_id());
            sketch.insert_entity(
                ray,
                Entity::Line {
                    start: hub,
                    end: point,
                },
            )?;
            sketch.set_construction(ray, true)?;
            rays.insert(point, ray);
            Ok(ray)
        };
        for (instance, map) in plan.instances.iter().zip(maps) {
            let Step::Turn(angle) = &instance.step else {
                continue;
            };
            let previous = instance.from.and_then(|index| maps.get(index));
            for point in &plan.points {
                let (Some(&earlier), Some(&later)) = (
                    previous.map_or(Some(point), |map| map.get(point)),
                    map.get(point),
                ) else {
                    continue;
                };
                let from = ray_to(self, earlier)?;
                let to = ray_to(self, later)?;
                self.add_constraint(Constraint::Equal(from, to))?;
                self.add_constraint(Constraint::Angle {
                    from,
                    to,
                    reversed: false,
                    value: angle.clone(),
                })?;
            }
        }
        Ok(())
    }

    fn hub_for(&mut self, centre: EntityId) -> Result<EntityId, SketchError> {
        if !centre.is_reference() {
            return Ok(centre);
        }
        let hub = self.add_point(Point2::ZERO);
        self.add_constraint(Constraint::Coincident(hub, centre))?;
        Ok(hub)
    }
}

fn counted(count: usize) -> Result<(), PatternError> {
    if count < 2 {
        return Err(PatternError::CountTooSmall { count });
    }
    Ok(())
}

fn rectangular_instances(pattern: &RectangularPattern) -> Result<Vec<Instance>, PatternError> {
    counted(pattern.first.count)?;
    let first = row_vector(&pattern.first)?;
    let second = match &pattern.second {
        Some(row) => {
            counted(row.count)?;
            Some((row, row_vector(row)?))
        }
        None => None,
    };
    let columns = pattern.first.count;
    let rows = pattern.second.as_ref().map_or(1, |row| row.count);
    let total = columns.saturating_mul(rows);
    if total > MAX_PATTERN_INSTANCES {
        return Err(PatternError::TooManyInstances {
            count: total,
            most: MAX_PATTERN_INSTANCES,
        });
    }
    if let Some((_, across)) = &second
        && first
            .normalize_or_zero()
            .perp_dot(across.normalize_or_zero())
            .abs()
            <= AXIS_TOLERANCE
    {
        return Err(PatternError::ParallelDirections);
    }
    let slot = |column: usize, row: usize| (row * columns + column).checked_sub(1);
    let mut instances = Vec::with_capacity(total.saturating_sub(1));
    for row in 0..rows {
        for column in 0..columns {
            if column == 0 && row == 0 {
                continue;
            }
            let across = second.as_ref().map_or(Vector2::ZERO, |(_, vector)| *vector);
            let motion = Motion::Shift(first * column as f64 + across * row as f64);
            let (from, step) = match (column.checked_sub(1), row.checked_sub(1), &second) {
                (Some(before), _, _) => (
                    slot(before, row),
                    Step::Shift {
                        row: pattern.first.clone(),
                        vector: first,
                    },
                ),
                (None, Some(above), Some((spacing, vector))) => (
                    slot(column, above),
                    Step::Shift {
                        row: (*spacing).clone(),
                        vector: *vector,
                    },
                ),
                (None, _, _) => continue,
            };
            instances.push(Instance { motion, from, step });
        }
    }
    Ok(instances)
}

fn row_vector(row: &PatternRow) -> Result<Vector2, PatternError> {
    if !row.spacing.value.is_finite() || !row.angle.value.is_finite() {
        return Err(PatternError::NotFinite);
    }
    if row.spacing.value.abs() <= AXIS_TOLERANCE {
        return Err(PatternError::ZeroSpacing);
    }
    Ok(Vector2::from_angle(row.angle.value.to_radians()) * row.spacing.value)
}

fn circular_instances(
    centre: Point2,
    pattern: &CircularPattern,
) -> Result<Vec<Instance>, PatternError> {
    counted(pattern.count)?;
    if pattern.count > MAX_PATTERN_INSTANCES {
        return Err(PatternError::TooManyInstances {
            count: pattern.count,
            most: MAX_PATTERN_INSTANCES,
        });
    }
    let (step_radians, step_angle) = match &pattern.spread {
        Spread::FullTurn => (
            TAU / pattern.count as f64,
            Expression::binary(
                BinaryOperator::Divide,
                Expression::Measure(FULL_TURN_DEGREES, Unit::Degree),
                Expression::Number(pattern.count as f64),
            ),
        ),
        Spread::Total(total) => {
            if !total.value.is_finite() {
                return Err(PatternError::NotFinite);
            }
            if total.value == 0.0 || total.value.abs() >= FULL_TURN_DEGREES {
                return Err(PatternError::SpreadOutsideTurn);
            }
            let gaps = (pattern.count - 1) as f64;
            let angle = if pattern.count == 2 {
                total.expression.clone()
            } else {
                Expression::binary(
                    BinaryOperator::Divide,
                    total.expression.clone(),
                    Expression::Number(gaps),
                )
            };
            (total.value.to_radians() / gaps, angle)
        }
    };
    Ok((1..pattern.count)
        .map(|turn| Instance {
            motion: Motion::Turn {
                centre,
                radians: step_radians * turn as f64,
            },
            from: turn.checked_sub(2),
            step: Step::Turn(step_angle.clone()),
        })
        .collect())
}

fn shift_constraints(
    row: &PatternRow,
    vector: Vector2,
    earlier: EntityId,
    later: EntityId,
) -> Vec<Constraint> {
    let literal = row.angle.expression.is_literal();
    let tolerance = AXIS_TOLERANCE * vector.length();
    let vertical = literal && vector.x.abs() <= tolerance;
    let horizontal = literal && vector.y.abs() <= tolerance;
    let across = if vertical {
        Constraint::VerticalPoints(earlier, later)
    } else {
        Constraint::HorizontalDistance {
            from: earlier,
            to: later,
            value: if horizontal {
                magnitude(&row.spacing)
            } else {
                component(row, Function::Cos)
            },
        }
    };
    let up = if horizontal {
        Constraint::HorizontalPoints(earlier, later)
    } else {
        Constraint::VerticalDistance {
            from: earlier,
            to: later,
            value: if vertical {
                magnitude(&row.spacing)
            } else {
                component(row, Function::Sin)
            },
        }
    };
    vec![across, up]
}

fn component(row: &PatternRow, function: Function) -> Expression {
    Expression::Call(
        Function::Abs,
        vec![Expression::binary(
            BinaryOperator::Multiply,
            row.spacing.expression.clone(),
            Expression::Call(function, vec![row.angle.expression.clone()]),
        )],
    )
}

fn magnitude(spacing: &PatternValue) -> Expression {
    if spacing.value >= 0.0 {
        return spacing.expression.clone();
    }
    match &spacing.expression {
        Expression::Negate(inner) => (**inner).clone(),
        other => Expression::Negate(Box::new(other.clone())),
    }
}

#[cfg(test)]
mod tests;
