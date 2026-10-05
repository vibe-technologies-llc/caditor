use std::sync::Arc;

use caditor_kernel::Solid;

use crate::{
    document::Feature,
    recompute::{Failure, FeatureError, FeatureResult, FixTarget},
    solid::SolidResult,
};

#[derive(Debug, Clone)]
pub struct Import {
    pub source: String,
    pub solid: Arc<Solid>,
    pub step: Arc<str>,
}

impl Import {
    pub fn new(source: impl Into<String>, solid: Solid, step: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            solid: Arc::new(solid),
            step: step.into(),
        }
    }
}

impl PartialEq for Import {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && (Arc::ptr_eq(&self.step, &other.step) || self.step == other.step)
    }
}

pub(crate) fn evaluate(feature: &Feature, definition: &Import) -> Result<FeatureResult, Failure> {
    if definition.solid.shells().next().is_none() {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!(
                "The shape imported from “{}” could not be read back from the model file.",
                definition.source
            ),
            remedy: format!(
                "Delete {} and import “{}” again.",
                feature.name, definition.source
            ),
            fix: Some(FixTarget::Feature(feature.id())),
            constraints: Vec::new(),
            place: None,
        })));
    }
    let solid = (*definition.solid).clone().imported(feature.id().raw());
    Ok(FeatureResult::Solid(SolidResult::new(feature.id(), solid)))
}
