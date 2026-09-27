use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    sync::Arc,
};

use caditor_expression::{Expression, ParameterId, ParseError};
use caditor_sketch::Sketch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(u64);

impl FeatureId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) const fn raw(self) -> u64 {
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
    pub(crate) fn new(id: ParameterId, name: String, expression: Expression) -> Self {
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
    Sketch(Sketch),
}

impl FeatureKind {
    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        match self {
            Self::Sketch(sketch) => sketch.parameters(),
        }
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        match self {
            Self::Sketch(sketch) => sketch.uses_parameter(parameter),
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Sketch(_) => BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    id: FeatureId,
    pub name: String,
    pub kind: FeatureKind,
}

impl Feature {
    pub(crate) fn new(id: FeatureId, name: String, kind: FeatureKind) -> Self {
        Self { id, name, kind }
    }

    pub fn id(&self) -> FeatureId {
        self.id
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
