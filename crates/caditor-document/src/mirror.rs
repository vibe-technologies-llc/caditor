use std::collections::BTreeSet;

use caditor_geometry::Similarity;
use caditor_kernel::{
    BooleanError, BooleanOperation, Bounds, EnclosureError, FaceId, FaceReference, PatternCopy,
    PatternError, Solid, TransformError, boolean, enclose_faces, pattern, pattern_copies,
};

use crate::{
    datum::{PlaneReference, Resolver},
    document::{Feature, FeatureId},
    origins,
    pattern::{Seed, SeedWords, seed},
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
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
    pub mirrored: Vec<FeatureId>,
    pub faces: Vec<FaceReference>,
}

const MIRROR_WORDS: SeedWords = SeedWords {
    feature: "mirror",
    repeats: "mirrors",
    places: "reflects it on",
    repeated: "mirrored",
    verb: "mirror",
};

impl Mirror {
    pub fn new(body: FeatureId, plane: PlaneReference) -> Self {
        Self {
            body,
            plane,
            keep_original: true,
            mirrored: Vec::new(),
            faces: Vec::new(),
        }
    }

    #[must_use]
    pub fn mirroring(mut self, features: Vec<FeatureId>) -> Self {
        self.mirrored = features;
        self
    }

    #[must_use]
    pub fn mirroring_faces(mut self, faces: Vec<FaceReference>) -> Self {
        self.faces = faces;
        self
    }

    pub fn mirrors_features(&self) -> bool {
        !self.mirrored.is_empty()
    }

    pub fn mirrors_faces(&self) -> bool {
        self.mirrored.is_empty() && !self.faces.is_empty()
    }

    pub fn mirrors_whole_body(&self) -> bool {
        self.mirrored.is_empty() && self.faces.is_empty()
    }

    pub fn heap_size(&self) -> usize {
        self.plane.heap_size()
            + self.mirrored.len() * size_of::<FeatureId>()
            + size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut origins = self.plane.origin_features();
        origins.extend(self.faces.iter().flat_map(origins::of_face));
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

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.mirrored.iter().copied());
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

    fn reflect_features(
        &self,
        definition: &Mirror,
        solid: &Solid,
        image: PatternCopy,
        cancel: &CancelToken,
    ) -> Result<SolidResult, Failure> {
        let inputs = self.resolver.inputs;
        let raw = self.resolver.feature.id().raw();
        let copies = [image];
        let mut body = solid.clone();
        let mut cuts = Vec::new();
        for &feature in &definition.mirrored {
            let seed = seed(inputs, feature, definition.body, &MIRROR_WORDS)?;
            for tool in &seed.tools {
                if cancel.is_cancelled() {
                    return Err(Failure::Cancelled);
                }
                let placed = pattern_copies(tool, &copies, raw)
                    .map_err(|error| self.seed_placement_failure(&seed, &error))?;
                let Some(placed) = placed else {
                    continue;
                };
                body = boolean(&body, &placed, seed.operation).map_err(|error| {
                    self.combine_failure(inputs, [&body, &placed], &seed, &error)
                })?;
                if seed.operation == BooleanOperation::Difference {
                    cuts.push(placed);
                }
            }
        }
        Ok(SolidResult::new(definition.body, body).cutting(cuts))
    }

    fn reflect_faces(
        &self,
        definition: &Mirror,
        solid: &Solid,
        image: PatternCopy,
        cancel: &CancelToken,
    ) -> Result<SolidResult, Failure> {
        let raw = self.resolver.feature.id().raw();
        let faces = definition
            .resolve(solid)
            .map_err(|unresolved| self.unresolved_faces(unresolved))?;
        let enclosure =
            enclose_faces(solid, &faces, raw).map_err(|error| self.enclosure_failure(&error))?;
        if cancel.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let placed = pattern_copies(&enclosure.solid, &[image], raw)
            .map_err(|error| self.faces_placement_failure(&error))?
            .ok_or_else(|| self.unbuildable(&"the chosen faces have no image"))?;
        let operation = match enclosure.bounds {
            Bounds::Cavity => BooleanOperation::Difference,
            Bounds::Material => BooleanOperation::Union,
        };
        let body = boolean(solid, &placed, operation)
            .map_err(|error| self.faces_combine_failure([solid, &placed], operation, &error))?;
        let result = SolidResult::new(definition.body, body);
        Ok(match enclosure.bounds {
            Bounds::Cavity => result.cutting([placed]),
            Bounds::Material => result.joining([placed]),
        })
    }

    fn unresolved_faces(&self, unresolved: Unresolved) -> Failure {
        let body = &self.body_name;
        let reason = match unresolved {
            Unresolved::Missing(1) => {
                format!("A face to mirror is no longer part of the body of {body}.")
            }
            Unresolved::Missing(missing) => {
                format!("{missing} faces to mirror are no longer part of the body of {body}.")
            }
            Unresolved::Unrelated(1) => format!(
                "A face to mirror now matches several separate faces of the body of {body}."
            ),
            Unresolved::Unrelated(unrelated) => format!(
                "{unrelated} faces to mirror now match several separate faces of the body of \
                 {body}."
            ),
        };
        self.error(
            reason,
            "Choose the faces again, or undo the change that removed them.",
        )
    }

