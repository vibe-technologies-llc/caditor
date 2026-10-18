use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_kernel::{
    BooleanError, BooleanOperation, FaceOrigin, MIN_FLANK_DEGREES, Solid, SweepError, WallError,
    boolean,
};
use caditor_sketch::{EntityId, Sketch};

use crate::{
    datum::feature_name,
    document::{Document, Feature, FeatureId, list_names},
    failure::Cause,
    primitive::Context,
    recompute::{CancelToken, Evaluation, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    rib_tool::{
        RibDirection, RibEnds, RibError, RibInput, RibSide, ThicknessStand, chains, facing_body,
        is_flank, is_run, rib_tools, run_curve, thickness_stand,
    },
    solid::{SolidResult, profile_curves},
    sweep::capital,
    trouble::{self, Adjust, Trouble},
};

pub const RIB_SHAPE: &str = "rib";
pub const MAX_RIB_DRAFT_DEGREES: f64 = 30.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Rib {
    pub sketch: FeatureId,
    pub body: FeatureId,
    pub thickness: Expression,
    pub draft: Expression,
    pub side: RibSide,
    pub direction: RibDirection,
    pub ends: RibEnds,
    pub reversed: bool,
}

impl Rib {
    pub fn new(sketch: FeatureId, body: FeatureId, thickness: Expression) -> Self {
        Self {
            sketch,
            body,
            thickness,
            draft: Expression::measure(0.0, Unit::Degree),
            side: RibSide::Symmetric,
            direction: RibDirection::Parallel,
            ends: RibEnds::Straight,
            reversed: false,
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        vec![&self.thickness, &self.draft]
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        vec![&mut self.thickness, &mut self.draft]
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        let mut parameters = self.thickness.parameters();
        parameters.extend(self.draft.parameters());
        parameters
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.thickness.uses(parameter) || self.draft.uses(parameter)
    }

    pub fn is_drafted(&self) -> bool {
        !matches!(
            self.draft,
            Expression::Number(value) | Expression::Measure(value, _) if value == 0.0
        )
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        BTreeSet::from([self.sketch, self.body])
    }

    pub fn heap_size(&self) -> usize {
        self.thickness.heap_size() + self.draft.heap_size()
    }
}

pub fn direction_name(direction: RibDirection) -> &'static str {
    match direction {
        RibDirection::Parallel => "Parallel to sketch",
        RibDirection::Square => "Square to sketch",
    }
}

pub fn ends_name(ends: RibEnds) -> &'static str {
    match ends {
        RibEnds::Straight => "Straight on",
        RibEnds::RoundArcs => "Round the arc",
    }
}

pub fn side_name(side: RibSide) -> &'static str {
    match side {
        RibSide::OneSide => "One side",
        RibSide::OtherSide => "Other side",
        RibSide::Symmetric => "Symmetric",
    }
}

pub(crate) fn face_words(document: &Document, rib: &Rib, origin: FaceOrigin) -> String {
    let curve = |entity: u64| {
        document
            .feature(rib.sketch)
            .and_then(|sketch| sketch.kind.sketch())
            .map_or_else(
                || "a sketch curve".to_owned(),
                |sketch| sketch.entity_label(EntityId::from_raw(run_curve(entity))),
            )
    };
    match (origin, rib.direction) {
        (FaceOrigin::Side { entity, .. }, _) if is_flank(entity) => {
            format!("drafted face along {}", curve(entity))
        }
        (FaceOrigin::Side { entity, .. }, _) if is_run(entity) => {
            format!("side carried on from {}", curve(entity))
        }
        (FaceOrigin::Side { entity, .. }, RibDirection::Parallel) => {
            format!("edge face along {}", curve(entity))
        }
        (FaceOrigin::Side { entity, .. }, RibDirection::Square) => {
            format!("side from {}", curve(entity))
        }
        (FaceOrigin::StartCap { .. }, RibDirection::Parallel) => {
            "face behind the sketch".to_owned()
        }
        (FaceOrigin::EndCap { .. }, RibDirection::Parallel) => {
            "face in front of the sketch".to_owned()
        }
        (FaceOrigin::StartCap { .. } | FaceOrigin::EndCap { .. }, RibDirection::Square) => {
            "top face".to_owned()
        }
        _ => "face".to_owned(),
    }
}

