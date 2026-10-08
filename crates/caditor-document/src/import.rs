use std::{path::PathBuf, sync::Arc};

use caditor_kernel::Solid;

use crate::{
    document::Feature,
    movement::{BodyPlacement, placement_transform},
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
};

#[derive(Debug, Clone)]
pub struct Import {
    pub source: String,
    pub path: Option<PathBuf>,
    pub solid: Arc<Solid>,
    pub step: Arc<str>,
    pub placement: BodyPlacement,
}

impl Import {
    pub fn new(source: impl Into<String>, solid: Solid, step: impl Into<Arc<str>>) -> Self {
        Self::shared(source, Arc::new(solid), step)
    }

    pub fn shared(source: impl Into<String>, solid: Arc<Solid>, step: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            path: None,
            solid,
            step: step.into(),
            placement: BodyPlacement::default(),
        }
    }

    pub fn from_file(mut self, path: PathBuf) -> Self {
        self.path = path.to_str().is_some().then_some(path);
        self
    }

    pub fn placed(mut self, placement: BodyPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn path_len(&self) -> usize {
        self.path.as_ref().map_or(0, |path| path.as_os_str().len())
    }

    pub fn same_shape(&self, other: &Self) -> bool {
        self.source == other.source
            && self.path == other.path
            && (Arc::ptr_eq(&self.step, &other.step) || self.step == other.step)
    }
}

impl PartialEq for Import {
    fn eq(&self, other: &Self) -> bool {
        self.same_shape(other) && self.placement == other.placement
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Import,
    inputs: &Inputs<'_>,
) -> Result<FeatureResult, Failure> {
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
    if definition.placement.is_at_origin() {
        return Ok(FeatureResult::Solid(SolidResult::new(feature.id(), solid)));
    }
    let placement = placement_transform(feature, inputs, &definition.placement)?;
    let placed = solid.transformed(&placement).map_err(|_| {
        Failure::Error(Box::new(FeatureError {
            reason: format!(
                "The placement would take the body of {} too far from the origin.",
                feature.name
            ),
            remedy: "Enter smaller distances.".to_owned(),
            fix: Some(FixTarget::Feature(feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    })?;
    Ok(FeatureResult::Solid(SolidResult::new(feature.id(), placed)))
}
