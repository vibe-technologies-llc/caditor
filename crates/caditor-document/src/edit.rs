mod sketch;

use std::{collections::BTreeMap, sync::Arc};

use caditor_expression::{Expression, NameError, ParameterId, ParseError, check_name};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, SketchError};

use crate::{
    document::{Document, Feature, FeatureId, FeatureKind, Parameter, list_names},
    solid::BodyOperation,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    InsertParameter {
        index: usize,
        parameter: Parameter,
    },
    RemoveParameter {
        id: ParameterId,
    },
    RenameParameter {
        id: ParameterId,
        name: String,
    },
    SetParameterExpression {
        id: ParameterId,
        expression: Expression,
    },
    InsertFeature {
        index: usize,
        feature: Arc<Feature>,
    },
    RemoveFeature {
        id: FeatureId,
    },
    RenameFeature {
        id: FeatureId,
        name: String,
    },
    MoveFeature {
        id: FeatureId,
        index: usize,
    },
    SetFeatureKind {
        id: FeatureId,
        kind: FeatureKind,
    },
    SetDimension {
        feature: FeatureId,
        constraint: ConstraintId,
        value: Expression,
    },
    AddSketchEntity {
        feature: FeatureId,
        id: EntityId,
        entity: Entity,
    },
    RemoveSketchEntity {
        feature: FeatureId,
        id: EntityId,
    },
    SetSketchEntity {
        feature: FeatureId,
        id: EntityId,
        entity: Entity,
    },
    AddSketchConstraint {
        feature: FeatureId,
        id: ConstraintId,
        constraint: Constraint,
    },
    RemoveSketchConstraint {
        feature: FeatureId,
        id: ConstraintId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    label: String,
    edits: Vec<Edit>,
}

impl Transaction {
    pub fn new(label: impl Into<String>, edits: Vec<Edit>) -> Self {
        Self {
            label: label.into(),
            edits,
        }
    }

    pub fn single(label: impl Into<String>, edit: Edit) -> Self {
        Self::new(label, vec![edit])
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error("That parameter no longer exists")]
    MissingParameter,
    #[error("That feature no longer exists")]
    MissingFeature,
    #[error("An item with this ID already exists")]
    DuplicateId,
    #[error("Position {0} is outside the list")]
    OutOfRange(usize),
    #[error(transparent)]
    InvalidName(#[from] NameError),
    #[error("There is already a parameter named '{0}'")]
    DuplicateName(String),
    #[error("A feature needs a name")]
    EmptyFeatureName,
    #[error("{name} is used by {users}. Remove those uses first.")]
    ParameterInUse { name: String, users: String },
    #[error("This would make {name} depend on itself ({path})")]
    Cycle { name: String, path: String },
    #[error("{name} is used by {users}. Remove those features first.")]
    FeatureInUse { name: String, users: String },
    #[error("{name} cannot move above {other}, which it uses")]
    AboveDependency { name: String, other: String },
    #[error("{name} cannot move below {other}, which uses it")]
    BelowDependent { name: String, other: String },
    #[error("{name}: {error}")]
    Sketch { name: String, error: SketchError },
    #[error("{0} is not a sketch")]
    NotASketch(String),
    #[error("{0} does not make a body")]
    NotABody(String),
    #[error("{0} cannot become a different kind of feature")]
    KindChange(String),
    #[error("{name} makes the body that {users} change, so it must keep making a new body")]
    BodyInUse { name: String, users: String },
    #[error("In {feature}, {name} is used by {users}. Remove those first.")]
    EntityInUse {
        feature: String,
        name: String,
        users: String,
    },
}

pub struct TransactionBuilder<'a> {
    document: &'a Document,
    label: String,
    edits: Vec<Edit>,
    added_parameters: Vec<(String, ParameterId)>,
    next_parameter_id: u64,
    next_feature_id: u64,
    parameter_count: usize,
    feature_count: usize,
    next_sketch_ids: BTreeMap<FeatureId, u64>,
}

impl TransactionBuilder<'_> {
    pub fn parse(&self, text: &str) -> Result<Expression, ParseError> {
        Expression::parse(text, &|name| {
            self.added_parameters
                .iter()
                .find(|(added, _)| added == name)
                .map(|(_, id)| *id)
                .or_else(|| self.document.parameter_named(name).map(Parameter::id))
        })
    }

    pub fn add_parameter(
        &mut self,
        name: impl Into<String>,
        expression: Expression,
    ) -> ParameterId {
        let id = ParameterId::from_raw(self.next_parameter_id);
        self.next_parameter_id += 1;
        let name = name.into();
        self.added_parameters.push((name.clone(), id));
        self.edits.push(Edit::InsertParameter {
            index: self.parameter_count,
            parameter: Parameter::new(id, name, expression),
        });
        self.parameter_count += 1;
        id
    }

    pub fn add_feature(&mut self, name: impl Into<String>, kind: FeatureKind) -> FeatureId {
        let id = FeatureId::from_raw(self.next_feature_id);
        self.next_feature_id += 1;
        self.edits.push(Edit::InsertFeature {
            index: self.feature_count,
            feature: Arc::new(Feature::new(id, name.into(), kind)),
        });
        self.feature_count += 1;
        id
    }

    pub fn edit(&mut self, edit: Edit) -> &mut Self {
        self.edits.push(edit);
        self
    }

    pub fn finish(self) -> Transaction {
        Transaction::new(self.label, self.edits)
    }
}