struct Words<'a> {
    feature: &'a Feature,
    sketch: &'a Sketch,
    sketch_name: String,
    body_name: String,
    definition: &'a Rib,
}

impl Words<'_> {
    fn curves(&self, entities: &[u64]) -> String {
        let labels: Vec<String> = entities
            .iter()
            .map(|entity| self.sketch.entity_label(EntityId::from_raw(*entity)))
            .collect();
        list_names(&labels)
    }

    fn failed(&self, cause: RibCause, fix: FeatureId) -> Failure {
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(fix)))
    }

    fn own(&self, cause: RibCause) -> Failure {
        self.failed(cause, self.feature.id())
    }

    fn in_sketch(&self, cause: RibCause) -> Failure {
        self.failed(cause, self.definition.sketch)
    }

    fn drawn(&self, entities: &[u64]) -> String {
        let mut drawn: Vec<u64> = entities.iter().map(|entity| run_curve(*entity)).collect();
        drawn.sort_unstable();
        drawn.dedup();
        self.curves(&drawn)
    }

    fn failure(&self, error: RibError) -> Failure {
        let name = self.feature.name.clone();
        let sketch = self.sketch_name.clone();
        let body = self.body_name.clone();
        let direction = self.definition.direction;
        match error {
            RibError::NoCurves => self.in_sketch(RibCause::NoCurves { sketch, name }),
            RibError::Closed { entities } => self.in_sketch(RibCause::Closed {
                curves: self.curves(&entities),
                sketch,
            }),
            RibError::Branches { entities } => self.in_sketch(RibCause::Branches {
                curves: self.curves(&entities),
                sketch,
                name,
            }),
            RibError::Degenerate { entities } | RibError::CrossesItself { entities } => self
                .in_sketch(RibCause::CurvesCross {
                    curves: self.curves(&entities),
                    sketch,
                }),
            RibError::NoExtent => {
                self.failed(RibCause::NoExtent { body, name }, self.definition.body)
            }
            RibError::Behind => self.own(RibCause::Behind { body, sketch, name }),
            RibError::Inside { entities } => self.own(RibCause::Inside {
                curves: self.curves(&entities),
                single: entities.len() == 1,
                sketch,
                body,
                name,
            }),
            RibError::MeetsNothing { entities } => self.own(RibCause::MeetsNothing {
                name,
                body,
                curves: self.curves(&entities),
                direction,
            }),
            RibError::RunsPastEnd { entity } => self.own(RibCause::RunsPastEnd {
                name,
                curve: self.sketch.entity_label(EntityId::from_raw(entity)),
                body,
                ends: self.definition.ends,
            }),
            RibError::RunsPastDepth { entities } => self.own(RibCause::RunsPastDepth {
                curves: self.curves(&entities),
                sketch,
                name,
                body,
            }),
            RibError::RunsPast { entities } => self.own(RibCause::RunsPast {
                name,
                body,
                curves: self.curves(&entities),
                direction,
            }),
            RibError::Wall(WallError::TooThick { entities }) => self.own(RibCause::ThickBend {
                name,
                curves: self.curves(&entities),
            }),
            RibError::Wall(WallError::CrossesItself)
                if self.definition.ends == RibEnds::RoundArcs =>
            {
                self.own(RibCause::ArcsCross { name })
            }
            RibError::Wall(WallError::CrossesItself) => self.own(RibCause::Crosses { name }),
            RibError::RunsPastFlank { entities } => self.own(RibCause::DraftCloses {
                name,
                curves: self.drawn(&entities),
                body,
            }),
            RibError::Sweep(SweepError::TaperCloses { entities }) if !entities.is_empty() => self
                .own(RibCause::DraftCloses {
                    name,
                    curves: self.drawn(&entities),
                    body,
                }),
            RibError::Wall(WallError::Cancelled(_)) => Failure::Cancelled,
            RibError::Boolean(BooleanError::Cancelled(_)) => Failure::Cancelled,
            other => {
                log::warn!("{name} could not be built: {other}");
                self.own(RibCause::Unbuilt { name, body })
            }
        }
    }
}

