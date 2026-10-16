use std::collections::BTreeSet;

use caditor_geometry::Similarity;
use caditor_kernel::{
    BooleanError, BooleanOperation, Bounds, EnclosureError, FaceId, FaceReference, PatternCopy,
    PatternError, Solid, TransformError, boolean, enclose_faces, pattern, pattern_copies,
};

use crate::{
    datum::{PlaneReference, Resolver},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Unmatched, Upstream},
    origins,
    pattern::{Instance, Repeated, Seed, Seeds},
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    repeat_blend::{self, CopiedNames, Repeat, Repeater},
    repeat_shell,
    solid::SolidResult,
    trouble::{self, Adjust, Trouble},
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
    fn error(&self, cause: MirrorCause) -> Failure {
        Failure::from(
            FeatureError::new(cause).fixing(FixTarget::Feature(self.resolver.feature.id())),
        )
    }

    fn unbuildable(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        self.error(MirrorCause::Unbuilt {
            of: format!("the body of {}", self.body_name),
        })
    }

    fn transform_failure(&self, error: &TransformError) -> Failure {
        match error {
            TransformError::Cancelled(_) => Failure::Cancelled,
            TransformError::Geometry(_) => self.error(MirrorCause::TooFar {
                of: format!("the body of {}", self.body_name),
            }),
            error => self.unbuildable(error),
        }
    }

    fn pattern_failure(&self, error: &PatternError) -> Failure {
        let body = self.body_name.clone();
        match error {
            PatternError::Cancelled(_) => Failure::Cancelled,
            PatternError::Placement { error, .. } => self.transform_failure(error),
            PatternError::Union {
                error: BooleanError::NonManifold(_),
                ..
            } => self.error(MirrorCause::Edgewise { body }),
            PatternError::Union {
                error: BooleanError::Ambiguous(_),
                ..
            } => self.error(MirrorCause::Touching { body }),
            PatternError::Union { error, .. } => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(MirrorCause::Unjoined { body })
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
        let seeds = Seeds::of(
            inputs,
            &definition.mirrored,
            definition.body,
            Repeater::Mirror,
        )?;
        let mut body = seeds.start(solid).clone();
        let mut cuts = Vec::new();
        let mut names = CopiedNames::new(self.resolver.feature.id());
        let image_words = |_: Instance| "the mirror image".to_owned();
        let base = Repeat {
            feature: self.resolver.feature.id(),
            repeater: Repeater::Mirror,
            copy: &image_words,
        };
        for (feature, repeated) in seeds.repeated {
            let seed = match repeated {
                Repeated::Tools(seed) => seed,
                Repeated::Blend { name, recipe } => {
                    body = names
                        .blend(body, recipe, &[MIRROR_IMAGE], cancel)
                        .map_err(|error| repeat_blend::failure(&error, recipe, &base, &name))?;
                    continue;
                }
                Repeated::Shell(_) => continue,
            };
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
            names.copying_tools_of(feature);
        }
        if let Some(shell) = &seeds.shell {
            body = names
                .shell(&body, shell.recipe, &[MIRROR_IMAGE], cancel)
                .map_err(|error| repeat_shell::failure(&error, &base, &shell.name))?;
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
        Failure::from(
            FeatureError::new(Unmatched {
                chosen: Chosen::FacesToMirror,
                unresolved,
                body: self.body_name.clone(),
            })
            .fixing(FixTarget::Feature(self.resolver.feature.id())),
        )
    }

    fn enclosure_failure(&self, error: &EnclosureError) -> Failure {
        let problem = match error {
            EnclosureError::Cancelled(_) => return Failure::Cancelled,
            EnclosureError::NoFaces | EnclosureError::UnknownFace(_) => {
                return self.error(MirrorCause::NoFaces);
            }
            EnclosureError::NotFlat { .. } => Opening::NotFlat,
            EnclosureError::NotElementary { .. } => Opening::NotElementary,
            EnclosureError::AroundSurface { .. } => Opening::AroundSurface,
            EnclosureError::OpenBoundary { .. } => Opening::OpenBoundary,
            EnclosureError::Openings { .. } => Opening::Openings,
            EnclosureError::Build(build) => {
                log::warn!("{} could not be built: {build}", self.resolver.feature.name);
                Opening::Build
            }
        };
        self.error(MirrorCause::Enclosure {
            problem,
            body: self.body_name.clone(),
        })
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
            } => self.error(MirrorCause::TooFar {
                of: "the chosen faces".to_owned(),
            }),
            error => self.unbuildable(error),
        }
    }

    fn faces_combine_failure(
        &self,
        operands: [&Solid; 2],
        operation: BooleanOperation,
        error: &BooleanError,
    ) -> Failure {
        let body = self.body_name.clone();
        let cutting = match (error, operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => return self.error(MirrorCause::FacesVanish { body }),
            (_, BooleanOperation::Difference) => true,
            (_, _) => false,
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble = trouble::boolean_trouble(
            self.resolver.inputs.document,
            operands,
            error,
            Adjust::MoveThePlaneSlightly,
        );
        let place = trouble.place;
        self.error(MirrorCause::FacesUncombined {
            body,
            cutting,
            trouble,
        })
        .placed(place)
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
            } => self.error(MirrorCause::TooFar {
                of: seed.name.clone(),
            }),
            error => {
                log::warn!("{} could not be built: {error}", self.resolver.feature.name);
                self.error(MirrorCause::Unbuilt {
                    of: seed.name.clone(),
                })
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
        let name = seed.name.clone();
        let body = self.body_name.clone();
        let cutting = match (error, seed.operation) {
            (BooleanError::Cancelled(_), _) => return Failure::Cancelled,
            (BooleanError::Empty, _) => {
                return self.error(MirrorCause::ImageVanishes { name, body });
            }
            (_, BooleanOperation::Difference) => true,
            (_, _) => false,
        };
        log::warn!("{} could not be built: {error}", self.resolver.feature.name);
        let trouble = trouble::boolean_trouble(
            inputs.document,
            operands,
            error,
            Adjust::MoveThePlaneSlightly,
        );
        let place = trouble.place;
        self.error(MirrorCause::ImageUncombined {
            name,
            body,
            cutting,
            trouble,
        })
        .placed(place)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    NotFlat,
    NotElementary,
    AroundSurface,
    OpenBoundary,
    Openings,
    Build,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MirrorCause {
    Unbuilt {
        of: String,
    },
    TooFar {
        of: String,
    },
    Edgewise {
        body: String,
    },
    Touching {
        body: String,
    },
    Unjoined {
        body: String,
    },
    NoFaces,
    Enclosure {
        problem: Opening,
        body: String,
    },
    FacesVanish {
        body: String,
    },
    FacesUncombined {
        body: String,
        cutting: bool,
        trouble: Trouble,
    },
    ImageVanishes {
        name: String,
        body: String,
    },
    ImageUncombined {
        name: String,
        body: String,
        cutting: bool,
        trouble: Trouble,
    },
}

impl MirrorCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Unbuilt { of } => format!("The mirror image of {of} could not be built."),
            Self::TooFar { of } => format!(
                "The mirror image of {of} would lie farther than a kilometre from the origin, \
                 the largest size caditor models."
            ),
            Self::Edgewise { body } => format!(
                "The body of {body} and its mirror image meet only along an edge or at a \
                 corner, and could not be kept as separate shells."
            ),
            Self::Touching { body } => format!(
                "The body of {body} and its mirror image touch where it cannot be told which \
                 side is inside."
            ),
            Self::Unjoined { body } => {
                format!("The body of {body} could not be joined with its mirror image.")
            }
            Self::NoFaces => "No face is chosen to mirror.".to_owned(),
            Self::Enclosure { problem, body } => match problem {
                Opening::NotFlat => format!(
                    "The chosen faces of the body of {body} leave an opening that lies neither \
                     in one plane nor on one face beside it, so they do not bound a region that \
                     can be closed and mirrored."
                ),
                Opening::NotElementary => format!(
                    "The chosen faces of the body of {body} leave an opening on a freeform face, \
                     which cannot be carried across it, so they do not bound a region that can \
                     be closed and mirrored."
                ),
                Opening::AroundSurface => format!(
                    "The chosen faces of the body of {body} leave an opening that runs all the \
                     way around the round face it lies on, so they do not bound a region that \
                     can be closed and mirrored."
                ),
                Opening::OpenBoundary => format!(
                    "The chosen faces of the body of {body} meet the rest of the body along \
                     edges that do not run in separate loops, so they do not bound a region that \
                     can be closed and mirrored."
                ),
                Opening::Openings => format!(
                    "The openings of the chosen faces of the body of {body} lie on one surface \
                     but wind so that no face of it closes them, so they do not bound a region \
                     that can be mirrored."
                ),
                Opening::Build => format!(
                    "The chosen faces of the body of {body}, closed across their openings, \
                     enclose no region that can be mirrored."
                ),
            },
            Self::FacesVanish { body } => format!(
                "The mirror image of the chosen faces would leave nothing of the body of {body}."
            ),
            Self::FacesUncombined {
                body,
                cutting,
                trouble,
            } => trouble.reason(if *cutting {
                format!(
                    "The mirror image of the region the chosen faces bound could not be cut into \
                     the body of {body}."
                )
            } else {
                format!(
                    "The mirror image of the region the chosen faces bound could not be joined \
                     to the body of {body}."
                )
            }),
            Self::ImageVanishes { name, body } => {
                format!("The mirror image of {name} would leave nothing of the body of {body}.")
            }
            Self::ImageUncombined {
                name,
                body,
                cutting,
                trouble,
            } => trouble.reason(if *cutting {
                format!("The mirror image of {name} could not be cut into the body of {body}.")
            } else {
                format!("The mirror image of {name} could not be joined to the body of {body}.")
            }),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::Unbuilt { .. } => "Choose another plane, or move the plane slightly.".to_owned(),
            Self::TooFar { .. } => "Choose a plane nearer the body.".to_owned(),
            Self::Edgewise { .. } => {
                "Move the plane so the two overlap, share a face or stand apart.".to_owned()
            }
            Self::Touching { .. } => "Move the plane slightly.".to_owned(),
            Self::Unjoined { .. } => {
                "Move the plane slightly, or clear Keep the original.".to_owned()
            }
            Self::NoFaces => "Choose the faces to mirror.".to_owned(),
            Self::Enclosure { .. } => {
                "Choose every face around the pocket or boss, such as a pocket's walls and floor \
                 or a boss's sides and top, so each opening lies in one plane or on one face \
                 beside it."
                    .to_owned()
            }
            Self::FacesVanish { .. } => "Choose another plane, or other faces.".to_owned(),
            Self::ImageVanishes { .. } => {
                "Choose another plane, or leave the feature out of the mirror.".to_owned()
            }
            Self::FacesUncombined { trouble, .. } | Self::ImageUncombined { trouble, .. } => {
                trouble.remedy()
            }
        }
    }
}

impl From<MirrorCause> for Cause {
    fn from(cause: MirrorCause) -> Self {
        Self::Mirror(cause)
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
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        resolver: Resolver { feature, inputs },
        body_name,
    };
    let plane = context.resolver.plane(&definition.plane)?;
    let reflection = Similarity::reflection(&plane)
        .ok_or_else(|| context.unbuildable(&"the plane has no direction"))?;
    let Some(solid) = inputs.body(definition.body) else {
        return Err(Failure::from(
            FeatureError::new(Upstream::BodyShapeless {
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(definition.body)),
        ));
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
