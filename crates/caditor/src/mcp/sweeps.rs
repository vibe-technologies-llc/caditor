use caditor_document::{
    BodyOperation, Bore, Coil, CoilLength, CoilTurning, DEFAULT_PITCH, DEFAULT_TURNS, FeatureKind,
    Loft, LoftSection, Pipe, Rib, RibDirection, RibEnds, RibSide, SolidFeature, Sweep, SweepPath,
    Thread, ThreadFamily, ThreadForm, ThreadHand, ThreadLength, ThreadSide, ThreadSize,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::{
    CoilPlacement, CoilShape, FaceReference, LoftSkin, PipeShape, SweepOrientation,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    bodies, coil_tools, loft_tools,
    mcp::{
        features::{
            Amount, Built, FeatureError, added, amount, body_of, count, edge_references, length,
            needed, operation_asked, parsed, sketch_of, turn,
        },
        places::{self, AxisChoice},
        references::{self, EntityName, Named, body_label},
        session::{Session, Work},
        solids::{FormName, HandName},
        tools::{Answer, ToolError},
        writing::{OperationName, operation_words},
    },
    pipe_tools, rib_tools,
    solid_tools::{self, Reach},
    sweep_tools, thread_tools,
    units::LengthUnit,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchPath {
    pub sketch: Named,
    pub curves: Vec<EntityName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgePath {
    pub body: Named,
    pub edges: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelixPath {
    pub helix: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PathChoice {
    Curves(SketchPath),
    Edges(EdgePath),
    Helix(HelixPath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientationName {
    Square,
    Parallel,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sweeping {
    pub sketch: Option<Named>,
    pub path: Option<PathChoice>,
    pub orientation: Option<OrientationName>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeShapeName {
    Round,
    Square,
    Triangle,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piping {
    pub path: Option<PathChoice>,
    pub shape: Option<PipeShapeName>,
    pub size: Option<Amount>,
    pub wall: Option<Amount>,
    pub hollow: Option<bool>,
    pub bend: Option<Amount>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinName {
    Smooth,
    Ruled,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lofting {
    pub profiles: Option<Vec<Named>>,
    pub skin: Option<SkinName>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoilShapeName {
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementName {
    Inside,
    Centred,
    Outside,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coiling {
    pub axis: Option<AxisChoice>,
    pub reversed: Option<bool>,
    pub diameter: Option<Amount>,
    pub turns: Option<Amount>,
    pub pitch: Option<Amount>,
    pub height: Option<Amount>,
    pub shape: Option<CoilShapeName>,
    pub section: Option<Amount>,
    pub placement: Option<PlacementName>,
    pub taper: Option<Amount>,
    pub clockwise: Option<bool>,
    pub operation: Option<OperationName>,
    pub body: Option<Named>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RibSideName {
    Symmetric,
    OneSide,
    OtherSide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RibDirectionName {
    Parallel,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RibEndsName {
    Straight,
    RoundArcs,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ribbing {
    pub sketch: Option<Named>,
    pub body: Option<Named>,
    pub thickness: Option<Amount>,
    pub side: Option<RibSideName>,
    pub direction: Option<RibDirectionName>,
    pub draft: Option<Amount>,
    pub ends: Option<RibEndsName>,
    pub reversed: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum FamilyName {
    #[serde(rename = "metric_coarse")]
    MetricCoarse,
    #[serde(rename = "metric_fine")]
    MetricFine,
    #[serde(rename = "iso_228")]
    ParallelPipe,
    #[serde(rename = "iso_7")]
    TaperPipe,
    #[serde(rename = "trapezoidal")]
    Trapezoidal,
}

impl FamilyName {
    fn family(self) -> ThreadFamily {
        match self {
            Self::MetricCoarse => ThreadFamily::MetricCoarse,
            Self::MetricFine => ThreadFamily::MetricFine,
            Self::ParallelPipe => ThreadFamily::ParallelPipe,
            Self::TaperPipe => ThreadFamily::TaperPipe,
            Self::Trapezoidal => ThreadFamily::Trapezoidal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threading {
    pub body: Option<Named>,
    pub face: Option<String>,
    pub family: Option<FamilyName>,
    pub size: Option<String>,
    pub class: Option<String>,
    pub hand: Option<HandName>,
    pub length: Option<Amount>,
    pub full: Option<bool>,
    pub reversed: Option<bool>,
    pub form: Option<FormName>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SweepToolError {
    #[error("give the path's curves: entity ids of lines, arcs or curves from get_sketch")]
    NoPathCurves,
    #[error("give the path's edges: keys from list_edges")]
    NoPathEdges,
    #[error("{entity} of {sketch} is not a line, arc or curve a path can follow")]
    NotAPathCurve { sketch: String, entity: EntityName },
    #[error("{name} is not a helix; list_features names each helix")]
    NotAHelix { name: String },
    #[error("a loft goes through at least two sketches, given in order")]
    TooFewProfiles,
    #[error("give the wall's thickness or hollow false, not both")]
    WallAndSolid,
    #[error("a coil is sized by two of turns, pitch and height, not all three")]
    ThreeCoilSizes,
    #[error("there is no body for the rib to join; make one first or give body")]
    NoBodyForRib,
    #[error("face {face} of {body} is not a cylinder: a thread goes on a bore, a shaft or a boss")]
    NotRound { body: String, face: String },
    #[error("{family} has no size “{size}”; its sizes include {sizes}")]
    UnknownSize {
        family: &'static str,
        size: String,
        sizes: String,
    },
    #[error("{family} has no class “{class}”; its classes are {classes}")]
    UnknownClass {
        family: &'static str,
        class: String,
        classes: String,
    },
    #[error("give the thread a length or full, not both")]
    LengthAndFull,
}

pub(super) fn path(session: &Session, choice: &PathChoice) -> Result<SweepPath, ToolError> {
    let document = session.document();
    match choice {
        PathChoice::Curves(asked) => {
            let (feature, sketch) = references::sketch(document, &asked.sketch)?;
            if asked.curves.is_empty() {
                return Err(SweepToolError::NoPathCurves.into());
            }
            let mut curves = Vec::with_capacity(asked.curves.len());
            for named in &asked.curves {
                let curve = references::entity(&feature.name, sketch, named)?;
                if !sweep_tools::is_path_curve(document, feature.id(), curve) {
                    return Err(SweepToolError::NotAPathCurve {
                        sketch: feature.name.clone(),
                        entity: named.clone(),
                    }
                    .into());
                }
                curves.push(curve);
            }
            Ok(SweepPath::Curves {
                sketch: feature.id(),
                curves,
            })
        }
        PathChoice::Edges(asked) => {
            if asked.edges.is_empty() {
                return Err(SweepToolError::NoPathEdges.into());
            }
            let body = references::body(document, session.evaluation(), &asked.body)?;
            Ok(SweepPath::Edges {
                body,
                edges: edge_references(session, body, &asked.edges)?,
            })
        }
        PathChoice::Helix(asked) => {
            let feature = references::feature(document, &asked.helix)?;
            if !matches!(feature.kind, FeatureKind::Helix(_)) {
                return Err(SweepToolError::NotAHelix {
                    name: feature.name.clone(),
                }
                .into());
            }
            Ok(SweepPath::Helix {
                helix: feature.id(),
            })
        }
    }
}

fn kept_path(
    session: &Session,
    asked: Option<&PathChoice>,
    kept: Option<&SweepPath>,
) -> Result<SweepPath, ToolError> {
    match asked {
        Some(choice) => path(session, choice),
        None => needed(kept.cloned(), "path"),
    }
}

pub fn sweep_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Sweeping,
    base: Option<&Sweep>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let values = session.parameters();
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, _, _) = sketch_of(
        document,
        asked.sketch.as_ref(),
        base.map(|base| base.sketch),
    )?;
    let path = kept_path(session, asked.path.as_ref(), base.map(|base| &base.path))?;
    let mut sweep = match base {
        Some(base) => Sweep {
            sketch,
            path,
            regions: if base.sketch == sketch {
                base.regions.clone()
            } else {
                caditor_document::RegionChoice::All
            },
            ..base.clone()
        },
        None => Sweep::new(sketch, path),
    };
    if let Some(orientation) = asked.orientation {
        sweep.orientation = match orientation {
            OrientationName::Square => SweepOrientation::Square,
            OrientationName::Parallel => SweepOrientation::Parallel,
        };
    }
    let asked_operation = operation_asked(
        session,
        (asked.operation, asked.body.as_ref()),
        base.map(|base| base.operation),
    )?;
    let solid = SolidFeature::Sweep(sweep);
    let operation = asked_operation.unwrap_or_else(|| {
        solid_tools::default_operation(
            document,
            Reach {
                evaluation,
                parameters: &values,
            },
            &solid,
        )
    });
    let solid = solid_tools::with_operation(&solid, operation);
    Ok(
        Built::new(FeatureKind::Solid(solid), sweep_tools::TITLE, asked.name)
            .hiding(sketch)
            .telling("operation", json!(operation_words(document, operation))),
    )
}

pub fn sweep(session: &mut Session, work: &Work<'_>, asked: Sweeping) -> Result<Answer, ToolError> {
    let built = sweep_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn pipe_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Piping,
    base: Option<&Pipe>,
) -> Result<Built, ToolError> {
    let size = parsed(session, asked.size.as_ref(), |session, size| {
        length(session, "size", size)
    })?;
    let wall = parsed(session, asked.wall.as_ref(), |session, wall| {
        length(session, "wall", wall)
    })?;
    let bend = parsed(session, asked.bend.as_ref(), |session, bend| {
        length(session, "bend", bend)
    })?;
    if wall.is_some() && asked.hollow == Some(false) {
        return Err(SweepToolError::WallAndSolid.into());
    }
    session.evaluate(work)?;
    let unit = LengthUnit::Millimetre;
    let path = kept_path(session, asked.path.as_ref(), base.map(|base| &base.path))?;
    let path_body = path.body();
    let mut pipe = match base {
        Some(base) => Pipe {
            path,
            ..base.clone()
        },
        None => Pipe::new(
            path,
            unit.default_length(pipe_tools::DEFAULT_SIZE),
            unit.default_length(pipe_tools::DEFAULT_BEND),
        ),
    };
    if let Some(shape) = asked.shape {
        pipe.shape = match shape {
            PipeShapeName::Round => PipeShape::Round,
            PipeShapeName::Square => PipeShape::Square,
            PipeShapeName::Triangle => PipeShape::Triangle,
        };
    }
    if let Some(size) = size {
        pipe.size = size;
    }
    if let Some(bend) = bend {
        pipe.bend = bend;
    }
    match (wall, asked.hollow) {
        (Some(wall), _) => pipe.wall = Some(wall),
        (None, Some(false)) => pipe.wall = None,
        (None, Some(true)) if pipe.wall.is_none() => {
            pipe = pipe_tools::hollowed(&pipe, true, unit);
        }
        (None, Some(true) | None) => {}
    }
    pipe.operation = operation_asked(
        session,
        (asked.operation, asked.body.as_ref()),
        base.map(|base| base.operation),
    )?
    .unwrap_or_else(|| path_body.map_or(BodyOperation::NewBody, BodyOperation::Add));
    let document = session.document();
    let operation = pipe.operation;
    Ok(
        Built::new(FeatureKind::Pipe(pipe), pipe_tools::TITLE, asked.name)
            .telling("operation", json!(operation_words(document, operation))),
    )
}

pub fn pipe(session: &mut Session, work: &Work<'_>, asked: Piping) -> Result<Answer, ToolError> {
    let built = pipe_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn loft_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Lofting,
    base: Option<&Loft>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let profiles = match (&asked.profiles, base) {
        (Some(named), _) => {
            let mut profiles = Vec::with_capacity(named.len());
            for sketch in named {
                let (feature, _) = references::sketch(document, sketch)?;
                profiles.push(LoftSection::whole(feature.id()));
            }
            let least = if base.is_some_and(|base| base.end.is_some()) {
                1
            } else {
                2
            };
            if profiles.len() < least {
                return Err(SweepToolError::TooFewProfiles.into());
            }
            profiles
        }
        (None, Some(base)) => base.profiles.clone(),
        (None, None) => return Err(FeatureError::Missing { field: "profiles" }.into()),
    };
    let hidden: Vec<_> = profiles.iter().map(|section| section.sketch).collect();
    let mut loft = match base {
        Some(base) => Loft {
            profiles,
            ..base.clone()
        },
        None => Loft::new(profiles),
    };
    if let Some(skin) = asked.skin {
        loft.skin = match skin {
            SkinName::Smooth => LoftSkin::Smooth,
            SkinName::Ruled => LoftSkin::Ruled,
        };
    }
    loft.operation = operation_asked(
        session,
        (asked.operation, asked.body.as_ref()),
        base.map(|base| base.operation),
    )?
    .unwrap_or(BodyOperation::NewBody);
    let operation = loft.operation;
    let built = Built::new(FeatureKind::Loft(loft), loft_tools::TITLE, asked.name)
        .telling("operation", json!(operation_words(document, operation)));
    Ok(hidden.into_iter().fold(built, Built::hiding))
}

pub fn loft(session: &mut Session, work: &Work<'_>, asked: Lofting) -> Result<Answer, ToolError> {
    let built = loft_built(session, work, asked, None)?;
    added(session, work, built)
}

pub(super) fn coil_length(
    kept: &CoilLength,
    (turns, pitch, height): (Option<Expression>, Option<Expression>, Option<Expression>),
) -> Result<CoilLength, ToolError> {
    if turns.is_some() && pitch.is_some() && height.is_some() {
        return Err(SweepToolError::ThreeCoilSizes.into());
    }
    let (kept_turns, kept_pitch, kept_height) = match kept {
        CoilLength::TurnsAndPitch { turns, pitch } => {
            (Some(turns.clone()), Some(pitch.clone()), None)
        }
        CoilLength::HeightAndTurns { height, turns } => {
            (Some(turns.clone()), None, Some(height.clone()))
        }
        CoilLength::HeightAndPitch { height, pitch } => {
            (None, Some(pitch.clone()), Some(height.clone()))
        }
    };
    let pitch_given = pitch.is_some();
    let turns_given = turns.is_some();
    let mut turns = turns.or(kept_turns);
    let mut pitch = pitch.or(kept_pitch);
    let mut height = height.or(kept_height);
    if turns.is_some() && pitch.is_some() && height.is_some() {
        if !pitch_given {
            pitch = None;
        } else if !turns_given {
            turns = None;
        } else {
            height = None;
        }
    }
    Ok(match (turns, pitch, height) {
        (Some(turns), Some(pitch), None) => CoilLength::TurnsAndPitch { turns, pitch },
        (Some(turns), None, Some(height)) => CoilLength::HeightAndTurns { height, turns },
        (None, Some(pitch), Some(height)) => CoilLength::HeightAndPitch { height, pitch },
        _ => kept.clone(),
    })
}

pub fn coil_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Coiling,
    base: Option<&Coil>,
) -> Result<Built, ToolError> {
    let diameter = parsed(session, asked.diameter.as_ref(), |session, size| {
        length(session, "diameter", size)
    })?;
    let section = parsed(session, asked.section.as_ref(), |session, size| {
        length(session, "section", size)
    })?;
    let turns = parsed(session, asked.turns.as_ref(), |session, turns| {
        count(session, "turns", turns)
    })?;
    let pitch = parsed(session, asked.pitch.as_ref(), |session, pitch| {
        length(session, "pitch", pitch)
    })?;
    let height = parsed(session, asked.height.as_ref(), |session, height| {
        length(session, "height", height)
    })?;
    let taper = parsed(session, asked.taper.as_ref(), |session, taper| {
        turn(session, "taper", taper)
    })?;
    session.evaluate(work)?;
    let unit = LengthUnit::Millimetre;
    let mut coil = match base {
        Some(base) => base.clone(),
        None => Coil {
            length: CoilLength::TurnsAndPitch {
                turns: Expression::number(DEFAULT_TURNS),
                pitch: unit.default_length(DEFAULT_PITCH),
            },
            ..Coil::new(
                coil_tools::DEFAULT_AXIS,
                unit.default_length(coil_tools::DEFAULT_DIAMETER),
                unit.default_length(coil_tools::DEFAULT_SECTION),
            )
        },
    };
    if let Some(choice) = &asked.axis {
        coil.axis = places::axis(session.document(), session.evaluation(), choice)?;
    }
    if let Some(reversed) = asked.reversed {
        coil.reversed = reversed;
    }
    if let Some(diameter) = diameter {
        coil.diameter = diameter;
    }
    if let Some(section) = section {
        coil.size = section;
    }
    if let Some(taper) = taper {
        coil.taper = taper;
    }
    coil.length = coil_length(&coil.length, (turns, pitch, height))?;
    if let Some(shape) = asked.shape {
        coil.shape = match shape {
            CoilShapeName::Round => CoilShape::Round,
            CoilShapeName::Square => CoilShape::Square,
        };
    }
    if let Some(placement) = asked.placement {
        coil.placement = match placement {
            PlacementName::Inside => CoilPlacement::Inside,
            PlacementName::Centred => CoilPlacement::Centred,
            PlacementName::Outside => CoilPlacement::Outside,
        };
    }
    if let Some(clockwise) = asked.clockwise {
        coil.turning = if clockwise {
            CoilTurning::Clockwise
        } else {
            CoilTurning::CounterClockwise
        };
    }
    coil.operation = operation_asked(
        session,
        (asked.operation, asked.body.as_ref()),
        base.map(|base| base.operation),
    )?
    .unwrap_or(BodyOperation::NewBody);
    let operation = coil.operation;
    Ok(
        Built::new(FeatureKind::Coil(coil), coil_tools::TITLE, asked.name).telling(
            "operation",
            json!(operation_words(session.document(), operation)),
        ),
    )
}

pub fn coil(session: &mut Session, work: &Work<'_>, asked: Coiling) -> Result<Answer, ToolError> {
    let built = coil_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn rib_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Ribbing,
    base: Option<&Rib>,
) -> Result<Built, ToolError> {
    let thickness = parsed(session, asked.thickness.as_ref(), |session, thickness| {
        length(session, "thickness", thickness)
    })?;
    let draft = parsed(session, asked.draft.as_ref(), |session, draft| {
        turn(session, "draft", draft)
    })?;
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, _, _) = sketch_of(
        document,
        asked.sketch.as_ref(),
        base.map(|base| base.sketch),
    )?;
    let attached = document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body());
    let body = match (&asked.body, base) {
        (Some(named), _) => references::body(document, evaluation, named)?,
        (None, Some(base)) => base.body,
        (None, None) => {
            let standing = document.bodies_standing_in(evaluation);
            attached
                .filter(|body| standing.contains(body))
                .or_else(|| standing.last().copied())
                .ok_or(SweepToolError::NoBodyForRib)?
        }
    };
    let mut rib = match base {
        Some(base) => Rib {
            sketch,
            body,
            ..base.clone()
        },
        None => Rib {
            direction: if attached.is_some() {
                RibDirection::Square
            } else {
                RibDirection::Parallel
            },
            ..Rib::new(
                sketch,
                body,
                LengthUnit::Millimetre.default_length(rib_tools::DEFAULT_THICKNESS),
            )
        },
    };
    if let Some(thickness) = thickness {
        rib.thickness = thickness;
    }
    if let Some(side) = asked.side {
        rib.side = match side {
            RibSideName::Symmetric => RibSide::Symmetric,
            RibSideName::OneSide => RibSide::OneSide,
            RibSideName::OtherSide => RibSide::OtherSide,
        };
    }
    if let Some(direction) = asked.direction {
        rib.direction = match direction {
            RibDirectionName::Parallel => RibDirection::Parallel,
            RibDirectionName::Square => RibDirection::Square,
        };
    }
    if let Some(draft) = draft {
        rib.draft = draft;
    }
    if let Some(ends) = asked.ends {
        rib.ends = match ends {
            RibEndsName::Straight => RibEnds::Straight,
            RibEndsName::RoundArcs => RibEnds::RoundArcs,
        };
    }
    let rib = match (asked.reversed, base) {
        (Some(reversed), _) => Rib { reversed, ..rib },
        (None, Some(_)) => rib,
        (None, None) => rib_tools::facing_body(evaluation, rib, None),
    };
    Ok(
        Built::new(FeatureKind::Rib(rib), rib_tools::TITLE, asked.name)
            .telling("body", json!(body_label(document, body))),
    )
}

pub fn rib(session: &mut Session, work: &Work<'_>, asked: Ribbing) -> Result<Answer, ToolError> {
    let built = rib_built(session, work, asked, None)?;
    added(session, work, built)
}

struct ThreadFace {
    reference: FaceReference,
    bore: Option<Bore>,
    described: String,
}

fn thread_face(
    session: &Session,
    body: caditor_document::FeatureId,
    key: &str,
) -> Result<ThreadFace, ToolError> {
    let document = session.document();
    let (_, face, shown) = references::face(document, session.evaluation(), body, key)?;
    let not_round = || SweepToolError::NotRound {
        body: body_label(document, body),
        face: key.to_owned(),
    };
    let reference = FaceReference::capture(&shown.solid, face).ok_or_else(not_round)?;
    let bore = Bore::of(&shown.solid, &[face]).map_err(|_| not_round())?;
    Ok(ThreadFace {
        reference,
        bore: Some(bore),
        described: bodies::describe_face_id(document, shown, face),
    })
}

fn kept_bore(session: &Session, base: &Thread) -> Option<Bore> {
    let shown = bodies::shown(session.evaluation(), base.body)?;
    let face = base.face.resolve(&shown.solid).ok()?;
    Bore::of(&shown.solid, &[face]).ok()
}

fn sizes_words(family: ThreadFamily) -> String {
    family
        .sizes()
        .iter()
        .take(12)
        .map(|size| size.id())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn thread_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Threading,
    base: Option<&Thread>,
) -> Result<Built, ToolError> {
    if asked.length.is_some() && asked.full.is_some() {
        return Err(SweepToolError::LengthAndFull.into());
    }
    let depth = parsed(session, asked.length.as_ref(), |session, depth| {
        amount(session, "length", depth, Dimension::LENGTH, true)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let face = match (&asked.face, base) {
        (Some(key), _) => Some(thread_face(session, body, key)?),
        (None, Some(_)) => None,
        (None, None) => return Err(FeatureError::Missing { field: "face" }.into()),
    };
    let bore = match (&face, base) {
        (Some(face), _) => face.bore,
        (None, Some(base)) => kept_bore(session, base),
        (None, None) => None,
    };
    let side = bore.as_ref().map_or(ThreadSide::Internal, |bore| bore.side);
    let family = asked
        .family
        .map(FamilyName::family)
        .or_else(|| base.map(|base| base.size.family()))
        .unwrap_or(ThreadFamily::MetricCoarse);
    let size = match (&asked.size, base) {
        (Some(id), _) => {
            ThreadSize::from_id(family, id.trim()).ok_or_else(|| SweepToolError::UnknownSize {
                family: family.label(),
                size: id.clone(),
                sizes: sizes_words(family),
            })?
        }
        (None, Some(base)) if base.size.family() == family && face.is_none() => base.size,
        (None, _) => bore.as_ref().map_or_else(
            || family.sizes().first().copied().unwrap_or(ThreadSize::M8),
            |bore| family.nearest(bore.diameter, bore.side),
        ),
    };
    let class = match (&asked.class, base) {
        (Some(id), _) => {
            family
                .class_from_id(id.trim())
                .ok_or_else(|| SweepToolError::UnknownClass {
                    family: family.label(),
                    class: id.clone(),
                    classes: family
                        .classes(side)
                        .iter()
                        .map(|class| class.id())
                        .collect::<Vec<_>>()
                        .join(", "),
                })?
        }
        (None, Some(base)) => family
            .class_from_id(base.class.id())
            .unwrap_or_else(|| family.default_class(side)),
        (None, None) => family.default_class(side),
    };
    let length = match (depth, asked.full, base) {
        (Some(depth), _, _) => ThreadLength::Depth(depth),
        (None, Some(_), _) | (None, None, None) => ThreadLength::Full,
        (None, None, Some(base)) => base.length.clone(),
    };
    let thread = Thread {
        body,
        face: match (&face, base) {
            (Some(face), _) => face.reference.clone(),
            (None, Some(base)) => base.face.clone(),
            (None, None) => return Err(FeatureError::Missing { field: "face" }.into()),
        },
        size,
        class,
        hand: asked
            .hand
            .map(HandName::hand)
            .or_else(|| base.map(|base| base.hand))
            .unwrap_or(ThreadHand::Right),
        length,
        reversed: asked
            .reversed
            .or_else(|| base.map(|base| base.reversed))
            .unwrap_or(false),
        form: asked
            .form
            .map(FormName::form)
            .or_else(|| base.map(|base| base.form))
            .unwrap_or(ThreadForm::Modelled),
    };
    let document = session.document();
    let name = match (asked.name, &face, base) {
        (Some(name), _, _) => Some(name),
        (None, Some(face), None) => Some(thread_tools::unique_name(
            document,
            &format!("{} on {}", thread_tools::TITLE, face.described),
        )),
        (None, _, _) => None,
    };
    let designation = thread.designation().text();
    Ok(
        Built::new(FeatureKind::Thread(thread), thread_tools::TITLE, name)
            .telling("body", json!(body_label(document, body)))
            .telling("thread", json!(designation)),
    )
}

pub fn thread(
    session: &mut Session,
    work: &Work<'_>,
    asked: Threading,
) -> Result<Answer, ToolError> {
    let built = thread_built(session, work, asked, None)?;
    added(session, work, built)
}