fn draft(
    context: &Context<'_>,
    expression: &Expression,
    direction: RibDirection,
) -> Result<f64, Failure> {
    let degrees = context.value_as(expression, "draft angle", Dimension::ANGLE)?;
    let shape = context.shape();
    if degrees < 0.0 {
        return Err(context.error(RibCause::DraftNegative {
            shape: shape.to_owned(),
        }));
    }
    if degrees >= MAX_RIB_DRAFT_DEGREES {
        return Err(context.error(RibCause::DraftSteep {
            shape: shape.to_owned(),
        }));
    }
    if direction == RibDirection::Parallel && degrees > 0.0 && degrees < MIN_FLANK_DEGREES {
        return Err(context.error(RibCause::DraftShallow {
            shape: shape.to_owned(),
        }));
    }
    Ok(degrees.to_radians())
}

fn sketch_of<'a>(
    inputs: &'a Inputs<'a>,
    feature: &Feature,
    sketch: FeatureId,
) -> Result<&'a Sketch, Failure> {
    inputs
        .features
        .get(&sketch)
        .and_then(|result| result.sketch())
        .map(|result| &result.geometry)
        .ok_or_else(|| {
            let name = feature_name(inputs.document, sketch);
            Failure::from(
                FeatureError::new(RibCause::SketchFailed {
                    feature: feature.name.clone(),
                    name,
                })
                .fixing(FixTarget::Feature(sketch)),
            )
        })
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Rib,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context::new(feature, inputs, RIB_SHAPE);
    let thickness = context.size(&definition.thickness, "thickness")?;
    let draft = draft(&context, &definition.draft, definition.direction)?;
    let sketch = sketch_of(inputs, feature, definition.sketch)?;
    let body = inputs
        .body(definition.body)
        .ok_or_else(|| inputs.missing_body(definition.body))?;
    let words = Words {
        feature,
        sketch,
        sketch_name: feature_name(inputs.document, definition.sketch),
        body_name: inputs
            .document
            .body_label(definition.body)
            .unwrap_or_default(),
        definition,
    };
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let input = RibInput {
        plane: sketch.plane(),
        curves: profile_curves(sketch),
        body,
        thickness,
        draft,
        side: definition.side,
        direction: definition.direction,
        ends: match definition.direction {
            RibDirection::Square => definition.ends,
            RibDirection::Parallel => RibEnds::Straight,
        },
        reversed: definition.reversed,
        feature: feature.id().raw(),
    };
    let tools = rib_tools(&input).map_err(|error| words.failure(error))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let mut joined: Solid = body.clone();
    for tool in &tools {
        joined = boolean(&joined, tool, BooleanOperation::Union).map_err(|error| {
            if let BooleanError::Cancelled(_) = error {
                return Failure::Cancelled;
            }
            let trouble = trouble::boolean_trouble(
                inputs.document,
                [&joined, tool],
                &error,
                Adjust::MoveItsCurves,
            );
            log::warn!("{} could not be joined: {error}", feature.name);
            let place = trouble.place;
            words
                .own(RibCause::Uncombined {
                    name: feature.name.clone(),
                    body: words.body_name.clone(),
                    trouble,
                })
                .placed(place)
        })?;
    }
    Ok(FeatureResult::Solid(
        SolidResult::new(definition.body, joined).joining(tools),
    ))
}

pub fn is_rib_path(sketch: &Sketch) -> bool {
    chains(&profile_curves(sketch)).is_ok()
}

pub fn facing(evaluation: &Evaluation, definition: &Rib, at: Option<FeatureId>) -> Option<bool> {
    let sketch = &evaluation
        .feature(definition.sketch)?
        .result
        .as_ref()?
        .sketch()?
        .geometry;
    let body = match at {
        Some(feature) => evaluation.body_seen_by(feature, definition.body)?,
        None => evaluation.body(definition.body)?,
    };
    facing_body(
        &sketch.plane(),
        &profile_curves(sketch),
        body,
        definition.direction,
    )
}

