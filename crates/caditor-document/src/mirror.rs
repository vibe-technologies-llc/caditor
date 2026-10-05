use std::collections::BTreeSet;

use caditor_geometry::Similarity;
use caditor_kernel::{BooleanError, PatternCopy, PatternError, TransformError, pattern};

use crate::{
    datum::{PlaneReference, Resolver},
    document::{Feature, FeatureId},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    solid::SolidResult,
    trouble,
};

pub const MIRROR_IMAGE: [u32; 2] = [1, 0];

#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    pub body: FeatureId,
    pub plane: PlaneReference,
    pub keep_original: bool,
}

impl Mirror {
    pub fn heap_size(&self) -> usize {
        self.plane.heap_size()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.plane.datum());
        used.extend(self.plane.body());
        used
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy: remedy.to_owned(),
            fix: Some(FixTarget::Feature(self.resolver.feature.id())),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(
            format!(
                "The mirror image of the body of {} could not be built.",
                self.body_name
            ),
            "Choose another plane, or move the plane slightly.",
        )
    }

    fn transform_failure(&self, error: &TransformError) -> Failure {
        match error {
            TransformError::Cancelled(_) => Failure::Cancelled,
            TransformError::Geometry(_) => self.error(
                format!(
                    "The mirror image of the body of {} would lie farther than a kilometre from \
                     the origin, the largest size caditor models.",
                    self.body_name
                ),
                "Choose a plane nearer the body.",
            ),
            error => self.unbuildable(error),
        }
    }

    fn pattern_failure(&self, error: &PatternError) -> Failure {
        let body = &self.body_name;
        match error {
            PatternError::Cancelled(_) => Failure::Cancelled,
            PatternError::Placement { error, .. } => self.transform_failure(error),
            PatternError::Union {
                error: BooleanError::NonManifold(_),
                ..
            } => self.error(
                format!(
                    "The body of {body} and its mirror image meet only along an edge or at a \
                     corner, and could not be kept as separate shells."
                ),
                "Move the plane so the two overlap, share a face or stand apart.",
            ),
            PatternError::Union {
                error: BooleanError::Ambiguous(_),
                ..
            } => self.error(
                format!(
                    "The body of {body} and its mirror image touch where it cannot be told which \
                     side is inside."
                ),
                "Move the plane slightly.",
            ),
            PatternError::Union { error, .. } => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    format!("The body of {body} could not be joined with its mirror image."),
                    "Move the plane slightly, or clear Keep the original.",
                )
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Mirror,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .feature(definition.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
    };
    let plane = context.resolver.plane(&definition.plane)?;
    let reflection = Similarity::reflection(&plane)
        .ok_or_else(|| context.unbuildable(&"the plane has no direction"))?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::Error(Box::new(FeatureError {
            reason: format!("The body made by {} has no shape.", context.body_name),
            remedy: format!("Fix {} first.", context.body_name),
            fix: Some(FixTarget::Feature(definition.body)),
            constraints: Vec::new(),
            place: None,
        })));
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let result = if definition.keep_original {
        let image = PatternCopy {
            index: MIRROR_IMAGE,
            placement: reflection,
        };
        pattern(solid, &[image], feature.id().raw()).map_err(|error| {
            context
                .pattern_failure(&error)
                .placed(trouble::union_place(&error))
        })?
    } else {
        solid
            .mapped(&reflection)
            .map_err(|error| context.transform_failure(&error))?
    };
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
