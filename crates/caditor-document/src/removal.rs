use std::collections::BTreeSet;

use crate::{
    document::{Feature, FeatureId},
    recompute::{Failure, FeatureResult, Inputs},
    solid::SolidResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remove {
    pub body: FeatureId,
}

impl Remove {
    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
    }
}

pub(crate) fn evaluate(
    _feature: &Feature,
    definition: &Remove,
    inputs: &Inputs<'_>,
) -> Result<FeatureResult, Failure> {
    let solid = inputs
        .body(definition.body)
        .ok_or_else(|| inputs.missing_body(definition.body))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        solid.clone(),
    )))
}
