use caditor_document::{
    AxisReference, DeleteFace, Draft, Emboss, EmbossForm, FaceAngle, FeatureId, FeatureKind,
    MoveFace, RegionChoice, ReplaceFace, Scale, Split, SplitAlong, SplitCarry, SplitFace,
};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_kernel::FaceReference;
use serde::Deserialize;
use serde_json::json;

use crate::{
    delete_face_tools, draft_tools, emboss_tools,
    mcp::{
        features::{
            Amount, Built, FeatureError, OffName, RegionsAsk, added, amount, body_of,
            face_references, length, needed, parsed, regions_chosen, signed_length, sketch_of,
            turn,
        },
        places::{self, AxisChoice, PlaneChoice},
        references::{self, Named, body_label},
        session::{Session, Work},
        tools::{Answer, ToolError},
    },
    move_face_tools, replace_face_tools, scale_tools, split_face_tools,
    units::LengthUnit,
};

const SPLIT_TITLE: &str = "Split";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlongSketch {
    pub sketch: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlongBody {
    pub tool: Named,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum AlongChoice {
    Plane(PlaneChoice),
    Sketch(AlongSketch),
    Body(AlongBody),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarryName {
    Square,
    Wrapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum CarryChoice {
    Named(CarryName),
    Along(AxisChoice),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSplitting {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub along: Option<AlongChoice>,
    pub carry: Option<CarryChoice>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceDeleting {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceReplacing {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub target: Option<PlaneChoice>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbossFormName {
    Raise,
    Recess,
}

impl EmbossFormName {
    fn form(self) -> EmbossForm {
        match self {
            Self::Raise => EmbossForm::Raised,
            Self::Recess => EmbossForm::Recessed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embossing {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub sketch: Option<Named>,
    pub regions: Option<RegionsAsk>,
    pub depth: Option<Amount>,
    pub draft: Option<Amount>,
    pub form: Option<EmbossFormName>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionName {
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum DirectionChoice {
    Named(DirectionName),
    Along(AxisChoice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TiltName {
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum TiltChoice {
    Named(TiltName),
    About(AxisChoice),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceMoving {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub distance: Option<Amount>,
    pub direction: Option<DirectionChoice>,
    pub angle: Option<Amount>,
    pub axis: Option<TiltChoice>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum BeyondAsk {
    Off(OffName),
    Angle(Amount),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnAngle {
    pub face: String,
    pub angle: Amount,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceDrafting {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub angle: Option<Amount>,
    pub neutral: Option<PlaneChoice>,
    pub flipped: Option<bool>,
    pub tangent: Option<bool>,
    pub beyond: Option<BeyondAsk>,
    pub own_angles: Option<Vec<OwnAngle>>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Splitting {
    pub body: Option<Named>,
    pub along: Option<AlongChoice>,
    pub flipped: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaling {
    pub body: Option<Named>,
    pub factor: Option<Amount>,
    pub center: Option<[Amount; 3]>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReshapeError {
    #[error("give the faces to {verb}, as keys from list_faces")]
    NoFaces { verb: &'static str },
    #[error("{body} cannot be split along itself; give another body as the tool")]
    AlongItself { body: String },
    #[error("carry wrapped or along an axis goes with curves of a sketch: give along {{sketch}}")]
    CarryWithoutSketch,
}

fn along(
    session: &Session,
    body: FeatureId,
    choice: &AlongChoice,
) -> Result<SplitAlong, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    match choice {
        AlongChoice::Plane(plane) => Ok(SplitAlong::Plane(places::plane(
            document, evaluation, plane,
        )?)),
        AlongChoice::Sketch(asked) => {
            let (feature, _) = references::sketch(document, &asked.sketch)?;
            Ok(SplitAlong::Sketch(feature.id()))
        }
        AlongChoice::Body(asked) => {
            let tool = references::body(document, evaluation, &asked.tool)?;
            if tool == body {
                return Err(ReshapeError::AlongItself {
                    body: body_label(document, body),
                }
                .into());
            }
            Ok(SplitAlong::Body(tool))
        }
    }
}

fn kept_along(
    session: &Session,
    body: FeatureId,
    asked: Option<&AlongChoice>,
    kept: Option<&SplitAlong>,
) -> Result<SplitAlong, ToolError> {
    match asked {
        Some(choice) => along(session, body, choice),
        None => needed(kept.cloned(), "along"),
    }
}

fn faces_of(
    session: &Session,
    body: FeatureId,
    (asked, kept): (Option<&[String]>, Option<&[FaceReference]>),
    verb: &'static str,
) -> Result<Vec<FaceReference>, ToolError> {
    match (asked, kept) {
        (Some([]), _) | (None, None) => Err(ReshapeError::NoFaces { verb }.into()),
        (Some(keys), _) => face_references(session, body, keys),
        (None, Some(kept)) => Ok(kept.to_vec()),
    }
}

pub fn split_face_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceSplitting,
    base: Option<&SplitFace>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = faces_of(
        session,
        body,
        (
            asked.faces.as_deref(),
            base.map(|base| base.faces.as_slice()),
        ),
        "split",
    )?;
    let along = kept_along(
        session,
        body,
        asked.along.as_ref(),
        base.map(|base| &base.along),
    )?;
    let carry = match &asked.carry {
        None => None,
        Some(CarryChoice::Named(CarryName::Square)) => Some(SplitCarry::Square),
        Some(CarryChoice::Named(CarryName::Wrapped)) => Some(SplitCarry::Wrapped),
        Some(CarryChoice::Along(choice)) => Some(SplitCarry::Along(Box::new(places::axis(
            session.document(),
            session.evaluation(),
            choice,
        )?))),
    };
    if carry
        .as_ref()
        .is_some_and(|carry| *carry != SplitCarry::Square)
        && along.sketch().is_none()
    {
        return Err(ReshapeError::CarryWithoutSketch.into());
    }
    let mut split = match base {
        Some(base) => SplitFace {
            body,
            faces,
            ..base.with_along(along)
        },
        None => SplitFace {
            body,
            faces,
            along,
            carry: SplitCarry::Square,
        },
    };
    if let Some(carry) = carry {
        split.carry = carry;
    }
    let label = body_label(session.document(), body);
    Ok(Built::new(
        FeatureKind::SplitFace(split),
        split_face_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(label)))
}

pub fn split_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceSplitting,
) -> Result<Answer, ToolError> {
    let built = split_face_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn delete_face_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceDeleting,
    base: Option<&DeleteFace>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = faces_of(
        session,
        body,
        (
            asked.faces.as_deref(),
            base.map(|base| base.faces.as_slice()),
        ),
        "delete",
    )?;
    let label = body_label(session.document(), body);
    Ok(Built::new(
        FeatureKind::DeleteFace(DeleteFace { body, faces }),
        delete_face_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(label)))
}

pub fn replace_face_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceReplacing,
    base: Option<&ReplaceFace>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = faces_of(
        session,
        body,
        (
            asked.faces.as_deref(),
            base.map(|base| base.faces.as_slice()),
        ),
        "replace",
    )?;
    let target = match &asked.target {
        Some(choice) => Some(places::plane(
            session.document(),
            session.evaluation(),
            choice,
        )?),
        None => base.and_then(|base| base.target.clone()),
    };
    let label = body_label(session.document(), body);
    Ok(Built::new(
        FeatureKind::ReplaceFace(ReplaceFace {
            body,
            faces,
            target,
        }),
        replace_face_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(label)))
}

pub fn replace_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceReplacing,
) -> Result<Answer, ToolError> {
    let built = replace_face_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn emboss_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Embossing,
    base: Option<&Emboss>,
) -> Result<Built, ToolError> {
    let depth = parsed(session, asked.depth.as_ref(), |session, depth| {
        length(session, "depth", depth)
    })?;
    let draft = parsed(session, asked.draft.as_ref(), |session, draft| {
        turn(session, "draft", draft)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = faces_of(
        session,
        body,
        (
            asked.faces.as_deref(),
            base.map(|base| base.faces.as_slice()),
        ),
        "emboss",
    )?;
    let (sketch, sketch_name, _) = sketch_of(
        session.document(),
        asked.sketch.as_ref(),
        base.and_then(|base| base.sketch),
    )?;
    let regions = regions_chosen(
        session.evaluation(),
        (sketch, sketch_name),
        asked.regions.as_ref(),
        match base {
            Some(base) if base.sketch == Some(sketch) => base.regions.clone(),
            Some(_) | None => RegionChoice::All,
        },
    )?;
    let emboss = Emboss {
        body,
        faces,
        sketch: Some(sketch),
        regions,
        depth: needed(
            depth.or_else(|| base.map(|base| base.depth.clone())),
            "depth",
        )?,
        draft: draft
            .or_else(|| base.map(|base| base.draft.clone()))
            .unwrap_or_else(Emboss::undrafted),
        form: asked
            .form
            .map(EmbossFormName::form)
            .or_else(|| base.map(|base| base.form))
            .unwrap_or_default(),
    };
    let label = body_label(session.document(), body);
    Ok(
        Built::new(FeatureKind::Emboss(emboss), emboss_tools::TITLE, asked.name)
            .telling("body", json!(label)),
    )
}

pub fn emboss(
    session: &mut Session,
    work: &Work<'_>,
    asked: Embossing,
) -> Result<Answer, ToolError> {
    let built = emboss_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn delete_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceDeleting,
) -> Result<Answer, ToolError> {
    let built = delete_face_built(session, work, asked, None)?;
    added(session, work, built)
}

fn axis_of(session: &Session, choice: &AxisChoice) -> Result<AxisReference, ToolError> {
    places::axis(session.document(), session.evaluation(), choice)
}

pub fn move_face_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceMoving,
    base: Option<&MoveFace>,
) -> Result<Built, ToolError> {
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        signed_length(session, "distance", distance)
    })?;
    let angle = parsed(session, asked.angle.as_ref(), |session, angle| {
        turn(session, "angle", angle)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let faces = faces_of(
        session,
        body,
        (
            asked.faces.as_deref(),
            base.map(|base| base.faces.as_slice()),
        ),
        "move",
    )?;
    let direction = match &asked.direction {
        None => base.and_then(|base| base.direction.clone()),
        Some(DirectionChoice::Named(DirectionName::Normal)) => None,
        Some(DirectionChoice::Along(choice)) => Some(axis_of(session, choice)?),
    };
    let axis = match &asked.axis {
        None => base.and_then(|base| base.axis.clone()),
        Some(TiltChoice::Named(TiltName::None)) => None,
        Some(TiltChoice::About(choice)) => Some(axis_of(session, choice)?),
    };
    let moving = MoveFace {
        body,
        faces,
        distance: distance
            .or_else(|| base.map(|base| base.distance.clone()))
            .unwrap_or(Expression::Measure(0.0, Unit::Millimetre)),
        direction,
        angle: angle
            .or_else(|| base.map(|base| base.angle.clone()))
            .unwrap_or(Expression::Measure(0.0, Unit::Degree)),
        axis,
    };
    let label = body_label(session.document(), body);
    Ok(Built::new(
        FeatureKind::MoveFace(moving),
        move_face_tools::TITLE,
        asked.name,
    )
    .telling("body", json!(label)))
}

pub fn move_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceMoving,
) -> Result<Answer, ToolError> {
    let built = move_face_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn draft_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceDrafting,
    base: Option<&Draft>,
) -> Result<Built, ToolError> {
    let angle = parsed(session, asked.angle.as_ref(), |session, angle| {
        turn(session, "angle", angle)
    })?;
    let beyond = match &asked.beyond {
        None => base.map(|base| base.beyond.clone()).unwrap_or_default(),
        Some(BeyondAsk::Off(_)) => None,
        Some(BeyondAsk::Angle(given)) => Some(turn(session, "beyond", given)?),
    };
    let own_angles = asked
        .own_angles
        .as_ref()
        .map(|owns| {
            owns.iter()
                .map(|own| turn(session, "own_angles angle", &own.angle))
                .collect::<Result<Vec<_>, ToolError>>()
        })
        .transpose()?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let own = match (&asked.own_angles, own_angles) {
        (Some(owns), Some(angles)) => owns
            .iter()
            .zip(angles)
            .map(|(own, angle)| {
                let face = face_references(session, body, std::slice::from_ref(&own.face))?;
                face.into_iter()
                    .next()
                    .map(|face| FaceAngle { face, angle })
                    .ok_or_else(|| {
                        ToolError::from(references::ReferenceError::NoFace {
                            body: body_label(session.document(), body),
                            face: own.face.clone(),
                        })
                    })
            })
            .collect::<Result<Vec<_>, ToolError>>()?,
        _ => base.map(|base| base.own.clone()).unwrap_or_default(),
    };
    let faces = match (asked.faces.as_deref(), base, own.is_empty()) {
        (None, None, false) => Vec::new(),
        _ => faces_of(
            session,
            body,
            (
                asked.faces.as_deref(),
                base.map(|base| base.faces.as_slice()),
            ),
            "draft",
        )?,
    };
    let (neutral, flipped) = match (&asked.neutral, base) {
        (Some(choice), _) => {
            let neutral = places::plane(session.document(), session.evaluation(), choice)?;
            let chosen: Vec<FaceReference> = faces
                .iter()
                .chain(own.iter().map(|own| &own.face))
                .cloned()
                .collect();
            let flipped =
                draft_tools::flipped_on_shown(session.evaluation(), body, &chosen, &neutral);
            (neutral, flipped)
        }
        (None, Some(base)) => (base.neutral.clone(), base.flipped),
        (None, None) => {
            let chosen: Vec<FaceReference> = faces
                .iter()
                .chain(own.iter().map(|own| &own.face))
                .cloned()
                .collect();
            draft_tools::default_neutral_on(session.evaluation(), body, &chosen)
        }
    };
    let draft = Draft {
        body,
        faces,
        neutral,
        angle: angle
            .or_else(|| base.map(|base| base.angle.clone()))
            .unwrap_or(Expression::Measure(
                draft_tools::DEFAULT_ANGLE,
                Unit::Degree,
            )),
        flipped: asked.flipped.unwrap_or(flipped),
        tangent: asked
            .tangent
            .or_else(|| base.map(|base| base.tangent))
            .unwrap_or(false),
        beyond,
        own,
    };
    let label = body_label(session.document(), body);
    Ok(
        Built::new(FeatureKind::Draft(draft), draft_tools::TITLE, asked.name)
            .telling("body", json!(label)),
    )
}

pub fn draft(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceDrafting,
) -> Result<Answer, ToolError> {
    let built = draft_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn split_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Splitting,
    base: Option<&Split>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let along = kept_along(
        session,
        body,
        asked.along.as_ref(),
        base.map(|base| &base.along),
    )?;
    let split = Split {
        body,
        along,
        flipped: asked
            .flipped
            .or_else(|| base.map(|base| base.flipped))
            .unwrap_or(false),
    };
    let label = body_label(session.document(), body);
    Ok(
        Built::new(FeatureKind::Split(split), SPLIT_TITLE, asked.name)
            .telling("body", json!(label)),
    )
}

pub fn split(
    session: &mut Session,
    work: &Work<'_>,
    asked: Splitting,
) -> Result<Answer, ToolError> {
    let built = split_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn scale_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Scaling,
    base: Option<&Scale>,
) -> Result<Built, ToolError> {
    let factor = parsed(session, asked.factor.as_ref(), |session, factor| {
        amount(session, "factor", factor, Dimension::NONE, true)
    })?;
    let center: Option<[Expression; 3]> = match &asked.center {
        Some([x, y, z]) => Some([
            signed_length(session, "center", x)?,
            signed_length(session, "center", y)?,
            signed_length(session, "center", z)?,
        ]),
        None => None,
    };
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let zero = || LengthUnit::Millimetre.default_length(0.0);
    let scale = Scale {
        body,
        factor: match (factor, base) {
            (Some(factor), _) => factor,
            (None, Some(base)) => base.factor.clone(),
            (None, None) => return Err(FeatureError::Missing { field: "factor" }.into()),
        },
        center: center
            .or_else(|| base.map(|base| base.center.clone()))
            .unwrap_or_else(|| [zero(), zero(), zero()]),
        frame: base.and_then(|base| base.frame),
    };
    let label = body_label(session.document(), body);
    Ok(
        Built::new(FeatureKind::Scale(scale), scale_tools::TITLE, asked.name)
            .telling("body", json!(label)),
    )
}

pub fn scale(session: &mut Session, work: &Work<'_>, asked: Scaling) -> Result<Answer, ToolError> {
    let built = scale_built(session, work, asked, None)?;
    added(session, work, built)
}