    fn enclosure_failure(&self, error: &EnclosureError) -> Failure {
        const REMEDY: &str = "Choose every face around the pocket or boss, such as a pocket's \
                              walls and floor or a boss's sides and top, so each opening lies in \
                              one plane or on one face beside it.";
        let body = &self.body_name;
        let reason = match error {
            EnclosureError::Cancelled(_) => return Failure::Cancelled,
            EnclosureError::NoFaces | EnclosureError::UnknownFace(_) => {
                return self.error(
                    "No face is chosen to mirror.".to_owned(),
                    "Choose the faces to mirror.",
                );
            }
            EnclosureError::NotFlat { .. } => format!(
                "The chosen faces of the body of {body} leave an opening that lies neither in \
                 one plane nor on one face beside it, so they do not bound a region that can be \
                 closed and mirrored."
            ),
            EnclosureError::NotElementary { .. } => format!(
                "The chosen faces of the body of {body} leave an opening on a freeform face, \
                 which cannot be carried across it, so they do not bound a region that can be \
                 closed and mirrored."
            ),
            EnclosureError::AroundSurface { .. } => format!(
                "The chosen faces of the body of {body} leave an opening that runs all the way \
                 around the round face it lies on, so they do not bound a region that can be \
                 closed and mirrored."
            ),
            EnclosureError::OpenBoundary { .. } => format!(
                "The chosen faces of the body of {body} meet the rest of the body along edges \
                 that do not run in separate loops, so they do not bound a region that can be \
                 closed and mirrored."
            ),
            EnclosureError::Openings { .. } => format!(
                "The openings of the chosen faces of the body of {body} lie on one surface but \
                 wind so that no face of it closes them, so they do not bound a region that can \
                 be mirrored."
            ),
            EnclosureError::Build(build) => {
                log::warn!("{} could not be built: {build}", self.resolver.feature.name);
                format!(
                    "The chosen faces of the body of {body}, closed across their openings, \
                     enclose no region that can be mirrored."
                )
            }
        };
        self.error(reason, REMEDY)
    }

    fn faces_placement_failure(&self, error: &PatternError) -> Failure {
        match error {
            PatternError::Cancelled(_)
            | PatternError::Placement {
                error: TransformError::Cancelled(_),
                ..
            } => Failure::Cancelled,
            PatternError::Placement {
                error: TransformError::Geometry(_),
                ..
            } => self.error(
                "The mirror image of the chosen faces would lie farther than a kilometre from \
                 the origin, the largest size caditor models."
                    .to_owned(),
                "Choose a plane nearer the body.",
            ),
            error => self.unbuildable(error),
        }
    }

    fn faces_combine_failure(
        &self,
        operands: [&Solid; 2],
        operation: BooleanOperation,
        error: &BooleanError,
    ) -> Failure {
        let body = &self.body_name;
        let headline = match (error, operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => {
                return self.error(
                    format!(
                        "The mirror image of the chosen faces would leave nothing of the body of \
                         {body}."
                    ),
                    "Choose another plane, or other faces.",
                );
            }
            (_, BooleanOperation::Difference) => format!(
                "The mirror image of the region the chosen faces bound could not be cut into \
                 the body of {body}."
            ),
            (_, _) => format!(
                "The mirror image of the region the chosen faces bound could not be joined to \
                 the body of {body}."
            ),
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble = trouble::boolean_trouble(
            self.resolver.inputs.document,
            operands,
            error,
            "Move the plane slightly",
        );
        self.error(trouble.reason(headline), &trouble.remedy)
            .placed(trouble.place)
    }

    fn seed_placement_failure(&self, seed: &Seed<'_>, error: &PatternError) -> Failure {
        match error {
            PatternError::Cancelled(_)
            | PatternError::Placement {
                error: TransformError::Cancelled(_),
                ..
            } => Failure::Cancelled,
            PatternError::Placement {
                error: TransformError::Geometry(_),
                ..
            } => self.error(
                format!(
                    "The mirror image of {} would lie farther than a kilometre from the origin, \
                     the largest size caditor models.",
                    seed.name
                ),
                "Choose a plane nearer the body.",
            ),
            error => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(
                    format!("The mirror image of {} could not be built.", seed.name),
                    "Choose another plane, or move the plane slightly.",
                )
            }
        }
    }

    fn combine_failure(
        &self,
        inputs: &Inputs<'_>,
        operands: [&Solid; 2],
        seed: &Seed<'_>,
        error: &BooleanError,
    ) -> Failure {
        let name = &seed.name;
        let body = &self.body_name;
        let headline = match (error, seed.operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => {
                return self.error(
                    format!(
                        "The mirror image of {name} would leave nothing of the body of {body}."
                    ),
                    "Choose another plane, or leave the feature out of the mirror.",
                );
            }
            (_, BooleanOperation::Difference) => {
                format!("The mirror image of {name} could not be cut into the body of {body}.")
            }
            (_, _) => {
                format!("The mirror image of {name} could not be joined to the body of {body}.")
            }
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble =
            trouble::boolean_trouble(inputs.document, operands, error, "Move the plane slightly");
        self.error(trouble.reason(headline), &trouble.remedy)
            .placed(trouble.place)
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
    let image = PatternCopy {
        index: MIRROR_IMAGE,
        placement: reflection,
    };
    if definition.mirrors_features() {
        return context
            .reflect_features(definition, solid, image, cancel)
            .map(FeatureResult::Solid);
    }
    if definition.mirrors_faces() {
        return context
            .reflect_faces(definition, solid, image, cancel)
            .map(FeatureResult::Solid);
    }
    let result = if definition.keep_original {
        pattern(solid, &[image], feature.id().raw()).map_err(|error| {
            context
                .pattern_failure(&error)
                .placed(trouble::union_place(&error))
        })?
    } else {
        solid
            .mapped(&image.placement)
            .map_err(|error| context.transform_failure(&error))?
    };
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        result,
    )))
}
