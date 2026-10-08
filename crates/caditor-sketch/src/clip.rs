use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb2, Point2, Vector2};

use crate::{
    constraint::Constraint,
    entity::Entity,
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
};

#[derive(Debug, Clone, PartialEq)]
pub struct SketchClip {
    entities: BTreeMap<EntityId, Entity>,
    construction: BTreeSet<EntityId>,
    constraints: Vec<(Constraint, bool)>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClipError {
    #[error("no sketch geometry is selected")]
    NothingSelected,
    #[error(transparent)]
    Edit(#[from] SketchError),
}

impl SketchClip {
    pub fn curve_count(&self) -> usize {
        self.entities
            .values()
            .filter(|entity| !matches!(entity, Entity::Point(_)))
            .count()
    }

    pub fn lone_point_count(&self) -> usize {
        self.lone_points().count()
    }

    pub fn constraint_count(&self) -> usize {
        self.constraints.len()
    }

    pub fn centre(&self) -> Option<Point2> {
        let positions = self.entities.values().filter_map(|entity| match entity {
            Entity::Point(position) => Some(*position),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. } => None,
        });
        Aabb2::from_points(positions).map(|bounds| bounds.center())
    }

    pub fn size(&self) -> f64 {
        let positions = self.entities.values().filter_map(|entity| match entity {
            Entity::Point(position) => Some(*position),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. } => None,
        });
        Aabb2::from_points(positions).map_or(0.0, |bounds| {
            let extent = bounds.max() - bounds.min();
            extent.x.max(extent.y)
        })
    }

    fn lone_points(&self) -> impl Iterator<Item = EntityId> + '_ {
        let used: BTreeSet<EntityId> = self.entities.values().flat_map(Entity::points).collect();
        self.entities
            .iter()
            .filter(move |(id, entity)| matches!(entity, Entity::Point(_)) && !used.contains(id))
            .map(|(id, _)| *id)
    }
}

impl Sketch {
    pub fn clip(&self, items: &[EntityId]) -> Result<SketchClip, ClipError> {
        let mut entities = BTreeMap::new();
        for item in items.iter().filter(|item| !item.is_reference()) {
            let Some(entity) = self.entity(*item) else {
                continue;
            };
            for point in entity.points() {
                if let Some(defined) = self.entity(point) {
                    entities.insert(point, defined.clone());
                }
            }
            entities.insert(*item, entity.clone());
        }
        if entities.is_empty() {
            return Err(ClipError::NothingSelected);
        }
        let construction = entities
            .keys()
            .copied()
            .filter(|id| self.is_construction(*id))
            .collect();
        let constraints = self
            .constraints()
            .filter(|(_, constraint)| {
                constraint
                    .entities()
                    .iter()
                    .all(|entity| entities.contains_key(entity))
            })
            .map(|(id, constraint)| (constraint.clone(), self.is_active(id)))
            .collect();
        Ok(SketchClip {
            entities,
            construction,
            constraints,
        })
    }

    pub fn paste(
        &mut self,
        clip: &SketchClip,
        offset: Vector2,
    ) -> Result<Vec<EntityId>, ClipError> {
        let mut working = self.clone();
        let mut renamed = BTreeMap::new();
        for (old, entity) in &clip.entities {
            if let Entity::Point(position) = entity {
                renamed.insert(*old, working.add_point(*position + offset));
            }
        }
        let rename = |renamed: &BTreeMap<EntityId, EntityId>, id: EntityId| {
            renamed.get(&id).copied().unwrap_or(id)
        };
        let mut curves = Vec::new();
        for (old, entity) in &clip.entities {
            let moved = match entity {
                Entity::Point(_) => continue,
                Entity::Line { start, end } => Entity::Line {
                    start: rename(&renamed, *start),
                    end: rename(&renamed, *end),
                },
                Entity::Circle { center, radius } => Entity::Circle {
                    center: rename(&renamed, *center),
                    radius: *radius,
                },
                Entity::Arc { center, start, end } => Entity::Arc {
                    center: rename(&renamed, *center),
                    start: rename(&renamed, *start),
                    end: rename(&renamed, *end),
                },
                Entity::Spline { control_points } => Entity::Spline {
                    control_points: control_points
                        .iter()
                        .map(|point| rename(&renamed, *point))
                        .collect(),
                },
            };
            let id = EntityId::from_raw(working.next_id());
            working.insert_entity(id, moved)?;
            if clip.construction.contains(old) {
                working.set_construction(id, true)?;
            }
            renamed.insert(*old, id);
            curves.push(id);
        }
        for (constraint, active) in &clip.constraints {
            let mut moved = constraint.with_entities_mapped(|entity| rename(&renamed, entity));
            if let Constraint::Fix { at, .. } = &mut moved {
                *at += offset;
            }
            let id = ConstraintId::from_raw(working.next_id());
            working.insert_constraint(id, moved)?;
            if !active {
                working.set_active(id, false)?;
            }
        }
        *self = working;
        let lone = clip.lone_points().map(|point| rename(&renamed, point));
        Ok(curves.into_iter().chain(lone).collect())
    }
}