pub fn rib_stand(evaluation: &Evaluation, definition: &Rib) -> Option<ThicknessStand> {
    let sketch = &evaluation
        .feature(definition.sketch)?
        .result
        .as_ref()?
        .sketch()?
        .geometry;
    thickness_stand(
        &sketch.plane(),
        &profile_curves(sketch),
        definition.side,
        definition.direction,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RibCause {
    NoCurves {
        sketch: String,
        name: String,
    },
    Closed {
        curves: String,
        sketch: String,
    },
    Branches {
        curves: String,
        sketch: String,
        name: String,
    },
    CurvesCross {
        curves: String,
        sketch: String,
    },
    NoExtent {
        body: String,
        name: String,
    },
    Behind {
        body: String,
        sketch: String,
        name: String,
    },
    Inside {
        curves: String,
        single: bool,
        sketch: String,
        body: String,
        name: String,
    },
    MeetsNothing {
        name: String,
        body: String,
        curves: String,
        direction: RibDirection,
    },
    RunsPastEnd {
        name: String,
        curve: String,
        body: String,
        ends: RibEnds,
    },
    RunsPastDepth {
        curves: String,
        sketch: String,
        name: String,
        body: String,
    },
    RunsPast {
        name: String,
        body: String,
        curves: String,
        direction: RibDirection,
    },
    ThickBend {
        name: String,
        curves: String,
    },
    ArcsCross {
        name: String,
    },
    Crosses {
        name: String,
    },
    DraftCloses {
        name: String,
        curves: String,
        body: String,
    },
    Unbuilt {
        name: String,
        body: String,
    },
    Uncombined {
        name: String,
        body: String,
        trouble: Trouble,
    },
    DraftNegative {
        shape: String,
    },
    DraftSteep {
        shape: String,
    },
    DraftShallow {
        shape: String,
    },
    SketchFailed {
        feature: String,
        name: String,
    },
}

impl RibCause {
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::NoCurves { sketch, name } => {
                format!("{sketch} has no curve for {name} to follow.")
            }
            Self::Closed { curves, sketch } => format!(
                "{} of {sketch} close on themselves, but a rib follows an open chain of curves.",
                capital(curves)
            ),
            Self::Branches {
                curves,
                sketch,
                name,
            } => format!(
                "{} of {sketch} come back to a point they already pass through, so {name} cannot \
                 tell which way to run.",
                capital(curves)
            ),
            Self::CurvesCross { curves, sketch } => format!(
                "{} of {sketch}, carried on straight past their ends, cross themselves, so they \
                 do not bound one side of the rib.",
                capital(curves)
            ),
            Self::NoExtent { body, name } => format!("{body} has no shape for {name} to join."),
            Self::Behind { body, sketch, name } => format!(
                "{body} lies wholly on the other side of {sketch}, so {name} meets no face of the \
                 body."
            ),
            Self::Inside {
                curves,
                single,
                sketch,
                body,
                name,
            } => format!(
                "{} of {sketch} {} inside {body}, so {name} adds nothing.",
                capital(curves),
                if *single { "lies" } else { "lie" }
            ),
            Self::MeetsNothing {
                name, body, curves, ..
            } => format!(
                "{name} meets no face of {body}: carried on from {curves}, it never reaches the \
                 body."
            ),
            Self::RunsPastEnd {
                name, curve, body, ..
            } => format!(
                "The end of {name} at {curve} runs on past {body} without meeting a face of it."
            ),
            Self::RunsPastDepth {
                curves,
                sketch,
                name,
                body,
            } => format!(
                "Running on from {curves} of {sketch}, {name} meets no face of {body} and would \
                 reach past it."
            ),
            Self::RunsPast {
                name, body, curves, ..
            } => format!(
                "On the side it fills, {name} is not closed off by faces of {body}: from {curves} \
                 it would run on past the body."
            ),
            Self::ThickBend { name, curves } => {
                format!("{name} is too thick for the bend of {curves}.")
            }
            Self::ArcsCross { name } => {
                format!("Carried on round its arcs, {name} runs into itself.")
            }
            Self::Crosses { name } => format!("Thickened, {name} runs into itself."),
            Self::DraftCloses { name, curves, body } => format!(
                "With its draft, {name} widens until the bend of {curves} closes up before it \
                 has run past {body}."
            ),
            Self::Unbuilt { name, body } => {
                format!("{name} cannot be built where it meets {body}.")
            }
            Self::Uncombined {
                name,
                body,
                trouble,
            } => trouble.reason(format!("{name} could not be joined to {body}.")),
            Self::DraftNegative { shape } => {
                format!("The draft of the {shape} cannot be negative.")
            }
            Self::DraftSteep { shape } => {
                format!("The draft of the {shape} is {MAX_RIB_DRAFT_DEGREES}° or steeper.")
            }
            Self::DraftShallow { shape } => {
                format!("The draft of the {shape} is under {MIN_FLANK_DEGREES}°.")
            }
            Self::SketchFailed { feature, name } => {
                format!("{feature} uses {name}, which has an error.")
            }
        }
    }

    pub(crate) fn remedy(&self) -> String {
        match self {
            Self::NoCurves { .. } => {
                "Draw the line or arc the rib runs along, as curves that are not construction \
                 geometry."
                    .to_owned()
            }
            Self::Closed { .. } => {
                "Leave a gap in the outline, or make the curves that close it construction \
                 geometry."
                    .to_owned()
            }
            Self::Branches { .. } => {
                "Draw the curves so that a chain passes each point where curves meet only once."
                    .to_owned()
            }
            Self::CurvesCross { .. } => {
                "Draw the curves so they do not turn back across themselves.".to_owned()
            }
            Self::NoExtent { body, .. } => format!("Fix {body} first."),
            Self::Behind { .. } => "Reverse its direction.".to_owned(),
            Self::Inside { .. } => {
                "Draw the curves where the rib should stand, outside the body's material."
                    .to_owned()
            }
            Self::MeetsNothing { direction, .. } | Self::RunsPast { direction, .. } => {
                Self::flip(*direction).to_owned()
            }
            Self::RunsPastEnd { ends, .. } => match ends {
                RibEnds::Straight => {
                    "Draw the curve so that, carried on straight past its end, it runs into a \
                     wall of the body."
                        .to_owned()
                }
                RibEnds::RoundArcs => {
                    "Draw the curve so that, carried on past its end (an arc round its circle), \
                     it runs into a wall of the body, or carry its ends on straight."
                        .to_owned()
                }
            },
            Self::RunsPastDepth { .. } => {
                "Reverse its direction, or move the curves over the body.".to_owned()
            }
            Self::ThickBend { .. } => "Make the rib thinner or the bend gentler.".to_owned(),
            Self::ArcsCross { .. } => {
                "Carry its ends on straight instead, or draw the arcs so that, carried on round \
                 their circles, they run into walls of the body."
                    .to_owned()
            }
            Self::Crosses { .. } => {
                "Make the rib thinner, or move its curves farther apart.".to_owned()
            }
            Self::DraftCloses { .. } => {
                "Make the draft smaller, or the bend of the curves gentler.".to_owned()
            }
            Self::Unbuilt { .. } => {
                "Move its curves slightly, so the rib does not run exactly along a face or edge \
                 of the body."
                    .to_owned()
            }
            Self::Uncombined { trouble, .. } => trouble.remedy(),
            Self::DraftNegative { .. } => format!(
                "Enter a draft angle from 0° up to below {MAX_RIB_DRAFT_DEGREES}°; the rib \
                 widens by it toward the body."
            ),
            Self::DraftSteep { .. } => {
                format!("Enter a draft angle from 0° up to below {MAX_RIB_DRAFT_DEGREES}°.")
            }
            Self::DraftShallow { .. } => format!(
                "Enter a draft angle of 0° for none, or from {MIN_FLANK_DEGREES}° up to below \
                 {MAX_RIB_DRAFT_DEGREES}°."
            ),
            Self::SketchFailed { name, .. } => format!("Fix {name} first."),
        }
    }

    fn flip(direction: RibDirection) -> &'static str {
        match direction {
            RibDirection::Parallel => {
                "Reverse the side it fills, or draw its curves so that, carried on straight at \
                 their ends, they run into the body."
            }
            RibDirection::Square => {
                "Reverse its direction, or draw its curves so that, carried on straight at their \
                 ends, they run into walls of the body."
            }
        }
    }
}

impl From<RibCause> for Cause {
    fn from(cause: RibCause) -> Self {
        Self::Rib(cause)
    }
}
