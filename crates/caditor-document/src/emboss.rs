use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_geometry::Plane;
use caditor_kernel::{
    BooleanError, BooleanOperation, FaceId, FaceName, FaceReference, FaceSplitError,
    LINEAR_RESOLUTION, LinearExtent, MAX_SIZE, Profile, ProfileError, Region, RegionReference,
    Selection, Solid, SplitPiece, SweepError, WrapBand, WrapError, boolean, extrude_tapered,
    resolve_regions, split_faces, wrap_regions_within,
};

use crate::{
    attachment::face_plane,
    datum::{Resolver, feature_name},
    describe::{describe_origin, edge_faces},
    document::{Feature, FeatureId},
    failure::{Cause, Chosen, Expected, Unmatched},
    origins,
    pieces::{Resolution, Unresolved, pieces_of_one_face, tally},
    recompute::{
        CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs, MissingInput,
    },
    solid::{RegionChoice, SolidResult, profile_curves},
    split::SweptError,
    split_face::{
        Context as SplitContext, Round, Unrolled, sketch_result, wrap_failure, wrapping_surface,
    },
    tolerance,
    trouble::{self, Adjust, Trouble},
};

pub const EMBED: f64 = 0.01;
pub const MAX_EMBOSS_DRAFT_DEGREES: f64 = 45.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbossForm {
    #[default]
    Raised,
    Recessed,
}

impl EmbossForm {
    pub fn is_raised(self) -> bool {
        self == Self::Raised
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Emboss {
    pub body: FeatureId,
    pub faces: Vec<FaceReference>,
    pub sketch: Option<FeatureId>,
    pub regions: RegionChoice,
    pub depth: Expression,
    pub draft: Expression,
    pub form: EmbossForm,
}

impl Emboss {
    pub fn undrafted() -> Expression {
        Expression::measure(0.0, Unit::Degree)
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.depth
            .parameters()
            .into_iter()
            .chain(self.draft.parameters())
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.depth.uses(parameter) || self.draft.uses(parameter)
    }

    pub fn is_drafted(&self) -> bool {
        !matches!(
            self.draft,
            Expression::Number(value) | Expression::Measure(value, _) if value == 0.0
        )
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        vec![&mut self.depth, &mut self.draft]
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.body])
            .into_iter()
            .chain(self.sketch)
            .collect()
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.faces.iter().flat_map(origins::of_face).collect()
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

    pub fn heap_size(&self) -> usize {
        size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(FaceReference::heap_size)
                .sum::<usize>()
            + self.depth.heap_size()
            + self.draft.heap_size()
            + match &self.regions {
                RegionChoice::All => 0,
                RegionChoice::Chosen(references) => {
                    size_of_val(references.as_slice())
                        + references
                            .iter()
                            .map(RegionReference::heap_size)
                            .sum::<usize>()
                }
            }
    }
}

struct Outlines {
    name: String,
    plane: Plane,
    regions: Vec<Region>,
}

enum Ground {
    Flat(Plane),
    Round { unrolled: Unrolled, concave: bool },
}

struct Context<'a> {
    feature: &'a Feature,
    definition: &'a Emboss,
    inputs: &'a Inputs<'a>,
    body_name: String,
}

