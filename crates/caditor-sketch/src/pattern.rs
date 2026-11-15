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
const HALF_TURN_DEGREES: f64 = 180.0;

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
    #[error("the extent cannot be zero")]
    ZeroExtent,
    #[error("a value is too large to use")]
    NotFinite,
    #[error("the two directions run along one line, so their copies would overlap")]
    ParallelDirections,
    #[error("{label} is not a point, so nothing can be patterned about it")]
    NotAPoint { entity: EntityId, label: String },
    #[error("{label} is not a line, so it gives the pattern no direction")]
    NotALine { entity: EntityId, label: String },
    #[error("{label} has no length, so it gives the pattern no direction")]
    NoDirection { entity: EntityId, label: String },
    #[error("everything selected lies on the centre, so turning it makes no copies")]
    NothingToTurn,
    #[error(
        "the angle must be more than 0° and less than 360°; leave it out to space the copies over a full turn"
    )]
    SpreadOutsideTurn,
    #[error(
        "a symmetric pattern turns more than 0° and less than 180° each way, or the copies of the two sides would meet"
    )]
    SymmetricSpreadOutsideHalfTurn,
    #[error("the pattern reaches further than {} m from the origin", MAX_LENGTH / 1_000.0)]
    OutOfReach,
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dimensioned {
    pub expression: Expression,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Heading {
    Angle(Dimensioned),
    Along { line: EntityId, square: bool },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sides {
    #[default]
    One,
    Both,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Distribution {
    #[default]
    Spacing,
    Extent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternRow {
    pub count: usize,
    pub distance: Dimensioned,
    pub heading: Heading,
    pub reversed: bool,
    pub sides: Sides,
}

impl PatternRow {
    pub fn along(count: usize, spacing: Dimensioned, angle: Dimensioned) -> Self {
        Self {
            count,
            distance: spacing,
            heading: Heading::Angle(angle),
            reversed: false,
            sides: Sides::One,
        }
    }

    pub fn instances(&self) -> usize {
        match self.sides {
            Sides::One => self.count,
            Sides::Both => self.count.saturating_mul(2).saturating_sub(1),
        }
    }

    fn spacing(&self, distribution: Distribution) -> Dimensioned {
        let gaps = self.count.saturating_sub(1).max(1);
        match distribution {
            Distribution::Extent if gaps > 1 => Dimensioned {
                expression: Expression::binary(
                    BinaryOperator::Divide,
                    self.distance.expression.clone(),
                    Expression::Number(gaps as f64),
                ),
                value: self.distance.value / gaps as f64,
            },
            Distribution::Spacing | Distribution::Extent => self.distance.clone(),
        }
    }

    fn slots(&self) -> std::ops::RangeInclusive<i64> {
        let last = i64::try_from(self.count.saturating_sub(1)).unwrap_or(i64::MAX);
        match self.sides {
            Sides::One => 0..=last,
            Sides::Both => -last..=last,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RectangularPattern {
    pub first: PatternRow,
    pub second: Option<PatternRow>,
    pub distribution: Distribution,
}

impl RectangularPattern {
    pub fn instances(&self) -> usize {
        let rows = self.second.as_ref().map_or(1, PatternRow::instances);
        self.first.instances().saturating_mul(rows)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Spread {
    FullTurn,
    Total(Dimensioned),
    Symmetric(Dimensioned),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CircularPattern {
    pub count: usize,
    pub spread: Spread,
}

impl CircularPattern {
    pub fn instances(&self) -> usize {
        match self.spread {
            Spread::FullTurn | Spread::Total(_) => self.count,
            Spread::Symmetric(_) => self.count.saturating_mul(2).saturating_sub(1),
        }
    }
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
struct Course {
    spacing: Dimensioned,
    heading: Heading,
    vector: Vector2,
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Shift(Course),
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
        let first = self.pattern_course(&pattern.first, pattern.distribution)?;
        let second = match &pattern.second {
            Some(row) => Some((row, self.pattern_course(row, pattern.distribution)?)),
            None => None,
        };
        let instances = rectangular_instances(pattern, &first, second.as_ref())?;
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

    fn pattern_course(
        &self,
        row: &PatternRow,
        distribution: Distribution,
    ) -> Result<Course, PatternError> {
        counted(row.count)?;
        let spacing = row.spacing(distribution);
        if !spacing.value.is_finite() {
            return Err(PatternError::NotFinite);
        }
        if spacing.value.abs() <= AXIS_TOLERANCE {
            return Err(match distribution {
                Distribution::Spacing => PatternError::ZeroSpacing,
                Distribution::Extent => PatternError::ZeroExtent,
            });
        }
        let unit = self.heading_direction(&row.heading)?;
        let unit = if row.reversed { -unit } else { unit };
        Ok(Course {
            vector: unit * spacing.value,
            spacing,
            heading: row.heading.clone(),
        })
    }

    fn heading_direction(&self, heading: &Heading) -> Result<Vector2, PatternError> {
        let (line, square) = match heading {
            Heading::Angle(angle) => {
                if !angle.value.is_finite() {
                    return Err(PatternError::NotFinite);
                }
                return Ok(Vector2::from_angle(angle.value.to_radians()));
            }
            Heading::Along { line, square } => (*line, *square),
        };
        let not_a_line = || PatternError::NotALine {
            entity: line,
            label: self.entity_label(line),
        };
        let along = match line.reference() {
            Some(Reference::HorizontalAxis) => Vector2::X,
            Some(Reference::VerticalAxis) => Vector2::Y,
            Some(Reference::Origin) => return Err(not_a_line()),
            None => match self.entity(line) {
                Some(&Entity::Line { start, end }) => {
                    let (Some(start), Some(end)) = (self.point(start), self.point(end)) else {
                        return Err(not_a_line());
                    };
                    let reach = start
                        .abs()
                        .max_element()
                        .max(end.abs().max_element())
                        .max(1.0);
                    let run = end - start;
                    if run.length() <= TOLERANCE * reach {
                        return Err(PatternError::NoDirection {
                            entity: line,
                            label: self.entity_label(line),
                        });
                    }
                    run.normalize()
                }
                _ => return Err(not_a_line()),
            },
        };
        Ok(if square { along.perp() } else { along })
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
        let (lone, curves): (Vec<EntityId>, Vec<EntityId>) = chosen
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
        let mut curve_maps = Vec::with_capacity(plan.instances.len());
        for instance in &plan.instances {
            let mut curve_images = BTreeMap::new();
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
                    Entity::Spline { points, kind } => Entity::Spline {
                        points: points.into_iter().map(image_of).collect(),
                        kind,
                    },
                    Entity::Ellipse {
                        center,
                        major,
                        minor_radius,
                    } => Entity::Ellipse {
                        center: image_of(center),
                        major: image_of(major),
                        minor_radius,
                    },
                    Entity::EllipticalArc {
                        center,
                        major,
                        minor_radius,
                        start,
                        end,
                    } => Entity::EllipticalArc {
                        center: image_of(center),
                        major: image_of(major),
                        minor_radius,
                        start: image_of(start),
                        end: image_of(end),
                    },
                    Entity::Point(_) => continue,
                };
                let round = self.copy_needs_equal(*curve);

                let copy = EntityId::from_raw(self.next_id());
                self.insert_entity(copy, image)?;
                if self.is_construction(*curve) {
                    self.set_construction(copy, true)?;
                }
                if round {
                    equal.push((*curve, copy));
                }
                curve_images.insert(*curve, copy);
                copies.push(copy);
            }
            curve_maps.push(curve_images);
            let lone = plan
                .points
                .iter()
                .filter(|point| !self.is_used_by(**point, &plan.curves))
                .filter_map(|point| map.get(point).copied());
            copies.extend(lone);
            maps.push(map);
        }
        let implied = self.implied_gradients(&plan.curves);
        match centre {
            None => self.tie_shifts(plan, &maps, &implied)?,
            Some(centre) => self.tie_turns(plan, &maps, &implied, centre)?,
        }
        for (original, copy) in equal {
            self.add_constraint(Constraint::Equal(original, copy))?;
        }
        for curve_images in &curve_maps {
            self.copy_handles(curve_images)?;
        }
        Ok(copies)
    }

    fn implied_gradients(&self, curves: &[EntityId]) -> BTreeMap<EntityId, Vec<Vector2>> {
        let mut implied: BTreeMap<EntityId, Vec<Vector2>> = BTreeMap::new();
        for curve in curves {
            match self.entity(*curve) {
                Some(&Entity::Arc { center, end, .. }) => {
                    if let (Some(center), Some(at)) = (self.point(center), self.point(end)) {
                        implied.entry(end).or_default().push(at - center);
                    }
                }
                Some(&Entity::EllipticalArc { start, end, .. }) => {
                    let Some(ellipse) = self.ellipse(*curve) else {
                        continue;
                    };
                    let ends = [start, end];
                    let across = |point: EntityId| {
                        self.point(point).map_or(0.0, |at| {
                            (at - ellipse.center).dot(ellipse.axis().perp()).abs()
                        })
                    };
                    let minor_setter = ends
                        .into_iter()
                        .filter(|point| !self.lies_on_major_axis(*curve, *point))
                        .max_by(|a, b| across(*a).total_cmp(&across(*b)));
                    let axis = ellipse.axis();
                    let major = ellipse.major_radius();
                    let minor = ellipse.minor_radius;
                    for point in ends
                        .into_iter()
                        .filter(|point| Some(*point) != minor_setter)
                    {
                        let Some(at) = self.point(point) else {
                            continue;
                        };
                        let offset = at - ellipse.center;
                        let gradient = axis * (offset.dot(axis) / (major * major))
                            + axis.perp() * (offset.dot(axis.perp()) / (minor * minor));
                        implied.entry(point).or_default().push(gradient);
                    }
                }
                _ => {}
            }
        }
        implied
    }

    fn tie_shifts(
        &mut self,
        plan: &Plan,
        maps: &[BTreeMap<EntityId, EntityId>],
        implied: &BTreeMap<EntityId, Vec<Vector2>>,
    ) -> Result<(), SketchError> {
        for (instance, map) in plan.instances.iter().zip(maps) {
            let Step::Shift(course) = &instance.step else {
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
                let ties = shift_constraints(course, earlier, later);
                let kept = kept_rows(implied.get(point), tie_directions(course));
                for (index, constraint) in ties.into_iter().enumerate() {
                    if kept.contains(&index) {
                        self.add_constraint(constraint)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn tie_turns(
        &mut self,
        plan: &Plan,
        maps: &[BTreeMap<EntityId, EntityId>],
        implied: &BTreeMap<EntityId, Vec<Vector2>>,
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
                let outward = match (self.point(*point), self.point(hub)) {
                    (Some(at), Some(from)) => (at - from).normalize_or_zero(),
                    _ => Vector2::X,
                };
                let kept = kept_rows(implied.get(point), [outward, outward.perp()]);
                if kept.is_empty() {
                    continue;
                }
                let from = ray_to(self, earlier)?;
                let to = ray_to(self, later)?;
                if kept.contains(&0) {
                    self.add_constraint(Constraint::Equal(from, to))?;
                }
                if kept.contains(&1) {
                    self.add_constraint(Constraint::Angle {
                        from,
                        to,
                        reversed: false,
                        value: angle.clone(),
                    })?;
                }
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

fn kept_rows(implied: Option<&Vec<Vector2>>, directions: [Vector2; 2]) -> Vec<usize> {
    match implied.map(Vec::as_slice) {
        None | Some([]) => vec![0, 1],
        Some([gradient]) => {
            let independence = |direction: Vector2| gradient.perp_dot(direction).abs();
            let best = if independence(directions[0]) >= independence(directions[1]) {
                0
            } else {
                1
            };
            vec![best]
        }
        Some(_) => Vec::new(),
    }
}

fn counted(count: usize) -> Result<(), PatternError> {
    if count < 2 {
        return Err(PatternError::CountTooSmall { count });
    }
    Ok(())
}

fn rectangular_instances(
    pattern: &RectangularPattern,
    first: &Course,
    second: Option<&(&PatternRow, Course)>,
) -> Result<Vec<Instance>, PatternError> {
    let total = pattern.instances();
    if total > MAX_PATTERN_INSTANCES {
        return Err(PatternError::TooManyInstances {
            count: total,
            most: MAX_PATTERN_INSTANCES,
        });
    }
    if let Some((_, across)) = second
        && first
            .vector
            .normalize_or_zero()
            .perp_dot(across.vector.normalize_or_zero())
            .abs()
            <= AXIS_TOLERANCE
    {
        return Err(PatternError::ParallelDirections);
    }
    let rows = second.map_or(0..=0, |(row, _)| row.slots());
    let slots: Vec<(i64, i64)> = rows
        .flat_map(|row| pattern.first.slots().map(move |column| (column, row)))
        .filter(|slot| *slot != (0, 0))
        .collect();
    let index: BTreeMap<(i64, i64), usize> = slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (*slot, index))
        .collect();
    let across = second.map_or(Vector2::ZERO, |(_, course)| course.vector);
    let mut instances = Vec::with_capacity(slots.len());
    for (column, row) in slots {
        let motion = Motion::Shift(first.vector * column as f64 + across * row as f64);
        let (before, course) = match (column, second) {
            (0, Some((_, course))) => ((0, row - row.signum()), course),
            (0, None) => continue,
            _ => ((column - column.signum(), row), first),
        };
        instances.push(Instance {
            motion,
            from: index.get(&before).copied(),
            step: Step::Shift(course.clone()),
        });
    }
    Ok(instances)
}

fn circular_instances(
    centre: Point2,
    pattern: &CircularPattern,
) -> Result<Vec<Instance>, PatternError> {
    counted(pattern.count)?;
    let total = pattern.instances();
    if total > MAX_PATTERN_INSTANCES {
        return Err(PatternError::TooManyInstances {
            count: total,
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
            spread_step(pattern.count, total)
        }
        Spread::Symmetric(each) => {
            if !each.value.is_finite() {
                return Err(PatternError::NotFinite);
            }
            if each.value == 0.0 || each.value.abs() >= HALF_TURN_DEGREES {
                return Err(PatternError::SymmetricSpreadOutsideHalfTurn);
            }
            spread_step(pattern.count, each)
        }
    };
    let last = i64::try_from(pattern.count - 1).unwrap_or(i64::MAX);
    let turns: Vec<i64> = match pattern.spread {
        Spread::FullTurn | Spread::Total(_) => (1..=last).collect(),
        Spread::Symmetric(_) => (1..=last).chain((1..=last).map(|turn| -turn)).collect(),
    };
    let index: BTreeMap<i64, usize> = turns
        .iter()
        .enumerate()
        .map(|(index, turn)| (*turn, index))
        .collect();
    let backwards = Expression::Negate(Box::new(step_angle.clone()));
    Ok(turns
        .iter()
        .map(|turn| Instance {
            motion: Motion::Turn {
                centre,
                radians: step_radians * *turn as f64,
            },
            from: index.get(&(turn - turn.signum())).copied(),
            step: Step::Turn(if *turn < 0 {
                backwards.clone()
            } else {
                step_angle.clone()
            }),
        })
        .collect())
}

fn spread_step(count: usize, spread: &Dimensioned) -> (f64, Expression) {
    let gaps = (count - 1) as f64;
    let angle = if count == 2 {
        spread.expression.clone()
    } else {
        Expression::binary(
            BinaryOperator::Divide,
            spread.expression.clone(),
            Expression::Number(gaps),
        )
    };
    (spread.value.to_radians() / gaps, angle)
}

fn tie_directions(course: &Course) -> [Vector2; 2] {
    match course.heading {
        Heading::Angle(_) => [Vector2::X, Vector2::Y],
        Heading::Along { .. } => {
            let unit = course.vector.normalize_or_zero();
            [unit, unit.perp()]
        }
    }
}

fn shift_constraints(course: &Course, earlier: EntityId, later: EntityId) -> Vec<Constraint> {
    let angle = match &course.heading {
        Heading::Angle(angle) => angle,
        Heading::Along { line, square } => {
            return vec![
                Constraint::Distance {
                    from: earlier,
                    to: later,
                    value: magnitude(&course.spacing),
                },
                Constraint::OnDirection {
                    point: later,
                    through: earlier.into(),
                    line: *line,
                    square: *square,
                },
            ];
        }
    };
    let vector = course.vector;
    let literal = angle.expression.is_literal();
    let tolerance = AXIS_TOLERANCE * vector.length();
    let vertical = literal && vector.x.abs() <= tolerance;
    let horizontal = literal && vector.y.abs() <= tolerance;
    let across = if vertical {
        Constraint::VerticalPoints(earlier, later.into())
    } else {
        Constraint::HorizontalDistance {
            from: earlier,
            to: later,
            value: if horizontal {
                magnitude(&course.spacing)
            } else {
                component(&course.spacing, angle, Function::Cos)
            },
        }
    };
    let up = if horizontal {
        Constraint::HorizontalPoints(earlier, later.into())
    } else {
        Constraint::VerticalDistance {
            from: earlier,
            to: later,
            value: if vertical {
                magnitude(&course.spacing)
            } else {
                component(&course.spacing, angle, Function::Sin)
            },
        }
    };
    vec![across, up]
}

fn component(spacing: &Dimensioned, angle: &Dimensioned, function: Function) -> Expression {
    Expression::Call(
        Function::Abs,
        vec![Expression::binary(
            BinaryOperator::Multiply,
            spacing.expression.clone(),
            Expression::Call(function, vec![angle.expression.clone()]),
        )],
    )
}

fn magnitude(spacing: &Dimensioned) -> Expression {
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
