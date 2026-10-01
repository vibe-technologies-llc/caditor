use std::mem;

use crate::{
    constraint::Constraint,
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::Sketch,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Level,
    Plumb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Subject {
    Direction(Direction, [EntityId; 2]),
    Size(EntityId),
    Pair(mem::Discriminant<Constraint>, [EntityId; 2]),
    Single(mem::Discriminant<Constraint>, EntityId),
    Triple(mem::Discriminant<Constraint>, EntityId, [EntityId; 2]),
}

impl Sketch {
    pub fn restating(&self, constraint: &Constraint) -> Option<ConstraintId> {
        let subject = self.subject(constraint);
        self.constraints()
            .find(|(_, existing)| self.subject(existing) == subject)
            .map(|(id, _)| id)
    }

    pub fn contradicting(&self, constraint: &Constraint) -> Option<ConstraintId> {
        self.constraints()
            .find(|(_, existing)| contradict(constraint, existing))
            .map(|(id, _)| id)
    }

    fn subject(&self, constraint: &Constraint) -> Subject {
        let kind = mem::discriminant(constraint);
        match *constraint {
            Constraint::Horizontal(line) => Subject::Direction(Direction::Level, self.ends(line)),
            Constraint::Vertical(line) => Subject::Direction(Direction::Plumb, self.ends(line)),
            Constraint::HorizontalPoints(a, b) => {
                Subject::Direction(Direction::Level, ordered(a, b))
            }
            Constraint::VerticalPoints(a, b) => Subject::Direction(Direction::Plumb, ordered(a, b)),
            Constraint::Coincident(a, b)
            | Constraint::Parallel(a, b)
            | Constraint::Perpendicular(a, b)
            | Constraint::Tangent(a, b)
            | Constraint::Equal(a, b)
            | Constraint::Concentric(a, b)
            | Constraint::Collinear(a, b)
            | Constraint::Distance { from: a, to: b, .. }
            | Constraint::HorizontalDistance { from: a, to: b, .. }
            | Constraint::VerticalDistance { from: a, to: b, .. }
            | Constraint::Angle { from: a, to: b, .. } => Subject::Pair(kind, ordered(a, b)),
            Constraint::Midpoint { point, line } => Subject::Pair(kind, [point, line]),
            Constraint::Symmetric {
                first,
                second,
                about,
            } => Subject::Triple(kind, about, ordered(first, second)),
            Constraint::Fix { point, .. } => Subject::Single(kind, point),
            Constraint::Radius { entity, .. } | Constraint::Diameter { entity, .. } => {
                Subject::Size(entity)
            }
        }
    }

    fn ends(&self, line: EntityId) -> [EntityId; 2] {
        match self.entity(line) {
            Some(&Entity::Line { start, end }) => ordered(start, end),
            _ => [line, line],
        }
    }
}

fn ordered(a: EntityId, b: EntityId) -> [EntityId; 2] {
    if a <= b { [a, b] } else { [b, a] }
}

fn contradict(first: &Constraint, second: &Constraint) -> bool {
    match (first, second) {
        (Constraint::Horizontal(a), Constraint::Vertical(b))
        | (Constraint::Vertical(a), Constraint::Horizontal(b)) => a == b,
        (Constraint::Parallel(a, b), Constraint::Perpendicular(c, d))
        | (Constraint::Perpendicular(a, b), Constraint::Parallel(c, d)) => {
            ordered(*a, *b) == ordered(*c, *d)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::{Expression, Unit};
    use caditor_geometry::{Plane, Point2};

    use super::*;

    fn mm(value: f64) -> Expression {
        Expression::Measure(value, Unit::Millimetre)
    }

    fn line_with_ends(sketch: &mut Sketch) -> (EntityId, EntityId, EntityId) {
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 1.0));
        let Some(&Entity::Line { start, end }) = sketch.entity(line) else {
            panic!("expected a line");
        };
        (line, start, end)
    }

    #[test]
    fn the_same_relation_on_the_same_items_is_found_in_either_order() {
        let mut sketch = Sketch::new(Plane::XY);
        let (line, start, end) = line_with_ends(&mut sketch);
        let other = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 7.0));
        let level = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        let parallel = sketch
            .add_constraint(Constraint::Parallel(line, other))
            .unwrap();
        let across = sketch
            .add_constraint(Constraint::HorizontalDistance {
                from: start,
                to: end,
                value: mm(10.0),
            })
            .unwrap();

        assert_eq!(sketch.restating(&Constraint::Horizontal(line)), Some(level));
        assert_eq!(
            sketch.restating(&Constraint::HorizontalPoints(end, start)),
            Some(level)
        );
        assert_eq!(
            sketch.restating(&Constraint::Parallel(other, line)),
            Some(parallel)
        );
        assert_eq!(
            sketch.restating(&Constraint::HorizontalDistance {
                from: end,
                to: start,
                value: mm(4.0),
            }),
            Some(across)
        );

        assert_eq!(sketch.restating(&Constraint::Vertical(line)), None);
        assert_eq!(sketch.restating(&Constraint::Horizontal(other)), None);
        assert_eq!(
            sketch.restating(&Constraint::VerticalDistance {
                from: start,
                to: end,
                value: mm(1.0),
            }),
            None
        );
        assert_eq!(sketch.restating(&Constraint::Equal(line, other)), None);
    }

    #[test]
    fn a_radius_and_a_diameter_of_one_circle_restate_each_other() {
        let mut sketch = Sketch::new(Plane::XY);
        let circle = sketch.add_circle(Point2::ZERO, 2.0);
        let other = sketch.add_circle(Point2::X, 2.0);
        let radius = sketch
            .add_constraint(Constraint::Radius {
                entity: circle,
                value: mm(2.0),
            })
            .unwrap();

        assert_eq!(
            sketch.restating(&Constraint::Diameter {
                entity: circle,
                value: mm(4.0),
            }),
            Some(radius)
        );
        assert_eq!(
            sketch.restating(&Constraint::Diameter {
                entity: other,
                value: mm(4.0),
            }),
            None
        );
    }

    #[test]
    fn a_line_cannot_be_level_and_upright_nor_parallel_and_perpendicular() {
        let mut sketch = Sketch::new(Plane::XY);
        let (line, ..) = line_with_ends(&mut sketch);
        let other = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 7.0));
        let level = sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        let parallel = sketch
            .add_constraint(Constraint::Parallel(line, other))
            .unwrap();

        assert_eq!(
            sketch.contradicting(&Constraint::Vertical(line)),
            Some(level)
        );
        assert_eq!(
            sketch.contradicting(&Constraint::Perpendicular(other, line)),
            Some(parallel)
        );
        assert_eq!(sketch.contradicting(&Constraint::Vertical(other)), None);
        assert_eq!(sketch.contradicting(&Constraint::Horizontal(line)), None);
    }
}
