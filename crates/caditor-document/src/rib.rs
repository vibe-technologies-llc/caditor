use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId, Unit};
use caditor_kernel::{
    BooleanError, BooleanOperation, FaceOrigin, Solid, SweepError, WallError, boolean,
};
use caditor_sketch::{EntityId, Sketch};

use crate::{
    datum::feature_name,
    document::{Document, Feature, FeatureId, list_names},
    primitive::Context,
    recompute::{CancelToken, Evaluation, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    rib_tool::{
        RibDirection, RibEnds, RibError, RibInput, RibSide, ThicknessStand, chains, facing_body,
        is_run, rib_tools, run_curve, thickness_stand,
    },
    solid::{SolidResult, profile_curves},
    sweep::capital,
    trouble,
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

    fn failed(&self, reason: String, remedy: String, fix: FeatureId) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(fix)),
            constraints: Vec::new(),
            place: None,
        }))
    }

    fn own(&self, reason: String, remedy: &str) -> Failure {
        self.failed(reason, remedy.to_owned(), self.feature.id())
    }

    fn in_sketch(&self, reason: String, remedy: &str) -> Failure {
        self.failed(reason, remedy.to_owned(), self.definition.sketch)
    }

    fn flip_remedy(&self) -> &'static str {
        match self.definition.direction {
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

    fn failure(&self, error: RibError) -> Failure {
        let name = &self.feature.name;
        let sketch = &self.sketch_name;
        let body = &self.body_name;
        match error {
            RibError::NoCurves => self.in_sketch(
                format!("{sketch} has no curve for {name} to follow."),
                "Draw the line or arc the rib runs along, as curves that are not construction \
                 geometry.",
            ),
            RibError::Closed { entities } => self.in_sketch(
                format!(
                    "{} of {sketch} close on themselves, but a rib follows an open chain of \
                     curves.",
                    capital(&self.curves(&entities))
                ),
                "Leave a gap in the outline, or make the curves that close it construction \
                 geometry.",
            ),
            RibError::Branches { entities } => self.in_sketch(
                format!(
                    "{} of {sketch} come back to a point they already pass through, so {name} \
                     cannot tell which way to run.",
                    capital(&self.curves(&entities))
                ),
                "Draw the curves so that a chain passes each point where curves meet only once.",
            ),
            RibError::Degenerate { entities } | RibError::CrossesItself { entities } => self
                .in_sketch(
                    format!(
                        "{} of {sketch}, carried on straight past their ends, cross themselves, \
                         so they do not bound one side of the rib.",
                        capital(&self.curves(&entities))
                    ),
                    "Draw the curves so they do not turn back across themselves.",
                ),
            RibError::NoExtent => self.failed(
                format!("{body} has no shape for {name} to join."),
                format!("Fix {body} first."),
                self.definition.body,
            ),
            RibError::Behind => self.own(
                format!(
                    "{body} lies wholly on the other side of {sketch}, so {name} meets no face \
                     of the body."
                ),
                "Reverse its direction.",
            ),
            RibError::Inside { entities } => self.own(
                format!(
                    "{} of {sketch} {} inside {body}, so {name} adds nothing.",
                    capital(&self.curves(&entities)),
                    if entities.len() == 1 { "lies" } else { "lie" }
                ),
                "Draw the curves where the rib should stand, outside the body's material.",
            ),
            RibError::MeetsNothing { entities } => self.own(
                format!(
                    "{name} meets no face of {body}: carried on from {}, it never reaches the \
                     body.",
                    self.curves(&entities)
                ),
                self.flip_remedy(),
            ),
            RibError::RunsPastEnd { entity } => self.own(
                format!(
                    "The end of {name} at {} runs on past {body} without meeting a face of it.",
                    self.sketch.entity_label(EntityId::from_raw(entity))
                ),
                match self.definition.ends {
                    RibEnds::Straight => {
                        "Draw the curve so that, carried on straight past its end, it runs into a \
                         wall of the body."
                    }
                    RibEnds::RoundArcs => {
                        "Draw the curve so that, carried on past its end (an arc round its \
                         circle), it runs into a wall of the body, or carry its ends on straight."
                    }
                },
            ),
            RibError::RunsPastDepth { entities } => self.own(
                format!(
                    "Running on from {} of {sketch}, {name} meets no face of {body} and would \
                     reach past it.",
                    self.curves(&entities)
                ),
                "Reverse its direction, or move the curves over the body.",
            ),
            RibError::RunsPast { entities } => self.own(
                format!(
                    "On the side it fills, {name} is not closed off by faces of {body}: from {} \
                     it would run on past the body.",
                    self.curves(&entities)
                ),
                self.flip_remedy(),
            ),
            RibError::Sweep(SweepError::TaperedUnsupportedCurve { entities }) => {
                let mut drawn: Vec<u64> =
                    entities.iter().map(|entity| run_curve(*entity)).collect();
                drawn.sort_unstable();
                drawn.dedup();
                self.in_sketch(
                    format!(
                        "With a draft, {sketch} cannot give {name} a web along {}; a draft \
                         follows only lines and arcs.",
                        self.curves(&drawn)
                    ),
                    "Draw the web with lines and arcs, or set the draft to 0°.",
                )
            }
            RibError::Wall(WallError::TooThick { entities }) => self.own(
                format!(
                    "{name} is too thick for the bend of {}.",
                    self.curves(&entities)
                ),
                "Make the rib thinner or the bend gentler.",
            ),
            RibError::Wall(WallError::CrossesItself)
                if self.definition.ends == RibEnds::RoundArcs =>
            {
                self.own(
                    format!("Carried on round its arcs, {name} runs into itself."),
                    "Carry its ends on straight instead, or draw the arcs so that, carried on \
                     round their circles, they run into walls of the body.",
                )
            }
            RibError::Wall(WallError::CrossesItself) => self.own(
                format!("Thickened, {name} runs into itself."),
                "Make the rib thinner, or move its curves farther apart.",
            ),
            RibError::Sweep(SweepError::TaperCloses { entities }) if !entities.is_empty() => {
                let mut drawn: Vec<u64> =
                    entities.iter().map(|entity| run_curve(*entity)).collect();
                drawn.sort_unstable();
                drawn.dedup();
                self.own(
                    format!(
                        "With its draft, {name} widens until the bend of {} closes up before the \
                         web has run past {body}.",
                        self.curves(&drawn)
                    ),
                    "Make the draft smaller, or the bend of the curves gentler.",
                )
            }
            RibError::Wall(WallError::Cancelled(_)) => Failure::Cancelled,
            RibError::Boolean(BooleanError::Cancelled(_)) => Failure::Cancelled,
            other => {
                log::warn!("{name} could not be built: {other}");
                self.own(
                    format!("{name} cannot be built where it meets {body}."),
                    "Move its curves slightly, so the rib does not run exactly along a face or \
                     edge of the body.",
                )
            }
        }
    }
}

