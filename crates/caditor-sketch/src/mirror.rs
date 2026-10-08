use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::Faceting,
    entity::Entity,
    id::{EntityId, Reference},
    sketch::{Sketch, SketchError},
};

const TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MirrorError {
    #[error("select the geometry to mirror")]
    NothingSelected,
    #[error("{label} is not a line or an axis, so nothing can be mirrored about it")]
    NotALine { entity: EntityId, label: String },
    #[error("{label} has no length to mirror about")]
    NoLength { entity: EntityId, label: String },
    #[error("everything selected lies on {label} or is its own mirror image about it")]
    NothingToMirror { entity: EntityId, label: String },
    #[error(transparent)]
    Edit(SketchError),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MirrorImage {
    pub curves: Vec<Vec<Point2>>,
    pub points: Vec<Point2>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Reflection {
    anchor: Point2,
    direction: Vector2,
}

impl Reflection {
    fn of(&self, point: Point2) -> Point2 {
        let offset = point - self.anchor;
        let along = self.direction * offset.dot(self.direction);
        self.anchor + along * 2.0 - offset
    }

    fn distance(&self, point: Point2) -> f64 {
        self.direction.perp_dot(point - self.anchor).abs()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Plan {
    reflection: Reflection,
    curves: Vec<EntityId>,
    points: BTreeSet<EntityId>,
    on_mirror: BTreeSet<EntityId>,
}

impl Sketch {
    pub fn mirror_image(
        &self,
        items: &[EntityId],
        about: EntityId,
        faceting: Faceting,
    ) -> Result<MirrorImage, MirrorError> {
        let plan = self.mirror_plan(items, about)?;
        let reflect = |point: Point2| plan.reflection.of(point);
        let curves = plan
            .curves
            .iter()
            .filter_map(|curve| self.faceted(*curve, faceting))
            .map(|points| points.into_iter().map(reflect).collect())
            .collect();
        let used: BTreeSet<EntityId> = plan
            .curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .flat_map(Entity::points)
            .collect();
        let points = plan
            .points
            .iter()
            .filter(|point| !used.contains(point) && !plan.on_mirror.contains(point))
            .filter_map(|point| self.point(*point))
            .map(reflect)
            .collect();
        Ok(MirrorImage { curves, points })
    }

    pub fn mirror(
        &mut self,
        items: &[EntityId],
        about: EntityId,
    ) -> Result<Vec<EntityId>, MirrorError> {
        let plan = self.mirror_plan(items, about)?;
        let mut working = self.clone();
        let copies = working
            .add_mirror_image(&plan, about)
            .map_err(MirrorError::Edit)?;
        *self = working;
        Ok(copies)
    }

    fn mirror_reflection(&self, about: EntityId) -> Result<Reflection, MirrorError> {
        let label = || self.entity_label(about);
        match about.reference() {
            Some(Reference::HorizontalAxis) => Ok(Reflection {
                anchor: Point2::ZERO,
                direction: Vector2::X,
            }),
            Some(Reference::VerticalAxis) => Ok(Reflection {
                anchor: Point2::ZERO,
                direction: Vector2::Y,
            }),
            Some(Reference::Origin) => Err(MirrorError::NotALine {
                entity: about,
                label: label(),
            }),
            None => {
                let (start, end) =
                    self.line_endpoints(about)
                        .ok_or_else(|| MirrorError::NotALine {
                            entity: about,
                            label: label(),
                        })?;
                let direction =
                    (end - start)
                        .try_normalize()
                        .ok_or_else(|| MirrorError::NoLength {
                            entity: about,
                            label: label(),
                        })?;
                Ok(Reflection {
                    anchor: start,
                    direction,
                })
            }
        }
    }

    fn mirror_plan(&self, items: &[EntityId], about: EntityId) -> Result<Plan, MirrorError> {
        let reflection = self.mirror_reflection(about)?;
        let chosen: BTreeSet<EntityId> = items
            .iter()
            .copied()
            .filter(|item| *item != about && !item.is_reference() && self.entity(*item).is_some())
            .collect();
        if chosen.is_empty() {
            return Err(MirrorError::NothingSelected);
        }
        let scale = chosen
            .iter()
            .filter_map(|item| self.entity(*item))
            .flat_map(Entity::points)
            .filter_map(|point| self.point(point))
            .map(|position| position.abs().max_element())
            .fold(1.0, f64::max);
        let tolerance = TOLERANCE * scale;
        let on_line = |point: EntityId| {
            self.point(point)
                .is_some_and(|position| reflection.distance(position) <= tolerance)
        };
        let curves: Vec<EntityId> = chosen
            .iter()
            .copied()
            .filter(|item| !matches!(self.entity(*item), Some(Entity::Point(_))))
            .filter(|curve| !self.is_own_image(*curve, &reflection, tolerance))
            .collect();
        let lone = chosen
            .iter()
            .copied()
            .filter(|item| matches!(self.entity(*item), Some(Entity::Point(_))))
            .filter(|point| !on_line(*point));
        let points: BTreeSet<EntityId> = curves
            .iter()
            .filter_map(|curve| self.entity(*curve))
            .flat_map(Entity::points)
            .chain(lone)
            .collect();
        if points.is_empty() {
            return Err(MirrorError::NothingToMirror {
                entity: about,
                label: self.entity_label(about),
            });
        }
        let on_mirror = points
            .iter()
            .copied()
            .filter(|point| on_line(*point))
            .collect();
        Ok(Plan {
            reflection,
            curves,
            points,
            on_mirror,
        })
    }

    fn is_own_image(&self, curve: EntityId, reflection: &Reflection, tolerance: f64) -> bool {
        let at = |point: EntityId| self.point(point);
        let same = |a: Option<Point2>, b: Option<Point2>| match (a, b) {
            (Some(a), Some(b)) => reflection.of(a).distance(b) <= tolerance,
            _ => false,
        };
        match self.entity(curve) {
            Some(Entity::Line { start, end }) => {
                (same(at(*start), at(*start)) && same(at(*end), at(*end)))
                    || same(at(*start), at(*end))
            }
            Some(Entity::Arc { center, start, end }) => {
                same(at(*center), at(*center)) && same(at(*start), at(*end))
            }
            Some(Entity::Circle { center, .. }) => same(at(*center), at(*center)),
            Some(Entity::Spline { control_points }) => {
                let forward = control_points.iter().map(|point| at(*point));
                let backward = control_points.iter().rev().map(|point| at(*point));
                forward.zip(backward).all(|(a, b)| same(a, b))
            }
            Some(Entity::Point(_)) | None => false,
        }
    }

    fn add_mirror_image(
        &mut self,
        plan: &Plan,
        about: EntityId,
    ) -> Result<Vec<EntityId>, SketchError> {
        let mut images: BTreeMap<EntityId, EntityId> = BTreeMap::new();
        let mut symmetric = Vec::new();
        let mut held = Vec::new();
        for point in &plan.points {
            if plan.on_mirror.contains(point) {
                images.insert(*point, *point);
                if !self.held_on(*point, about) {
                    held.push(*point);
                }
                continue;
            }
            let position = self.point(*point).ok_or(SketchError::NotAPoint(*point))?;
            let image = self.add_point(plan.reflection.of(position));
            images.insert(*point, image);
            symmetric.push((*point, image));
        }
        let image_of = |point: EntityId| images.get(&point).copied().unwrap_or(point);
        let mut copies = Vec::with_capacity(plan.curves.len());
        let mut equal = Vec::new();
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
                    start: image_of(end),
                    end: image_of(start),
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
            let circle = matches!(image, Entity::Circle { .. });
            let copy = EntityId::from_raw(self.next_id());
            self.insert_entity(copy, image)?;
            if self.is_construction(*curve) {
                self.set_construction(copy, true)?;
            }
            if circle {
                equal.push((*curve, copy));
            }
            copies.push(copy);
        }
        for (first, second) in symmetric {
            self.add_constraint(Constraint::Symmetric {
                first,
                second,
                about,
            })?;
        }
        for (original, copy) in equal {
            self.add_constraint(Constraint::Equal(original, copy))?;
        }
        for point in held {
            self.add_constraint(Constraint::Coincident(point, about))?;
        }
        Ok(copies)
    }

    pub(crate) fn held_on(&self, point: EntityId, about: EntityId) -> bool {
        let ends = self.entity(about).map(Entity::points).unwrap_or_default();
        if ends.contains(&point) {
            return true;
        }
        let axis = about.is_reference();
        self.constraints_using(point)
            .into_iter()
            .filter_map(|id| self.constraint(id))
            .any(|constraint| match *constraint {
                Constraint::Coincident(a, b) => {
                    let other = if a == point { b } else { a };
                    other == about || ends.contains(&other) || (axis && other == EntityId::ORIGIN)
                }
                Constraint::Midpoint { curve, .. } => curve == about,
                _ => false,
            })
    }
}

#[cfg(test)]
mod tests;