impl Document {
    pub fn transaction(&self, label: impl Into<String>) -> TransactionBuilder<'_> {
        TransactionBuilder {
            document: self,
            label: label.into(),
            edits: Vec::new(),
            added_parameters: Vec::new(),
            next_parameter_id: self.next_parameter_id,
            next_feature_id: self.next_feature_id,
            parameter_count: self.parameters.len(),
            feature_count: self.features.len(),
            next_sketch_ids: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, transaction: Transaction) -> Result<Transaction, EditError> {
        let before = self.clone();
        let mut inverse = Vec::with_capacity(transaction.edits.len());
        for edit in transaction.edits {
            match self.apply_edit(edit) {
                Ok(undo) => inverse.push(undo),
                Err(error) => {
                    *self = before;
                    return Err(error);
                }
            }
        }
        inverse.reverse();
        Ok(Transaction::new(transaction.label, inverse))
    }

    pub fn check(&self, transaction: &Transaction) -> Result<(), EditError> {
        self.clone().apply(transaction.clone()).map(|_| ())
    }

    fn apply_edit(&mut self, edit: Edit) -> Result<Edit, EditError> {
        match edit {
            Edit::InsertParameter { index, parameter } => self.insert_parameter(index, parameter),
            Edit::RemoveParameter { id } => self.remove_parameter(id),
            Edit::RenameParameter { id, name } => self.rename_parameter(id, name),
            Edit::SetParameterExpression { id, expression } => {
                self.set_parameter_expression(id, expression)
            }
            Edit::InsertFeature { index, feature } => self.insert_feature(index, feature),
            Edit::RemoveFeature { id } => self.remove_feature(id),
            Edit::RenameFeature { id, name } => self.rename_feature(id, name),
            Edit::MoveFeature { id, index } => self.move_feature(id, index),
            Edit::SetFeatureKind { id, kind } => self.set_feature_kind(id, kind),
            Edit::SetDimension {
                feature,
                constraint,
                value,
            } => self.set_dimension(feature, constraint, value),
            Edit::AddSketchEntity {
                feature,
                id,
                entity,
            } => self.add_sketch_entity(feature, id, entity),
            Edit::RemoveSketchEntity { feature, id } => self.remove_sketch_entity(feature, id),
            Edit::SetSketchEntity {
                feature,
                id,
                entity,
            } => self.set_sketch_entity(feature, id, entity),
            Edit::AddSketchConstraint {
                feature,
                id,
                constraint,
            } => self.add_sketch_constraint(feature, id, constraint),
            Edit::RemoveSketchConstraint { feature, id } => {
                self.remove_sketch_constraint(feature, id)
            }
        }
    }

    fn check_parameter_name(
        &self,
        name: &str,
        renaming: Option<ParameterId>,
    ) -> Result<(), EditError> {
        check_name(name)?;
        match self.parameter_named(name) {
            Some(existing) if Some(existing.id()) != renaming => {
                Err(EditError::DuplicateName(name.to_owned()))
            }
            _ => Ok(()),
        }
    }

