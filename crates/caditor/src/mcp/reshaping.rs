use caditor_document::{
    DeleteFace, FeatureId, FeatureKind, Scale, Split, SplitAlong, SplitCarry, SplitFace,
};
use caditor_expression::{Dimension, Expression};
use serde::Deserialize;
use serde_json::json;

use crate::{
    delete_face_tools,
    mcp::{
        features::{
            Amount, Built, FeatureError, added, amount, body_of, face_references, needed, parsed,
            signed_length,
        },
        places::{self, PlaneChoice},
        references::{self, Named, body_label},
        session::{Session, Work},
        tools::{Answer, ToolError},
    },
    scale_tools, split_face_tools,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSplitting {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
    pub along: Option<AlongChoice>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceDeleting {
    pub body: Option<Named>,
    pub faces: Option<Vec<String>>,
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
    (asked, kept): (Option<&[String]>, Option<&[caditor_kernel::FaceReference]>),
    verb: &'static str,
) -> Result<Vec<caditor_kernel::FaceReference>, ToolError> {
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
    let split = match base {
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

pub fn delete_face(
    session: &mut Session,
    work: &Work<'_>,
    asked: FaceDeleting,
) -> Result<Answer, ToolError> {
    let built = delete_face_built(session, work, asked, None)?;
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
