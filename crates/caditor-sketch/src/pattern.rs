use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
    fmt,
};

use caditor_expression::{BinaryOperator, Expression, Function, Unit};
use caditor_geometry::{Point2, Vector2, larger};

use crate::{
    constraint::{Constraint, MAX_LENGTH},
    curve::Faceting,
    entity::Entity,
    id::{EntityId, Reference},
    sketch::{Sketch, SketchError},
};

pub const MAX_PATTERN_INSTANCES: usize = 2_000;

const TOLERANCE: f64 = 1e-7;
const AXIS_TOLERANCE: f64 = 1e-9;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
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
    #[error("two of the directions run along one line, so their copies would overlap")]
    ParallelDirections,
    #[error("a pattern needs at least one direction")]
    NoDirections,
    #[error(
        "two copies would land on each other: the directions and spacings reach one place by two routes"
    )]
    OverlappingCopies,
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
    #[error("{copy} is not one of the pattern's copies, so it cannot be left out")]
    NoSuchCopy { copy: PatternCopy },
    #[error("the original always stays; only its copies can be left out")]
    OriginalStays,
    #[error("every copy is left out, so the pattern would make nothing")]
    EveryCopyLeftOut,
    #[error(
        "{copy} has every copy in line with it left out, so it cannot follow the line its direction runs along; keep one in its row, or give the directions angles"
    )]
    CopyCutOff { copy: PatternCopy },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dimensioned {
    pub expression: Expression,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Heading {
    Angle(Dimensioned),
    Along { line: EntityId, square: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sides {
    #[default]
    One,
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Distribution {
    #[default]
    Spacing,
    Extent,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PatternCopy {
    Steps(Vec<i64>),
    Turn(i64),
}

impl PatternCopy {
    pub fn is_original(&self) -> bool {
        match self {
            Self::Steps(steps) => steps.iter().all(|step| *step == 0),
            Self::Turn(turn) => *turn == 0,
        }
    }
}

impl fmt::Display for PatternCopy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_original() {
            return formatter.write_str("the original");
        }
        match self {
            Self::Steps(steps) => match steps.as_slice() {
                [step] => write!(formatter, "copy {step}"),
                _ => {
                    let listed: Vec<String> = steps.iter().map(i64::to_string).collect();
                    write!(formatter, "copy ({})", listed.join(", "))
                }
            },
            Self::Turn(turn) => write!(formatter, "copy {turn}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectangularPattern {
    pub directions: Vec<PatternRow>,
    pub distribution: Distribution,
    pub left_out: BTreeSet<Vec<i64>>,
}

impl RectangularPattern {
    pub fn instances(&self) -> usize {
        self.directions
            .iter()
            .map(PatternRow::instances)
            .fold(1, usize::saturating_mul)
    }

    pub fn has_copy(&self, steps: &[i64]) -> bool {
        steps.len() == self.directions.len()
            && steps.iter().any(|step| *step != 0)
            && steps
                .iter()
                .zip(&self.directions)
                .all(|(step, row)| row.slots().contains(step))
    }

    pub fn copies(&self) -> Vec<Vec<i64>> {
        if self.instances() > MAX_PATTERN_INSTANCES {
            return Vec::new();
        }
        self.directions
            .iter()
            .fold(vec![Vec::new()], |earlier: Vec<Vec<i64>>, row| {
                row.slots()
                    .flat_map(|step| {
                        earlier.iter().map(move |before| {
                            let mut slot = before.clone();

                            slot.push(step);
                            slot
                        })
                    })
                    .collect()
            })
            .into_iter()
            .filter(|slot| slot.iter().any(|step| *step != 0))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Spread {
    FullTurn,
    Total(Dimensioned),
    Symmetric(Dimensioned),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CircularPattern {
    pub count: usize,
    pub spread: Spread,
    pub left_out: BTreeSet<i64>,
}

impl CircularPattern {
    pub fn instances(&self) -> usize {
        match self.spread {
            Spread::FullTurn | Spread::Total(_) => self.count,
            Spread::Symmetric(_) => self.count.saturating_mul(2).saturating_sub(1),
        }
    }

    pub fn has_turn(&self, turn: i64) -> bool {
        let last = i64::try_from(self.count.saturating_sub(1)).unwrap_or(i64::MAX);
        match self.spread {
            Spread::FullTurn | Spread::Total(_) => (1..=last).contains(&turn),
            Spread::Symmetric(_) => turn != 0 && (-last..=last).contains(&turn),
        }
    }

    pub fn turns(&self) -> Vec<i64> {
        if self.instances() > MAX_PATTERN_INSTANCES {
            return Vec::new();
        }

        let last = i64::try_from(self.count.saturating_sub(1)).unwrap_or(i64::MAX);
        match self.spread {
            Spread::FullTurn | Spread::Total(_) => (1..=last).collect(),
            Spread::Symmetric(_) => (1..=last).chain((1..=last).map(|turn| -turn)).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyImage {
    pub copy: PatternCopy,
    pub left_out: bool,
    pub curves: Vec<Vec<Point2>>,
    pub points: Vec<Point2>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatternImage {
    pub copies: Vec<CopyImage>,
}

impl PatternImage {
    pub fn kept(&self) -> impl Iterator<Item = &CopyImage> {
        self.copies.iter().filter(|copy| !copy.left_out)
    }

    pub fn left_out(&self) -> impl Iterator<Item = &CopyImage> {
        self.copies.iter().filter(|copy| copy.left_out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
struct Course {
    spacing: Dimensioned,
    heading: Heading,
    vector: Vector2,
    reversed: bool,
}

impl Course {
    fn times(&self, steps: u64) -> Self {
        if steps == 1 {
            return self.clone();
        }

        let factor = steps as f64;
        Self {
            spacing: Dimensioned {
                expression: Expression::binary(
                    BinaryOperator::Multiply,
                    Expression::Number(factor),
                    self.spacing.expression.clone(),
                ),
                value: self.spacing.value * factor,
            },
            heading: self.heading.clone(),
            vector: self.vector * factor,
            reversed: self.reversed,
        }
    }

    fn angle(&self) -> Option<&Dimensioned> {
        match &self.heading {
            Heading::Angle(angle) => Some(angle),
            Heading::Along { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Shift(Course),
    Across(Vec<(i64, Course)>),
    Turn(Expression),
}

#[derive(Clone, Debug, PartialEq)]
struct Placed {
    copy: PatternCopy,
    motion: Motion,
}

#[derive(Clone, Debug, PartialEq)]
struct Instance {
    placed: Placed,
    from: Option<usize>,
    step: Step,
}

#[derive(Clone, Debug, PartialEq)]
struct Laid {
    instances: Vec<Instance>,
    left_out: Vec<Placed>,
}

#[derive(Clone, Debug, PartialEq)]
struct Chosen {
    curves: Vec<EntityId>,
    lone: Vec<EntityId>,
}

#[derive(Clone, Debug, PartialEq)]
struct Plan {
    curves: Vec<EntityId>,
    points: Vec<EntityId>,
    shared: Vec<EntityId>,
    instances: Vec<Instance>,
    left_out: Vec<Placed>,
}

impl Plan {
    fn placed(&self) -> impl Iterator<Item = &Placed> {
        self.instances
            .iter()
            .map(|instance| &instance.placed)
            .chain(&self.left_out)
    }

    fn positions_stay_in_reach(&self, sketch: &Sketch) -> Result<(), PatternError> {
        let reaches =
            |position: Point2| position.is_finite() && position.abs().max_element() <= MAX_LENGTH;
        let originals: Vec<Point2> = self
            .points
            .iter()
            .filter_map(|point| sketch.point(*point))
            .collect();

        let inside = self
            .placed()
            .all(|placed| originals.iter().all(|at| reaches(placed.motion.apply(*at))));
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
        let left_out: BTreeSet<&PatternCopy> =
            plan.left_out.iter().map(|placed| &placed.copy).collect();

        let copies = plan
            .placed()
            .map(|placed| {
                let moved = |points: &[Point2]| -> Vec<Point2> {
                    points.iter().map(|at| placed.motion.apply(*at)).collect()
                };
                CopyImage {
                    copy: placed.copy.clone(),
                    left_out: left_out.contains(&placed.copy),
                    curves: outlines.iter().map(|outline| moved(outline)).collect(),
                    points: moved(&lone),
                }
            })
            .collect();
        PatternImage { copies }
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
        let courses = pattern
            .directions
            .iter()
            .map(|row| self.pattern_course(row, pattern.distribution))
            .collect::<Result<Vec<Course>, PatternError>>()?;
        let laid = rectangular_instances(pattern, &courses)?;

        let plan = Plan {
            points: self.points_of(&chosen.curves, &chosen.lone),
            curves: chosen.curves,
            shared: Vec::new(),
            instances: laid.instances,
            left_out: laid.left_out,
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
        let laid = circular_instances(centre_at, pattern)?;

        let reach = chosen
            .curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .flat_map(Entity::points)
            .chain(chosen.lone.iter().copied())
            .filter_map(|point| self.point(point))
            .map(|position| (position - centre_at).abs().max_element())
            .fold(1.0, larger);
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
            instances: laid.instances,
            left_out: laid.left_out,
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
            reversed: row.reversed,
        })
    }

    pub fn heading_direction(&self, heading: &Heading) -> Result<Vector2, PatternError> {
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
                let copy = self.add_point(instance.placed.motion.apply(position));

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
            let previous = instance.from.and_then(|index| maps.get(index));
            for point in &plan.points {
                let (Some(&earlier), Some(&later)) = (
                    previous.map_or(Some(point), |map| map.get(point)),
                    map.get(point),
                ) else {
                    continue;
                };

                let (ties, directions) = match &instance.step {
                    Step::Shift(course) => (
                        shift_constraints(course, earlier, later),
                        tie_directions(course),
                    ),
                    Step::Across(steps) => (
                        across_constraints(steps, earlier, later),
                        [Vector2::X, Vector2::Y],
                    ),
                    Step::Turn(_) => continue,
                };
                let kept = kept_rows(implied.get(point), directions);
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
    courses: &[Course],
) -> Result<Laid, PatternError> {
    if courses.is_empty() {
        return Err(PatternError::NoDirections);
    }

    let total = pattern.instances();

    if total > MAX_PATTERN_INSTANCES {
        return Err(PatternError::TooManyInstances {
            count: total,
            most: MAX_PATTERN_INSTANCES,
        });
    }

    let units: Vec<Vector2> = courses
        .iter()
        .map(|course| course.vector.normalize_or_zero())
        .collect();

    for (index, unit) in units.iter().enumerate() {
        if units
            .iter()
            .skip(index + 1)
            .any(|other| unit.perp_dot(*other).abs() <= AXIS_TOLERANCE)
        {
            return Err(PatternError::ParallelDirections);
        }
    }

    let slots = pattern.copies();
    let offsets: Vec<Vector2> = slots
        .iter()
        .map(|slot| {
            slot.iter()
                .zip(courses)
                .fold(Vector2::ZERO, |sum, (step, course)| {
                    sum + course.vector * *step as f64
                })
        })
        .collect();
    let reach = offsets
        .iter()
        .map(|offset| offset.length())
        .fold(1.0, larger);

    for (at, offset) in offsets.iter().enumerate() {
        let lands_on = |other: &Vector2| offset.distance(*other) <= TOLERANCE * reach;
        if lands_on(&Vector2::ZERO) || offsets.iter().skip(at + 1).any(lands_on) {
            return Err(PatternError::OverlappingCopies);
        }
    }

    for steps in &pattern.left_out {
        let copy = PatternCopy::Steps(steps.clone());

        if copy.is_original() {
            return Err(PatternError::OriginalStays);
        }
        if !pattern.has_copy(steps) {
            return Err(PatternError::NoSuchCopy { copy });
        }
    }

    let (kept, left_out): (Vec<Placed>, Vec<Placed>) = slots
        .into_iter()
        .zip(offsets)
        .map(|(slot, offset)| Placed {
            copy: PatternCopy::Steps(slot),
            motion: Motion::Shift(offset),
        })
        .partition(|placed| {
            !matches!(&placed.copy, PatternCopy::Steps(steps) if pattern.left_out.contains(steps))
        });

    if kept.is_empty() {
        return Err(PatternError::EveryCopyLeftOut);
    }

    let kept_slots: Vec<Vec<i64>> = kept
        .iter()
        .filter_map(|placed| match &placed.copy {
            PatternCopy::Steps(steps) => Some(steps.clone()),
            PatternCopy::Turn(_) => None,
        })
        .collect();
    let mut links = Tree::of(pattern, courses, &kept_slots)?;
    let index: BTreeMap<&[i64], usize> = kept_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (slot.as_slice(), index))
        .collect();

    let mut instances = Vec::with_capacity(kept.len());

    for (placed, slot) in kept.into_iter().zip(&kept_slots) {
        let Some(link) = links.remove(slot) else {
            continue;
        };

        instances.push(Instance {
            placed,
            from: index.get(link.from.as_slice()).copied(),
            step: link.step,
        });
    }
    Ok(Laid {
        instances,
        left_out,
    })
}

struct Link {
    from: Vec<i64>,
    step: Step,
}

struct Tree<'a> {
    pattern: &'a RectangularPattern,
    courses: &'a [Course],
    tied: BTreeSet<Vec<i64>>,
    links: BTreeMap<Vec<i64>, Link>,
}

impl<'a> Tree<'a> {
    fn of(
        pattern: &'a RectangularPattern,
        courses: &'a [Course],
        kept: &[Vec<i64>],
    ) -> Result<BTreeMap<Vec<i64>, Link>, PatternError> {
        let mut tree = Self {
            pattern,
            courses,
            tied: BTreeSet::from([vec![0; courses.len()]]),
            links: BTreeMap::new(),
        };
        let mut waiting: Vec<&Vec<i64>> = kept.iter().collect();

        waiting.sort_by_key(|slot| slot.iter().map(|step| step.unsigned_abs()).sum::<u64>());

        while !waiting.is_empty() {
            let before = waiting.len();

            waiting.retain(|slot| !tree.tie_along_one(slot));
            if waiting.len() < before {
                continue;
            }

            let Some(across) = waiting.iter().position(|slot| tree.tie_across(slot)) else {
                let copy = waiting
                    .first()
                    .map_or(PatternCopy::Steps(Vec::new()), |slot| {
                        PatternCopy::Steps((*slot).clone())
                    });

                return Err(PatternError::CopyCutOff { copy });
            };

            waiting.remove(across);
        }
        Ok(tree.links)
    }

    fn tie(&mut self, slot: &[i64], from: Vec<i64>, step: Step) {
        self.tied.insert(slot.to_vec());
        self.links.insert(slot.to_vec(), Link { from, step });
    }

    fn tie_along_one(&mut self, slot: &[i64]) -> bool {
        let first = slot.iter().position(|step| *step != 0);
        let toward = (0..slot.len())
            .filter_map(|along| {
                let class = if Some(along) == first { 0 } else { 1 };
                let at = *slot.get(along)?;
                let nearer = (1..=at.unsigned_abs()).map(move |gap| at - at.signum() * gap as i64);
                self.nearest_tied(slot, along, nearer)
                    .map(|(gap, from)| ((class, gap, along), from))
            })
            .min_by_key(|(key, _)| *key);
        let beyond = || {
            self.pattern
                .directions
                .iter()
                .enumerate()
                .filter_map(|(along, row)| {
                    let at = *slot.get(along)?;
                    let range = row.slots();
                    let reach = range.start().abs_diff(*range.end());

                    let away = (1..=reach)
                        .flat_map(move |gap| {
                            let gap = gap as i64;
                            [at + gap, at - gap]
                        })
                        .filter(move |other| {
                            range.contains(other)
                                && (other.signum() * at.signum() < 0 || other.abs() > at.abs())
                        });
                    self.nearest_tied(slot, along, away)
                        .map(|(gap, from)| ((2, gap, along), from))
                })
                .min_by_key(|(key, _)| *key)
        };

        let Some(((_, gap, along), from)) = toward.or_else(beyond) else {
            return false;
        };
        let Some(course) = self.courses.get(along) else {
            return false;
        };

        self.tie(slot, from, Step::Shift(course.times(gap)));
        true
    }

    fn nearest_tied(
        &self,
        slot: &[i64],
        along: usize,
        candidates: impl Iterator<Item = i64>,
    ) -> Option<(u64, Vec<i64>)> {
        let at = *slot.get(along)?;
        candidates.into_iter().find_map(|other| {
            let mut from = slot.to_vec();

            *from.get_mut(along)? = other;
            self.tied
                .contains(&from)
                .then(|| (at.abs_diff(other), from))
        })
    }

    fn tie_across(&mut self, slot: &[i64]) -> bool {
        let differing = |from: &Vec<i64>| -> Option<Vec<(i64, Course)>> {
            slot.iter()
                .zip(from)
                .zip(self.courses)
                .filter(|((at, other), _)| at != other)
                .map(|((at, other), course)| course.angle().map(|_| (at - other, course.clone())))
                .collect()
        };
        let nearest = self
            .tied
            .iter()
            .filter_map(|from| Some((differing(from)?, from)))
            .min_by_key(|(steps, _)| {
                steps
                    .iter()
                    .map(|(steps, _)| steps.unsigned_abs())
                    .sum::<u64>()
            })
            .map(|(steps, from)| (steps, from.clone()));

        let Some((steps, from)) = nearest else {
            return false;
        };

        self.tie(slot, from, Step::Across(steps));
        true
    }
}

fn circular_instances(centre: Point2, pattern: &CircularPattern) -> Result<Laid, PatternError> {
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
    let turns = pattern.turns();

    for turn in &pattern.left_out {
        if *turn == 0 {
            return Err(PatternError::OriginalStays);
        }
        if !pattern.has_turn(*turn) {
            return Err(PatternError::NoSuchCopy {
                copy: PatternCopy::Turn(*turn),
            });
        }
    }

    let placed = |turn: i64| Placed {
        copy: PatternCopy::Turn(turn),
        motion: Motion::Turn {
            centre,
            radians: step_radians * turn as f64,
        },
    };
    let (kept, left_out): (Vec<i64>, Vec<i64>) = turns
        .iter()
        .partition(|turn| !pattern.left_out.contains(turn));

    if kept.is_empty() {
        return Err(PatternError::EveryCopyLeftOut);
    }

    let index: BTreeMap<i64, usize> = kept
        .iter()
        .enumerate()
        .map(|(index, turn)| (*turn, index))
        .collect();
    let instances = kept
        .iter()
        .map(|turn| {
            let from = (1..turn.abs())
                .rev()
                .map(|nearer| nearer * turn.signum())
                .find(|nearer| index.contains_key(nearer))
                .unwrap_or(0);
            Instance {
                placed: placed(*turn),
                from: index.get(&from).copied(),
                step: Step::Turn(turned_by(&step_angle, turn - from)),
            }
        })
        .collect();
    Ok(Laid {
        instances,
        left_out: left_out.into_iter().map(placed).collect(),
    })
}

fn turned_by(step: &Expression, steps: i64) -> Expression {
    match steps {
        1 => step.clone(),
        -1 => Expression::Negate(Box::new(step.clone())),
        _ => Expression::binary(
            BinaryOperator::Multiply,
            Expression::Number(steps as f64),
            step.clone(),
        ),
    }
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

fn across_constraints(
    steps: &[(i64, Course)],
    earlier: EntityId,
    later: EntityId,
) -> Vec<Constraint> {
    let along = |function: Function, part: fn(Vector2) -> f64| -> Option<Expression> {
        steps
            .iter()
            .filter_map(|(count, course)| {
                let angle = course.angle()?;
                let tolerance = AXIS_TOLERANCE * course.vector.length();

                if angle.expression.is_literal() && part(course.vector).abs() <= tolerance {
                    return None;
                }

                let sign = if course.reversed { -1.0 } else { 1.0 };
                let projected = Expression::binary(
                    BinaryOperator::Multiply,
                    course.spacing.expression.clone(),
                    Expression::Call(function, vec![angle.expression.clone()]),
                );
                Some(Expression::binary(
                    BinaryOperator::Multiply,
                    Expression::Number(*count as f64 * sign),
                    projected,
                ))
            })
            .reduce(|sum, term| Expression::binary(BinaryOperator::Add, sum, term))
            .map(|sum| Expression::Call(Function::Abs, vec![sum]))
    };

    let across = match along(Function::Cos, |vector| vector.x) {
        Some(value) => Constraint::HorizontalDistance {
            from: earlier,
            to: later,
            value,
        },
        None => Constraint::VerticalPoints(earlier, later.into()),
    };
    let up = match along(Function::Sin, |vector| vector.y) {
        Some(value) => Constraint::VerticalDistance {
            from: earlier,
            to: later,
            value,
        },
        None => Constraint::HorizontalPoints(earlier, later.into()),
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
