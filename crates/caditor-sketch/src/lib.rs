use std::{collections::BTreeMap, fmt};

use caditor_geometry::{Plane, Point2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(u64);

impl fmt::Display for EntityId {
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
    Distance(EntityId, EntityId, f64),
}

impl Constraint {
    fn references(&self, id: EntityId) -> bool {
        match *self {
            Self::Horizontal(entity) | Self::Vertical(entity) => entity == id,
            Self::Coincident(a, b) | Self::Distance(a, b, _) => a == id || b == id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sketch {
    plane: Plane,
    entities: BTreeMap<EntityId, Entity>,
    constraints: Vec<Constraint>,
    next_id: u64,
}

impl Sketch {
    pub fn new(plane: Plane) -> Self {
        Self {
            plane,
            entities: BTreeMap::new(),
            constraints: Vec::new(),
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

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    pub fn add_point(&mut self, position: Point2) -> EntityId {
        self.insert(Entity::Point(position))
    }

    pub fn add_line(&mut self, start: Point2, end: Point2) -> EntityId {
        let start = self.add_point(start);
        let end = self.add_point(end);
        self.insert(Entity::Line { start, end })
    }

    pub fn add_constraint(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
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
            .retain(|constraint| !constraint.references(id));
        Some(removed)
    }

    fn insert(&mut self, entity: Entity) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        self.entities.insert(id, entity);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(sketch.constraints().is_empty());
    }
}
