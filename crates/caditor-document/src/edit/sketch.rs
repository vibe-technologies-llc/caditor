use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::Expression;
use caditor_geometry::{Plane, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch, SketchError};

use crate::{
    attachment::SketchFeature,
    document::{Document, FeatureId, FeatureKind, list_names},
    edit::{Edit, EditError, Transaction, TransactionBuilder},
    projection::ProjectionSource,
};

impl TransactionBuilder<'_> {
    pub fn add_sketch_entity(&mut self, feature: FeatureId, entity: Entity) -> EntityId {
        self.add_sketch_entity_as(feature, entity, false)
    }

    pub fn add_sketch_entity_as(
        &mut self,
        feature: FeatureId,
        entity: Entity,
        construction: bool,
    ) -> EntityId {
        let id = EntityId::from_raw(self.allocate_sketch_id(feature));
        self.edits.push(Edit::AddSketchEntity {
            feature,
            id,
            entity,
            construction,
        });
        id
    }

    pub fn add_sketch_constraint(
        &mut self,
        feature: FeatureId,
        constraint: Constraint,
    ) -> ConstraintId {
        let id = ConstraintId::from_raw(self.allocate_sketch_id(feature));
        self.edits.push(Edit::AddSketchConstraint {
            feature,
            id,
            constraint,
            inactive: false,
            label: None,
        });
        id
    }

    pub fn set_sketch_label(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
        offset: Option<Vector2>,
    ) -> &mut Self {
        self.edits.push(Edit::SetSketchLabel {
            feature,
            id,
            offset,
        });
        self
    }

    pub fn set_sketch_constraint_active(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
        active: bool,
    ) -> &mut Self {
        self.edits.push(Edit::SetSketchConstraintActive {
            feature,
            id,
            active,
        });
        self
    }

    pub fn remove_sketch_items(
        &mut self,
        feature: FeatureId,
        entities: impl IntoIterator<Item = EntityId>,
        constraints: impl IntoIterator<Item = ConstraintId>,
    ) -> &mut Self {
        let edits = match self.sketch_feature(feature) {
            Some(definition) => {
                let mut entities: BTreeSet<EntityId> = entities.into_iter().collect();
                let projected_points: Vec<EntityId> = entities
                    .iter()
                    .filter(|id| definition.projections.contains_key(id))
                    .filter_map(|id| definition.sketch.entity(*id))
                    .flat_map(Entity::points)
                    .collect();
                entities.extend(projected_points);
                let removal = Removal::plan(&definition.sketch, entities, constraints);
                let unprojected = removal
                    .entities
                    .iter()
                    .filter(|id| definition.projections.contains_key(id))
                    .map(|id| Edit::SetSketchProjection {
                        feature,
                        id: *id,
                        source: None,
                    })
                    .collect::<Vec<_>>();
                unprojected
                    .into_iter()
                    .chain(removal.edits(feature))
                    .collect()
            }
            None => Removal {
                constraints: constraints.into_iter().collect(),
                entities: entities.into_iter().collect(),
            }
            .edits(feature),
        };
        let removed: BTreeSet<ConstraintId> = edits
            .iter()
            .filter_map(|edit| match edit {
                Edit::RemoveSketchConstraint { feature: owner, id } if *owner == feature => {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        self.edits.extend(edits);
        self.release_dimensions(feature, &removed);
        self
    }

    fn release_dimensions(&mut self, sketch: FeatureId, constraints: &BTreeSet<ConstraintId>) {
        let owned = self.document.owned_by_dimensions(sketch, constraints);
        if owned.is_empty() {
            return;
        }
        let pending = Transaction::new(self.label.clone(), std::mem::take(&mut self.edits));
        let (_, edits) = self.document.releasing(pending, owned).into_parts();
        self.edits = edits;
    }

    pub fn settle_sketch(&mut self, feature: FeatureId, solved: &Sketch) -> &mut Self {
        let edits: Vec<Edit> = self
            .sketch(feature)
            .into_iter()
            .flat_map(Sketch::entities)
            .filter_map(|(id, defined)| {
                let settled = solved.entity(id)?;
                let usable =
                    settled != defined && settled.same_structure(defined) && is_valid(settled);
                usable.then(|| Edit::SetSketchEntity {
                    feature,
                    id,
                    entity: settled.clone(),
                })
            })
            .collect();
        self.edits.extend(edits);
        self
    }

    pub fn reshape_sketch(
        &mut self,
        feature: FeatureId,
        before: &Sketch,
        after: &Sketch,
    ) -> &mut Self {
        let edits = Reshape::between(before, after).edits(feature);
        self.edits.extend(edits);
        let next = self
            .next_sketch_ids
            .get(&feature)
            .copied()
            .or_else(|| self.sketch(feature).map(Sketch::next_id))
            .unwrap_or(0)
            .max(after.next_id());
        self.next_sketch_ids.insert(feature, next);
        self
    }

    fn sketch(&self, feature: FeatureId) -> Option<&Sketch> {
        self.sketch_feature(feature)
            .map(|definition| &definition.sketch)
    }

    fn sketch_feature(&self, feature: FeatureId) -> Option<&SketchFeature> {
        fn sketch_of(kind: &FeatureKind) -> Option<&SketchFeature> {
            match kind {
                FeatureKind::Sketch(definition) => Some(definition),
                _ => None,
            }
        }
        match self.document.feature(feature) {
            Some(existing) => sketch_of(&existing.kind),
            None => self.edits.iter().find_map(|edit| match edit {
                Edit::InsertFeature { feature: added, .. } if added.id() == feature => {
                    sketch_of(&added.kind)
                }
                _ => None,
            }),
        }
    }

    fn allocate_sketch_id(&mut self, feature: FeatureId) -> u64 {
        let id = match self.next_sketch_ids.get(&feature) {
            Some(next) => *next,
            None => self.sketch(feature).map_or(0, Sketch::next_id),
        };
        self.next_sketch_ids.insert(feature, id.saturating_add(1));
        id
    }
}

struct Removal {
    constraints: Vec<ConstraintId>,
    entities: Vec<EntityId>,
}

impl Removal {
    fn plan(
        sketch: &Sketch,
        entities: impl IntoIterator<Item = EntityId>,
        constraints: impl IntoIterator<Item = ConstraintId>,
    ) -> Self {
        let mut entity_users: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for (user, entity) in sketch.entities() {
            for point in entity.points() {
                entity_users.entry(point).or_default().push(user);
            }
        }
        let mut constraint_users: BTreeMap<EntityId, Vec<ConstraintId>> = BTreeMap::new();
        for (user, constraint) in sketch.constraints() {
            for entity in constraint.entities() {
                constraint_users.entry(entity).or_default().push(user);
            }
        }
        let mut doomed_entities = BTreeSet::new();
        let mut pending: Vec<EntityId> = entities
            .into_iter()
            .filter(|id| sketch.entity(*id).is_some())
            .collect();
        while let Some(id) = pending.pop() {
            if doomed_entities.insert(id) {
                pending.extend(entity_users.get(&id).into_iter().flatten());
            }
        }
        let mut doomed_constraints: BTreeSet<ConstraintId> = constraints
            .into_iter()
            .filter(|id| sketch.constraint(*id).is_some())
            .collect();
        for entity in &doomed_entities {
            doomed_constraints.extend(constraint_users.get(entity).into_iter().flatten());
        }
        let uses_points = |id: &EntityId| {
            sketch
                .entity(*id)
                .is_some_and(|entity| !entity.points().is_empty())
        };
        let (curves, points): (Vec<EntityId>, Vec<EntityId>) =
            doomed_entities.into_iter().partition(uses_points);
        Self {
            constraints: doomed_constraints.into_iter().collect(),
            entities: curves.into_iter().chain(points).collect(),
        }
    }

    fn edits(self, feature: FeatureId) -> Vec<Edit> {
        let constraints = self
            .constraints
            .into_iter()
            .map(|id| Edit::RemoveSketchConstraint { feature, id });
        let entities = self
            .entities
            .into_iter()
            .map(|id| Edit::RemoveSketchEntity { feature, id });
        constraints.chain(entities).collect()
    }
}

struct Reshape {
    removed_constraints: Vec<ConstraintId>,
    removed_entities: Vec<EntityId>,
    added_entities: Vec<(EntityId, Entity, bool)>,
    changed_entities: Vec<(EntityId, Entity)>,
    construction: Vec<(EntityId, bool)>,
    added_constraints: Vec<(ConstraintId, Constraint, bool, Option<Vector2>)>,
    activity: Vec<(ConstraintId, bool)>,
    labels: Vec<(ConstraintId, Option<Vector2>)>,
}

impl Reshape {
    fn between(before: &Sketch, after: &Sketch) -> Self {
        let mut replaced: BTreeSet<EntityId> = before
            .entities()
            .filter(|(id, entity)| {
                after
                    .entity(*id)
                    .is_some_and(|kept| !kept.same_structure(entity))
            })
            .map(|(id, _)| id)
            .collect();
        loop {
            let users: Vec<EntityId> = before
                .entities()
                .filter(|(id, entity)| {
                    !replaced.contains(id)
                        && after.entity(*id).is_some()
                        && entity.points().iter().any(|point| replaced.contains(point))
                })
                .map(|(id, _)| id)
                .collect();
            if users.is_empty() {
                break;
            }
            replaced.extend(users);
        }
        let renewed = |constraint: &Constraint| {
            constraint
                .entities()
                .iter()
                .any(|entity| replaced.contains(entity))
        };
        let removed_constraints = before
            .constraints()
            .filter(|(id, constraint)| {
                after.constraint(*id) != Some(*constraint) || renewed(constraint)
            })
            .map(|(id, _)| id)
            .collect();
        let gone = |id: &EntityId| after.entity(*id).is_none() || replaced.contains(id);
        let (removed_curves, removed_points): (Vec<EntityId>, Vec<EntityId>) = before
            .entities()
            .map(|(id, _)| id)
            .filter(gone)
            .partition(|id| {
                before
                    .entity(*id)
                    .is_some_and(|entity| !entity.points().is_empty())
            });
        let fresh = |id: &EntityId| before.entity(*id).is_none() || replaced.contains(id);
        let (added_points, added_curves): (Vec<_>, Vec<_>) = after
            .entities()
            .filter(|(id, _)| fresh(id))
            .map(|(id, entity)| (id, entity.clone(), after.is_construction(id)))
            .partition(|(_, entity, _)| entity.points().is_empty());
        let changed_entities = after
            .entities()
            .filter(|(id, entity)| {
                !replaced.contains(id)
                    && before
                        .entity(*id)
                        .is_some_and(|previous| previous != *entity)
            })
            .map(|(id, entity)| (id, entity.clone()))
            .collect();
        let construction = after
            .entities()
            .filter(|(id, _)| {
                !replaced.contains(id)
                    && before.entity(*id).is_some()
                    && before.is_construction(*id) != after.is_construction(*id)
            })
            .map(|(id, _)| (id, after.is_construction(id)))
            .collect();
        let added_constraints = after
            .constraints()
            .filter(|(id, constraint)| {
                before.constraint(*id) != Some(*constraint) || renewed(constraint)
            })
            .map(|(id, constraint)| {
                (
                    id,
                    constraint.clone(),
                    !after.is_active(id),
                    after.label_offset(id),
                )
            })
            .collect();
        let activity = after
            .constraints()
            .filter(|(id, constraint)| {
                before.constraint(*id) == Some(*constraint)
                    && !renewed(constraint)
                    && before.is_active(*id) != after.is_active(*id)
            })
            .map(|(id, _)| (id, after.is_active(id)))
            .collect();
        let labels = after
            .constraints()
            .filter(|(id, constraint)| {
                before.constraint(*id) == Some(*constraint)
                    && !renewed(constraint)
                    && before.label_offset(*id) != after.label_offset(*id)
            })
            .map(|(id, _)| (id, after.label_offset(id)))
            .collect();
        Self {
            removed_constraints,
            removed_entities: removed_curves.into_iter().chain(removed_points).collect(),
            added_entities: added_points.into_iter().chain(added_curves).collect(),
            changed_entities,
            construction,
            added_constraints,
            activity,
            labels,
        }
    }

    fn edits(self, feature: FeatureId) -> Vec<Edit> {
        let removed_constraints = self
            .removed_constraints
            .into_iter()
            .map(|id| Edit::RemoveSketchConstraint { feature, id });
        let removed_entities = self
            .removed_entities
            .into_iter()
            .map(|id| Edit::RemoveSketchEntity { feature, id });
        let added_entities = self
            .added_entities
            .into_iter()
            .map(|(id, entity, construction)| Edit::AddSketchEntity {
                feature,
                id,
                entity,
                construction,
            });
        let changed_entities =
            self.changed_entities
                .into_iter()
                .map(|(id, entity)| Edit::SetSketchEntity {
                    feature,
                    id,
                    entity,
                });
        let construction =
            self.construction
                .into_iter()
                .map(|(id, construction)| Edit::SetSketchConstruction {
                    feature,
                    id,
                    construction,
                });
        let added_constraints =
            self.added_constraints
                .into_iter()
                .map(
                    |(id, constraint, inactive, label)| Edit::AddSketchConstraint {
                        feature,
                        id,
                        constraint,
                        inactive,
                        label,
                    },
                );
        let activity =
            self.activity
                .into_iter()
                .map(|(id, active)| Edit::SetSketchConstraintActive {
                    feature,
                    id,
                    active,
                });
        let labels = self
            .labels
            .into_iter()
            .map(|(id, offset)| Edit::SetSketchLabel {
                feature,
                id,
                offset,
            });
        removed_constraints
            .chain(removed_entities)
            .chain(added_entities)
            .chain(changed_entities)
            .chain(construction)
            .chain(added_constraints)
            .chain(activity)
            .chain(labels)
            .collect()
    }
}

impl Document {
    pub(super) fn set_dimension(
        &mut self,
        feature: FeatureId,
        constraint: ConstraintId,
        value: Expression,
    ) -> Result<Edit, EditError> {
        self.check_references(&value)?;
        let (name, sketch) = self.sketch_mut(feature)?;
        let previous = sketch
            .set_dimension(constraint, value)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::SetDimension {
            feature,
            constraint,
            value: previous,
        })
    }

    pub(super) fn add_sketch_entity(
        &mut self,
        feature: FeatureId,
        id: EntityId,
        entity: Entity,
        construction: bool,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let inserted = sketch.insert_entity(id, entity).and_then(|()| {
            construction
                .then(|| sketch.set_construction(id, true))
                .transpose()
        });
        inserted.map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::RemoveSketchEntity { feature, id })
    }

    pub(super) fn set_sketch_construction(
        &mut self,
        feature: FeatureId,
        id: EntityId,
        construction: bool,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let previous = sketch
            .set_construction(id, construction)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::SetSketchConstruction {
            feature,
            id,
            construction: previous,
        })
    }

    pub(super) fn set_sketch_projection(
        &mut self,
        feature: FeatureId,
        id: EntityId,
        source: Option<ProjectionSource>,
    ) -> Result<Edit, EditError> {
        let index = self.feature_position(feature)?;
        if let Some(source) = &source {
            let probe = FeatureKind::Sketch(SketchFeature {
                sketch: Sketch::new(Plane::XY),
                attachment: None,
                projections: BTreeMap::from([(id, source.clone())]),
            });
            self.check_feature_references(&probe, index)?;
            self.reserve_past_references(&probe);
        }
        let existing = self.feature_mut(feature)?;
        let name = existing.name.clone();
        let FeatureKind::Sketch(definition) = &mut existing.kind else {
            return Err(EditError::NotASketch(name));
        };
        let points = definition
            .sketch
            .entity(id)
            .ok_or_else(|| EditError::Sketch {
                name: name.clone(),
                error: SketchError::NoSuchEntity(id),
            })?
            .points();
        for marked in points.into_iter().chain([id]) {
            definition
                .sketch
                .set_projected(marked, source.is_some())
                .map_err(|error| EditError::Sketch {
                    name: name.clone(),
                    error,
                })?;
        }
        let previous = match source {
            Some(source) => definition.projections.insert(id, source),
            None => definition.projections.remove(&id),
        };
        Ok(Edit::SetSketchProjection {
            feature,
            id,
            source: previous,
        })
    }

    pub(super) fn remove_sketch_entity(
        &mut self,
        feature: FeatureId,
        id: EntityId,
    ) -> Result<Edit, EditError> {
        let axis_users: Vec<String> =
            self.features()
                .filter(|other| {
                    other.kind.solid().is_some_and(|solid| {
                        solid.sketch() == feature && solid.axis_line() == Some(id)
                    })
                })
                .map(|other| other.name.clone())
                .collect();
        if !axis_users.is_empty() {
            let (name, sketch) = self.sketch_mut(feature)?;
            return Err(EditError::EntityInUse {
                feature: name,
                name: sketch.entity_label(id),
                users: list_names(&axis_users),
            });
        }
        let (name, sketch) = self.sketch_mut(feature)?;
        if sketch.is_projected(id) {
            return Err(EditError::StillProjected {
                feature: name,
                name: sketch.entity_label(id),
            });
        }
        let construction = sketch.is_construction(id);
        match sketch.remove_unused_entity(id) {
            Ok(entity) => Ok(Edit::AddSketchEntity {
                feature,
                id,
                entity,
                construction,
            }),
            Err(SketchError::InUse { entity, label }) => Err(EditError::EntityInUse {
                feature: name,
                name: label,
                users: list_names(&users_of(sketch, entity)),
            }),
            Err(error) => Err(EditError::Sketch { name, error }),
        }
    }

    pub(super) fn set_sketch_entity(
        &mut self,
        feature: FeatureId,
        id: EntityId,
        entity: Entity,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let previous = sketch
            .replace_entity(id, entity)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::SetSketchEntity {
            feature,
            id,
            entity: previous,
        })
    }

    pub(super) fn add_sketch_constraint(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
        constraint: Constraint,
        inactive: bool,
        label: Option<Vector2>,
    ) -> Result<Edit, EditError> {
        if let Some(value) = constraint.dimension() {
            self.check_references(value)?;
        }
        let (name, sketch) = self.sketch_mut(feature)?;
        sketch
            .insert_constraint(id, constraint)
            .and_then(|()| {
                inactive
                    .then(|| sketch.set_active(id, false).map(|_| ()))
                    .transpose()
                    .map(|_| ())
            })
            .and_then(|()| {
                label
                    .map(|offset| sketch.set_label_offset(id, Some(offset)).map(|_| ()))
                    .transpose()
                    .map(|_| ())
            })
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::RemoveSketchConstraint { feature, id })
    }

    pub(super) fn set_sketch_constraint_active(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
        active: bool,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let previous = sketch
            .set_active(id, active)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::SetSketchConstraintActive {
            feature,
            id,
            active: previous,
        })
    }

    pub(super) fn set_sketch_label(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
        offset: Option<Vector2>,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let previous = sketch
            .set_label_offset(id, offset)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::SetSketchLabel {
            feature,
            id,
            offset: previous,
        })
    }

    pub(super) fn remove_sketch_constraint(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let inactive = !sketch.is_active(id);
        let label = sketch.label_offset(id);
        let constraint = sketch
            .remove_constraint(id)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::AddSketchConstraint {
            feature,
            id,
            constraint,
            inactive,
            label,
        })
    }

    fn sketch_mut(&mut self, id: FeatureId) -> Result<(String, &mut Sketch), EditError> {
        let feature = self.feature_mut(id)?;
        let name = feature.name.clone();
        match feature.kind.sketch_mut() {
            Some(sketch) => Ok((name, sketch)),
            None => Err(EditError::NotASketch(name)),
        }
    }
}

fn is_valid(entity: &Entity) -> bool {
    match entity {
        Entity::Point(position) => position.is_finite(),
        Entity::Circle { radius, .. } => radius.is_finite() && *radius > 0.0,
        Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => true,
    }
}

fn users_of(sketch: &Sketch, entity: EntityId) -> Vec<String> {
    let entities = sketch
        .entities_using(entity)
        .into_iter()
        .map(|user| sketch.entity_label(user));
    let constraints = sketch
        .constraints_using(entity)
        .into_iter()
        .map(|user| sketch.describe_constraint(user));
    entities.chain(constraints).collect()
}