    fn check_references(&self, expression: &Expression) -> Result<(), EditError> {
        self.check_parameters_exist(expression.parameters())
    }

    fn check_parameters_exist(
        &self,
        used: impl IntoIterator<Item = ParameterId>,
    ) -> Result<(), EditError> {
        if used.into_iter().all(|id| self.parameter(id).is_some()) {
            Ok(())
        } else {
            Err(EditError::MissingParameter)
        }
    }

    fn check_feature_references(&self, kind: &FeatureKind, index: usize) -> Result<(), EditError> {
        self.check_parameters_exist(kind.parameters())?;
        for used in kind.features() {
            match self.feature_index(used) {
                Some(position) if position < index => {}
                _ => return Err(EditError::MissingFeature),
            }
        }
        let Some(solid) = kind.solid() else {
            return Ok(());
        };
        let sketch = self
            .feature(solid.sketch())
            .ok_or(EditError::MissingFeature)?;
        if sketch.kind.sketch().is_none() {
            return Err(EditError::NotASketch(sketch.name.clone()));
        }
        if let Some(target) = solid.operation().target() {
            let body = self.feature(target).ok_or(EditError::MissingFeature)?;
            if !body.makes_body() {
                return Err(EditError::NotABody(body.name.clone()));
            }
        }
        Ok(())
    }

