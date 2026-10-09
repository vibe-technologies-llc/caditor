use std::{path::PathBuf, sync::Arc};

use caditor_expression::{Dimension, format_number};
use caditor_geometry::{Point3, Similarity};
use caditor_kernel::Solid;

use crate::{
    document::Feature,
    movement::{BodyPlacement, Context, placement_transform},
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    scaling::{MAX_SCALE_FACTOR, MIN_SCALE_FACTOR},
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
    let failure = |reason: String, remedy: &str| {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy: remedy.to_owned(),
            fix: Some(FixTarget::Feature(feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    };
    let solid = if definition.placement.is_unscaled() {
        solid
    } else {
        let factor = Context { feature, inputs }.value(
            &definition.placement.scale,
            Dimension::NONE,
            "scale",
        )?;
        if !(MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&factor) {
            return Err(failure(
                format!(
                    "The scale of {} must be from {} to {}, and {} is not.",
                    feature.name,
                    format_number(MIN_SCALE_FACTOR),
                    format_number(MAX_SCALE_FACTOR),
                    format_number(factor)
                ),
                "Enter a scale above zero, such as 25.4 for a part drawn in inches.",
            ));
        }
        let scaling = Similarity::scaling(Point3::ZERO, factor).ok_or_else(|| {
            failure(
                format!("The body of {} could not be scaled.", feature.name),
                "Enter a scale nearer 1.",
            )
        })?;
        solid.mapped(&scaling).map_err(|_| {
            failure(
                format!(
                    "Scaling the body of {} by {} would make it too large or too small to model.",
                    feature.name,
                    format_number(factor)
                ),
                "Enter a scale nearer 1.",
            )
        })?
    };
    let placement = placement_transform(feature, inputs, &definition.placement)?;
    let placed = solid.transformed(&placement).map_err(|_| {
        failure(
            format!(
                "The placement would take the body of {} too far from the origin.",
                feature.name
            ),
            "Enter smaller distances.",
        )
    })?;
    Ok(FeatureResult::Solid(SolidResult::new(feature.id(), placed)))
}
