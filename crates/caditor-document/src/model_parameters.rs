use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_sketch::ConstraintId;

use crate::{
    document::{Document, FeatureId, Parameter},
    edit::{Edit, EditError, Transaction},
    inlining::expressions_mut,
};

pub const MAX_VALUE_LABEL_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterOwner {
    Feature {
        feature: FeatureId,
        value: String,
    },
    Dimension {
        sketch: FeatureId,
        constraint: ConstraintId,
    },
}

impl ParameterOwner {
    pub fn feature(&self) -> FeatureId {
        match self {
            Self::Feature { feature, .. } => *feature,
            Self::Dimension { sketch, .. } => *sketch,
        }
    }

    pub(crate) fn heap_size(&self) -> usize {
        match self {
            Self::Feature { value, .. } => value.capacity(),
            Self::Dimension { .. } => 0,
        }
    }

    pub(crate) fn checked(self) -> Result<Self, EditError> {
        match self {
            Self::Feature { feature, value } => {
                let value = value_label(&value);
                let length = value.chars().count();
                if length > MAX_VALUE_LABEL_CHARS {
                    return Err(EditError::ValueLabelTooLong(length));
                }
                Ok(Self::Feature { feature, value })
            }
            dimension @ Self::Dimension { .. } => Ok(dimension),
        }
    }
}

pub fn value_label(text: &str) -> String {
    let one_line = text.split(['\r', '\n']).collect::<Vec<_>>().join(" ");
    one_line.trim().to_owned()
}

impl Parameter {
    pub fn is_model_parameter(&self) -> bool {
        self.owner.is_some()
    }
}

impl Document {
    pub fn owned_parameter(
        &self,
        owner: &ParameterOwner,
        expression: &Expression,
    ) -> Option<&Parameter> {
        let Expression::Parameter(id) = expression else {
            return None;
        };
        self.parameter(*id)
            .filter(|parameter| parameter.owner.as_ref() == Some(owner))
    }

    pub fn owner_text(&self, owner: &ParameterOwner) -> Option<String> {
        match owner {
            ParameterOwner::Feature { feature, value } => {
                let feature = self.feature(*feature)?;
                Some(if value.is_empty() {
                    feature.name.clone()
                } else {
                    format!("{} · {value}", feature.name)
                })
            }
            ParameterOwner::Dimension { sketch, constraint } => {
                let feature = self.feature(*sketch)?;
                let held = feature.kind.sketch()?.constraint(*constraint)?;
                Some(format!("{} · {}", feature.name, held.kind_name()))
            }
        }
    }

    pub fn releasing(
        &self,
        transaction: Transaction,
        parameters: impl IntoIterator<Item = ParameterId>,
    ) -> Transaction {
        let mut pending: BTreeSet<ParameterId> = parameters
            .into_iter()
            .filter(|id| {
                self.parameter(*id)
                    .is_some_and(Parameter::is_model_parameter)
            })
            .collect();
        if pending.is_empty() {
            return transaction;
        }
        let mut after = self.clone();
        if after.apply(transaction.clone()).is_err() {
            return transaction;
        }
        let (label, mut edits) = transaction.into_parts();
        let mut refused = BTreeSet::new();
        loop {
            let unused: Vec<ParameterId> = pending
                .iter()
                .filter(|id| !refused.contains(*id) && after.parameter_user_ids(**id).is_empty())
                .copied()
                .collect();
            if unused.is_empty() {
                break;
            }
            for id in unused {
                let removal = Edit::RemoveParameter { id };
                if after
                    .apply(Transaction::single(label.clone(), removal.clone()))
                    .is_ok()
                {
                    pending.remove(&id);
                    edits.push(removal);
                } else {
                    refused.insert(id);
                }
            }
        }
        edits.extend(
            pending
                .into_iter()
                .map(|id| Edit::SetParameterOwner { id, owner: None }),
        );
        Transaction::new(label, edits)
    }

    pub fn carrying_owners<'a>(
        &self,
        transaction: Transaction,
        feature: FeatureId,
        fields: impl IntoIterator<Item = (&'a str, &'a Expression)>,
    ) -> Transaction {
        let owned: Vec<(ParameterId, &str)> =
            self.parameters()
                .iter()
                .filter_map(|parameter| match &parameter.owner {
                    Some(ParameterOwner::Feature {
                        feature: owner,
                        value,
                    }) if *owner == feature => Some((parameter.id(), value.as_str())),
                    Some(ParameterOwner::Feature { .. } | ParameterOwner::Dimension { .. })
                    | None => None,
                })
                .collect();
        if owned.is_empty() {
            return transaction;
        }
        let held: Vec<(&str, ParameterId)> = fields
            .into_iter()
            .filter_map(|(caption, expression)| match expression {
                Expression::Parameter(id) => Some((caption, *id)),
                _ => None,
            })
            .collect();

        let (label, mut edits) = transaction.into_parts();
        let mut dropped = Vec::new();
        for (id, caption) in owned {
            let captions: Vec<&str> = held
                .iter()
                .filter(|(_, holding)| *holding == id)
                .map(|(caption, _)| *caption)
                .collect();
            if captions.contains(&caption) {
                continue;
            }
            match captions.first() {
                Some(moved) => edits.push(Edit::SetParameterOwner {
                    id,
                    owner: Some(ParameterOwner::Feature {
                        feature,
                        value: (*moved).to_owned(),
                    }),
                }),
                None => dropped.push(id),
            }
        }
        let carried = Transaction::new(label, edits);
        if dropped.is_empty() {
            return carried;
        }

        let mut after = self.clone();
        if after.apply(carried.clone()).is_err() {
            return carried;
        }
        let unused: Vec<ParameterId> = dropped
            .into_iter()
            .filter(|id| {
                after
                    .feature(feature)
                    .is_none_or(|kept| !kept.uses_parameter(*id))
            })
            .collect();
        self.releasing(carried, unused)
    }

    pub(crate) fn owned_by(&self, features: &BTreeSet<FeatureId>) -> Vec<ParameterId> {
        self.parameters()
            .iter()
            .filter(|parameter| {
                parameter
                    .owner
                    .as_ref()
                    .is_some_and(|owner| features.contains(&owner.feature()))
            })
            .map(Parameter::id)
            .collect()
    }

    pub(crate) fn owned_by_dimensions(
        &self,
        sketch: FeatureId,
        constraints: &BTreeSet<ConstraintId>,
    ) -> Vec<ParameterId> {
        self.parameters()
            .iter()
            .filter(|parameter| match &parameter.owner {
                Some(ParameterOwner::Dimension {
                    sketch: owner,
                    constraint,
                }) => *owner == sketch && constraints.contains(constraint),
                Some(ParameterOwner::Feature { .. }) | None => false,
            })
            .map(Parameter::id)
            .collect()
    }
}

impl Transaction {
    pub fn substituting(self, from: &Expression, to: &Expression) -> (Self, usize) {
        let mut count = 0;
        let mut replace = |expression: &mut Expression| {
            if expression == from {
                *expression = to.clone();
                count += 1;
            }
        };
        let (label, mut edits) = self.into_parts();
        for edit in &mut edits {
            match edit {
                Edit::SetFeatureKind { kind, .. } => {
                    for expression in expressions_mut(kind) {
                        replace(expression);
                    }
                }
                Edit::SetDimension { value, .. } => replace(value),
                Edit::SetParameterExpression { expression, .. } => replace(expression),
                _ => {}
            }
        }
        (Self::new(label, edits), count)
    }
}