fn draft(context: &Context<'_>, expression: &Expression) -> Result<f64, Failure> {
    let degrees = context.value_as(expression, "draft angle", Dimension::ANGLE)?;
    let shape = context.shape();
    if degrees < 0.0 {
        return Err(context.error(
            format!("The draft of the {shape} cannot be negative."),
            format!(
                "Enter a draft angle from 0° up to below {MAX_RIB_DRAFT_DEGREES}°; the web \
                 widens by it toward the body."
            ),
        ));
    }
    if degrees >= MAX_RIB_DRAFT_DEGREES {
        return Err(context.error(
            format!("The draft of the {shape} is {MAX_RIB_DRAFT_DEGREES}° or steeper."),
            format!("Enter a draft angle from 0° up to below {MAX_RIB_DRAFT_DEGREES}°."),
        ));
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
            Failure::Error(Box::new(FeatureError {
                reason: format!("{} uses {name}, which has an error.", feature.name),
                remedy: format!("Fix {name} first."),
                fix: Some(FixTarget::Feature(sketch)),
                constraints: Vec::new(),
                place: None,
            }))
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
    let draft = match definition.direction {
        RibDirection::Square => draft(&context, &definition.draft)?,
        RibDirection::Parallel => 0.0,
    };
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
                "Move its curves",
            );
            log::warn!("{} could not be joined: {error}", feature.name);
            words
                .own(
                    trouble.reason(format!(
                        "{} could not be joined to {}.",
                        feature.name, words.body_name
                    )),
                    &trouble.remedy,
                )
                .placed(trouble.place)
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
