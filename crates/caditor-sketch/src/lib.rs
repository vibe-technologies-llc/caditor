use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(u64);

impl fmt::Display for EntityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstraintId(u64);

impl fmt::Display for ConstraintId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
    Point(Point2),
    Line { start: EntityId, end: EntityId },
}

impl Entity {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Point(_) => "Point",
            Self::Line { .. } => "Line",
        }
    }

    fn references(&self, id: EntityId) -> bool {
        match *self {
            Self::Point(_) => false,
            Self::Line { start, end } => start == id || end == id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    Coincident(EntityId, EntityId),
    Horizontal(EntityId),
    Vertical(EntityId),
    Distance {
        from: EntityId,
        to: EntityId,
        value: Expression,
    },
}

impl Constraint {
    pub fn entities(&self) -> Vec<EntityId> {
        match *self {
            Self::Horizontal(entity) | Self::Vertical(entity) => vec![entity],
            Self::Coincident(a, b) | Self::Distance { from: a, to: b, .. } => vec![a, b],
        }
    }

    pub fn dimension(&self) -> Option<&Expression> {
        match self {
            Self::Distance { value, .. } => Some(value),
            Self::Coincident(..) | Self::Horizontal(_) | Self::Vertical(_) => None,
        }
    }

    pub fn dimension_kind(&self) -> Option<Dimension> {
        match self {
            Self::Distance { .. } => Some(Dimension::LENGTH),
            Self::Coincident(..) | Self::Horizontal(_) | Self::Vertical(_) => None,
        }
    }

    fn dimension_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Self::Distance { value, .. } => Some(value),
            Self::Coincident(..) | Self::Horizontal(_) | Self::Vertical(_) => None,
        }
    }

    fn references(&self, id: EntityId) -> bool {
        self.entities().contains(&id)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DimensionError {
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error("a distance cannot be negative")]
    Negative,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchError {
    #[error("the constraint no longer exists")]
    MissingConstraint(ConstraintId),
    #[error("the constraint has no value to set")]
    NotADimension(ConstraintId),
    #[error("{reason}")]
    Dimension {
        constraint: ConstraintId,
        reason: DimensionError,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SketchSolution {
    dimensions: BTreeMap<ConstraintId, f64>,
}

impl SketchSolution {
    pub fn dimension(&self, constraint: ConstraintId) -> Option<f64> {
        self.dimensions.get(&constraint).copied()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sketch {
    plane: Plane,
    entities: BTreeMap<EntityId, Entity>,
    constraints: BTreeMap<ConstraintId, Constraint>,
    next_id: u64,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Self {
            plane,
            entities: BTreeMap::new(),
            constraints: BTreeMap::new(),
            next_id: 0,
        }
    }

    pub fn plane(&self) -> Plane {
        self.plane
    }

    pub fn entities(&self) -> impl ExactSizeIterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(id, entity)| (*id, entity))
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn entity_label(&self, id: EntityId) -> String {
        match self.entities.get(&id) {
            Some(entity) => format!("{} {id}", entity.kind_name()),
            None => format!("Missing entity {id}"),
        }
    }

    pub fn point(&self, id: EntityId) -> Option<Point2> {
        match self.entities.get(&id)? {
            Entity::Point(position) => Some(*position),
            Entity::Line { .. } => None,
        }
    }

    pub fn line_endpoints(&self, id: EntityId) -> Option<(Point2, Point2)> {
        match self.entities.get(&id)? {
            Entity::Line { start, end } => Some((self.point(*start)?, self.point(*end)?)),
            Entity::Point(_) => None,
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
        let Some(constraint) = self.constraints.get(&id) else {
            return format!("Missing constraint {id}");
        };
        let label = |entity: EntityId| self.entity_label(entity);
        match *constraint {
            Constraint::Coincident(a, b) => format!("Coincident {} and {}", label(a), label(b)),
            Constraint::Horizontal(entity) => format!("Horizontal {}", label(entity)),
            Constraint::Vertical(entity) => format!("Vertical {}", label(entity)),
            Constraint::Distance { from, to, .. } => {
                format!("Distance between {} and {}", label(from), label(to))
            }
        }
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

    pub fn add_point(&mut self, position: Point2) -> EntityId {
        self.insert(Entity::Point(position))
    }

    pub fn add_line(&mut self, start: Point2, end: Point2) -> EntityId {
        let start = self.add_point(start);
        let end = self.add_point(end);
        self.insert(Entity::Line { start, end })
    }

    pub fn add_constraint(&mut self, constraint: Constraint) -> ConstraintId {
        let id = ConstraintId(self.allocate());
        self.constraints.insert(id, constraint);
        id
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

    pub fn remove_entity(&mut self, id: EntityId) -> Option<Entity> {
        let removed = self.entities.remove(&id)?;
        let dependents: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.references(id))
            .map(|(dependent, _)| *dependent)
            .collect();
        for dependent in dependents {
            self.remove_entity(dependent);
        }
        self.constraints
            .retain(|_, constraint| !constraint.references(id));
        Some(removed)
    }

    pub fn evaluate<F>(&self, value_of: &F) -> Result<SketchSolution, SketchError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let mut dimensions = BTreeMap::new();
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
            if value < 0.0 {
                return Err(failed(DimensionError::Negative));
            }
            dimensions.insert(*id, value);
        }
        Ok(SketchSolution { dimensions })
    }

    fn insert(&mut self, entity: Entity) -> EntityId {
        let id = EntityId(self.allocate());
        self.entities.insert(id, entity);
        id
    }

    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: ParameterId = ParameterId::from_raw(0);

    fn distance(value: Expression) -> (Sketch, ConstraintId) {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
            panic!("expected a line");
        };
        let constraint = sketch.add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value,
        });
        (sketch, constraint)
    }

    fn width_is(value: Quantity) -> impl Fn(ParameterId) -> Result<Quantity, EvalError> {
        move |_| Ok(value)
    }

    #[test]
    fn line_adds_its_endpoints_as_points() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        sketch.add_constraint(Constraint::Horizontal(line));

        assert_eq!(sketch.entities().len(), 3);
        let Some(Entity::Line { start, end }) = sketch.entity(line) else {
            panic!("expected a line");
        };
        assert_eq!(sketch.entity(*start), Some(&Entity::Point(Point2::ZERO)));
        assert_eq!(sketch.entity(*end), Some(&Entity::Point(Point2::X)));
        assert_eq!(sketch.line_endpoints(line), Some((Point2::ZERO, Point2::X)));
        assert_eq!(sketch.point(*end), Some(Point2::X));
        assert_eq!(sketch.point(line), None);
        assert_eq!(sketch.line_endpoints(*start), None);
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
    fn removing_a_point_removes_dependent_lines_and_constraints() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::X);
        sketch.add_constraint(Constraint::Horizontal(line));
        let Some(Entity::Line { start, end }) = sketch.entity(line).cloned() else {
            panic!("expected a line");
        };

        sketch.remove_entity(start);

        assert_eq!(sketch.entity(line), None);
        assert!(sketch.entity(end).is_some());
        assert_eq!(sketch.constraints().len(), 0);
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
    fn setting_a_dimension_returns_the_previous_expression() {
        let (mut sketch, constraint) = distance(Expression::Number(5.0));
        let previous = sketch
            .set_dimension(constraint, Expression::Parameter(WIDTH))
            .unwrap();
        assert_eq!(previous, Expression::Number(5.0));
        assert!(sketch.uses_parameter(WIDTH));

        let line = sketch.add_line(Point2::ZERO, Point2::Y);
        let horizontal = sketch.add_constraint(Constraint::Horizontal(line));
        assert_eq!(
            sketch.set_dimension(horizontal, Expression::Number(1.0)),
            Err(SketchError::NotADimension(horizontal))
        );
    }
}