impl Context<'_> {
    fn error(&self, cause: EmbossCause) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(self.feature.id())))
    }

    fn error_at(&self, cause: EmbossCause, fix: FeatureId) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(fix)))
    }

    fn depth(&self) -> Result<f64, Failure> {
        let depth = self
            .definition
            .depth
            .evaluate_as(Dimension::LENGTH, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    "depth",
                    Expected::Length(0.5),
                    error,
                ))
            })?;
        if depth <= LINEAR_RESOLUTION {
            return Err(self.error(EmbossCause::DepthNotPositive));
        }
        if depth > MAX_SIZE {
            return Err(self.error(EmbossCause::DepthTooBig));
        }
        Ok(depth)
    }

    fn draft(&self) -> Result<f64, Failure> {
        let degrees = self
            .definition
            .draft
            .evaluate_as(Dimension::ANGLE, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                Failure::from(FeatureError::unevaluable(
                    self.feature.id(),
                    "draft angle",
                    Expected::Angle(5.0),
                    error,
                ))
            })?;
        if degrees < 0.0 {
            return Err(self.error(EmbossCause::DraftNegative));
        }
        if degrees >= MAX_EMBOSS_DRAFT_DEGREES {
            return Err(self.error(EmbossCause::DraftSteep));
        }
        Ok(degrees.to_radians())
    }

    fn split_context(&self, sketch: &str) -> SplitContext<'_> {
        SplitContext::new(
            Resolver {
                feature: self.feature,
                inputs: self.inputs,
            },
            self.definition.body,
            self.body_name.clone(),
            format!("The outlines of {sketch}"),
        )
    }

    fn unbuilt(&self, error: &dyn std::fmt::Display) -> Failure {
        log::warn!("{} could not be built: {error}", self.feature.name);
        self.error(EmbossCause::Unbuilt {
            body: self.body_name.clone(),
        })
    }

    fn outlines(&self, sketch: FeatureId) -> Result<Outlines, Failure> {
        let result = sketch_result(self.inputs, sketch)?;
        let geometry = &result.geometry;
        let name = feature_name(self.inputs.document, sketch);
        let curves = profile_curves(geometry);
        let failed = |error: SweptError| match error {
            SweptError::NoCurves => {
                self.error_at(EmbossCause::NoOutlines { name: name.clone() }, sketch)
            }
            SweptError::Profile(ProfileError::NoClosedProfile { .. }) | SweptError::NotOneChain => {
                self.error_at(EmbossCause::NoOutlines { name: name.clone() }, sketch)
            }
            SweptError::Profile(ProfileError::Cancelled(_)) => Failure::Cancelled,
            other => self.unbuilt(&other),
        };
        if curves.is_empty() {
            return Err(failed(SweptError::NoCurves));
        }
        let profile = result
            .profile()
            .map_err(|error| failed(SweptError::Profile(error)))?;
        let selection = self.selection(sketch, &name, profile)?;
        let regions = profile
            .select(&selection)
            .map_err(|error| failed(SweptError::Profile(error)))?;
        if regions.is_empty() {
            return Err(failed(SweptError::NotOneChain));
        }
        Ok(Outlines {
            name,
            plane: geometry.plane(),
            regions,
        })
    }

    fn selection(
        &self,
        sketch: FeatureId,
        name: &str,
        profile: &Profile,
    ) -> Result<Selection, Failure> {
        let references = match &self.definition.regions {
            RegionChoice::All => return Ok(Selection::EvenDepth),
            RegionChoice::Chosen(references) => references,
        };
        if references.is_empty() {
            return Err(Failure::Waiting(MissingInput::RegionsToEmboss { sketch }));
        }
        resolve_regions(references, profile.regions())
            .map(|resolved| Selection::Regions(resolved.keys))
            .map_err(|error| match error {
                ProfileError::AmbiguousRegion { candidates, .. } => {
                    self.error(EmbossCause::RegionAmbiguous {
                        sketch: name.to_owned(),
                        candidates: candidates.len(),
                    })
                }
                _ => self.error(EmbossCause::RegionGone {
                    sketch: name.to_owned(),
                }),
            })
    }

    fn ground(
        &self,
        solid: &Solid,
        faces: &[FaceId],
        outlines: &Outlines,
    ) -> Result<Ground, Failure> {
        let planes: Vec<Option<Plane>> =
            faces.iter().map(|face| face_plane(solid, *face)).collect();
        if planes.iter().all(Option::is_some) {
            let mut planes = planes.into_iter().flatten();
            let first = planes.next().ok_or_else(|| self.waiting_for_faces())?;
            if planes.any(|other| !tolerance::same_plane(&first, &other)) {
                return Err(self.error(EmbossCause::MixedFaces {
                    body: self.body_name.clone(),
                    sketch: outlines.name.clone(),
                }));
            }
            return Ok(Ground::Flat(first));
        }
        let context = self.split_context(&outlines.name);
        let unrolled = wrapping_surface(&context, solid, faces, &outlines.name)?;
        let concave = faces
            .first()
            .and_then(|face| solid.face(*face))
            .is_some_and(|face| !face.sense().is_same());
        Ok(Ground::Round { unrolled, concave })
    }

    fn waiting_for_faces(&self) -> Failure {
        Failure::Waiting(MissingInput::FacesToEmboss {
            body: self.definition.body,
        })
    }

    fn heights(&self, depth: f64) -> (f64, f64) {
        match self.definition.form {
            EmbossForm::Raised => (-EMBED, depth),
            EmbossForm::Recessed => (-depth, EMBED),
        }
    }

    fn flat_tool(
        &self,
        face: &Plane,
        outlines: &Outlines,
        (depth, draft): (f64, f64),
    ) -> Result<Solid, Failure> {
        let sketch = &outlines.plane;
        if !tolerance::parallel(sketch.normal(), face.normal()) {
            return Err(self.error_at(
                EmbossCause::NotParallel {
                    sketch: outlines.name.clone(),
                },
                self.definition.sketch.unwrap_or(self.feature.id()),
            ));
        }
        let facing = sketch.normal().dot(face.normal()).signum();
        let level = (face.origin() - sketch.origin()).dot(sketch.normal());
        let (below, above) = self.heights(depth);
        let (start, end) = if facing > 0.0 {
            (below, above)
        } else {
            (-above, -below)
        };
        let extent = LinearExtent::new(start, end).map_err(|error| self.unbuilt(&error))?;
        let drawn = self.definition.sketch.unwrap_or(self.feature.id());

        let Some(ground) = Plane::from_frame(
            sketch.origin() + sketch.normal() * level,
            sketch.normal(),
            sketch.x_axis(),
        ) else {
            return Err(self.error(EmbossCause::Unbuilt {
                body: self.body_name.clone(),
            }));
        };
        extrude_tapered(
            &ground,
            &outlines.regions,
            extent,
            draft,
            self.feature.id().raw(),
        )
        .map_err(|error| match error {
            SweepError::Cancelled(_) => Failure::Cancelled,
            SweepError::TaperCloses { .. } => self.error(EmbossCause::DraftCloses {
                sketch: outlines.name.clone(),
            }),
            SweepError::TaperedUnsupportedCurve { .. } => self.error_at(
                EmbossCause::DraftUnsupported {
                    sketch: outlines.name.clone(),
                },
                drawn,
            ),
            other => self.unbuilt(&other),
        })
    }

    fn round_tool(
        &self,
        unrolled: Unrolled,
        concave: bool,
        outlines: &Outlines,
        (depth, draft): (f64, f64),
    ) -> Result<Solid, Failure> {
        if draft > 0.0 {
            return Err(self.error(EmbossCause::DraftWrapped {
                round: unrolled.round(),
            }));
        }
        if !tolerance::perpendicular(outlines.plane.normal(), unrolled.axis()) {
            return Err(self.error_at(
                EmbossCause::NotAlongAxis {
                    sketch: outlines.name.clone(),
                    round: unrolled.round(),
                },
                self.definition.sketch.unwrap_or(self.feature.id()),
            ));
        }
        let (below, above) = self.heights(depth);
        let (inward, outward) = if concave {
            (above, -below)
        } else {
            (-below, above)
        };
        let band = WrapBand::new(inward, outward).map_err(|error| self.unbuilt(&error))?;
        let sketch = self.definition.sketch.unwrap_or(self.feature.id());
        wrap_regions_within(
            &outlines.plane,
            &outlines.regions,
            &unrolled.surface(),
            Some(band),
            self.feature.id().raw(),
        )
        .map_err(|error| match error {
            WrapError::ReachesAxis => self.error(EmbossCause::TooDeep {
                round: unrolled.round(),
            }),
            other => {
                let context = self.split_context(&outlines.name);
                wrap_failure(&context, sketch, unrolled, other)
            }
        })
    }

    fn tool(
        &self,
        ground: &Ground,
        outlines: &Outlines,
        size: (f64, f64),
    ) -> Result<Solid, Failure> {
        match ground {
            Ground::Flat(face) => self.flat_tool(face, outlines, size),
            Ground::Round { unrolled, concave } => {
                self.round_tool(*unrolled, *concave, outlines, size)
            }
        }
    }

    fn combine_failure(&self, solid: &Solid, tool: &Solid, error: &BooleanError) -> Failure {
        match error {
            BooleanError::Cancelled(_) => Failure::Cancelled,
            BooleanError::Empty => self.error(EmbossCause::LeavesNothing {
                body: self.body_name.clone(),
            }),
            other => {
                let trouble = trouble::boolean_trouble(
                    self.inputs.document,
                    [solid, tool],
                    other,
                    Adjust::MoveItsCurves,
                );
                log::warn!("{} could not be built: {other}", self.feature.name);
                let place = trouble.place;
                self.error(EmbossCause::Uncombined {
                    body: self.body_name.clone(),
                    trouble,
                })
                .placed(place)
            }
        }
    }

    fn describe_face(&self, solid: &Solid, face: FaceId) -> String {
        describe_origin(
            self.inputs.document,
            solid.face(face).and_then(|face| face.origin()),
        )
    }

    fn within(
        &self,
        solid: &Solid,
        faces: &[FaceId],
        tool: &Solid,
        outlines: &Outlines,
    ) -> Result<(), Failure> {
        let feature = self.feature.id().raw();
        let split = match split_faces(solid, faces, tool, feature) {
            Ok(split) => split,
            Err(FaceSplitError::Cancelled(_)) => return Err(Failure::Cancelled),
            Err(FaceSplitError::NoFaces) => return Err(self.waiting_for_faces()),
            Err(FaceSplitError::Undivided) => {
                return Err(self.error(EmbossCause::Misses {
                    sketch: outlines.name.clone(),
                }));
            }
            Err(FaceSplitError::Boolean(error)) => {
                return Err(self.combine_failure(solid, tool, &error));
            }
        };
        match reaching_edge(solid, faces, &split, feature) {
            Some(face) => Err(self.error(EmbossCause::RunsOff {
                sketch: outlines.name.clone(),
                face: self.describe_face(solid, face),
            })),
            None => Ok(()),
        }
    }
}