#[cfg(test)]
mod tests {
    use caditor_expression::{Expression, Unit};
    use caditor_geometry::Plane;

    use super::*;

    fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
        match sketch.entity(line) {
            Some(Entity::Line { start, end }) => (*start, *end),
            other => panic!("expected a line, found {other:?}"),
        }
    }

    #[test]
    fn a_pasted_copy_keeps_its_shape_and_the_constraints_among_its_geometry() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
        let (start, end) = ends(&sketch, line);
        sketch.set_construction(circle, true).unwrap();
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        let length = sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value: Expression::Measure(10.0, Unit::Millimetre),
            })
            .unwrap();
        sketch.set_active(length, false).unwrap();
        sketch
            .add_constraint(Constraint::Fix {
                point: start,
                at: Point2::ZERO,
            })
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
            .unwrap();
        sketch
            .add_constraint(Constraint::Tangent(line, circle))
            .unwrap();

        let clip = sketch.clip(&[line, EntityId::HORIZONTAL_AXIS]).unwrap();
        assert_eq!(clip.curve_count(), 1);
        assert_eq!(clip.constraint_count(), 3);
        assert_eq!(clip.centre(), Some(Point2::new(5.0, 0.0)));
        assert_eq!(clip.size(), 10.0);

        let before = sketch.constraints().len();
        let pasted = sketch.paste(&clip, Vector2::new(0.0, 20.0)).unwrap();

        let [copy] = pasted[..] else {
            panic!("one line should be pasted, got {pasted:?}");
        };
        assert_ne!(copy, line);
        assert_eq!(
            sketch.line_endpoints(copy),
            Some((Point2::new(0.0, 20.0), Point2::new(10.0, 20.0)))
        );
        assert_eq!(sketch.constraints().len(), before + 3);
        let (copy_start, _) = ends(&sketch, copy);
        assert!(sketch.constraints().any(|(id, constraint)| {
            matches!(constraint, Constraint::Distance { from, .. } if *from == copy_start)
                && !sketch.is_active(id)
        }));
        assert!(sketch.constraints().any(|(_, constraint)| {
            *constraint
                == Constraint::Fix {
                    point: copy_start,
                    at: Point2::new(0.0, 20.0),
                }
        }));

        let with_circle = sketch.clip(&[circle]).unwrap();
        let pasted = sketch.paste(&with_circle, Vector2::X).unwrap();
        let [round] = pasted[..] else {
            panic!("one circle should be pasted");
        };
        assert!(sketch.is_construction(round));
        assert_eq!(sketch.circle(round), Some((Point2::new(6.0, 5.0), 2.0)));
    }

    #[test]
    fn lone_points_paste_too_and_reference_geometry_is_never_copied() {
        let mut sketch = Sketch::new(Plane::XY);
        let point = sketch.add_point(Point2::new(3.0, 4.0));

        assert_eq!(
            sketch.clip(&[EntityId::ORIGIN, EntityId::VERTICAL_AXIS]),
            Err(ClipError::NothingSelected)
        );
        let clip = sketch.clip(&[point]).unwrap();
        assert_eq!(clip.lone_point_count(), 1);
        let pasted = sketch.paste(&clip, Vector2::new(1.0, 1.0)).unwrap();
        let [copy] = pasted[..] else {
            panic!("one point should be pasted");
        };
        assert_eq!(sketch.point(copy), Some(Point2::new(4.0, 5.0)));
    }
}
