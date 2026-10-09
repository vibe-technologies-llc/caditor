use std::collections::{BTreeMap, BTreeSet};

use crate::{constraint::Constraint, entity::Entity, id::EntityId, sketch::Sketch};

impl Sketch {
    pub fn open_ends(&self) -> Vec<EntityId> {
        let mut classes = Classes::default();
        let mut on_profile_curve = BTreeSet::new();
        for (_, constraint) in self.active_constraints() {
            match *constraint {
                Constraint::Coincident(a, b) => match (self.entity(a), self.entity(b)) {
                    (Some(Entity::Point(_)), Some(Entity::Point(_))) => classes.join(a, b),
                    (Some(Entity::Point(_)), Some(_)) if self.is_profile_curve(b) => {
                        on_profile_curve.insert(a);
                    }
                    (Some(_), Some(Entity::Point(_))) if self.is_profile_curve(a) => {
                        on_profile_curve.insert(b);
                    }
                    _ => {}
                },
                Constraint::Midpoint { point, curve } if self.is_profile_curve(curve) => {
                    on_profile_curve.insert(point);
                }
                _ => {}
            }
        }
        let ends: Vec<EntityId> = self
            .entities()
            .filter(|(id, _)| self.is_profile_curve(*id))
            .flat_map(|(_, entity)| curve_ends(entity))
            .collect();
        let mut incidences: BTreeMap<EntityId, usize> = BTreeMap::new();
        let mut supported = BTreeSet::new();
        for end in &ends {
            *incidences.entry(classes.root(*end)).or_default() += 1;
        }
        for point in &on_profile_curve {
            supported.insert(classes.root(*point));
        }
        let mut open: Vec<EntityId> = ends
            .into_iter()
            .filter(|end| {
                let root = classes.root(*end);
                incidences.get(&root) == Some(&1) && !supported.contains(&root)
            })
            .collect();
        open.sort_unstable();
        open.dedup();
        open
    }

    fn is_profile_curve(&self, id: EntityId) -> bool {
        !id.is_reference()
            && !self.is_construction(id)
            && matches!(
                self.entity(id),
                Some(
                    Entity::Line { .. }
                        | Entity::Circle { .. }
                        | Entity::Arc { .. }
                        | Entity::Spline { .. }
                        | Entity::Ellipse { .. }
                        | Entity::EllipticalArc { .. }
                )
            )
    }
}

fn curve_ends(entity: &Entity) -> Vec<EntityId> {
    match entity {
        Entity::Line { start, end }
        | Entity::Arc { start, end, .. }
        | Entity::EllipticalArc { start, end, .. } => vec![*start, *end],
        spline @ Entity::Spline { .. } => spline
            .spline_ends()
            .map_or_else(Vec::new, |(first, last)| vec![first, last]),
        Entity::Point(_) | Entity::Circle { .. } | Entity::Ellipse { .. } => Vec::new(),
    }
}

#[derive(Default)]
struct Classes {
    parents: BTreeMap<EntityId, EntityId>,
}

impl Classes {
    fn root(&self, point: EntityId) -> EntityId {
        let mut current = point;
        for _ in 0..=self.parents.len() {
            match self.parents.get(&current) {
                Some(parent) if *parent != current => current = *parent,
                _ => break,
            }
        }
        current
    }

    fn join(&mut self, a: EntityId, b: EntityId) {
        let (a, b) = (self.root(a), self.root(b));
        if a != b {
            self.parents.insert(a.max(b), a.min(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2};

    use super::*;

    fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
        match sketch.entity(line) {
            Some(Entity::Line { start, end }) => (*start, *end),
            other => panic!("expected a line, found {other:?}"),
        }
    }

    #[test]
    fn ends_joined_to_nothing_are_open_and_a_closed_outline_has_none() {
        let mut sketch = Sketch::new(Plane::XY);
        let corners = [
            Point2::ZERO,
            Point2::new(10.0, 0.0),
            Point2::new(10.0, 5.0),
            Point2::new(0.0, 5.0),
        ];
        let sides: Vec<EntityId> = (0..4)
            .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
            .collect();
        for index in 0..4 {
            let (_, end) = ends(&sketch, sides[index]);
            let (start, _) = ends(&sketch, sides[(index + 1) % 4]);
            sketch
                .add_constraint(Constraint::Coincident(end, start))
                .unwrap();
        }
        sketch.add_circle(Point2::new(20.0, 0.0), 2.0);
        assert_eq!(sketch.open_ends(), Vec::<EntityId>::new());

        let stray = sketch.add_line(Point2::new(30.0, 0.0), Point2::new(40.0, 0.0));
        let (stray_start, stray_end) = ends(&sketch, stray);
        assert_eq!(sketch.open_ends(), vec![stray_start, stray_end]);

        sketch
            .add_constraint(Constraint::Coincident(stray_start, sides[0]))
            .unwrap();
        assert_eq!(sketch.open_ends(), vec![stray_end]);

        let guide = sketch.add_line(Point2::new(40.0, -5.0), Point2::new(40.0, 5.0));
        sketch.set_construction(guide, true).unwrap();
        sketch
            .add_constraint(Constraint::Coincident(stray_end, guide))
            .unwrap();
        assert_eq!(sketch.open_ends(), vec![stray_end]);

        sketch.set_construction(stray, true).unwrap();
        assert_eq!(sketch.open_ends(), Vec::<EntityId>::new());
    }
}
