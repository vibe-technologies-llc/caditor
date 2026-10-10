mod sketch;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_expression::{Expression, NameError, ParameterId, ParseError, check_name};
use caditor_geometry::{Plane, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, SketchError};

use crate::{
    attachment::SketchAttachment,
    body_appearance::{
        BodyAppearance, MAX_BODY_NAME_CHARS, MAX_MATERIAL_NAME_CHARS, MIN_OPACITY_PERCENT,
        OPAQUE_PERCENT, material_name,
    },
    configurations::{
        ConfigurationId, Configurations, ConfiguredValue, MAX_CONFIGURATION_NAME_CHARS,
        MAX_CONFIGURATIONS, MAX_CONFIGURED_VALUES,
    },
    datum::{Datum, PrincipalGeometry},
    dependencies::DependencyGraph,
    document::{
        Document, FIRST_UNSTORABLE_ID, Feature, FeatureId, FeatureKind, Parameter, RollbackBar,
        list_names,
    },
    grouping::{MAX_GROUP_NAME_CHARS, group_name},
    measurement::Measured,
    model_parameters::{MAX_VALUE_LABEL_CHARS, ParameterOwner},
    projection::ProjectionSource,
    properties::{ModelProperties, ModelProperty},
    selection_sets::{MAX_SELECTION_SETS, MAX_SET_MEMBERS, MAX_SET_NAME_CHARS, SelectionSets},
    solid::BodyOperation,
    views::{MAX_SAVED_VIEWS, MAX_VIEW_NAME_CHARS, SavedViews},
};

