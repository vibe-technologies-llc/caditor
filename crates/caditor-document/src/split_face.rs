use std::collections::BTreeSet;

use caditor_kernel::{FaceId, FaceReference, FaceSplitError, Solid, split_faces};

use crate::{
    datum::{Resolver, feature_name},
    document::{Feature, FeatureId},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
    solid::SolidResult,
    split::{
        HalfSpaceError, SplitAlong, SweptError, half_space_solid, is_open_chain, swept_half_space,
        swept_outlines,
    },
    trouble,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SplitFace {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub along: SplitAlong,
}

impl SplitFace {
    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + self.along.heap_size()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.along.datum());
        used.extend(self.along.body());
        used.extend(self.along.sketch());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut origins: BTreeSet<FeatureId> =
            self.faces.iter().flat_map(origins::of_face).collect();
        origins.extend(self.along.origin_features());
        origins
    }

    pub fn resolutions(&self, solid: &Solid) -> Vec<Resolution<FaceId>> {
        self.faces
            .iter()
            .map(|reference| {
                Resolution::of(reference.resolve(solid), |pieces| {
                    pieces_of_one_face(solid, pieces)
                })
            })
            .collect()
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Vec<FaceId>, Unresolved> {
        tally(self.resolutions(solid))
    }
}

struct Context<'a> {
    resolver: Resolver<'a>,
    body_name: String,
    tool: String,
}

impl Context<'_> {
    fn error(&self, reason: String, remedy: &str) -> Failure {
        self.resolver.own_error(reason, remedy)
    }

    fn misses(&self) -> Failure {
        self.error(
            format!("{} does not cross any of the chosen faces.", self.tool),
            "Choose faces it crosses, or move it so it runs across them.",
        )
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(
            format!(
                "The faces of {} could not be divided along {}.",
                self.body_name, self.tool
            ),
            "Move it slightly, so it does not run exactly along an edge.",
        )
    }

    fn unresolved(&self, unresolved: Unresolved) -> Failure {
        let body = &self.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to split is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to split are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => {
                format!("A face to split now matches several separate faces of the body of {body}.")
            }
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to split now match several separate faces of the body of \
                 {body}."
            ),
        };
        self.error(
            reason,
            "Choose the faces again, or undo the change that removed them.",
        )
    }

    fn split_failure(&self, solid: &Solid, tool: &Solid, error: FaceSplitError) -> Failure {
        match error {
            FaceSplitError::Cancelled(_) => Failure::Cancelled,
            FaceSplitError::NoFaces => self.error(
                "No face is chosen to split.".to_owned(),
                "Choose the faces to split.",
            ),
            FaceSplitError::Undivided => self.misses(),
            FaceSplitError::Boolean(error) => {
                let document = self.resolver.inputs.document;
                let trouble = trouble::boolean_trouble(document, [solid, tool], &error, "Move it");
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    trouble.reason(format!(
                        "The faces of {} could not be divided along {}.",
                        self.body_name, self.tool
                    )),
                    &trouble.remedy,
                )
                .placed(trouble.place)
            }
        }
    }

    fn swept_failure(&self, sketch: FeatureId, error: SweptError) -> Failure {
        let name = feature_name(self.resolver.inputs.document, sketch);
        let (reason, remedy) = match error {
            SweptError::NoCurves => (
                format!("{name} has no curve to split along."),
                format!("Draw a curve across the faces in {name}."),
            ),
            SweptError::NotOneChain | SweptError::Profile(_) => (
                format!(
                    "The curves of {name} form neither one open chain nor closed outlines, so \
                     they do not divide the faces into sides."
                ),
                format!(
                    "Leave one open chain of curves in {name}, or close its outlines, making \
                     other curves construction geometry."
                ),
            ),
            SweptError::CrossesItself => (
                format!(
                    "The curve of {name}, carried on straight past its ends, crosses itself, so \
                     it does not divide the faces into two sides."
                ),
                format!(
                    "Lengthen the curve of {name} so it runs past the faces, or draw it without \
                     crossing itself."
                ),
            ),
            other => return self.unbuildable(&other),
        };
        self.resolver.error(reason, remedy, sketch)
    }
}

fn tool_words(inputs: &Inputs<'_>, along: &SplitAlong) -> String {
    match along {
        SplitAlong::Plane(_) => "The plane".to_owned(),
        SplitAlong::Body(tool) => feature_name(inputs.document, *tool),
        SplitAlong::Sketch(sketch) => {
            format!("The curve of {}", feature_name(inputs.document, *sketch))
        }
    }
}

enum Tool<'a> {
    Made(Solid),
    Body(&'a Solid),
}

impl Tool<'_> {
    fn solid(&self) -> &Solid {
        match self {
            Self::Made(solid) => solid,
            Self::Body(solid) => solid,
        }
    }
}

fn tool<'a>(
    context: &'a Context<'_>,
    definition: &SplitFace,
    solid: &Solid,
) -> Result<Tool<'a>, Failure> {
    let feature = context.resolver.feature.id().raw();
    match &definition.along {
        SplitAlong::Plane(reference) => {
            let plane = context.resolver.plane(reference)?;
            half_space_solid(solid, &plane, false, feature)
                .map(Tool::Made)
                .map_err(|error| match error {
                    HalfSpaceError::Misses => context.misses(),
                    other => context.unbuildable(&other),
                })
        }
        SplitAlong::Body(tool) if *tool == definition.body => Err(context.error(
            format!(
                "The faces of {} cannot be split along its own body.",
                context.body_name
            ),
            "Choose another body to split them along.",
        )),
        SplitAlong::Body(tool) => context.resolver.body(*tool).map(Tool::Body),
        SplitAlong::Sketch(sketch) => {
            let inputs = context.resolver.inputs;
            let Some(result) = inputs
                .features
                .get(sketch)
                .and_then(|result| result.sketch())
            else {
                let name = feature_name(inputs.document, *sketch);
                return Err(context.resolver.error(
                    format!("It uses {name}, which has an error."),
                    format!("Fix {name} first."),
                    *sketch,
                ));
            };
            let made = if is_open_chain(&result.geometry) {
                swept_half_space(solid, &result.geometry, false, feature)
            } else {
                swept_outlines(solid, &result.geometry, feature)
            };
            made.map(Tool::Made)
                .map_err(|error| context.swept_failure(*sketch, error))
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &SplitFace,
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
        tool: tool_words(inputs, &definition.along),
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let faces = definition
        .resolve(solid)
        .map_err(|unresolved| context.unresolved(unresolved))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let tool = tool(&context, definition, solid)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let split = split_faces(solid, &faces, tool.solid(), feature.id().raw())
        .map_err(|error| context.split_failure(solid, tool.solid(), error))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        split,
    )))
}