fn reaching_edge(solid: &Solid, faces: &[FaceId], split: &Solid, feature: u64) -> Option<FaceId> {
    let chosen: BTreeSet<FaceName> = faces
        .iter()
        .filter_map(|face| solid.face(*face))
        .map(|face| face.name())
        .collect();
    let foreign: BTreeSet<FaceName> = solid
        .faces()
        .map(|(_, face)| face.name())
        .filter(|name| !chosen.contains(name))
        .collect();
    faces.iter().copied().find(|chosen_face| {
        let Some(original) = solid.face(*chosen_face) else {
            return false;
        };
        let inside = FaceName::split(feature, original.name(), SplitPiece::Inside);
        split
            .faces()
            .filter(|(_, piece)| piece.name() == inside)
            .any(|(piece_id, piece)| {
                piece
                    .loops()
                    .iter()
                    .filter_map(|id| split.face_loop(*id))
                    .flat_map(|ring| ring.coedges().iter().copied())
                    .filter_map(|id| split.coedge(id))
                    .flat_map(|coedge| edge_faces(split, coedge.edge()))
                    .filter(|other| *other != piece_id)
                    .filter_map(|other| split.face(other))
                    .any(|other| foreign.contains(&other.name()))
            })
    })
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Emboss,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let body_name = inputs
        .document
        .body_label(definition.body)
        .unwrap_or_default();
    let context = Context {
        feature,
        definition,
        inputs,
        body_name,
    };
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    if definition.faces.is_empty() {
        return Err(context.waiting_for_faces());
    }
    let Some(sketch) = definition.sketch else {
        return Err(Failure::Waiting(MissingInput::SketchToEmboss {
            body: definition.body,
        }));
    };
    let depth = context.depth()?;
    let draft = context.draft()?;
    let faces = definition.resolve(solid).map_err(|unresolved| {
        Failure::from(
            FeatureError::new(Unmatched {
                chosen: Chosen::FacesToEmboss,
                unresolved,
                body: context.body_name.clone(),
            })
            .fixing(FixTarget::Feature(feature.id())),
        )
    })?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let outlines = context.outlines(sketch)?;
    let ground = context.ground(solid, &faces, &outlines)?;
    let tool = context.tool(&ground, &outlines, (depth, draft))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    context.within(solid, &faces, &tool, &outlines)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let operation = match definition.form {
        EmbossForm::Raised => BooleanOperation::Union,
        EmbossForm::Recessed => BooleanOperation::Difference,
    };
    let embossed = boolean(solid, &tool, operation)
        .map_err(|error| context.combine_failure(solid, &tool, &error))?;
    let result = SolidResult::new(definition.body, embossed);
    Ok(FeatureResult::Solid(match definition.form {
        EmbossForm::Raised => result.joining([tool]),
        EmbossForm::Recessed => result.cutting([tool]),
    }))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EmbossCause {
    DepthNotPositive,
    DepthTooBig,
    DraftNegative,
    DraftSteep,
    DraftCloses { sketch: String },
    DraftUnsupported { sketch: String },
    DraftWrapped { round: Round },
    NoOutlines { name: String },
    RegionGone { sketch: String },
    RegionAmbiguous { sketch: String, candidates: usize },
    MixedFaces { body: String, sketch: String },
    NotParallel { sketch: String },
    NotAlongAxis { sketch: String, round: Round },
    TooDeep { round: Round },
    Misses { sketch: String },
    RunsOff { sketch: String, face: String },
    LeavesNothing { body: String },
    Unbuilt { body: String },
    Uncombined { body: String, trouble: Trouble },
}

impl EmbossCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::DepthNotPositive => "The depth must be more than zero.".to_owned(),
            Self::DepthTooBig => format!("The depth cannot be more than {} m.", MAX_SIZE / 1_000.0),
            Self::DraftNegative => "The draft cannot be negative.".to_owned(),
            Self::DraftSteep => {
                format!("The draft is {MAX_EMBOSS_DRAFT_DEGREES}° or steeper.")
            }
            Self::DraftCloses { sketch } => format!(
                "With its draft, an outline of {sketch} closes up before it reaches the depth."
            ),
            Self::DraftUnsupported { sketch } => format!(
                "An outline of {sketch} is one whole spline or ellipse, whose walls cannot be \
                 drafted."
            ),
            Self::DraftWrapped { round } => format!(
                "Outlines wrapped round a {} cannot have drafted walls yet.",
                round.noun()
            ),
            Self::NoOutlines { name } => format!(
                "{name} has no closed outline or text to emboss: its curves do not enclose any \
                 region."
            ),
            Self::RegionGone { sketch } => {
                format!("A chosen region of {sketch} no longer exists, because the sketch changed.")
            }
            Self::RegionAmbiguous { sketch, candidates } => format!(
                "A chosen region of {sketch} was divided, and {candidates} of the new regions \
                 match it equally."
            ),
            Self::MixedFaces { body, sketch } => format!(
                "The chosen faces of {body} do not lie in one plane or on one cylinder or cone, \
                 so the outlines of {sketch} cannot be embossed onto them all."
            ),
            Self::NotParallel { sketch } => format!(
                "{sketch} does not lie parallel to the chosen faces, so its outlines cannot be \
                 raised from them straight."
            ),
            Self::NotAlongAxis { sketch, round } => format!(
                "{sketch} does not lie on a plane along the axis of the {}, so its outlines \
                 cannot be wrapped round it.",
                round.noun()
            ),
            Self::TooDeep { round } => format!(
                "The depth reaches the axis of the {}, so there is no room to emboss that deep.",
                round.noun()
            ),
            Self::Misses { sketch } => {
                format!("The outlines of {sketch} do not cross any of the chosen faces.")
            }
            Self::RunsOff { sketch, face } => format!(
                "The outlines of {sketch} reach the edge of {face} or run past it, so they do \
                 not lie wholly on one face."
            ),
            Self::LeavesNothing { body } => {
                format!("Nothing would be left of the body of {body}.")
            }
            Self::Unbuilt { body } => {
                format!("The outlines could not be embossed onto the body of {body}.")
            }
            Self::Uncombined { body, trouble } => trouble.reason(format!(
                "The outlines could not be embossed onto the body of {body}."
            )),
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::DepthNotPositive | Self::DepthTooBig => {
                "Enter a depth above zero and below the size of the model.".to_owned()
            }
            Self::DraftNegative | Self::DraftSteep => format!(
                "Enter a draft angle from 0° up to below {MAX_EMBOSS_DRAFT_DEGREES}°; the walls \
                 lean in by it away from the face."
            ),
            Self::DraftCloses { .. } => "Make the draft or the depth smaller.".to_owned(),
            Self::DraftUnsupported { .. } => {
                "Split the curve into pieces in the sketch, or set the draft to 0°.".to_owned()
            }
            Self::DraftWrapped { .. } => "Set the draft to 0°.".to_owned(),
            Self::NoOutlines { name } => {
                format!("Close the outlines in {name}, or write some text in it.")
            }
            Self::RegionGone { .. } | Self::RegionAmbiguous { .. } => {
                "Choose the regions again.".to_owned()
            }
            Self::MixedFaces { .. } => {
                "Choose faces of one plane, or of one cylinder or cone, only.".to_owned()
            }
            Self::NotParallel { .. } => {
                "Place the sketch on the face, or on a plane parallel to it.".to_owned()
            }
            Self::NotAlongAxis { round, .. } => format!(
                "Place the sketch on a plane parallel to the {}'s axis, such as one touching it.",
                round.noun()
            ),
            Self::TooDeep { .. } => "Enter a smaller depth.".to_owned(),
            Self::Misses { .. } => {
                "Draw the outlines on the face, or move the sketch so they lie on it.".to_owned()
            }
            Self::RunsOff { .. } => {
                "Draw the outlines smaller or move them inside the face, away from its edges."
                    .to_owned()
            }
            Self::LeavesNothing { .. } => "Enter a smaller depth.".to_owned(),
            Self::Unbuilt { .. } => "Move the outlines slightly or change the depth.".to_owned(),
            Self::Uncombined { trouble, .. } => trouble.remedy(),
        }
    }
}

impl From<EmbossCause> for Cause {
    fn from(cause: EmbossCause) -> Self {
        Self::Emboss(cause)
    }
}
