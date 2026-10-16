use std::{path::PathBuf, sync::Arc};

use caditor_expression::{Dimension, format_number};
use caditor_geometry::{Point3, Similarity};
use caditor_kernel::Solid;

use crate::{
    document::Feature,
    failure::Cause,
    movement::{BodyPlacement, Context, placement_transform},
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    scaling::{MAX_SCALE_FACTOR, MIN_SCALE_FACTOR},
    solid::SolidResult,
    step_text::StepText,
};

#[derive(Debug, Clone)]
pub struct Import {
    pub source: String,
    pub path: Option<PathBuf>,
    pub solid: Arc<Solid>,
    pub step: StepText,
    pub placement: BodyPlacement,
}

impl Import {
    pub fn new(source: impl Into<String>, solid: Solid, step: impl Into<StepText>) -> Self {
        Self::shared(source, Arc::new(solid), step)
    }

    pub fn shared(source: impl Into<String>, solid: Arc<Solid>, step: impl Into<StepText>) -> Self {
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
        self.source == other.source && self.path == other.path && self.step == other.step
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
    let failure = |cause: ImportCause| {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(feature.id())))
    };
    if definition.solid.shells().next().is_none() {
        return Err(failure(ImportCause::Unreadable {
            feature: feature.name.clone(),
            source: definition.source.clone(),
        }));
    }
    let solid = (*definition.solid).clone().imported(feature.id().raw());
    if definition.placement.is_at_origin() {
        return Ok(FeatureResult::Solid(SolidResult::new(feature.id(), solid)));
    }
    let solid = if definition.placement.is_unscaled() {
        solid
    } else {
        let factor = Context { feature, inputs }.value(
            &definition.placement.scale,
            Dimension::NONE,
            "scale",
        )?;
        if !(MIN_SCALE_FACTOR..=MAX_SCALE_FACTOR).contains(&factor) {
            return Err(failure(ImportCause::ScaleRange {
                feature: feature.name.clone(),
                factor,
            }));
        }
        let scaling = Similarity::scaling(Point3::ZERO, factor).ok_or_else(|| {
            failure(ImportCause::Unscalable {
                feature: feature.name.clone(),
            })
        })?;
        solid.mapped(&scaling).map_err(|_| {
            failure(ImportCause::ScaledTooMuch {
                feature: feature.name.clone(),
                factor,
            })
        })?
    };
    let placement = placement_transform(feature, inputs, &definition.placement)?;
    let placed = solid.transformed(&placement).map_err(|_| {
        failure(ImportCause::TooFar {
            feature: feature.name.clone(),
        })
    })?;
    Ok(FeatureResult::Solid(SolidResult::new(feature.id(), placed)))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportCause {
    Unreadable { feature: String, source: String },
    ScaleRange { feature: String, factor: f64 },
    Unscalable { feature: String },
    ScaledTooMuch { feature: String, factor: f64 },
    TooFar { feature: String },
}

impl ImportCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unreadable { source, .. } => format!(
                "The shape imported from “{source}” could not be read back from the model file."
            ),
            Self::ScaleRange { feature, factor } => format!(
                "The scale of {feature} must be from {} to {}, and {} is not.",
                format_number(MIN_SCALE_FACTOR),
                format_number(MAX_SCALE_FACTOR),
                format_number(*factor)
            ),
            Self::Unscalable { feature } => format!("The body of {feature} could not be scaled."),
            Self::ScaledTooMuch { feature, factor } => format!(
                "Scaling the body of {feature} by {} would make it too large or too small to \
                 model.",
                format_number(*factor)
            ),
            Self::TooFar { feature } => {
                format!("The placement would take the body of {feature} too far from the origin.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Unreadable { feature, source } => {
                format!("Delete {feature} and import “{source}” again.")
            }
            Self::ScaleRange { .. } => {
                "Enter a scale above zero, such as 25.4 for a part drawn in inches.".to_owned()
            }
            Self::Unscalable { .. } | Self::ScaledTooMuch { .. } => {
                "Enter a scale nearer 1.".to_owned()
            }
            Self::TooFar { .. } => "Enter smaller distances.".to_owned(),
        }
    }
}

impl From<ImportCause> for Cause {
    fn from(cause: ImportCause) -> Self {
        Self::Import(cause)
    }
}
