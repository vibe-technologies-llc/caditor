use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    sync::Arc,
};

use caditor_expression::{Expression, ParameterId, ParseError};
use caditor_sketch::Sketch;

use crate::{
    attachment::{SketchAttachment, SketchFeature},
    blend::Blend,
    datum::Datum,
    edit::{Edit, Transaction},
    shell::Shell,
    solid::{BodyOperation, SolidFeature},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(u64);

impl FeatureId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for FeatureId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    id: ParameterId,
    pub name: String,
    pub expression: Expression,
}

impl Parameter {
    pub fn new(id: ParameterId, name: String, expression: Expression) -> Self {
        Self {
            id,
            name,
            expression,
        }
    }

    pub fn id(&self) -> ParameterId {
        self.id
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureKind {
    Sketch(SketchFeature),
    Solid(SolidFeature),
    Blend(Blend),
    Shell(Shell),
    Datum(Datum),
}

impl From<Sketch> for FeatureKind {
    fn from(sketch: Sketch) -> Self {
        Self::Sketch(SketchFeature::from(sketch))
    }
}

impl FeatureKind {
    pub fn sketch(&self) -> Option<&Sketch> {
        match self {
            Self::Sketch(sketch) => Some(&sketch.sketch),
            Self::Solid(_) | Self::Blend(_) | Self::Shell(_) | Self::Datum(_) => None,
        }
    }

    pub fn sketch_mut(&mut self) -> Option<&mut Sketch> {
        match self {
            Self::Sketch(sketch) => Some(&mut sketch.sketch),
            Self::Solid(_) | Self::Blend(_) | Self::Shell(_) | Self::Datum(_) => None,
        }
    }

    pub fn attachment(&self) -> Option<&SketchAttachment> {
        match self {
            Self::Sketch(sketch) => sketch.attachment.as_ref(),
            Self::Solid(_) | Self::Blend(_) | Self::Shell(_) | Self::Datum(_) => None,
        }
    }

    pub fn body_input(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch(sketch) => sketch.attachment.as_ref().and_then(SketchAttachment::body),
            Self::Solid(solid) => solid.operation().target(),
            Self::Blend(blend) => Some(blend.body),
            Self::Shell(shell) => Some(shell.body),
            Self::Datum(_) => None,
        }
    }

    pub fn bodies_used(&self) -> BTreeSet<FeatureId> {
        let mut used: BTreeSet<FeatureId> = self.body_input().into_iter().collect();
        match self {
            Self::Solid(solid) => used.extend(solid.axis_body()),
            Self::Datum(datum) => used.extend(datum.bodies()),
            Self::Sketch(_) | Self::Blend(_) | Self::Shell(_) => {}
        }
        used
    }

    pub fn planes_used(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Sketch(sketch) => sketch
                .attachment
                .as_ref()
                .and_then(SketchAttachment::datum)
                .into_iter()
                .collect(),
            Self::Datum(datum) => datum.plane_datums(),
            Self::Solid(_) | Self::Blend(_) | Self::Shell(_) => BTreeSet::new(),
        }
    }