pub const MAX_PARAMETER_NOTE_CHARS: usize = 2000;

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
    MoveParameter {
        id: ParameterId,
        index: usize,
    },
    SetParameterNote {
        id: ParameterId,
        note: String,
    },
    SetParameterOwner {
        id: ParameterId,
        owner: Option<ParameterOwner>,
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
    SetFeatureGroup {
        id: FeatureId,
        group: Option<String>,
    },
    SetBodyAppearance {
        id: FeatureId,
        appearance: BodyAppearance,
    },
    SetRollbackBar {
        bar: RollbackBar,
    },
    SetPrincipalHidden {
        geometry: PrincipalGeometry,
        hidden: bool,
    },
    SetModelProperties {
        properties: Box<ModelProperties>,
    },
    SetSavedViews {
        views: Box<SavedViews>,
    },
    SetSelectionSets {
        sets: Box<SelectionSets>,
    },
    SetConfigurations {
        configurations: Box<Configurations>,
    },
    SetActiveConfiguration {
        active: Option<ConfigurationId>,
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
    SetSketchProjection {
        feature: FeatureId,
        id: EntityId,
        source: Option<ProjectionSource>,
    },
    AddSketchConstraint {
        feature: FeatureId,
        id: ConstraintId,
        constraint: Constraint,
        inactive: bool,
        label: Option<Vector2>,
    },
    RemoveSketchConstraint {
        feature: FeatureId,
        id: ConstraintId,
    },
    SetSketchConstraintActive {
        feature: FeatureId,
        id: ConstraintId,
        active: bool,
    },
    SetSketchLabel {
        feature: FeatureId,
        id: ConstraintId,
        offset: Option<Vector2>,
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

    pub fn into_parts(self) -> (String, Vec<Edit>) {
        (self.label, self.edits)
    }

    pub fn touched(&self) -> Touched {
        let mut touched = Touched::default();
        for edit in &self.edits {
            match edit {
                Edit::InsertParameter { parameter, .. } => {
                    touched.parameters.insert(parameter.id());
                    touched
                        .named_parameters
                        .insert(parameter.id(), parameter.name.clone());
                }
                Edit::RemoveParameter { id }
                | Edit::RenameParameter { id, .. }
                | Edit::SetParameterExpression { id, .. }
                | Edit::MoveParameter { id, .. }
                | Edit::SetParameterNote { id, .. }
                | Edit::SetParameterOwner { id, .. } => {
                    touched.parameters.insert(*id);
                }
                Edit::InsertFeature { feature, .. } => {
                    touched.features.insert(feature.id());
                    touched
                        .named_features
                        .insert(feature.id(), feature.name.clone());
                }
                Edit::RemoveFeature { id }
                | Edit::RenameFeature { id, .. }
                | Edit::MoveFeature { id, .. }
                | Edit::SetFeatureHidden { id, .. }
                | Edit::SetFeatureSuppressed { id, .. }
                | Edit::SetFeatureGroup { id, .. }
                | Edit::SetBodyAppearance { id, .. }
                | Edit::SetFeatureKind { id, .. } => {
                    touched.features.insert(*id);
                }
                Edit::SetSketchPlacement { feature, .. } => {
                    touched.features.insert(*feature);
                }
                Edit::AddSketchEntity { feature, id, .. }
                | Edit::RemoveSketchEntity { feature, id }
                | Edit::SetSketchEntity { feature, id, .. }
                | Edit::SetSketchConstruction { feature, id, .. }
                | Edit::SetSketchProjection { feature, id, .. } => {
                    touched.features.insert(*feature);
                    touched.entities.insert((*feature, *id));
                }
                Edit::SetDimension {
                    feature,
                    constraint: id,
                    ..
                }
                | Edit::AddSketchConstraint { feature, id, .. }
                | Edit::RemoveSketchConstraint { feature, id }
                | Edit::SetSketchConstraintActive { feature, id, .. }
                | Edit::SetSketchLabel { feature, id, .. } => {
                    touched.features.insert(*feature);
                    touched.constraints.insert((*feature, *id));
                }
                Edit::SetRollbackBar { .. } => touched.rollback = true,
                Edit::SetPrincipalHidden { .. } => touched.principal = true,
                Edit::SetModelProperties { .. } => touched.properties = true,
                Edit::SetSavedViews { .. } => touched.views = true,
                Edit::SetSelectionSets { .. } => touched.selection_sets = true,
                Edit::SetConfigurations { .. } | Edit::SetActiveConfiguration { .. } => {
                    touched.configurations = true;
                }
            }
        }
        touched
    }

    pub fn approximate_size(&self) -> usize {
        let owned: usize = self
            .edits
            .iter()
            .map(|edit| match edit {
                Edit::InsertFeature { feature, .. } => feature.heap_size(),
                Edit::SetBodyAppearance { appearance, .. } => appearance.heap_size(),
                Edit::SetFeatureKind { kind, .. } => kind.approximate_size(),
                Edit::InsertParameter { parameter, .. } => {
                    parameter.name.len()
                        + parameter.note.len()
                        + parameter.expression.heap_size()
                        + parameter
                            .owner
                            .as_ref()
                            .map_or(0, ParameterOwner::heap_size)
                }
                Edit::SetParameterOwner { owner, .. } => {
                    owner.as_ref().map_or(0, ParameterOwner::heap_size)
                }
                Edit::RenameParameter { name, .. } | Edit::RenameFeature { name, .. } => name.len(),
                Edit::SetParameterNote { note, .. } => note.len(),
                Edit::SetFeatureGroup { group, .. } => group.as_ref().map_or(0, String::len),
                Edit::SetModelProperties { properties } => {
                    size_of::<ModelProperties>() + properties.heap_size()
                }
                Edit::SetSavedViews { views } => size_of::<SavedViews>() + views.heap_size(),
                Edit::SetSelectionSets { sets } => size_of::<SelectionSets>() + sets.heap_size(),
                Edit::SetConfigurations { configurations } => {
                    size_of::<Configurations>() + configurations.heap_size()
                }
                Edit::SetParameterExpression { expression, .. }
                | Edit::SetDimension {
                    value: expression, ..
                } => expression.heap_size(),
                Edit::AddSketchEntity { entity, .. } | Edit::SetSketchEntity { entity, .. } => {
                    entity.heap_size()
                }
                Edit::AddSketchConstraint { constraint, .. } => constraint.heap_size(),
                Edit::RemoveParameter { .. }
                | Edit::MoveParameter { .. }
                | Edit::RemoveFeature { .. }
                | Edit::MoveFeature { .. }
                | Edit::SetFeatureHidden { .. }
                | Edit::SetFeatureSuppressed { .. }
                | Edit::SetRollbackBar { .. }
                | Edit::SetActiveConfiguration { .. }
                | Edit::SetPrincipalHidden { .. }
                | Edit::SetSketchPlacement { .. }
                | Edit::RemoveSketchEntity { .. }
                | Edit::SetSketchConstruction { .. }
                | Edit::SetSketchProjection { .. }
                | Edit::SetSketchConstraintActive { .. }
                | Edit::SetSketchLabel { .. }
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
    #[error("{name} in {feature} still follows the geometry it was projected from")]
    StillProjected { feature: String, name: String },
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
    #[error(
        "{name} cannot be written into {user}: the expression there would grow too long or too deeply nested"
    )]
    InliningTooLong { name: String, user: String },
    #[error(
        "A parameter's note may be at most {MAX_PARAMETER_NOTE_CHARS} characters long, and this one has {0}"
    )]
    NoteTooLong(usize),
    #[error(
        "The model's {} may be at most {} characters long, and this one has {length}",
        property.in_sentence(),
        property.max_chars()
    )]
    PropertyTooLong {
        property: ModelProperty,
        length: usize,
    },
    #[error("A view needs a name")]
    ViewNameEmpty,
    #[error(
        "A view's name may be at most {MAX_VIEW_NAME_CHARS} characters long, and this one has {length}"
    )]
    ViewNameTooLong { length: usize },
    #[error("There is already a view named '{0}'. Choose another name.")]
    ViewNameTaken(String),
    #[error("A model keeps at most {MAX_SAVED_VIEWS} saved views. Delete one first.")]
    TooManyViews,
    #[error("The view '{0}' is not a view the camera can show, so it was not saved")]
    ViewNotUsable(String),
    #[error(
        "A named value's description may be at most {MAX_VALUE_LABEL_CHARS} characters long, and this one has {0}"
    )]
    ValueLabelTooLong(usize),
    #[error("A selection set needs a name")]
    SetNameEmpty,
    #[error(
        "A selection set's name may be at most {MAX_SET_NAME_CHARS} characters long, and this one has {length}"
    )]
    SetNameTooLong { length: usize },
    #[error("There is already a selection set named '{0}'. Choose another name.")]
    SetNameTaken(String),
    #[error("A model keeps at most {MAX_SELECTION_SETS} selection sets. Delete one first.")]
    TooManySets,
    #[error("The selection set '{0}' holds nothing. Select faces, edges or bodies to keep in it.")]
    SetEmpty(String),
    #[error(
        "A selection set holds at most {MAX_SET_MEMBERS} faces, edges and bodies, and '{name}' has {members}"
    )]
    SetTooLarge { name: String, members: usize },
    #[error("A configuration needs a name")]
    ConfigurationNameEmpty,
    #[error(
        "A configuration's name may be at most {MAX_CONFIGURATION_NAME_CHARS} characters long, and this one has {length}"
    )]
    ConfigurationNameTooLong { length: usize },
    #[error("There is already a configuration named '{0}'. Choose another name.")]
    ConfigurationNameTaken(String),
    #[error("A model keeps at most {MAX_CONFIGURATIONS} configurations. Delete one first.")]
    TooManyConfigurations,
    #[error(
        "Configurations set at most {MAX_CONFIGURED_VALUES} values. Stop configuring one first."
    )]
    TooManyConfiguredValues,
    #[error("That configuration no longer exists")]
    MissingConfiguration,
    #[error("That value cannot be set that way in a configuration")]
    SettingMismatch,
    #[error(
        "The model does not hold the values of {0}, so it cannot become the active configuration"
    )]
    ConfigurationOutOfStep(String),
    #[error("This would make {name} depend on itself ({path})")]
    Cycle { name: String, path: String },
    #[error(
        "{name} cannot use {parameter}, the reading of {measurement}: parameters are worked out before the model is measured ({name} → {parameter} → {measurement} → {name}); use {parameter} in a feature below {measurement} instead"
    )]
    ParameterReadsMeasurement {
        name: String,
        parameter: String,
        measurement: String,
    },
    #[error(
        "{feature} cannot use {parameter}, the reading of {measurement}, which is taken further down the tree ({feature} → {parameter} → {measurement} → {feature}); move {measurement} above {feature}"
    )]
    MeasurementBelowUser {
        feature: String,
        parameter: String,
        measurement: String,
    },
    #[error("{parameter} already holds the reading of {measurement}")]
    ParameterMeasuredTwice {
        parameter: String,
        measurement: String,
    },
    #[error(
        "{name} is the reading of {measurement}, which follows the model, so it cannot be written into what uses it; delete {measurement} or stop using {name} first"
    )]
    MeasuredParameterInlined { name: String, measurement: String },
    #[error("The rollback bar sits right above {0}; move the bar before deleting it")]
    RollbackBarAbove(String),
    #[error("{name} cannot move above {other}, which it uses")]
    AboveDependency { name: String, other: String },
    #[error("{name} cannot move below {other}, which uses it")]
    BelowDependent { name: String, other: String },
    #[error(
        "{name} cannot move below {other}, which combines the body of {body} it uses into another"
    )]
    BelowConsumer {
        name: String,
        other: String,
        body: String,
    },
    #[error(
        "{name} cannot move above {other}, which uses the body of {body} that {name} combines into another"
    )]
    AboveConsumedUse {
        name: String,
        other: String,
        body: String,
    },
    #[error("{name}: {error}")]
    Sketch { name: String, error: SketchError },
    #[error("{0} is not a sketch")]
    NotASketch(String),
    #[error("{0} does not make a body")]
    NotABody(String),
    #[error(
        "A material name may be at most {MAX_MATERIAL_NAME_CHARS} characters long, and this one has {0}"
    )]
    MaterialNameTooLong(usize),
    #[error(
        "A body's name may be at most {MAX_BODY_NAME_CHARS} characters long, and this one has {0}"
    )]
    BodyNameTooLong(usize),
    #[error(
        "A group's name may be at most {MAX_GROUP_NAME_CHARS} characters long, and this one has {0}"
    )]
    GroupNameTooLong(usize),
    #[error("A body may be see-through down to {MIN_OPACITY_PERCENT}% opacity, and {0}% is less")]
    OpacityTooLow(u8),
    #[error("{0} is not a plane")]
    NotAPlane(String),
    #[error("{0} is not an axis")]
    NotAnAxis(String),
    #[error("{0} is not a point")]
    NotAPoint(String),
    #[error("{0} is not a coordinate system")]
    NotACoordinateSystem(String),
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

