mod sketch;

use std::{collections::BTreeMap, sync::Arc};

use caditor_expression::{Expression, NameError, ParameterId, ParseError, check_name};
use caditor_geometry::Plane;
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, SketchError};

use crate::{
    attachment::SketchAttachment,
    datum::{Datum, PrincipalGeometry},
    dependencies::DependencyGraph,
    document::{
        Document, FIRST_UNSTORABLE_ID, Feature, FeatureId, FeatureKind, Parameter, RollbackBar,
        list_names,
    },
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
    SetFeatureHidden {
        id: FeatureId,
        hidden: bool,
    },
    SetFeatureSuppressed {
        id: FeatureId,
        suppressed: bool,
    },
    SetRollbackBar {
        bar: RollbackBar,
    },
    SetPrincipalHidden {
        geometry: PrincipalGeometry,
        hidden: bool,
    },
    SetFeatureKind {
        id: FeatureId,
        kind: FeatureKind,
    },
    SetSketchPlacement {
        feature: FeatureId,
        plane: Plane,
        attachment: Option<SketchAttachment>,
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
        construction: bool,
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
    SetSketchConstruction {
        feature: FeatureId,
        id: EntityId,
        construction: bool,
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

    pub fn approximate_size(&self) -> usize {
        let owned: usize = self
            .edits
            .iter()
            .map(|edit| match edit {
                Edit::InsertFeature { feature, .. } => {
                    feature.name.len() + feature.kind.approximate_size()
                }
                Edit::SetFeatureKind { kind, .. } => kind.approximate_size(),
                Edit::InsertParameter { parameter, .. } => parameter.name.len(),
                Edit::RenameParameter { name, .. } | Edit::RenameFeature { name, .. } => name.len(),
                Edit::RemoveParameter { .. }
                | Edit::SetParameterExpression { .. }
                | Edit::RemoveFeature { .. }
                | Edit::MoveFeature { .. }
                | Edit::SetFeatureHidden { .. }
                | Edit::SetFeatureSuppressed { .. }
                | Edit::SetRollbackBar { .. }
                | Edit::SetPrincipalHidden { .. }
                | Edit::SetSketchPlacement { .. }
                | Edit::SetDimension { .. }
                | Edit::AddSketchEntity { .. }
                | Edit::RemoveSketchEntity { .. }
                | Edit::SetSketchEntity { .. }
                | Edit::SetSketchConstruction { .. }
                | Edit::AddSketchConstraint { .. }
                | Edit::RemoveSketchConstraint { .. } => 0,
            })
            .sum();
        size_of::<Self>() + self.label.len() + size_of_val(self.edits.as_slice()) + owned
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error("That parameter no longer exists")]
    MissingParameter,
    #[error("That feature no longer exists")]
    MissingFeature,
    #[error("That item already exists")]
    DuplicateId,
    #[error("Its ID {0} is beyond the range caditor stores")]
    ReservedId(u64),
    #[error("The list has changed, so that place in it no longer exists")]
    OutOfRange(usize),
    #[error(transparent)]
    InvalidName(#[from] NameError),
    #[error("There is already a parameter named '{0}'")]
    DuplicateName(String),
    #[error("A feature needs a name")]
    EmptyFeatureName,
    #[error("There is already a feature named '{0}'")]
    DuplicateFeatureName(String),
    #[error("{name} is used by {users}. Remove those uses first.")]
    ParameterInUse { name: String, users: String },
    #[error("This would make {name} depend on itself ({path})")]
    Cycle { name: String, path: String },
    #[error("The rollback bar sits right above {0}; move the bar before deleting it")]
    RollbackBarAbove(String),
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
    #[error("{0} is not a plane")]
    NotAPlane(String),
    #[error("{0} is not an axis")]
    NotAnAxis(String),
    #[error("The revolution axis is not a line of {0}")]
    AxisNotALine(String),
    #[error("{0} cannot become a different kind of feature")]
    KindChange(String),
    #[error("{name} makes the body that {users} use, so it must keep making a new body")]
    BodyInUse { name: String, users: String },
    #[error("In {feature}, {name} is used by {users}. Remove those first.")]
    EntityInUse {
        feature: String,
        name: String,
        users: String,
    },
}

fn check_storable(raw: u64) -> Result<(), EditError> {
    if raw >= FIRST_UNSTORABLE_ID {
        return Err(EditError::ReservedId(raw));
    }
    Ok(())
}

#[derive(Default)]
struct ParameterGraph(Option<DependencyGraph>);

impl ParameterGraph {
    fn of(&mut self, document: &Document) -> &mut DependencyGraph {
        self.0.get_or_insert_with(|| DependencyGraph::of(document))
    }

    fn inserted(&mut self, id: ParameterId, expression: &Expression) {
        if let Some(graph) = &mut self.0 {
            graph.set(id, expression);
        }
    }

    fn removed(&mut self, id: ParameterId) {
        if let Some(graph) = &mut self.0 {
            graph.forget(id);
        }
    }
}

pub struct TransactionBuilder<'a> {
    document: &'a Document,
    label: String,
    edits: Vec<Edit>,
    added_parameters: Vec<(String, ParameterId)>,
    next_parameter_id: u64,
    next_feature_id: u64,
    parameter_count: usize,
    next_feature_index: usize,
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
        self.next_parameter_id = self.next_parameter_id.saturating_add(1);
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
        self.next_feature_id = self.next_feature_id.saturating_add(1);
        self.edits.push(Edit::InsertFeature {
            index: self.next_feature_index,
            feature: Arc::new(Feature::new(id, name.into(), kind)),
        });
        self.next_feature_index += 1;
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
            next_feature_index: self.bar_index(),
            next_sketch_ids: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, transaction: Transaction) -> Result<Transaction, EditError> {
        let before = self.clone();
        let mut inverse = Vec::with_capacity(transaction.edits.len());
        let mut graph = ParameterGraph::default();
        for edit in transaction.edits {
            match self.apply_edit(edit, &mut graph) {
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

    fn apply_edit(&mut self, edit: Edit, graph: &mut ParameterGraph) -> Result<Edit, EditError> {
        match edit {
            Edit::InsertParameter { index, parameter } => {
                self.insert_parameter(index, parameter, graph)
            }
            Edit::RemoveParameter { id } => self.remove_parameter(id, graph),
            Edit::RenameParameter { id, name } => self.rename_parameter(id, name),
            Edit::SetParameterExpression { id, expression } => {
                self.set_parameter_expression(id, expression, graph)
            }
            Edit::InsertFeature { index, feature } => self.insert_feature(index, feature),
            Edit::RemoveFeature { id } => self.remove_feature(id),
            Edit::RenameFeature { id, name } => self.rename_feature(id, name),
            Edit::MoveFeature { id, index } => self.move_feature(id, index),
            Edit::SetFeatureHidden { id, hidden } => self.set_feature_hidden(id, hidden),
            Edit::SetFeatureSuppressed { id, suppressed } => {
                self.set_feature_suppressed(id, suppressed)
            }
            Edit::SetRollbackBar { bar } => self.set_rollback_bar(bar),
            Edit::SetPrincipalHidden { geometry, hidden } => {
                Ok(self.set_principal_hidden(geometry, hidden))
            }
            Edit::SetFeatureKind { id, kind } => self.set_feature_kind(id, kind),
            Edit::SetSketchPlacement {
                feature,
                plane,
                attachment,
            } => self.set_sketch_placement(feature, plane, attachment),
            Edit::SetDimension {
                feature,
                constraint,
                value,
            } => self.set_dimension(feature, constraint, value),
            Edit::AddSketchEntity {
                feature,
                id,
                entity,
                construction,
            } => self.add_sketch_entity(feature, id, entity, construction),
            Edit::RemoveSketchEntity { feature, id } => self.remove_sketch_entity(feature, id),
            Edit::SetSketchEntity {
                feature,
                id,
                entity,
            } => self.set_sketch_entity(feature, id, entity),
            Edit::SetSketchConstruction {
                feature,
                id,
                construction,
            } => self.set_sketch_construction(feature, id, construction),
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
            check_storable(used.raw())?;
            if self
                .feature_index(used)
                .is_some_and(|position| position >= index)
            {
                return Err(EditError::MissingFeature);
            }
        }
        for body in kind
            .bodies_used()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if !body.makes_body() {
                return Err(EditError::NotABody(body.name.clone()));
            }
        }
        for plane in kind
            .planes_used()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if !plane.kind.datum().is_some_and(Datum::is_plane) {
                return Err(EditError::NotAPlane(plane.name.clone()));
            }
        }
        for axis in kind
            .axes_used()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if !axis.kind.datum().is_some_and(|datum| !datum.is_plane()) {
                return Err(EditError::NotAnAxis(axis.name.clone()));
            }
        }
        let Some(solid) = kind.solid() else {
            return Ok(());
        };
        let Some(sketch) = self.feature(solid.sketch()) else {
            return Ok(());
        };
        let Some(definition) = sketch.kind.sketch() else {
            return Err(EditError::NotASketch(sketch.name.clone()));
        };
        if let Some(axis) = solid.axis_line()
            && definition.line_direction(axis).is_none()
        {
            return Err(EditError::AxisNotALine(sketch.name.clone()));
        }
        Ok(())
    }

    fn set_feature_kind(&mut self, id: FeatureId, kind: FeatureKind) -> Result<Edit, EditError> {
        let index = self.feature_position(id)?;
        let existing = self.feature(id).ok_or(EditError::MissingFeature)?;
        let name = existing.name.clone();
        let same_kind = match (&existing.kind, &kind) {
            (FeatureKind::Solid(old), FeatureKind::Solid(new)) => old.same_kind(new),
            (FeatureKind::Datum(old), FeatureKind::Datum(new)) => old.same_kind(new),
            (FeatureKind::Blend(_), FeatureKind::Blend(_))
            | (FeatureKind::Shell(_), FeatureKind::Shell(_))
            | (FeatureKind::Pattern(_), FeatureKind::Pattern(_)) => true,
            _ => false,
        };
        if !same_kind {
            return Err(EditError::KindChange(name));
        }
        self.check_feature_references(&kind, index)?;
        self.reserve_past_references(&kind);
        let keeps_body = kind
            .solid()
            .is_some_and(|solid| solid.operation() == BodyOperation::NewBody);
        if !keeps_body {
            let users: Vec<String> = self
                .features()
                .filter(|other| other.kind.bodies_used().contains(&id))
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

    fn set_sketch_placement(
        &mut self,
        id: FeatureId,
        plane: Plane,
        attachment: Option<SketchAttachment>,
    ) -> Result<Edit, EditError> {
        let index = self.feature_position(id)?;
        let existing = self.feature(id).ok_or(EditError::MissingFeature)?;
        let FeatureKind::Sketch(sketch) = &existing.kind else {
            return Err(EditError::NotASketch(existing.name.clone()));
        };
        let mut placed = sketch.clone();
        placed.sketch.set_plane(plane);
        placed.attachment = attachment;
        let kind = FeatureKind::Sketch(placed);
        self.check_feature_references(&kind, index)?;
        self.reserve_past_references(&kind);
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.kind, kind);
        let FeatureKind::Sketch(previous) = previous else {
            return Err(EditError::NotASketch(feature.name.clone()));
        };
        Ok(Edit::SetSketchPlacement {
            feature: id,
            plane: previous.sketch.plane(),
            attachment: previous.attachment,
        })
    }

    fn parameter_position(&self, id: ParameterId) -> Result<usize, EditError> {
        self.parameters
            .position(id)
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

    fn insert_parameter(
        &mut self,
        index: usize,
        parameter: Parameter,
        graph: &mut ParameterGraph,
    ) -> Result<Edit, EditError> {
        if self.parameter(parameter.id()).is_some() {
            return Err(EditError::DuplicateId);
        }
        check_storable(parameter.id().raw())?;
        if index > self.parameters.len() {
            return Err(EditError::OutOfRange(index));
        }
        self.check_parameter_name(&parameter.name, None)?;
        self.check_references(&parameter.expression)?;
        let id = parameter.id();
        self.next_parameter_id = self.next_parameter_id.max(id.raw().saturating_add(1));
        graph.inserted(id, &parameter.expression);
        self.parameters.insert(index, parameter);
        Ok(Edit::RemoveParameter { id })
    }

    fn remove_parameter(
        &mut self,
        id: ParameterId,
        graph: &mut ParameterGraph,
    ) -> Result<Edit, EditError> {
        let index = self.parameter_position(id)?;
        let used = graph.of(self).has_users(id)
            || self
                .features
                .iter()
                .any(|feature| feature.kind.uses_parameter(id));
        if used {
            self.can_remove_parameter(id)?;
        }
        graph.removed(id);
        let parameter = self
            .parameters
            .remove(index)
            .ok_or(EditError::MissingParameter)?;
        Ok(Edit::InsertParameter { index, parameter })
    }

    fn rename_parameter(&mut self, id: ParameterId, name: String) -> Result<Edit, EditError> {
        self.check_parameter_name(&name, Some(id))?;
        let previous = self
            .parameters
            .rename(id, name)
            .ok_or(EditError::MissingParameter)?;
        Ok(Edit::RenameParameter { id, name: previous })
    }

    fn set_parameter_expression(
        &mut self,
        id: ParameterId,
        expression: Expression,
        graph: &mut ParameterGraph,
    ) -> Result<Edit, EditError> {
        self.parameter_position(id)?;
        self.check_references(&expression)?;
        let dependencies = graph.of(self);
        if let Some(cycle) = dependencies.cycle(id, &expression) {
            let names: Vec<&str> = cycle
                .iter()
                .map(|step| self.parameter_name(*step).unwrap_or("?"))
                .collect();
            return Err(EditError::Cycle {
                name: self.parameter_name(id).unwrap_or_default().to_owned(),
                path: names.join(" → "),
            });
        }
        dependencies.set(id, &expression);
        let previous = self
            .parameters
            .replace_expression(id, expression)
            .ok_or(EditError::MissingParameter)?;
        Ok(Edit::SetParameterExpression {
            id,
            expression: previous,
        })
    }

    pub fn can_remove_parameter(&self, id: ParameterId) -> Result<(), EditError> {
        self.parameter_position(id)?;
        let users = self.parameter_users(id);
        if users.is_empty() {
            return Ok(());
        }
        Err(EditError::ParameterInUse {
            name: self.parameter_name(id).unwrap_or_default().to_owned(),
            users: list_names(&users),
        })
    }

    fn insert_feature(
        &mut self,
        index: usize,
        mut feature: Arc<Feature>,
    ) -> Result<Edit, EditError> {
        if self.feature(feature.id()).is_some() {
            return Err(EditError::DuplicateId);
        }
        check_storable(feature.id().raw())?;
        if index > self.features.len() {
            return Err(EditError::OutOfRange(index));
        }
        let trimmed = feature.name.trim();
        if trimmed.is_empty() {
            return Err(EditError::EmptyFeatureName);
        }
        if trimmed.len() != feature.name.len() {
            let trimmed = trimmed.to_owned();
            Arc::make_mut(&mut feature).name = trimmed;
        }
        self.check_feature_name(&feature.name, feature.id())?;
        self.check_feature_references(&feature.kind, index)?;
        let id = feature.id();
        self.next_feature_id = self.next_feature_id.max(id.raw().saturating_add(1));
        self.reserve_past_references(&feature.kind);
        self.features.insert(index, feature);
        Ok(Edit::RemoveFeature { id })
    }

    fn reserve_past_references(&mut self, kind: &FeatureKind) {
        let beyond = kind
            .features()
            .into_iter()
            .map(|used| used.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        self.next_feature_id = self.next_feature_id.max(beyond);
    }

    fn remove_feature(&mut self, id: FeatureId) -> Result<Edit, EditError> {
        let index = self.feature_position(id)?;
        if self.rollback == RollbackBar::Before(id) {
            return Err(EditError::RollbackBarAbove(self.feature_name(id)));
        }
        let feature = self.features.remove(index);
        Ok(Edit::InsertFeature { index, feature })
    }

    fn rename_feature(&mut self, id: FeatureId, name: String) -> Result<Edit, EditError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(EditError::EmptyFeatureName);
        }
        self.check_feature_name(&name, id)?;
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.name, name);
        Ok(Edit::RenameFeature { id, name: previous })
    }

    fn set_feature_hidden(&mut self, id: FeatureId, hidden: bool) -> Result<Edit, EditError> {
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.hidden, hidden);
        Ok(Edit::SetFeatureHidden {
            id,
            hidden: previous,
        })
    }

    fn set_feature_suppressed(
        &mut self,
        id: FeatureId,
        suppressed: bool,
    ) -> Result<Edit, EditError> {
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.suppressed, suppressed);
        Ok(Edit::SetFeatureSuppressed {
            id,
            suppressed: previous,
        })
    }

    fn set_rollback_bar(&mut self, bar: RollbackBar) -> Result<Edit, EditError> {
        if let RollbackBar::Before(feature) = bar {
            self.feature_position(feature)?;
        }
        let previous = std::mem::replace(&mut self.rollback, bar);
        Ok(Edit::SetRollbackBar { bar: previous })
    }

    fn set_principal_hidden(&mut self, geometry: PrincipalGeometry, hidden: bool) -> Edit {
        let previous = if hidden {
            !self.hidden_principal.insert(geometry)
        } else {
            self.hidden_principal.remove(&geometry)
        };
        Edit::SetPrincipalHidden {
            geometry,
            hidden: previous,
        }
    }

    fn check_feature_name(&self, name: &str, id: FeatureId) -> Result<(), EditError> {
        let taken = self
            .features()
            .any(|other| other.id() != id && other.name == name);
        if taken {
            return Err(EditError::DuplicateFeatureName(name.to_owned()));
        }
        Ok(())
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