    pub fn axes_used(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Solid(solid) => solid.axis_datum().into_iter().collect(),
            Self::Datum(datum) => datum.axis_datums(),
            Self::Sketch(_) | Self::Blend(_) | Self::Shell(_) => BTreeSet::new(),
        }
    }

    pub fn modifies_body(&self) -> bool {
        matches!(self, Self::Blend(_) | Self::Shell(_))
    }

    pub fn solid(&self) -> Option<&SolidFeature> {
        match self {
            Self::Solid(solid) => Some(solid),
            Self::Sketch(_) | Self::Blend(_) | Self::Shell(_) | Self::Datum(_) => None,
        }
    }

    pub fn blend(&self) -> Option<&Blend> {
        match self {
            Self::Blend(blend) => Some(blend),
            Self::Sketch(_) | Self::Solid(_) | Self::Shell(_) | Self::Datum(_) => None,
        }
    }

    pub fn shell(&self) -> Option<&Shell> {
        match self {
            Self::Shell(shell) => Some(shell),
            Self::Sketch(_) | Self::Solid(_) | Self::Blend(_) | Self::Datum(_) => None,
        }
    }

    pub fn datum(&self) -> Option<&Datum> {
        match self {
            Self::Datum(datum) => Some(datum),
            Self::Sketch(_) | Self::Solid(_) | Self::Blend(_) | Self::Shell(_) => None,
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        match self {
            Self::Sketch(sketch) => sketch.sketch.parameters(),
            Self::Solid(solid) => solid.parameters(),
            Self::Blend(blend) => blend.parameters(),
            Self::Shell(shell) => shell.parameters(),
            Self::Datum(datum) => datum.parameters(),
        }
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        match self {
            Self::Sketch(sketch) => sketch.sketch.uses_parameter(parameter),
            Self::Solid(solid) => solid.uses_parameter(parameter),
            Self::Blend(blend) => blend.uses_parameter(parameter),
            Self::Shell(shell) => shell.uses_parameter(parameter),
            Self::Datum(datum) => datum.uses_parameter(parameter),
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = match self {
            Self::Sketch(_) => BTreeSet::new(),
            Self::Solid(solid) => solid.features(),
            Self::Blend(blend) => blend.features(),
            Self::Shell(shell) => shell.features(),
            Self::Datum(datum) => datum.features(),
        };
        used.extend(self.bodies_used());
        used.extend(self.planes_used());
        used.extend(self.axes_used());
        used
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    id: FeatureId,
    pub name: String,
    pub kind: FeatureKind,
}

impl Feature {
    pub fn new(id: FeatureId, name: String, kind: FeatureKind) -> Self {
        Self { id, name, kind }
    }

    pub fn id(&self) -> FeatureId {
        self.id
    }

    pub fn body(&self) -> Option<FeatureId> {
        match &self.kind {
            FeatureKind::Solid(solid) => Some(solid.operation().target().unwrap_or(self.id)),
            FeatureKind::Blend(blend) => Some(blend.body),
            FeatureKind::Shell(shell) => Some(shell.body),
            FeatureKind::Sketch(_) | FeatureKind::Datum(_) => None,
        }
    }

    pub fn makes_body(&self) -> bool {
        self.kind
            .solid()
            .is_some_and(|solid| solid.operation() == BodyOperation::NewBody)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) features: Vec<Arc<Feature>>,
    pub(crate) next_parameter_id: u64,
    pub(crate) next_feature_id: u64,
}

impl Document {
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    pub fn parameter(&self, id: ParameterId) -> Option<&Parameter> {
        self.parameters.iter().find(|parameter| parameter.id == id)
    }

    pub fn parameter_named(&self, name: &str) -> Option<&Parameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
    }

    pub fn parameter_name(&self, id: ParameterId) -> Option<&str> {
        self.parameter(id).map(|parameter| parameter.name.as_str())
    }

    pub fn features(&self) -> impl ExactSizeIterator<Item = &Feature> + DoubleEndedIterator {
        self.features.iter().map(Arc::as_ref)
    }

    pub(crate) fn feature_handles(&self) -> &[Arc<Feature>] {
        &self.features
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features
            .iter()
            .find(|feature| feature.id == id)
            .map(Arc::as_ref)
    }

    pub fn feature_index(&self, id: FeatureId) -> Option<usize> {
        self.features.iter().position(|feature| feature.id == id)
    }

    pub fn next_parameter_id(&self) -> u64 {
        self.next_parameter_id
    }

    pub fn next_feature_id(&self) -> u64 {
        self.next_feature_id
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.parameters == other.parameters && self.features == other.features
    }

    pub fn transaction_to(&self, target: &Self, label: impl Into<String>) -> Transaction {
        let removals = self
            .features
            .iter()
            .rev()
            .map(|feature| Edit::RemoveFeature { id: feature.id })
            .chain(
                self.parameters
                    .iter()
                    .map(|parameter| Edit::SetParameterExpression {
                        id: parameter.id,
                        expression: Expression::Number(0.0),
                    }),
            )
            .chain(
                self.parameters
                    .iter()
                    .map(|parameter| Edit::RemoveParameter { id: parameter.id }),
            );
        let insertions = target
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| Edit::InsertParameter {
                index,
                parameter: Parameter::new(
                    parameter.id,
                    parameter.name.clone(),
                    Expression::Number(0.0),
                ),
            })
            .chain(
                target
                    .parameters
                    .iter()
                    .map(|parameter| Edit::SetParameterExpression {
                        id: parameter.id,
                        expression: parameter.expression.clone(),
                    }),
            )
            .chain(target.features.iter().enumerate().map(|(index, feature)| {
                Edit::InsertFeature {
                    index,
                    feature: Arc::clone(feature),
                }
            }));
        Transaction::new(label, removals.chain(insertions).collect())
    }

    pub fn reserve_ids_below(&mut self, next_parameter_id: u64, next_feature_id: u64) {
        self.next_parameter_id = self.next_parameter_id.max(next_parameter_id);
        self.next_feature_id = self.next_feature_id.max(next_feature_id);
    }

    pub fn parse(&self, text: &str) -> Result<Expression, ParseError> {
        Expression::parse(text, &|name| self.parameter_named(name).map(Parameter::id))
    }

    pub fn expression_text(&self, expression: &Expression) -> String {
        expression.to_text(&|id| self.parameter_name(id))
    }

    pub fn parameter_users(&self, parameter: ParameterId) -> Vec<String> {
        let parameters = self
            .parameters
            .iter()
            .filter(|other| other.expression.uses(parameter))
            .map(|other| other.name.clone());
        let features = self
            .features
            .iter()
            .filter(|feature| feature.kind.uses_parameter(parameter))
            .map(|feature| feature.name.clone());
        parameters.chain(features).collect()
    }

    pub fn feature_dependents(&self, feature: FeatureId) -> Vec<FeatureId> {
        self.features
            .iter()
            .filter(|other| other.kind.features().contains(&feature))
            .map(|other| other.id)
            .collect()
    }

    pub(crate) fn parameter_dependencies(&self) -> BTreeMap<ParameterId, BTreeSet<ParameterId>> {
        self.parameters
            .iter()
            .map(|parameter| {
                let existing = parameter
                    .expression
                    .parameters()
                    .into_iter()
                    .filter(|used| self.parameter(*used).is_some())
                    .collect();
                (parameter.id, existing)
            })
            .collect()
    }

    pub fn cycle_through(
        &self,
        target: ParameterId,
        expression: &Expression,
    ) -> Option<Vec<ParameterId>> {
        path_to(
            target,
            expression.parameters(),
            &self.parameter_dependencies(),
        )
    }
}

pub(crate) fn path_to(
    target: ParameterId,
    first_steps: impl IntoIterator<Item = ParameterId>,
    dependencies: &BTreeMap<ParameterId, BTreeSet<ParameterId>>,
) -> Option<Vec<ParameterId>> {
    let mut parents = BTreeMap::new();
    let mut queue = VecDeque::new();
    for step in first_steps {
        if parents.insert(step, target).is_none() {
            queue.push_back(step);
        }
    }
    while let Some(current) = queue.pop_front() {
        if current == target {
            return Some(path_back_to(target, &parents));
        }
        for next in dependencies.get(&current).into_iter().flatten() {
            if !parents.contains_key(next) {
                parents.insert(*next, current);
                queue.push_back(*next);
            }
        }
    }
    None
}

fn path_back_to(
    target: ParameterId,
    parents: &BTreeMap<ParameterId, ParameterId>,
) -> Vec<ParameterId> {
    let mut path = vec![target];
    let mut current = parents.get(&target).copied();
    while let Some(step) = current {
        path.push(step);
        if step == target || path.len() > parents.len() + 1 {
            break;
        }
        current = parents.get(&step).copied();
    }
    path.reverse();
    path
}

pub(crate) fn list_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
