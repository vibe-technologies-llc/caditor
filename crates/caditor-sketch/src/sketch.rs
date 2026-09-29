use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    constraint::{Constraint, DimensionError},
    curve::{ArcGeometry, BSpline},
    entity::{Entity, Role},
    id::{ConstraintId, EntityId, FIRST_UNSTORABLE_ID, Reference},
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchError {
    #[error("the constraint no longer exists")]
    MissingConstraint(ConstraintId),
    #[error("the geometry it changes no longer exists")]
    NoSuchEntity(EntityId),
    #[error("{label} is reference geometry, which cannot be removed or changed")]
    ReferenceGeometry { entity: EntityId, label: String },
    #[error("{label} is still used by other geometry or constraints")]
    InUse { entity: EntityId, label: String },
    #[error("only the position or size of {label} can change, not its kind or the points it uses")]
    ChangesStructure { entity: EntityId, label: String },
    #[error("it uses an entity that does not exist")]
    MissingEntity(EntityId),
    #[error("it needs a point where the sketch has other geometry")]
    NotAPoint(EntityId),
    #[error("it would replace geometry or a constraint that already exists")]
    DuplicateId(u64),
    #[error("it would replace the sketch's origin or axes")]
    ReservedId(u64),
    #[error("a point must have finite coordinates")]
    NotFinite,
    #[error("a circle's radius must be a finite number greater than zero")]
    InvalidRadius,
    #[error("a spline needs at least two control points")]
    TooFewControlPoints,
    #[error("it uses {label} twice")]
    SameEntity { entity: EntityId, label: String },
    #[error("it needs {needed}, but {found} is not one")]
    WrongKind {
        entity: EntityId,
        found: String,
        needed: &'static str,
    },
    #[error("{constraint} does not apply to {first} and {second}")]
    NotApplicable {
        constraint: &'static str,
        first: String,
        second: String,
    },
    #[error("it only uses reference geometry, which never moves")]
    OnlyReference,
    #[error("{point} is part of {curve}")]
    OwnPoint { point: String, curve: String },
    #[error("{first} and {second} already share their centre")]
    SharedCentre { first: String, second: String },
    #[error("the constraint has no value to set")]
    NotADimension(ConstraintId),
    #[error("{reason}")]
    Dimension {
        constraint: ConstraintId,
        reason: DimensionError,
    },
    #[error("solving was cancelled")]
    Cancelled,
    #[error("some constraints conflict with each other")]
    Conflict { constraints: Vec<ConstraintId> },
    #[error("the sketch could not be solved from its current shape")]
    Unsolvable {
        entities: Vec<EntityId>,
        newest: Option<ConstraintId>,
    },
    #[error("{label} has no length")]
    NoLength { entity: EntityId, label: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DimensionValues {
    values: BTreeMap<ConstraintId, f64>,
}

impl DimensionValues {
    pub fn dimension(&self, constraint: ConstraintId) -> Option<f64> {
        self.values.get(&constraint).copied()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (ConstraintId, f64)> + '_ {
        self.values.iter().map(|(id, value)| (*id, *value))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sketch {
    plane: Plane,
    entities: BTreeMap<EntityId, Entity>,
    constraints: BTreeMap<ConstraintId, Constraint>,
    uses: BTreeMap<EntityId, usize>,
    next_id: u64,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Self {
            plane,
            entities: BTreeMap::new(),
            constraints: BTreeMap::new(),
            uses: BTreeMap::new(),
            next_id: 0,
        }
    }

    pub fn plane(&self) -> Plane {
        self.plane
    }

    pub fn set_plane(&mut self, plane: Plane) {
        self.plane = plane;
    }

    pub fn entities(&self) -> impl ExactSizeIterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, entity)| (*id, entity))
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        id.is_reference() || self.entities.contains_key(&id)
    }

    pub fn entity_label(&self, id: EntityId) -> String {
        if let Some(reference) = id.reference() {
            return reference.label().to_owned();
        }
        match self.entities.get(&id) {
            Some(entity) => format!("{} {id}", entity.kind_name()),
            None => format!("Missing entity {id}"),
        }
    }

    pub fn point(&self, id: EntityId) -> Option<Point2> {
        if id == EntityId::ORIGIN {
            return Some(Point2::ZERO);
        }
        match self.entities.get(&id)? {
            Entity::Point(position) => Some(*position),
            _ => None,
        }
    }

    pub fn line_endpoints(&self, id: EntityId) -> Option<(Point2, Point2)> {
        match self.entities.get(&id)? {
            Entity::Line { start, end } => Some((self.point(*start)?, self.point(*end)?)),
            _ => None,
        }
    }

    pub fn line_direction(&self, id: EntityId) -> Option<Vector2> {
        match id.reference() {
            Some(Reference::HorizontalAxis) => Some(Vector2::X),
            Some(Reference::VerticalAxis) => Some(Vector2::Y),
            Some(Reference::Origin) => None,
            None => self.line_endpoints(id).map(|(start, end)| end - start),
        }
    }

    pub fn circle(&self, id: EntityId) -> Option<(Point2, f64)> {
        match *self.entities.get(&id)? {
            Entity::Circle { center, radius } => Some((self.point(center)?, radius)),
            Entity::Arc { center, start, .. } => {
                let center = self.point(center)?;
                Some((center, self.point(start)?.distance(center)))
            }
            _ => None,
        }
    }

    pub fn arc(&self, id: EntityId) -> Option<ArcGeometry> {
        match *self.entities.get(&id)? {
            Entity::Arc { center, start, end } => Some(ArcGeometry::from_points(
                self.point(center)?,
                self.point(start)?,
                self.point(end)?,
            )),
            _ => None,
        }
    }

    pub fn spline(&self, id: EntityId) -> Option<BSpline> {
        match self.entities.get(&id)? {
            Entity::Spline { control_points } => BSpline::clamped(
                control_points
                    .iter()
                    .map(|point| self.point(*point))
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => None,
        }
    }

    pub fn polyline(&self, id: EntityId, max_segment_angle: f64) -> Option<Vec<Point2>> {
        match self.entities.get(&id)? {
            Entity::Point(_) => None,
            Entity::Line { .. } => self.line_endpoints(id).map(|(start, end)| vec![start, end]),
            Entity::Circle { .. } => self.circle(id).map(|(center, radius)| {
                ArcGeometry::full_circle(center, radius).polyline(max_segment_angle)
            }),
            Entity::Arc { .. } => self.arc(id).map(|arc| arc.polyline(max_segment_angle)),
            Entity::Spline { .. } => self
                .spline(id)
                .map(|spline| spline.polyline(max_segment_angle)),
        }
    }

    pub fn constraints(&self) -> impl ExactSizeIterator<Item = (ConstraintId, &Constraint)> {
        self.constraints
            .iter()
            .map(|(id, constraint)| (*id, constraint))
    }

    pub fn constraint(&self, id: ConstraintId) -> Option<&Constraint> {
        self.constraints.get(&id)
    }

    pub fn describe_constraint(&self, id: ConstraintId) -> String {
        match self.constraints.get(&id) {
            Some(constraint) => self.describe(constraint),
            None => format!("Missing constraint {id}"),
        }
    }

    pub fn describe(&self, constraint: &Constraint) -> String {
        let label = |entity: EntityId| self.entity_label(entity);
        let kind = constraint.kind_name();
        match *constraint {
            Constraint::Horizontal(entity)
            | Constraint::Vertical(entity)
            | Constraint::Fix { point: entity, .. } => {
                format!("{kind} {}", label(entity))
            }
            Constraint::Coincident(a, b)
            | Constraint::HorizontalPoints(a, b)
            | Constraint::VerticalPoints(a, b)
            | Constraint::Parallel(a, b)
            | Constraint::Perpendicular(a, b)
            | Constraint::Tangent(a, b)
            | Constraint::Equal(a, b)
            | Constraint::Concentric(a, b)
            | Constraint::Collinear(a, b) => format!("{kind} {} and {}", label(a), label(b)),
            Constraint::Midpoint { point, line } => {
                format!("{kind} of {} at {}", label(line), label(point))
            }
            Constraint::Symmetric {
                first,
                second,
                about,
            } => format!(
                "{kind} {} and {} about {}",
                label(first),
                label(second),
                label(about)
            ),
            Constraint::Distance { from, to, .. }
            | Constraint::HorizontalDistance { from, to, .. }
            | Constraint::VerticalDistance { from, to, .. }
            | Constraint::Angle { from, to, .. } => {
                format!("{kind} between {} and {}", label(from), label(to))
            }
            Constraint::Radius { entity, .. } | Constraint::Diameter { entity, .. } => {
                format!("{kind} of {}", label(entity))
            }
        }
    }

    pub fn measured(&self, constraint: &Constraint) -> Option<f64> {
        let value = match *constraint {
            Constraint::Distance { from, to, .. } => self.distance_between(from, to)?,
            Constraint::HorizontalDistance { from, to, .. } => {
                (self.point(to)?.x - self.point(from)?.x).abs()
            }
            Constraint::VerticalDistance { from, to, .. } => {
                (self.point(to)?.y - self.point(from)?.y).abs()
            }
            Constraint::Angle {
                from, to, reversed, ..
            } => {
                let (from, to) = (self.line_direction(from)?, self.line_direction(to)?);
                let from = if reversed { -from } else { from };
                from.perp_dot(to).atan2(from.dot(to)).to_degrees()
            }
            Constraint::Radius { entity, .. } => self.circle(entity)?.1,
            Constraint::Diameter { entity, .. } => 2.0 * self.circle(entity)?.1,
            Constraint::Coincident(..)
            | Constraint::Horizontal(_)
            | Constraint::Vertical(_)
            | Constraint::HorizontalPoints(..)
            | Constraint::VerticalPoints(..)
            | Constraint::Parallel(..)
            | Constraint::Perpendicular(..)
            | Constraint::Tangent(..)
            | Constraint::Equal(..)
            | Constraint::Midpoint { .. }
            | Constraint::Concentric(..)
            | Constraint::Collinear(..)
            | Constraint::Symmetric { .. }
            | Constraint::Fix { .. } => return None,
        };
        value.is_finite().then_some(value)
    }

    fn distance_between(&self, from: EntityId, to: EntityId) -> Option<f64> {
        match (self.role(from)?, self.role(to)?) {
            (Role::Point, Role::Point) => Some(self.point(from)?.distance(self.point(to)?)),
            (Role::Point, Role::Line) => self.distance_to_line(self.point(from)?, to),
            (Role::Line, Role::Point) => self.distance_to_line(self.point(to)?, from),
            (Role::Point, Role::Circular) => self.distance_to_circle(self.point(from)?, to),
            (Role::Circular, Role::Point) => self.distance_to_circle(self.point(to)?, from),
            (Role::Line, Role::Line) => {
                let (anchor, other) = if to.is_reference() {
                    (to, from)
                } else {
                    (from, to)
                };
                let (start, end) = self.line_endpoints(other)?;
                self.distance_to_line((start + end) / 2.0, anchor)
            }
            _ => None,
        }
    }

    fn distance_to_line(&self, point: Point2, line: EntityId) -> Option<f64> {
        let direction = self.line_direction(line)?.try_normalize()?;
        let anchor = match self.line_endpoints(line) {
            Some((start, _)) => start,
            None => Point2::ZERO,
        };
        Some(direction.perp_dot(point - anchor).abs())
    }

    fn distance_to_circle(&self, point: Point2, circle: EntityId) -> Option<f64> {
        let (center, radius) = self.circle(circle)?;
        Some((point.distance(center) - radius).abs())
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.constraints
            .values()
            .filter_map(Constraint::dimension)
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.constraints
            .values()
            .filter_map(Constraint::dimension)
            .any(|expression| expression.uses(parameter))
    }

    pub fn same_geometry(&self, other: &Self) -> bool {
        self.plane == other.plane && self.entities == other.entities
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.plane == other.plane
            && self.entities == other.entities
            && self.constraints == other.constraints
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn reserve_ids_below(&mut self, next_id: u64) {
        self.next_id = self.next_id.max(next_id.min(FIRST_UNSTORABLE_ID));
    }

    pub fn insert_entity(&mut self, id: EntityId, entity: Entity) -> Result<(), SketchError> {
        self.check_new_id(id.raw())?;
        self.check_entity(&entity)?;
        self.count_uses(&entity.points(), true);
        self.entities.insert(id, entity);
        self.reserve_ids_below(id.raw().saturating_add(1));
        Ok(())
    }

    pub fn insert_constraint(
        &mut self,
        id: ConstraintId,
        constraint: Constraint,
    ) -> Result<(), SketchError> {
        self.check_new_id(id.raw())?;
        self.check_constraint(&constraint)?;
        self.count_uses(&constraint.entities(), true);
        self.constraints.insert(id, constraint);
        self.reserve_ids_below(id.raw().saturating_add(1));
        Ok(())
    }

    pub fn add_point(&mut self, position: Point2) -> EntityId {
        self.insert(Entity::Point(position))
    }

    pub fn add_line(&mut self, start: Point2, end: Point2) -> EntityId {
        let start = self.add_point(start);
        let end = self.add_point(end);
        self.insert(Entity::Line { start, end })
    }

    pub fn add_circle(&mut self, center: Point2, radius: f64) -> EntityId {
        let center = self.add_point(center);
        self.insert(Entity::Circle { center, radius })
    }

    pub fn add_arc(&mut self, center: Point2, start: Point2, end: Point2) -> EntityId {
        let center = self.add_point(center);
        let start = self.add_point(start);
        let end = self.add_point(end);
        self.insert(Entity::Arc { center, start, end })
    }

    pub fn add_spline(&mut self, control_points: &[Point2]) -> EntityId {
        let control_points = control_points
            .iter()
            .map(|point| self.add_point(*point))
            .collect();
        self.insert(Entity::Spline { control_points })
    }

    pub fn add_constraint(&mut self, constraint: Constraint) -> Result<ConstraintId, SketchError> {
        self.check_constraint(&constraint)?;
        let id = ConstraintId::from_raw(self.allocate());
        self.count_uses(&constraint.entities(), true);
        self.constraints.insert(id, constraint);
        Ok(id)
    }

    pub fn check_constraint(&self, constraint: &Constraint) -> Result<(), SketchError> {
        let entities = constraint.entities();
        if let Some(missing) = entities.iter().find(|entity| !self.contains(**entity)) {
            return Err(SketchError::MissingEntity(*missing));
        }
        let mut seen = BTreeSet::new();
        if let Some(repeated) = entities.iter().find(|entity| !seen.insert(**entity)) {
            return Err(SketchError::SameEntity {
                entity: *repeated,
                label: self.entity_label(*repeated),
            });
        }
        match *constraint {
            Constraint::Coincident(a, b) => self.check_point_on_curve(constraint, a, b),
            Constraint::Horizontal(line) | Constraint::Vertical(line) => {
                self.expect(line, &[Role::Line], "a line")?;
                self.check_not_only_reference(&entities)
            }
            Constraint::HorizontalPoints(a, b)
            | Constraint::VerticalPoints(a, b)
            | Constraint::HorizontalDistance { from: a, to: b, .. }
            | Constraint::VerticalDistance { from: a, to: b, .. } => {
                self.expect(a, &[Role::Point], "a point")?;
                self.expect(b, &[Role::Point], "a point")?;
                self.check_not_only_reference(&entities)
            }
            Constraint::Parallel(a, b)
            | Constraint::Perpendicular(a, b)
            | Constraint::Collinear(a, b)
            | Constraint::Angle { from: a, to: b, .. } => {
                self.expect(a, &[Role::Line], "a line")?;
                self.expect(b, &[Role::Line], "a line")?;
                self.check_not_only_reference(&entities)
            }
            Constraint::Tangent(a, b) => {
                let needed = "a line, a circle or an arc";
                let first = self.expect(a, &[Role::Line, Role::Circular], needed)?;
                let second = self.expect(b, &[Role::Line, Role::Circular], needed)?;
                if first == Role::Line && second == Role::Line {
                    return Err(self.not_applicable(constraint, a, b));
                }
                Ok(())
            }
            Constraint::Equal(a, b) => {
                let needed = "a line, a circle or an arc";
                let first = self.expect(a, &[Role::Line, Role::Circular], needed)?;
                let second = self.expect(b, &[Role::Line, Role::Circular], needed)?;
                if first != second || a.is_reference() || b.is_reference() {
                    return Err(self.not_applicable(constraint, a, b));
                }
                Ok(())
            }
            Constraint::Midpoint { point, line } => {
                self.expect(point, &[Role::Point], "a point")?;
                self.expect(line, &[Role::Line], "a line")?;
                if line.is_reference() {
                    return Err(self.not_applicable(constraint, point, line));
                }
                self.check_not_own_point(point, line)
            }
            Constraint::Concentric(a, b) => self.check_concentric(constraint, a, b),
            Constraint::Symmetric {
                first,
                second,
                about,
            } => {
                self.expect(first, &[Role::Point], "a point")?;
                self.expect(second, &[Role::Point], "a point")?;
                self.expect(about, &[Role::Point, Role::Line], "a point or a line")?;
                self.check_not_only_reference(&entities)?;
                self.check_not_own_point(first, about)?;
                self.check_not_own_point(second, about)
            }
            Constraint::Fix { point, at } => {
                self.expect(point, &[Role::Point], "a point")?;
                self.check_not_only_reference(&entities)?;
                if at.is_finite() {
                    Ok(())
                } else {
                    Err(SketchError::NotFinite)
                }
            }
            Constraint::Distance { from, to, .. } => {
                if (self.role(from), self.role(to)) == (Some(Role::Line), Some(Role::Line)) {
                    return self.check_not_only_reference(&entities);
                }
                self.check_point_on_curve(constraint, from, to)
            }
            Constraint::Radius { entity, .. } | Constraint::Diameter { entity, .. } => self
                .expect(entity, &[Role::Circular], "a circle or an arc")
                .map(|_| ()),
        }
    }

    pub fn set_dimension(
        &mut self,
        id: ConstraintId,
        value: Expression,
    ) -> Result<Expression, SketchError> {
        let dimension = self
            .constraints
            .get_mut(&id)
            .ok_or(SketchError::MissingConstraint(id))?
            .dimension_mut()
            .ok_or(SketchError::NotADimension(id))?;
        Ok(std::mem::replace(dimension, value))
    }

    pub fn entities_using(&self, id: EntityId) -> Vec<EntityId> {
        self.entities
            .iter()
            .filter(|(_, entity)| entity.references(id))
            .map(|(user, _)| *user)
            .collect()
    }

    pub fn constraints_using(&self, id: EntityId) -> Vec<ConstraintId> {
        self.constraints
            .iter()
            .filter(|(_, constraint)| constraint.references(id))
            .map(|(user, _)| *user)
            .collect()
    }

    pub fn remove_unused_entity(&mut self, id: EntityId) -> Result<Entity, SketchError> {
        self.check_editable(id)?;
        if self.uses.contains_key(&id) {
            return Err(SketchError::InUse {
                entity: id,
                label: self.entity_label(id),
            });
        }
        let removed = self
            .entities
            .remove(&id)
            .ok_or(SketchError::NoSuchEntity(id))?;
        self.count_uses(&removed.points(), false);
        Ok(removed)
    }

    pub fn replace_entity(&mut self, id: EntityId, entity: Entity) -> Result<Entity, SketchError> {
        self.check_editable(id)?;
        let unchanged_structure = self
            .entities
            .get(&id)
            .is_some_and(|current| current.same_structure(&entity));
        if !unchanged_structure {
            return Err(SketchError::ChangesStructure {
                entity: id,
                label: self.entity_label(id),
            });
        }
        self.check_entity(&entity)?;
        self.entities
            .insert(id, entity)
            .ok_or(SketchError::NoSuchEntity(id))
    }

    pub fn remove_constraint(&mut self, id: ConstraintId) -> Result<Constraint, SketchError> {
        let removed = self
            .constraints
            .remove(&id)
            .ok_or(SketchError::MissingConstraint(id))?;
        self.count_uses(&removed.entities(), false);
        Ok(removed)
    }

    pub fn remove_entity(&mut self, id: EntityId) -> Option<Entity> {
        let removed = self.entities.remove(&id)?;
        let mut gone = BTreeSet::from([id]);
        let mut dropped = vec![removed.clone()];
        loop {
            let dependents: Vec<EntityId> = self
                .entities
                .iter()
                .filter(|(_, entity)| entity.points().iter().any(|point| gone.contains(point)))
                .map(|(dependent, _)| *dependent)
                .collect();
            if dependents.is_empty() {
                break;
            }
            for dependent in dependents {
                if let Some(entity) = self.entities.remove(&dependent) {
                    dropped.push(entity);
                }
                gone.insert(dependent);
            }
        }
        let doomed: Vec<ConstraintId> = self
            .constraints
            .iter()
            .filter(|(_, constraint)| constraint.entities().iter().any(|used| gone.contains(used)))
            .map(|(constraint, _)| *constraint)
            .collect();
        for constraint in doomed {
            if let Some(constraint) = self.constraints.remove(&constraint) {
                self.count_uses(&constraint.entities(), false);
            }
        }
        for entity in &dropped {
            self.count_uses(&entity.points(), false);
        }
        self.uses.retain(|used, _| !gone.contains(used));
        Some(removed)
    }

    fn count_uses(&mut self, used: &[EntityId], add: bool) {
        for id in used {
            if add {
                *self.uses.entry(*id).or_default() += 1;
            } else if let Some(count) = self.uses.get_mut(id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.uses.remove(id);
                }
            }
        }
    }

    #[cfg(test)]
    fn counted_from_scratch(&self) -> BTreeMap<EntityId, usize> {
        let mut fresh = Self::new(self.plane);
        let used: Vec<EntityId> = self
            .entities
            .values()
            .flat_map(Entity::points)
            .chain(self.constraints.values().flat_map(Constraint::entities))
            .collect();
        fresh.count_uses(&used, true);
        fresh.uses
    }

    pub fn evaluate<F>(&self, value_of: &F) -> Result<DimensionValues, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let mut values = BTreeMap::new();
        for (id, constraint) in &self.constraints {
            let (Some(expression), Some(kind)) =
                (constraint.dimension(), constraint.dimension_kind())
            else {
                continue;
            };
            let failed = |reason| SketchError::Dimension {
                constraint: *id,
                reason,
            };
            let value = expression
                .evaluate_as(kind, value_of)
                .map_err(|error| failed(DimensionError::Evaluation(error)))?;
            constraint.check_dimension_value(value).map_err(failed)?;
            values.insert(*id, value);
        }
        Ok(DimensionValues { values })
    }

    pub(crate) fn role(&self, id: EntityId) -> Option<Role> {
        match id.reference() {
            Some(Reference::Origin) => Some(Role::Point),
            Some(Reference::HorizontalAxis | Reference::VerticalAxis) => Some(Role::Line),
            None => self.entities.get(&id).map(Entity::role),
        }
    }

    pub(crate) fn set_point(&mut self, id: EntityId, position: Point2) {
        if let Some(Entity::Point(stored)) = self.entities.get_mut(&id) {
            *stored = position;
        }
    }

    pub(crate) fn set_radius(&mut self, id: EntityId, value: f64) {
        if let Some(Entity::Circle { radius, .. }) = self.entities.get_mut(&id) {
            *radius = value;
        }
    }

    fn check_editable(&self, id: EntityId) -> Result<(), SketchError> {
        if id.is_reference() {
            return Err(SketchError::ReferenceGeometry {
                entity: id,
                label: self.entity_label(id),
            });
        }
        if !self.entities.contains_key(&id) {
            return Err(SketchError::NoSuchEntity(id));
        }
        Ok(())
    }

    fn check_new_id(&self, raw: u64) -> Result<(), SketchError> {
        if raw >= FIRST_UNSTORABLE_ID {
            return Err(SketchError::ReservedId(raw));
        }
        let taken = self.entities.contains_key(&EntityId::from_raw(raw))
            || self.constraints.contains_key(&ConstraintId::from_raw(raw));
        if taken {
            return Err(SketchError::DuplicateId(raw));
        }
        Ok(())
    }

    fn check_entity(&self, entity: &Entity) -> Result<(), SketchError> {
        match *entity {
            Entity::Point(position) if !position.is_finite() => return Err(SketchError::NotFinite),
            Entity::Circle { radius, .. } if !(radius.is_finite() && radius > 0.0) => {
                return Err(SketchError::InvalidRadius);
            }
            Entity::Spline { ref control_points } if control_points.len() < 2 => {
                return Err(SketchError::TooFewControlPoints);
            }
            _ => {}
        }
        let points = entity.points();
        for point in &points {
            match self.entities.get(point) {
                Some(Entity::Point(_)) => {}
                Some(_) => return Err(SketchError::NotAPoint(*point)),
                None => return Err(SketchError::MissingEntity(*point)),
            }
        }
        let distinct_points_needed = matches!(entity, Entity::Line { .. } | Entity::Arc { .. });
        let mut seen = BTreeSet::new();
        match points.into_iter().find(|point| !seen.insert(*point)) {
            Some(repeated) if distinct_points_needed => Err(SketchError::SameEntity {
                entity: repeated,
                label: self.entity_label(repeated),
            }),
            _ => Ok(()),
        }
    }

    fn expect(
        &self,
        entity: EntityId,
        roles: &[Role],
        needed: &'static str,
    ) -> Result<Role, SketchError> {
        let role = self
            .role(entity)
            .ok_or(SketchError::MissingEntity(entity))?;
        if roles.contains(&role) {
            Ok(role)
        } else {
            Err(SketchError::WrongKind {
                entity,
                found: self.entity_label(entity),
                needed,
            })
        }
    }

    fn check_not_only_reference(&self, entities: &[EntityId]) -> Result<(), SketchError> {
        if entities.iter().all(|entity| entity.is_reference()) {
            Err(SketchError::OnlyReference)
        } else {
            Ok(())
        }
    }

    fn check_point_on_curve(
        &self,
        constraint: &Constraint,
        a: EntityId,
        b: EntityId,
    ) -> Result<(), SketchError> {
        let roles = (self.role(a), self.role(b));
        let (point, curve) = match roles {
            (Some(Role::Point), Some(Role::Point)) => {
                return self.check_not_only_reference(&[a, b]);
            }
            (Some(Role::Point), Some(Role::Line | Role::Circular)) => (a, b),
            (Some(Role::Line | Role::Circular), Some(Role::Point)) => (b, a),
            _ => return Err(self.not_applicable(constraint, a, b)),
        };
        self.check_not_only_reference(&[a, b])?;
        self.check_not_own_point(point, curve)
    }

    fn check_not_own_point(&self, point: EntityId, curve: EntityId) -> Result<(), SketchError> {
        if self
            .entities
            .get(&curve)
            .is_some_and(|entity| entity.references(point))
        {
            return Err(SketchError::OwnPoint {
                point: self.entity_label(point),
                curve: self.entity_label(curve),
            });
        }
        Ok(())
    }

    fn check_concentric(
        &self,
        constraint: &Constraint,
        a: EntityId,
        b: EntityId,
    ) -> Result<(), SketchError> {
        let needed = "a circle, an arc or a point";
        let first = self.expect(a, &[Role::Circular, Role::Point], needed)?;
        let second = self.expect(b, &[Role::Circular, Role::Point], needed)?;
        match (first, second) {
            (Role::Circular, Role::Circular) => {
                if self.center_of(a).is_some() && self.center_of(a) == self.center_of(b) {
                    return Err(SketchError::SharedCentre {
                        first: self.entity_label(a),
                        second: self.entity_label(b),
                    });
                }
                Ok(())
            }
            (Role::Point, Role::Circular) => self.check_not_own_point(a, b),
            (Role::Circular, Role::Point) => self.check_not_own_point(b, a),
            _ => Err(self.not_applicable(constraint, a, b)),
        }
    }

    pub fn center_of(&self, curve: EntityId) -> Option<EntityId> {
        match self.entities.get(&curve)? {
            Entity::Circle { center, .. } | Entity::Arc { center, .. } => Some(*center),
            Entity::Point(_) | Entity::Line { .. } | Entity::Spline { .. } => None,
        }
    }

    fn not_applicable(&self, constraint: &Constraint, a: EntityId, b: EntityId) -> SketchError {
        SketchError::NotApplicable {
            constraint: constraint.kind_name(),
            first: self.entity_label(a),
            second: self.entity_label(b),
        }
    }

    fn insert(&mut self, entity: Entity) -> EntityId {
        let id = EntityId::from_raw(self.allocate());
        self.count_uses(&entity.points(), true);
        self.entities.insert(id, entity);
        id
    }

    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::Dimension;

    use super::*;

    const WIDTH: ParameterId = ParameterId::from_raw(0);

    fn endpoints(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
        let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
            panic!("expected a line");
        };
        (start, end)
    }

    fn distance(value: Expression) -> (Sketch, ConstraintId) {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let (start, end) = endpoints(&sketch, line);
        let constraint = sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value,
            })
            .unwrap();
        (sketch, constraint)
    }

    fn width_is(value: Quantity) -> impl Fn(ParameterId) -> Result<Quantity, EvalError> {
        move |_| Ok(value)
    }

    #[test]
    fn line_adds_its_endpoints_as_points() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();

        assert_eq!(sketch.entities().len(), 3);
        let (start, end) = endpoints(&sketch, line);
        assert_eq!(sketch.entity(start), Some(&Entity::Point(Point2::ZERO)));
        assert_eq!(sketch.entity(end), Some(&Entity::Point(Point2::X)));
        assert_eq!(sketch.line_endpoints(line), Some((Point2::ZERO, Point2::X)));
        assert_eq!(sketch.point(end), Some(Point2::X));
        assert_eq!(sketch.point(line), None);
        assert_eq!(sketch.line_endpoints(start), None);
    }

    #[test]
    fn removal_keeps_other_ids_stable_and_never_reuses_them() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_point(Point2::ZERO);
        let second = sketch.add_point(Point2::X);

        sketch.remove_entity(first);
        let third = sketch.add_point(Point2::Y);

        assert_eq!(sketch.entity(second), Some(&Entity::Point(Point2::X)));
        assert_ne!(third, first);
        assert_eq!(sketch.entity(first), None);
    }

    #[test]
    fn removing_a_point_removes_dependent_curves_and_constraints() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        let (start, end) = endpoints(&sketch, line);
        let arc = sketch.add_arc(Point2::ZERO, Point2::X, Point2::Y);
        let spline = sketch.add_spline(&[Point2::ZERO, Point2::X, Point2::Y]);
        let circle = sketch.add_circle(Point2::ZERO, 2.0);
        sketch
            .add_constraint(Constraint::Tangent(line, circle))
            .unwrap();
        let Some(Entity::Arc { center, .. }) = sketch.entity(arc).cloned() else {
            panic!("expected an arc");
        };
        let Some(Entity::Spline { control_points }) = sketch.entity(spline).cloned() else {
            panic!("expected a spline");
        };

        sketch.remove_entity(start);
        assert_eq!(sketch.uses, sketch.counted_from_scratch());
        sketch.remove_entity(center);
        assert_eq!(sketch.uses, sketch.counted_from_scratch());
        sketch.remove_entity(control_points[1]);
        assert_eq!(sketch.uses, sketch.counted_from_scratch());

        assert_eq!(sketch.entity(line), None);
        assert_eq!(sketch.entity(arc), None);
        assert_eq!(sketch.entity(spline), None);
        assert!(sketch.entity(end).is_some());
        assert!(sketch.entity(circle).is_some());
        assert_eq!(sketch.constraints().len(), 0);
        assert_eq!(sketch.remove_entity(EntityId::ORIGIN), None);
    }

    #[test]
    fn inserting_with_explicit_ids_checks_references_and_keeps_ids_unique() {
        let mut original = Sketch::new(Plane::XY);
        let line = original.add_line(Point2::ZERO, Point2::X);
        let horizontal = original
            .add_constraint(Constraint::Horizontal(line))
            .unwrap();
        original.reserve_ids_below(10);

        let mut rebuilt = Sketch::new(Plane::XY);
        let mut entities: Vec<_> = original
            .entities()
            .map(|(id, entity)| (id, entity.clone()))
            .collect();
        entities.reverse();
        let (lines, points): (Vec<_>, Vec<_>) = entities
            .into_iter()
            .partition(|(_, entity)| matches!(entity, Entity::Line { .. }));
        assert_eq!(
            rebuilt.insert_entity(lines[0].0, lines[0].1.clone()),
            Err(SketchError::MissingEntity(EntityId::from_raw(0)))
        );
        for (id, entity) in points.into_iter().chain(lines) {
            rebuilt.insert_entity(id, entity).unwrap();
        }
        assert_eq!(
            rebuilt.insert_constraint(
                ConstraintId::from_raw(line.raw()),
                Constraint::Vertical(line)
            ),
            Err(SketchError::DuplicateId(line.raw()))
        );
        assert_eq!(
            rebuilt.insert_constraint(
                ConstraintId::from_raw(8),
                Constraint::Vertical(EntityId::from_raw(7))
            ),
            Err(SketchError::MissingEntity(EntityId::from_raw(7)))
        );
        rebuilt
            .insert_constraint(horizontal, Constraint::Horizontal(line))
            .unwrap();
        rebuilt.reserve_ids_below(original.next_id());

        assert_eq!(rebuilt, original);
        assert_eq!(
            rebuilt.insert_entity(
                EntityId::from_raw(5),
                Entity::Line {
                    start: line,
                    end: EntityId::from_raw(0)
                }
            ),
            Err(SketchError::NotAPoint(line))
        );
        assert_eq!(
            rebuilt.insert_entity(
                EntityId::from_raw(6),
                Entity::Point(Point2::new(f64::INFINITY, 0.0))
            ),
            Err(SketchError::NotFinite)
        );
        assert_eq!(rebuilt.add_point(Point2::Y), EntityId::from_raw(10));
    }

    #[test]
    fn new_entity_kinds_check_their_points_and_values() {
        let mut sketch = Sketch::new(Plane::XY);
        let a = sketch.add_point(Point2::ZERO);
        let b = sketch.add_point(Point2::X);
        let insert = |sketch: &mut Sketch, raw: u64, entity: Entity| {
            sketch.insert_entity(EntityId::from_raw(raw), entity)
        };

        assert_eq!(
            insert(
                &mut sketch,
                10,
                Entity::Circle {
                    center: a,
                    radius: 0.0
                }
            ),
            Err(SketchError::InvalidRadius)
        );
        assert_eq!(
            insert(
                &mut sketch,
                10,
                Entity::Circle {
                    center: EntityId::ORIGIN,
                    radius: 1.0
                }
            ),
            Err(SketchError::MissingEntity(EntityId::ORIGIN))
        );
        assert_eq!(
            insert(
                &mut sketch,
                10,
                Entity::Arc {
                    center: a,
                    start: b,
                    end: b
                }
            )
            .unwrap_err()
            .to_string(),
            "it uses Point 1 twice"
        );
        assert_eq!(
            insert(
                &mut sketch,
                10,
                Entity::Spline {
                    control_points: vec![a]
                }
            ),
            Err(SketchError::TooFewControlPoints)
        );
        assert_eq!(
            insert(&mut sketch, u64::MAX - 1, Entity::Point(Point2::ZERO)),
            Err(SketchError::ReservedId(u64::MAX - 1))
        );
        insert(
            &mut sketch,
            10,
            Entity::Circle {
                center: a,
                radius: 2.0,
            },
        )
        .unwrap();
        insert(
            &mut sketch,
            11,
            Entity::Spline {
                control_points: vec![a, b, a],
            },
        )
        .unwrap();

        sketch.reserve_ids_below(u64::MAX);
        let next = sketch.add_point(Point2::ZERO);
        assert!(!next.is_reference());
        assert!(!sketch.add_point(Point2::ZERO).is_reference());
    }

    #[test]
    fn reference_geometry_is_built_in_and_named() {
        let mut sketch = Sketch::new(Plane::XY);
        assert_eq!(sketch.entity_label(EntityId::ORIGIN), "Origin");
        assert_eq!(
            sketch.entity_label(EntityId::HORIZONTAL_AXIS),
            "Horizontal axis"
        );
        assert_eq!(
            sketch.entity_label(EntityId::VERTICAL_AXIS),
            "Vertical axis"
        );
        assert_eq!(sketch.point(EntityId::ORIGIN), Some(Point2::ZERO));
        assert_eq!(
            sketch.line_direction(EntityId::VERTICAL_AXIS),
            Some(Vector2::Y)
        );
        assert!(EntityId::REFERENCES.iter().all(|id| id.is_reference()));
        assert_eq!(sketch.entities().len(), 0);

        let point = sketch.add_point(Point2::X);
        let coincident = sketch
            .add_constraint(Constraint::Coincident(point, EntityId::HORIZONTAL_AXIS))
            .unwrap();
        assert_eq!(
            sketch.describe_constraint(coincident),
            "Coincident Point 0 and Horizontal axis"
        );
        assert_eq!(
            sketch.check_constraint(&Constraint::Perpendicular(
                EntityId::HORIZONTAL_AXIS,
                EntityId::VERTICAL_AXIS
            )),
            Err(SketchError::OnlyReference)
        );
    }

    #[test]
    fn constraints_refuse_the_wrong_kinds_of_entity() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let (start, _) = endpoints(&sketch, line);
        let other = sketch.add_line(Point2::Y, Point2::new(1.0, 2.0));
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 1.0);
        let arc = sketch.add_arc(Point2::ZERO, Point2::X, Point2::Y);
        let spline = sketch.add_spline(&[Point2::ZERO, Point2::Y]);
        let refused = |constraint: Constraint| {
            sketch
                .check_constraint(&constraint)
                .unwrap_err()
                .to_string()
        };

        assert_eq!(
            refused(Constraint::Horizontal(start)),
            "it needs a line, but Point 0 is not one"
        );
        assert_eq!(
            refused(Constraint::Radius {
                entity: line,
                value: Expression::Number(1.0)
            }),
            "it needs a circle or an arc, but Line 2 is not one"
        );
        assert_eq!(
            refused(Constraint::Parallel(line, line)),
            "it uses Line 2 twice"
        );
        assert_eq!(
            refused(Constraint::Tangent(line, other)),
            "Tangent does not apply to Line 2 and Line 5"
        );
        assert_eq!(
            refused(Constraint::Equal(line, circle)),
            "Equal does not apply to Line 2 and Circle 7"
        );
        assert_eq!(
            refused(Constraint::Coincident(start, line)),
            "Point 0 is part of Line 2"
        );
        assert_eq!(
            refused(Constraint::Coincident(spline, start)),
            "Coincident does not apply to Spline 14 and Point 0"
        );
        assert_eq!(
            refused(Constraint::Horizontal(EntityId::HORIZONTAL_AXIS)),
            "it only uses reference geometry, which never moves"
        );
        assert_eq!(
            refused(Constraint::Angle {
                from: line,
                to: arc,
                reversed: false,
                value: Expression::Number(1.0)
            }),
            "it needs a line, but Arc 11 is not one"
        );

        for allowed in [
            Constraint::Coincident(start, circle),
            Constraint::Coincident(EntityId::ORIGIN, start),
            Constraint::Parallel(line, EntityId::VERTICAL_AXIS),
            Constraint::Tangent(arc, line),
            Constraint::Tangent(circle, arc),
            Constraint::Tangent(EntityId::HORIZONTAL_AXIS, circle),
            Constraint::Equal(arc, circle),
            Constraint::Equal(line, other),
            Constraint::Distance {
                from: other,
                to: start,
                value: Expression::Number(1.0),
            },
        ] {
            assert_eq!(sketch.check_constraint(&allowed), Ok(()), "{allowed:?}");
        }
    }

    #[test]
    fn every_constraint_kind_has_a_plain_label() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let other = sketch.add_line(Point2::Y, Point2::new(1.0, 2.0));
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 1.0);
        let arc = sketch.add_arc(Point2::ZERO, Point2::X, Point2::Y);
        let value = Expression::Number(1.0);
        let labels = [
            (Constraint::Tangent(line, arc), "Tangent Line 2 and Arc 11"),
            (
                Constraint::Angle {
                    from: line,
                    to: other,
                    reversed: false,
                    value: value.clone(),
                },
                "Angle between Line 2 and Line 5",
            ),
            (
                Constraint::Radius {
                    entity: circle,
                    value: value.clone(),
                },
                "Radius of Circle 7",
            ),
            (
                Constraint::Perpendicular(line, EntityId::VERTICAL_AXIS),
                "Perpendicular Line 2 and Vertical axis",
            ),
            (Constraint::Equal(circle, arc), "Equal Circle 7 and Arc 11"),
            (
                Constraint::Parallel(line, other),
                "Parallel Line 2 and Line 5",
            ),
        ];
        for (constraint, label) in labels {
            let id = sketch.add_constraint(constraint).unwrap();
            assert_eq!(sketch.describe_constraint(id), label);
        }
    }

    #[test]
    fn added_constraint_kinds_check_their_entities_and_have_plain_labels() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let (start, end) = endpoints(&sketch, line);
        let other = sketch.add_line(Point2::Y, Point2::new(1.0, 2.0));
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 1.0);
        let arc = sketch.add_arc(Point2::new(5.0, 5.0), Point2::new(6.0, 5.0), Point2::Y);
        let lone = sketch.add_point(Point2::new(3.0, 4.0));
        let center = sketch.center_of(circle).unwrap();
        let value = Expression::Number(1.0);
        let refused = |constraint: Constraint| {
            sketch
                .check_constraint(&constraint)
                .unwrap_err()
                .to_string()
        };

        assert_eq!(
            refused(Constraint::Midpoint { point: start, line }),
            "Point 0 is part of Line 2"
        );
        assert_eq!(
            refused(Constraint::Midpoint {
                point: lone,
                line: EntityId::HORIZONTAL_AXIS
            }),
            "Midpoint does not apply to Point 12 and Horizontal axis"
        );
        assert_eq!(
            refused(Constraint::Concentric(circle, center)),
            "Point 6 is part of Circle 7"
        );
        assert_eq!(
            refused(Constraint::Concentric(lone, start)),
            "Concentric does not apply to Point 12 and Point 0"
        );
        let arc_center = sketch.center_of(arc).unwrap();
        let mut shared = sketch.clone();
        let shared_arc = shared.insert_entity(
            EntityId::from_raw(40),
            Entity::Arc {
                center,
                start: arc_center,
                end: lone,
            },
        );
        assert_eq!(shared_arc, Ok(()));
        assert_eq!(
            shared
                .check_constraint(&Constraint::Concentric(circle, EntityId::from_raw(40)))
                .unwrap_err()
                .to_string(),
            "Circle 7 and Arc 40 already share their centre"
        );
        assert_eq!(
            refused(Constraint::Collinear(line, circle)),
            "it needs a line, but Circle 7 is not one"
        );
        assert_eq!(
            refused(Constraint::Symmetric {
                first: start,
                second: lone,
                about: start
            }),
            "it uses Point 0 twice"
        );
        assert_eq!(
            refused(Constraint::Symmetric {
                first: start,
                second: lone,
                about: line
            }),
            "Point 0 is part of Line 2"
        );
        assert_eq!(
            refused(Constraint::Fix {
                point: EntityId::ORIGIN,
                at: Point2::ZERO
            }),
            "it only uses reference geometry, which never moves"
        );
        assert_eq!(
            refused(Constraint::Fix {
                point: lone,
                at: Point2::new(f64::NAN, 0.0)
            }),
            "a point must have finite coordinates"
        );
        assert_eq!(
            refused(Constraint::HorizontalPoints(line, lone)),
            "it needs a point, but Line 2 is not one"
        );
        assert_eq!(
            refused(Constraint::Distance {
                from: circle,
                to: line,
                value: value.clone()
            }),
            "Distance does not apply to Circle 7 and Line 2"
        );
        assert_eq!(
            refused(Constraint::Diameter {
                entity: line,
                value: value.clone()
            }),
            "it needs a circle or an arc, but Line 2 is not one"
        );

        let labels = [
            (
                Constraint::Midpoint { point: lone, line },
                "Midpoint of Line 2 at Point 12",
            ),
            (
                Constraint::Concentric(arc, circle),
                "Concentric Arc 11 and Circle 7",
            ),
            (
                Constraint::Collinear(line, EntityId::HORIZONTAL_AXIS),
                "Collinear Line 2 and Horizontal axis",
            ),
            (
                Constraint::Symmetric {
                    first: start,
                    second: end,
                    about: EntityId::VERTICAL_AXIS,
                },
                "Symmetric Point 0 and Point 1 about Vertical axis",
            ),
            (
                Constraint::Fix {
                    point: lone,
                    at: Point2::new(3.0, 4.0),
                },
                "Fix Point 12",
            ),
            (
                Constraint::HorizontalPoints(lone, EntityId::ORIGIN),
                "Horizontal Point 12 and Origin",
            ),
            (
                Constraint::VerticalPoints(lone, start),
                "Vertical Point 12 and Point 0",
            ),
            (
                Constraint::HorizontalDistance {
                    from: start,
                    to: lone,
                    value: value.clone(),
                },
                "Horizontal distance between Point 0 and Point 12",
            ),
            (
                Constraint::VerticalDistance {
                    from: start,
                    to: lone,
                    value: value.clone(),
                },
                "Vertical distance between Point 0 and Point 12",
            ),
            (
                Constraint::Diameter {
                    entity: circle,
                    value: value.clone(),
                },
                "Diameter of Circle 7",
            ),
            (
                Constraint::Distance {
                    from: lone,
                    to: circle,
                    value: value.clone(),
                },
                "Distance between Point 12 and Circle 7",
            ),
            (
                Constraint::Distance {
                    from: line,
                    to: other,
                    value,
                },
                "Distance between Line 2 and Line 5",
            ),
        ];
        for (constraint, label) in labels {
            let id = sketch.add_constraint(constraint).unwrap();
            assert_eq!(sketch.describe_constraint(id), label);
        }
        assert_eq!(sketch.uses, sketch.counted_from_scratch());
    }

    #[test]
    fn dimensions_evaluate_their_expressions_as_lengths() {
        let (sketch, constraint) = distance(Expression::Parameter(WIDTH));
        assert!(sketch.uses_parameter(WIDTH));
        assert_eq!(
            sketch.parameters().into_iter().collect::<Vec<_>>(),
            vec![WIDTH]
        );
        assert_eq!(
            sketch.describe_constraint(constraint),
            "Distance between Point 0 and Point 1"
        );

        let solution = sketch.evaluate(&width_is(Quantity::length(40.0))).unwrap();
        assert_eq!(solution.dimension(constraint), Some(40.0));
    }

    #[test]
    fn a_failing_dimension_names_its_constraint() {
        let (sketch, constraint) = distance(Expression::Parameter(WIDTH));

        let error = sketch
            .evaluate(&width_is(Quantity::angle(10.0)))
            .unwrap_err();
        assert_eq!(
            error,
            SketchError::Dimension {
                constraint,
                reason: DimensionError::Evaluation(EvalError::WrongKind {
                    expected: Dimension::LENGTH,
                    found: Dimension::ANGLE,
                }),
            }
        );
        assert_eq!(
            error.to_string(),
            "it gives an angle, but a length is needed"
        );

        let error = sketch
            .evaluate(&width_is(Quantity::length(-1.0)))
            .unwrap_err();
        assert_eq!(error.to_string(), "a distance cannot be negative");
    }

    #[test]
    fn angles_take_any_value_and_radii_must_be_positive() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let circle = sketch.add_circle(Point2::ZERO, 1.0);
        let angle = sketch
            .add_constraint(Constraint::Angle {
                from: line,
                to: EntityId::HORIZONTAL_AXIS,
                reversed: false,
                value: Expression::Parameter(WIDTH),
            })
            .unwrap();
        sketch.remove_entity(circle);
        let evaluated = sketch.evaluate(&width_is(Quantity::angle(-30.0))).unwrap();
        assert_eq!(evaluated.dimension(angle), Some(-30.0));

        let circle = sketch.add_circle(Point2::ZERO, 1.0);
        let radius = sketch
            .add_constraint(Constraint::Radius {
                entity: circle,
                value: Expression::Number(0.0),
            })
            .unwrap();
        let error = sketch
            .evaluate(&width_is(Quantity::angle(1.0)))
            .unwrap_err();
        assert_eq!(
            error,
            SketchError::Dimension {
                constraint: radius,
                reason: DimensionError::NotPositive
            }
        );
        assert_eq!(error.to_string(), "a radius must be greater than zero");

        sketch
            .set_dimension(
                radius,
                Expression::Measure(2e6, caditor_expression::Unit::Millimetre),
            )
            .unwrap();
        let error = sketch
            .evaluate(&width_is(Quantity::angle(1.0)))
            .unwrap_err();
        assert_eq!(error.to_string(), "a length cannot be more than 1000 m");
    }

    #[test]
    fn setting_a_dimension_returns_the_previous_expression() {
        let (mut sketch, constraint) = distance(Expression::Number(5.0));
        let previous = sketch
            .set_dimension(constraint, Expression::Parameter(WIDTH))
            .unwrap();
        assert_eq!(previous, Expression::Number(5.0));
        assert!(sketch.uses_parameter(WIDTH));

        let line = sketch.add_line(Point2::ZERO, Point2::Y);
        let horizontal = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        assert_eq!(
            sketch.set_dimension(horizontal, Expression::Number(1.0)),
            Err(SketchError::NotADimension(horizontal))
        );
    }

    #[test]
    fn curves_turn_into_polylines() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let circle = sketch.add_circle(Point2::new(1.0, 1.0), 2.0);
        let arc = sketch.add_arc(Point2::ZERO, Point2::X, Point2::new(-1.0, 0.0));
        let spline = sketch.add_spline(&[Point2::ZERO, Point2::Y, Point2::X]);
        let segment_angle = std::f64::consts::PI / 8.0;

        assert_eq!(
            sketch.polyline(line, segment_angle),
            Some(vec![Point2::ZERO, Point2::X])
        );
        let circle_points = sketch.polyline(circle, segment_angle).unwrap();
        assert_eq!(circle_points.len(), 17);
        assert!(circle_points[0].distance(circle_points[16]) < 1e-12);
        let arc_points = sketch.polyline(arc, segment_angle).unwrap();
        assert_eq!(arc_points.len(), 9);
        assert!(arc_points[8].distance(Point2::new(-1.0, 0.0)) < 1e-12);
        let spline_points = sketch.polyline(spline, segment_angle).unwrap();
        assert_eq!(spline_points.first(), Some(&Point2::ZERO));
        assert!(spline_points.last().unwrap().distance(Point2::X) < 1e-12);
        assert_eq!(sketch.circle(arc), Some((Point2::ZERO, 1.0)));
        assert_eq!(
            sketch.polyline(EntityId::HORIZONTAL_AXIS, segment_angle),
            None
        );
    }
}