    fn set_feature_kind(&mut self, id: FeatureId, kind: FeatureKind) -> Result<Edit, EditError> {
        let index = self.feature_position(id)?;
        let existing = self.feature(id).ok_or(EditError::MissingFeature)?;
        let name = existing.name.clone();
        let same_kind = match (&existing.kind, &kind) {
            (FeatureKind::Solid(old), FeatureKind::Solid(new)) => old.same_kind(new),
            _ => false,
        };
        if !same_kind {
            return Err(EditError::KindChange(name));
        }
        self.check_feature_references(&kind, index)?;
        let keeps_body = kind
            .solid()
            .is_some_and(|solid| solid.operation() == BodyOperation::NewBody);
        if !keeps_body {
            let users: Vec<String> = self
                .features()
                .filter(|other| {
                    other
                        .kind
                        .solid()
                        .and_then(|solid| solid.operation().target())
                        == Some(id)
                })
                .map(|other| other.name.clone())
                .collect();
            if !users.is_empty() {
                return Err(EditError::BodyInUse {
                    name,
                    users: list_names(&users),
                });
            }
        }
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.kind, kind);
        Ok(Edit::SetFeatureKind { id, kind: previous })
    }

    fn parameter_position(&self, id: ParameterId) -> Result<usize, EditError> {
        self.parameters
            .iter()
            .position(|parameter| parameter.id() == id)
            .ok_or(EditError::MissingParameter)
    }

    fn parameter_mut(&mut self, id: ParameterId) -> Result<&mut Parameter, EditError> {
        self.parameters
            .iter_mut()
            .find(|parameter| parameter.id() == id)
            .ok_or(EditError::MissingParameter)
    }

    fn feature_position(&self, id: FeatureId) -> Result<usize, EditError> {
        self.feature_index(id).ok_or(EditError::MissingFeature)
    }

    fn feature_mut(&mut self, id: FeatureId) -> Result<&mut Feature, EditError> {
        self.features
            .iter_mut()
            .find(|feature| feature.id() == id)
            .map(Arc::make_mut)
            .ok_or(EditError::MissingFeature)
    }

    fn insert_parameter(&mut self, index: usize, parameter: Parameter) -> Result<Edit, EditError> {
        if self.parameter(parameter.id()).is_some() {
            return Err(EditError::DuplicateId);
        }
        if index > self.parameters.len() {
            return Err(EditError::OutOfRange(index));
        }
        self.check_parameter_name(&parameter.name, None)?;
        self.check_references(&parameter.expression)?;
        let id = parameter.id();
        self.next_parameter_id = self.next_parameter_id.max(id.raw().saturating_add(1));
        self.parameters.insert(index, parameter);
        Ok(Edit::RemoveParameter { id })
    }

    fn remove_parameter(&mut self, id: ParameterId) -> Result<Edit, EditError> {
        let index = self.parameter_position(id)?;
        let users = self.parameter_users(id);
        if !users.is_empty() {
            return Err(EditError::ParameterInUse {
                name: self.parameter_name(id).unwrap_or_default().to_owned(),
                users: list_names(&users),
            });
        }
        let parameter = self.parameters.remove(index);
        Ok(Edit::InsertParameter { index, parameter })
    }

    fn rename_parameter(&mut self, id: ParameterId, name: String) -> Result<Edit, EditError> {
        self.check_parameter_name(&name, Some(id))?;
        let parameter = self.parameter_mut(id)?;
        let previous = std::mem::replace(&mut parameter.name, name);
        Ok(Edit::RenameParameter { id, name: previous })
    }

    fn set_parameter_expression(
        &mut self,
        id: ParameterId,
        expression: Expression,
    ) -> Result<Edit, EditError> {
        self.parameter_position(id)?;
        self.check_references(&expression)?;
        if let Some(cycle) = self.cycle_through(id, &expression) {
            let names: Vec<&str> = cycle
                .iter()
                .map(|step| self.parameter_name(*step).unwrap_or("?"))
                .collect();
            return Err(EditError::Cycle {
                name: self.parameter_name(id).unwrap_or_default().to_owned(),
                path: names.join(" → "),
            });
        }
        let parameter = self.parameter_mut(id)?;
        let previous = std::mem::replace(&mut parameter.expression, expression);
        Ok(Edit::SetParameterExpression {
            id,
            expression: previous,
        })
    }

    fn insert_feature(&mut self, index: usize, feature: Arc<Feature>) -> Result<Edit, EditError> {
        if self.feature(feature.id()).is_some() {
            return Err(EditError::DuplicateId);
        }
        if index > self.features.len() {
            return Err(EditError::OutOfRange(index));
        }
        if feature.name.trim().is_empty() {
            return Err(EditError::EmptyFeatureName);
        }
        self.check_feature_references(&feature.kind, index)?;
        let id = feature.id();
        self.next_feature_id = self.next_feature_id.max(id.raw().saturating_add(1));
        self.features.insert(index, feature);
        Ok(Edit::RemoveFeature { id })
    }

    fn remove_feature(&mut self, id: FeatureId) -> Result<Edit, EditError> {
        let index = self.feature_position(id)?;
        let dependents = self.feature_dependents(id);
        if !dependents.is_empty() {
            let users: Vec<String> = dependents
                .iter()
                .filter_map(|dependent| self.feature(*dependent))
                .map(|dependent| dependent.name.clone())
                .collect();
            return Err(EditError::FeatureInUse {
                name: self.feature_name(id),
                users: list_names(&users),
            });
        }
        let feature = self.features.remove(index);
        Ok(Edit::InsertFeature { index, feature })
    }

    fn rename_feature(&mut self, id: FeatureId, name: String) -> Result<Edit, EditError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(EditError::EmptyFeatureName);
        }
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.name, name);
        Ok(Edit::RenameFeature { id, name: previous })
    }

    fn move_feature(&mut self, id: FeatureId, index: usize) -> Result<Edit, EditError> {
        let from = self.feature_position(id)?;
        if index >= self.features.len() {
            return Err(EditError::OutOfRange(index));
        }
        let moving = self.feature(id).ok_or(EditError::MissingFeature)?;
        let name = moving.name.clone();
        let uses = moving.kind.features();
        let others: Vec<&Feature> = self.features().filter(|other| other.id() != id).collect();
        let (above, below) = others.split_at(index.min(others.len()));
        if let Some(needed) = below.iter().find(|other| uses.contains(&other.id())) {
            return Err(EditError::AboveDependency {
                name,
                other: needed.name.clone(),
            });
        }
        if let Some(user) = above
            .iter()
            .find(|other| other.kind.features().contains(&id))
        {
            return Err(EditError::BelowDependent {
                name,
                other: user.name.clone(),
            });
        }
        let feature = self.features.remove(from);
        self.features.insert(index, feature);
        Ok(Edit::MoveFeature { id, index: from })
    }

    fn feature_name(&self, id: FeatureId) -> String {
        self.feature(id)
            .map(|feature| feature.name.clone())
            .unwrap_or_default()
    }
}