fn check_appearance(appearance: &BodyAppearance) -> Result<(), EditError> {
    if let Some(material) = &appearance.material {
        let length = material.chars().count();
        if length > MAX_MATERIAL_NAME_CHARS {
            return Err(EditError::MaterialNameTooLong(length));
        }
    }
    if let Some(name) = &appearance.name {
        let length = name.chars().count();
        if length > MAX_BODY_NAME_CHARS {
            return Err(EditError::BodyNameTooLong(length));
        }
    }
    let opacities = appearance
        .faces
        .iter()
        .filter_map(|face| face.opacity)
        .chain(appearance.opacity);
    for opacity in opacities {
        if opacity < MIN_OPACITY_PERCENT {
            return Err(EditError::OpacityTooLow(opacity));
        }
    }
    Ok(())
}

fn check_note(note: &str) -> Result<(), EditError> {
    let length = note.chars().count();
    if length > MAX_PARAMETER_NOTE_CHARS {
        return Err(EditError::NoteTooLong(length));
    }
    Ok(())
}

fn check_storable(raw: u64) -> Result<(), EditError> {
    if raw >= FIRST_UNSTORABLE_ID {
        return Err(EditError::ReservedId(raw));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum FollowedValue {
    Parameter(ParameterId),
    Feature(FeatureId),
}

impl FollowedValue {
    fn covers(self, column: ConfiguredValue) -> bool {
        match (self, column) {
            (Self::Parameter(id), ConfiguredValue::Parameter(other)) => id == other,
            (
                Self::Feature(id),
                ConfiguredValue::Suppressed(other) | ConfiguredValue::Colour(other),
            ) => id == other,
            _ => false,
        }
    }
}

fn followed_value(edit: &Edit) -> Option<FollowedValue> {
    match edit {
        Edit::InsertParameter { parameter, .. } => Some(FollowedValue::Parameter(parameter.id())),
        Edit::SetParameterExpression { id, .. } => Some(FollowedValue::Parameter(*id)),
        Edit::InsertFeature { feature, .. } => Some(FollowedValue::Feature(feature.id())),
        Edit::SetFeatureSuppressed { id, .. } | Edit::SetBodyAppearance { id, .. } => {
            Some(FollowedValue::Feature(*id))
        }
        _ => None,
    }
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Touched {
    pub features: BTreeSet<FeatureId>,
    pub named_features: BTreeMap<FeatureId, String>,
    pub parameters: BTreeSet<ParameterId>,
    pub named_parameters: BTreeMap<ParameterId, String>,
    pub entities: BTreeSet<(FeatureId, EntityId)>,
    pub constraints: BTreeSet<(FeatureId, ConstraintId)>,
    pub rollback: bool,
    pub principal: bool,
    pub properties: bool,
    pub views: bool,
    pub selection_sets: bool,
    pub configurations: bool,
}

pub struct TransactionBuilder<'a> {
    document: &'a Document,
    label: String,
    pub(crate) edits: Vec<Edit>,
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

    pub fn add_owned_parameter(
        &mut self,
        name: impl Into<String>,
        expression: Expression,
        owner: ParameterOwner,
    ) -> ParameterId {
        let id = self.add_parameter(name, expression);
        if let Some(Edit::InsertParameter { parameter, .. }) = self.edits.last_mut() {
            parameter.owner = Some(owner);
        }
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

    pub fn add_copied_feature(&mut self, mut feature: Feature) -> FeatureId {
        let id = FeatureId::from_raw(self.next_feature_id);
        self.next_feature_id = self.next_feature_id.saturating_add(1);
        feature.set_id(id);
        self.edits.push(Edit::InsertFeature {
            index: self.next_feature_index,
            feature: Arc::new(feature),
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
        let followed = followed_value(&edit);
        let undo = self.apply_bare_edit(edit, graph)?;
        if let Some(value) = followed {
            self.follow_active_configuration(value);
        }
        Ok(undo)
    }

    fn apply_bare_edit(
        &mut self,
        edit: Edit,
        graph: &mut ParameterGraph,
    ) -> Result<Edit, EditError> {
        match edit {
            Edit::InsertParameter { index, parameter } => {
                self.insert_parameter(index, parameter, graph)
            }
            Edit::RemoveParameter { id } => self.remove_parameter(id, graph),
            Edit::RenameParameter { id, name } => self.rename_parameter(id, name),
            Edit::SetParameterExpression { id, expression } => {
                self.set_parameter_expression(id, expression, graph)
            }
            Edit::MoveParameter { id, index } => self.move_parameter(id, index),
            Edit::SetParameterNote { id, note } => self.set_parameter_note(id, note),
            Edit::SetParameterOwner { id, owner } => self.set_parameter_owner(id, owner),
            Edit::InsertFeature { index, feature } => self.insert_feature(index, feature),
            Edit::RemoveFeature { id } => self.remove_feature(id),
            Edit::RenameFeature { id, name } => self.rename_feature(id, name),
            Edit::MoveFeature { id, index } => self.move_feature(id, index),
            Edit::SetFeatureHidden { id, hidden } => self.set_feature_hidden(id, hidden),
            Edit::SetFeatureSuppressed { id, suppressed } => {
                self.set_feature_suppressed(id, suppressed)
            }
            Edit::SetFeatureGroup { id, group } => self.set_feature_group(id, group),
            Edit::SetBodyAppearance { id, appearance } => self.set_body_appearance(id, appearance),
            Edit::SetRollbackBar { bar } => self.set_rollback_bar(bar),
            Edit::SetPrincipalHidden { geometry, hidden } => {
                Ok(self.set_principal_hidden(geometry, hidden))
            }
            Edit::SetModelProperties { properties } => self.set_model_properties(*properties),
            Edit::SetSavedViews { views } => self.set_saved_views(*views),
            Edit::SetSelectionSets { sets } => self.set_selection_sets(*sets),
            Edit::SetConfigurations { configurations } => self.set_configurations(*configurations),
            Edit::SetActiveConfiguration { active } => self.set_active_configuration(active),
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
            Edit::SetSketchProjection {
                feature,
                id,
                source,
            } => self.set_sketch_projection(feature, id, source),
            Edit::AddSketchConstraint {
                feature,
                id,
                constraint,
                inactive,
                label,
            } => self.add_sketch_constraint(feature, id, constraint, inactive, label),
            Edit::RemoveSketchConstraint { feature, id } => {
                self.remove_sketch_constraint(feature, id)
            }
            Edit::SetSketchConstraintActive {
                feature,
                id,
                active,
            } => self.set_sketch_constraint_active(feature, id, active),
            Edit::SetSketchLabel {
                feature,
                id,
                offset,
            } => self.set_sketch_label(feature, id, offset),
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
        if let FeatureKind::Sketch(sketch) = kind {
            for source in sketch
                .projected_sketches()
                .filter_map(|id| self.feature(id))
            {
                if source.kind.sketch().is_none() {
                    return Err(EditError::NotASketch(source.name.clone()));
                }
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
            if !axis.kind.datum().is_some_and(Datum::is_axis) {
                return Err(EditError::NotAnAxis(axis.name.clone()));
            }
        }
        for frame in kind
            .frames_used()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if !frame.kind.datum().is_some_and(Datum::is_frame) {
                return Err(EditError::NotACoordinateSystem(frame.name.clone()));
            }
        }
        for point in kind
            .points_used()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if !point.kind.datum().is_some_and(Datum::is_point) {
                return Err(EditError::NotAPoint(point.name.clone()));
            }
        }
        for sketch in kind
            .reference_sketches()
            .into_iter()
            .filter_map(|id| self.feature(id))
        {
            if sketch.kind.sketch().is_none() {
                return Err(EditError::NotASketch(sketch.name.clone()));
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
            | (FeatureKind::OffsetFace(_), FeatureKind::OffsetFace(_))
            | (FeatureKind::SplitFace(_), FeatureKind::SplitFace(_))
            | (FeatureKind::Primitive(_), FeatureKind::Primitive(_))
            | (FeatureKind::Thread(_), FeatureKind::Thread(_))
            | (FeatureKind::Combine(_), FeatureKind::Combine(_))
            | (FeatureKind::Move(_), FeatureKind::Move(_))
            | (FeatureKind::Mate(_), FeatureKind::Mate(_))
            | (FeatureKind::Mirror(_), FeatureKind::Mirror(_))
            | (FeatureKind::Split(_), FeatureKind::Split(_))
            | (FeatureKind::Scale(_), FeatureKind::Scale(_))
            | (FeatureKind::Hole(_), FeatureKind::Hole(_))
            | (FeatureKind::Pattern(_), FeatureKind::Pattern(_))
            | (FeatureKind::Import(_), FeatureKind::Import(_))
            | (FeatureKind::Remove(_), FeatureKind::Remove(_))
            | (FeatureKind::Measurement(_), FeatureKind::Measurement(_)) => true,
            _ => false,
        };
        if !same_kind {
            return Err(EditError::KindChange(name));
        }
        self.check_feature_references(&kind, index)?;
        self.check_measurement_order(&Feature::new(id, name.clone(), kind.clone()), index)?;
        self.reserve_past_references(&kind);
        let keeps_body = matches!(kind, FeatureKind::Import(_))
            || kind
                .solid()
                .is_some_and(|solid| solid.operation() == BodyOperation::NewBody)
            || kind
                .primitive()
                .is_some_and(|primitive| primitive.operation == BodyOperation::NewBody);
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
        mut parameter: Parameter,
        graph: &mut ParameterGraph,
    ) -> Result<Edit, EditError> {
        parameter.owner = parameter.owner.map(ParameterOwner::checked).transpose()?;
        if self.parameter(parameter.id()).is_some() {
            return Err(EditError::DuplicateId);
        }
        check_storable(parameter.id().raw())?;
        if index > self.parameters.len() {
            return Err(EditError::OutOfRange(index));
        }
        self.check_parameter_name(&parameter.name, None)?;
        check_note(&parameter.note)?;
        self.check_references(&parameter.expression)?;
        self.check_reads_no_measurement(&parameter.name, &parameter.expression)?;
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
                .any(|feature| feature.uses_parameter(id));
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

    fn move_parameter(&mut self, id: ParameterId, index: usize) -> Result<Edit, EditError> {
        let from = self.parameter_position(id)?;
        if index >= self.parameters.len() {
            return Err(EditError::OutOfRange(index));
        }
        let parameter = self
            .parameters
            .remove(from)
            .ok_or(EditError::MissingParameter)?;
        self.parameters.insert(index, parameter);
        Ok(Edit::MoveParameter { id, index: from })
    }

    fn set_parameter_note(&mut self, id: ParameterId, note: String) -> Result<Edit, EditError> {
        let note = note.trim().to_owned();
        check_note(&note)?;
        let previous = self
            .parameters
            .set_note(id, note)
            .ok_or(EditError::MissingParameter)?;
        Ok(Edit::SetParameterNote { id, note: previous })
    }

    fn set_parameter_owner(
        &mut self,
        id: ParameterId,
        owner: Option<ParameterOwner>,
    ) -> Result<Edit, EditError> {
        let owner = owner.map(ParameterOwner::checked).transpose()?;
        let previous = self
            .parameters
            .set_owner(id, owner)
            .ok_or(EditError::MissingParameter)?;
        Ok(Edit::SetParameterOwner {
            id,
            owner: previous,
        })
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
        self.check_reads_no_measurement(self.parameter_name(id).unwrap_or_default(), &expression)?;
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
        self.check_measurement_order(&feature, index)?;
        check_appearance(&feature.appearance)?;
        self.check_parameters_exist(feature.appearance.parameters())?;
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

    fn set_feature_group(
        &mut self,
        id: FeatureId,
        group: Option<String>,
    ) -> Result<Edit, EditError> {
        let group = group.as_deref().and_then(group_name);
        if let Some(name) = &group {
            let length = name.chars().count();
            if length > MAX_GROUP_NAME_CHARS {
                return Err(EditError::GroupNameTooLong(length));
            }
        }
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.group, group);
        Ok(Edit::SetFeatureGroup {
            id,
            group: previous,
        })
    }

    fn set_body_appearance(
        &mut self,
        id: FeatureId,
        mut appearance: BodyAppearance,
    ) -> Result<Edit, EditError> {
        appearance.material = appearance.material.as_deref().and_then(material_name);
        appearance.opacity = appearance
            .opacity
            .filter(|opacity| *opacity < OPAQUE_PERCENT);
        appearance.name = appearance
            .name
            .as_deref()
            .map(|name| name.split(['\r', '\n']).collect::<Vec<_>>().join(" "))
            .as_deref()
            .and_then(material_name);
        let existing = self.feature(id).ok_or(EditError::MissingFeature)?;
        if !existing.makes_body() {
            return Err(EditError::NotABody(existing.name.clone()));
        }
        check_appearance(&appearance)?;
        self.check_parameters_exist(appearance.parameters())?;
        let feature = self.feature_mut(id)?;
        let previous = std::mem::replace(&mut feature.appearance, appearance);
        Ok(Edit::SetBodyAppearance {
            id,
            appearance: previous,
        })
    }

    fn set_rollback_bar(&mut self, bar: RollbackBar) -> Result<Edit, EditError> {
        if let RollbackBar::Before(feature) = bar {
            self.feature_position(feature)?;
        }
        let previous = std::mem::replace(&mut self.rollback, bar);
        Ok(Edit::SetRollbackBar { bar: previous })
    }

    fn set_model_properties(&mut self, properties: ModelProperties) -> Result<Edit, EditError> {
        let properties = properties.normalized();
        if let Some((property, length)) = properties.too_long() {
            return Err(EditError::PropertyTooLong { property, length });
        }
        let previous = std::mem::replace(&mut self.properties, Arc::new(properties));
        Ok(Edit::SetModelProperties {
            properties: Box::new(Arc::unwrap_or_clone(previous)),
        })
    }

    fn set_saved_views(&mut self, views: SavedViews) -> Result<Edit, EditError> {
        let views = views.normalized();
        if views.named.len() > MAX_SAVED_VIEWS {
            return Err(EditError::TooManyViews);
        }
        for (index, named) in views.named.iter().enumerate() {
            let length = named.name.chars().count();
            if length == 0 {
                return Err(EditError::ViewNameEmpty);
            }
            if length > MAX_VIEW_NAME_CHARS {
                return Err(EditError::ViewNameTooLong { length });
            }
            let repeated = views
                .named
                .iter()
                .take(index)
                .any(|other| other.name.to_lowercase() == named.name.to_lowercase());
            if repeated {
                return Err(EditError::ViewNameTaken(named.name.clone()));
            }
        }
        if let Some(name) = views.unusable() {
            return Err(EditError::ViewNotUsable(name.to_owned()));
        }
        let previous = std::mem::replace(&mut self.views, Arc::new(views));
        Ok(Edit::SetSavedViews {
            views: Box::new(Arc::unwrap_or_clone(previous)),
        })
    }

    fn set_selection_sets(&mut self, sets: SelectionSets) -> Result<Edit, EditError> {
        if sets.sets.len() > MAX_SELECTION_SETS {
            return Err(EditError::TooManySets);
        }
        if let Some(set) = sets
            .sets
            .iter()
            .find(|set| set.members.len() > MAX_SET_MEMBERS)
        {
            return Err(EditError::SetTooLarge {
                name: set.name.clone(),
                members: set.members.len(),
            });
        }
        let sets = sets.normalized();
        for (index, set) in sets.sets.iter().enumerate() {
            let length = set.name.chars().count();
            if length == 0 {
                return Err(EditError::SetNameEmpty);
            }
            if length > MAX_SET_NAME_CHARS {
                return Err(EditError::SetNameTooLong { length });
            }
            let repeated = sets
                .sets
                .iter()
                .take(index)
                .any(|other| other.name.to_lowercase() == set.name.to_lowercase());
            if repeated {
                return Err(EditError::SetNameTaken(set.name.clone()));
            }
            if set.members.is_empty() {
                return Err(EditError::SetEmpty(set.name.clone()));
            }
        }
        for body in sets.bodies() {
            check_storable(body.raw())?;
        }
        let beyond = sets
            .bodies()
            .map(|body| body.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        self.next_feature_id = self.next_feature_id.max(beyond);
        let previous = std::mem::replace(&mut self.selection_sets, Arc::new(sets));
        Ok(Edit::SetSelectionSets {
            sets: Box::new(Arc::unwrap_or_clone(previous)),
        })
    }

    fn set_configurations(&mut self, configurations: Configurations) -> Result<Edit, EditError> {
        let mut configurations = configurations.checked()?;
        let live = self.live_settings_of(&configurations.values);
        if let Some(active) = configurations.active
            && let Some(row) = configurations.rows.iter_mut().find(|row| row.id == active)
        {
            row.settings.extend(live);
        }
        for id in configurations.parameters_named() {
            check_storable(id.raw())?;
        }
        for id in configurations.features_named() {
            check_storable(id.raw())?;
        }
        check_storable(configurations.next_id)?;
        let parameters_beyond = configurations
            .parameters_named()
            .map(|id| id.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        let features_beyond = configurations
            .features_named()
            .map(|id| id.raw().saturating_add(1))
            .max()
            .unwrap_or(0);
        self.next_parameter_id = self.next_parameter_id.max(parameters_beyond);
        self.next_feature_id = self.next_feature_id.max(features_beyond);
        configurations.next_id = configurations.next_id.max(self.configurations.next_id);
        let previous = std::mem::replace(&mut self.configurations, Arc::new(configurations));
        Ok(Edit::SetConfigurations {
            configurations: Box::new(Arc::unwrap_or_clone(previous)),
        })
    }

    fn set_active_configuration(
        &mut self,
        active: Option<ConfigurationId>,
    ) -> Result<Edit, EditError> {
        if let Some(id) = active {
            let row = self
                .configurations
                .row(id)
                .ok_or(EditError::MissingConfiguration)?;
            if self.out_of_step(row).is_some() {
                return Err(EditError::ConfigurationOutOfStep(row.name.clone()));
            }
        }
        let previous = if self.configurations.active == active {
            active
        } else {
            std::mem::replace(&mut Arc::make_mut(&mut self.configurations).active, active)
        };
        Ok(Edit::SetActiveConfiguration { active: previous })
    }

    fn follow_active_configuration(&mut self, value: FollowedValue) {
        let Some(active) = self.configurations.active else {
            return;
        };
        let columns: Vec<ConfiguredValue> = self
            .configurations
            .values
            .iter()
            .copied()
            .filter(|column| value.covers(*column))
            .collect();
        for column in columns {
            let Some(live) = self.live_setting(column) else {
                continue;
            };
            let current = self
                .configurations
                .row(active)
                .and_then(|row| row.setting(column));
            if current == Some(&live) {
                continue;
            }
            if let Some(row) = Arc::make_mut(&mut self.configurations)
                .rows
                .iter_mut()
                .find(|row| row.id == active)
            {
                row.settings.insert(column, live);
            }
        }
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
        let measured = Measured::of(self);
        let mut uses = measured.dependencies_of(&moving.kind);
        uses.remove(&id);
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
            .find(|other| measured.dependencies_of(&other.kind).contains(&id))
        {
            return Err(EditError::BelowDependent {
                name,
                other: user.name.clone(),
            });
        }
        let consumed = moving.kind.consumed_bodies();
        let used: Vec<FeatureId> = moving
            .kind
            .bodies_used()
            .into_iter()
            .filter(|body| !consumed.contains(body))
            .collect();
        let consuming_one_of = |other: &Feature, bodies: &[FeatureId]| {
            other
                .kind
                .consumed_bodies()
                .into_iter()
                .find(|body| bodies.contains(body))
        };
        if let Some((consumer, body)) = above
            .iter()
            .find_map(|other| Some((other, consuming_one_of(other, &used)?)))
        {
            return Err(EditError::BelowConsumer {
                name,
                other: consumer.name.clone(),
                body: self.feature_name(body),
            });
        }
        if let Some((user, body)) = below.iter().find_map(|other| {
            let body = other
                .kind
                .bodies_used()
                .into_iter()
                .find(|body| consumed.contains(body))?;
            Some((other, body))
        }) {
            return Err(EditError::AboveConsumedUse {
                name,
                other: user.name.clone(),
                body: self.feature_name(body),
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

    fn check_reads_no_measurement(
        &self,
        name: &str,
        expression: &Expression,
    ) -> Result<(), EditError> {
        let used = expression.parameters();
        if used.is_empty() {
            return Ok(());
        }
        let measured = Measured::of(self);
        let Some((parameter, measurement)) = used
            .into_iter()
            .find_map(|parameter| Some((parameter, measured.measurement(parameter)?)))
        else {
            return Ok(());
        };
        Err(EditError::ParameterReadsMeasurement {
            name: name.to_owned(),
            parameter: self
                .parameter_name(parameter)
                .unwrap_or_default()
                .to_owned(),
            measurement: self.feature_name(measurement),
        })
    }

    fn check_measurement_order(&self, feature: &Feature, index: usize) -> Result<(), EditError> {
        let used = feature.kind.parameters();
        if used.is_empty() && feature.kind.measurement().is_none() {
            return Ok(());
        }
        let measured = Measured::of(self);
        let id = feature.id();
        for parameter in used {
            let Some(measurement) = measured.measurement(parameter).filter(|other| *other != id)
            else {
                continue;
            };
            if self
                .feature_index(measurement)
                .is_some_and(|position| position >= index)
            {
                return Err(EditError::MeasurementBelowUser {
                    feature: feature.name.clone(),
                    parameter: self
                        .parameter_name(parameter)
                        .unwrap_or_default()
                        .to_owned(),
                    measurement: self.feature_name(measurement),
                });
            }
        }
        let Some(parameter) = feature
            .kind
            .measurement()
            .and_then(|measurement| measurement.parameter)
        else {
            return Ok(());
        };
        let parameter_name = self
            .parameter_name(parameter)
            .unwrap_or_default()
            .to_owned();
        if let Some(other) = measured.measurement(parameter).filter(|other| *other != id) {
            return Err(EditError::ParameterMeasuredTwice {
                parameter: parameter_name,
                measurement: self.feature_name(other),
            });
        }
        if let Some(reader) = self
            .parameters()
            .iter()
            .find(|other| other.expression.uses(parameter))
        {
            return Err(EditError::ParameterReadsMeasurement {
                name: reader.name.clone(),
                parameter: parameter_name,
                measurement: feature.name.clone(),
            });
        }
        let at_or_above = self
            .features()
            .enumerate()
            .filter(|(position, other)| *position < index && other.id() != id)
            .map(|(_, other)| other);
        if let Some(user) = at_or_above
            .into_iter()
            .find(|other| other.kind.uses_parameter(parameter))
        {
            return Err(EditError::MeasurementBelowUser {
                feature: user.name.clone(),
                parameter: parameter_name,
                measurement: feature.name.clone(),
            });
        }
        Ok(())
    }
}
