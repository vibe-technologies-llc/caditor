use std::collections::BTreeSet;

use caditor_expression::Expression;
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch, SketchError};

use crate::{
    document::{Document, FeatureId, list_names},
    edit::{Edit, EditError, TransactionBuilder},
};

impl TransactionBuilder<'_> {
    pub fn add_sketch_entity(&mut self, feature: FeatureId, entity: Entity) -> EntityId {
        let id = EntityId::from_raw(self.allocate_sketch_id(feature));
        self.edits.push(Edit::AddSketchEntity {
            feature,
            id,
            entity,
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
        });
        id
    }

    pub fn remove_sketch_items(
        &mut self,
        feature: FeatureId,
        entities: impl IntoIterator<Item = EntityId>,
        constraints: impl IntoIterator<Item = ConstraintId>,
    ) -> &mut Self {
        let edits = match self.sketch(feature) {
            Some(sketch) => Removal::plan(sketch, entities, constraints).edits(feature),
            None => Removal {
                constraints: constraints.into_iter().collect(),
                entities: entities.into_iter().collect(),
            }
            .edits(feature),
        };
        self.edits.extend(edits);
        self
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

    fn sketch(&self, feature: FeatureId) -> Option<&Sketch> {
        match self.document.feature(feature) {
            Some(existing) => existing.kind.sketch(),
            None => self.edits.iter().find_map(|edit| match edit {
                Edit::InsertFeature { feature: added, .. } if added.id() == feature => {
                    added.kind.sketch()
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
        let mut doomed_entities = BTreeSet::new();
        let mut pending: Vec<EntityId> = entities
            .into_iter()
            .filter(|id| sketch.entity(*id).is_some())
            .collect();
        while let Some(id) = pending.pop() {
            if doomed_entities.insert(id) {
                pending.extend(sketch.entities_using(id));
            }
        }
        let mut doomed_constraints: BTreeSet<ConstraintId> = constraints
            .into_iter()
            .filter(|id| sketch.constraint(*id).is_some())
            .collect();
        for entity in &doomed_entities {
            doomed_constraints.extend(sketch.constraints_using(*entity));
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
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        sketch
            .insert_entity(id, entity)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::RemoveSketchEntity { feature, id })
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
        match sketch.remove_unused_entity(id) {
            Ok(entity) => Ok(Edit::AddSketchEntity {
                feature,
                id,
                entity,
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
    ) -> Result<Edit, EditError> {
        if let Some(value) = constraint.dimension() {
            self.check_references(value)?;
        }
        let (name, sketch) = self.sketch_mut(feature)?;
        sketch
            .insert_constraint(id, constraint)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::RemoveSketchConstraint { feature, id })
    }

    pub(super) fn remove_sketch_constraint(
        &mut self,
        feature: FeatureId,
        id: ConstraintId,
    ) -> Result<Edit, EditError> {
        let (name, sketch) = self.sketch_mut(feature)?;
        let constraint = sketch
            .remove_constraint(id)
            .map_err(|error| EditError::Sketch { name, error })?;
        Ok(Edit::AddSketchConstraint {
            feature,
            id,
            constraint,
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
