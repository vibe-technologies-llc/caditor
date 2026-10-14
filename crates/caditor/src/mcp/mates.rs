use std::mem;

use caditor_document::{
    AngleMate, AngleSides, AxisMate, AxisReference, FaceAttachment, FaceAxisMate, FaceMate,
    FaceOnRound, FacePair, FeatureId, FeatureKind, Mate, MatePair, PlaneReference, PointMate,
    PointReference, PointTarget, round_face,
};
use caditor_expression::Expression;
use caditor_kernel::{FaceReference, Surface};
use serde::Deserialize;
use serde_json::json;

use crate::{
    mate_tools,
    mcp::{
        features::{Amount, Built, added, body_of, parsed, signed_length, turn},
        places::{self, AxisChoice, OnFace, PlaceError, PlaneChoice, PointChoice},
        references::{Named, body_label},
        session::{Session, Work},
        tools::{Answer, ToolError},
    },
    units::LengthUnit,
};

const NEITHER_FLAT_NOR_ROUND: &str =
    "is neither flat nor cylindrical or spherical, so nothing can be mated by it";
const NOT_ROUND: &str = "is not cylindrical or spherical, so nothing can rest on it";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mating {
    pub body: Option<Named>,
    pub face: Option<String>,
    pub axis: Option<AxisChoice>,
    pub point: Option<PointChoice>,
    pub to_plane: Option<PlaneChoice>,
    pub to_axis: Option<AxisChoice>,
    pub to_point: Option<PointChoice>,
    pub to_round: Option<OnFace>,
    pub distance: Option<Amount>,
    pub angle: Option<Amount>,
    pub flipped: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MateError {
    #[error(
        "say what to mate: a flat face of the body with to_plane (flush) or to_round (resting on \
         a round face), an axis of the body with to_axis (concentric), both together (flush and \
         concentric), a round face with to_plane (resting on it), or a point of the body with \
         to_point or to_plane; angle turns a face or an axis to an angle instead"
    )]
    NoPair,
    #[error("the mate's {part} must lie on {body}, the body it moves")]
    NotOnBody { part: &'static str, body: String },
    #[error("mate {body} onto another body, a plane, an axis or a point, not onto itself")]
    OntoItself { body: String },
    #[error("a mate keeps its kind ({kind}); add another mate to mate it differently")]
    KindKept { kind: &'static str },
}

#[derive(Debug, Clone, PartialEq)]
enum MovingFace {
    Flat(FaceReference),
    Round(FaceReference),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct MateParts {
    face: Option<MovingFace>,
    axis: Option<AxisReference>,
    point: Option<PointReference>,
    to_plane: Option<PlaneReference>,
    to_axis: Option<AxisReference>,
    to_point: Option<PointReference>,
    to_round: Option<FaceAttachment>,
    distance: Option<Expression>,
    angle: Option<Expression>,
}

fn parts_of(pair: &MatePair) -> MateParts {
    match pair {
        MatePair::Faces(faces) => MateParts {
            face: Some(MovingFace::Flat(faces.face.clone())),
            to_plane: Some(faces.target.clone()),
            distance: Some(faces.distance.clone()),
            ..MateParts::default()
        },
        MatePair::Axes(axes) => MateParts {
            axis: Some(axes.axis.clone()),
            to_axis: Some(axes.target.clone()),
            ..MateParts::default()
        },
        MatePair::FaceAxis(both) => MateParts {
            face: Some(MovingFace::Flat(both.faces.face.clone())),
            to_plane: Some(both.faces.target.clone()),
            distance: Some(both.faces.distance.clone()),
            axis: Some(both.axes.axis.clone()),
            to_axis: Some(both.axes.target.clone()),
            ..MateParts::default()
        },
        MatePair::Angle(angle) => {
            let sides = match &angle.sides {
                AngleSides::Faces(faces) => MateParts {
                    face: Some(MovingFace::Flat(faces.face.clone())),
                    to_plane: Some(faces.target.clone()),
                    ..MateParts::default()
                },
                AngleSides::Axes(axes) => MateParts {
                    axis: Some(axes.axis.clone()),
                    to_axis: Some(axes.target.clone()),
                    ..MateParts::default()
                },
            };
            MateParts {
                angle: Some(angle.angle.clone()),
                ..sides
            }
        }
        MatePair::Tangent(resting) => MateParts {
            face: Some(MovingFace::Round(resting.face.clone())),
            to_plane: Some(resting.target.clone()),
            ..MateParts::default()
        },
        MatePair::Point(point) => {
            let target = match &point.target {
                PointTarget::Point(target) => MateParts {
                    to_point: Some(target.clone()),
                    ..MateParts::default()
                },
                PointTarget::Plane(target) => MateParts {
                    to_plane: Some(target.clone()),
                    ..MateParts::default()
                },
            };
            MateParts {
                point: Some(point.point.clone()),
                ..target
            }
        }
        MatePair::FaceOnRound(resting) => MateParts {
            face: Some(MovingFace::Flat(resting.face.clone())),
            to_round: Some(resting.round.clone()),
            ..MateParts::default()
        },
    }
}

fn zero_length() -> Expression {
    LengthUnit::Millimetre.default_length(0.0)
}

fn angled(parts: MateParts, angle: Expression) -> Option<MatePair> {
    let sides = match parts {
        MateParts {
            face: Some(MovingFace::Flat(face)),
            to_plane: Some(target),
            ..
        } => AngleSides::Faces(FacePair { face, target }),
        MateParts {
            axis: Some(axis),
            to_axis: Some(target),
            ..
        } => AngleSides::Axes(AxisMate { axis, target }),
        MateParts { .. } => return None,
    };
    Some(MatePair::Angle(Box::new(AngleMate { sides, angle })))
}

fn paired(mut parts: MateParts) -> Option<MatePair> {
    if let Some(angle) = parts.angle.take() {
        return angled(parts, angle);
    }
    let distance = parts.distance.take().unwrap_or_else(zero_length);
    Some(match parts {
        MateParts {
            face: Some(MovingFace::Flat(face)),
            to_plane: Some(target),
            axis: Some(axis),
            to_axis: Some(axis_target),
            ..
        } => MatePair::FaceAxis(Box::new(FaceAxisMate {
            faces: FaceMate {
                face,
                target,
                distance,
            },
            axes: AxisMate {
                axis,
                target: axis_target,
            },
        })),
        MateParts {
            face: Some(MovingFace::Flat(face)),
            to_plane: Some(target),
            ..
        } => MatePair::Faces(Box::new(FaceMate {
            face,
            target,
            distance,
        })),
        MateParts {
            axis: Some(axis),
            to_axis: Some(target),
            ..
        } => MatePair::Axes(Box::new(AxisMate { axis, target })),
        MateParts {
            face: Some(MovingFace::Round(face)),
            to_plane: Some(target),
            ..
        } => MatePair::Tangent(Box::new(FacePair { face, target })),
        MateParts {
            face: Some(MovingFace::Flat(face)),
            to_round: Some(round),
            ..
        } => MatePair::FaceOnRound(Box::new(FaceOnRound { face, round })),
        MateParts {
            point: Some(point),
            to_point: Some(target),
            ..
        } => MatePair::Point(Box::new(PointMate {
            point,
            target: PointTarget::Point(target),
        })),
        MateParts {
            point: Some(point),
            to_plane: Some(target),
            ..
        } => MatePair::Point(Box::new(PointMate {
            point,
            target: PointTarget::Plane(target),
        })),
        MateParts { .. } => return None,
    })
}

fn kind_words(pair: &MatePair) -> &'static str {
    match pair {
        MatePair::Faces(_) => "flush",
        MatePair::Axes(_) => "concentric",
        MatePair::FaceAxis(_) => "flush and concentric",
        MatePair::Angle(_) => "angle",
        MatePair::Tangent(_) | MatePair::FaceOnRound(_) => "rests on",
        MatePair::Point(_) => "point",
    }
}

fn moving_face(session: &Session, body: FeatureId, key: &str) -> Result<MovingFace, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    let on = OnFace {
        body: Named::Id(body.raw()),
        face: key.to_owned(),
    };
    let (at, face) = places::face_at_bar(document, evaluation, &on)?;
    let flat = face
        .resolve(at.state)
        .ok()
        .and_then(|found| at.state.face(found))
        .is_some_and(|found| matches!(found.surface(), Surface::Plane(_)));
    if flat {
        return match places::plane(document, evaluation, &PlaneChoice::Face(on))? {
            PlaneReference::Face(attachment) => Ok(MovingFace::Flat(attachment.face)),
            PlaneReference::Principal(_)
            | PlaneReference::Datum(_)
            | PlaneReference::Frame { .. } => Err(MateError::NoPair.into()),
        };
    }
    if round_face(at.state, &face).is_ok() {
        return Ok(MovingFace::Round(face));
    }
    Err(PlaceError::UnusableFace {
        body: body_label(document, body),
        face: key.to_owned(),
        reason: NEITHER_FLAT_NOR_ROUND,
    }
    .into())
}

fn round_target(session: &Session, on: &OnFace) -> Result<FaceAttachment, ToolError> {
    let document = session.document();
    let (at, face) = places::face_at_bar(document, session.evaluation(), on)?;
    if round_face(at.state, &face).is_err() {
        return Err(PlaceError::UnusableFace {
            body: body_label(document, at.body),
            face: on.face.clone(),
            reason: NOT_ROUND,
        }
        .into());
    }
    Ok(FaceAttachment {
        body: at.body,
        face,
    })
}

fn on_body(
    session: &Session,
    body: FeatureId,
    part: &'static str,
    found: Option<FeatureId>,
) -> Result<(), MateError> {
    if found == Some(body) {
        Ok(())
    } else {
        Err(MateError::NotOnBody {
            part,
            body: body_label(session.document(), body),
        })
    }
}

fn off_body(session: &Session, body: FeatureId, found: Option<FeatureId>) -> Result<(), MateError> {
    if found == Some(body) {
        Err(MateError::OntoItself {
            body: body_label(session.document(), body),
        })
    } else {
        Ok(())
    }
}

fn asked_parts(
    session: &Session,
    body: FeatureId,
    asked: &Mating,
    mut parts: MateParts,
) -> Result<MateParts, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    if let Some(key) = &asked.face {
        parts.face = Some(moving_face(session, body, key)?);
    }
    if let Some(choice) = &asked.axis {
        let axis = places::axis(document, evaluation, choice)?;
        on_body(session, body, "axis", axis.body())?;
        parts.axis = Some(axis);
    }
    if let Some(choice) = &asked.point {
        let point = places::point(document, evaluation, choice)?;
        on_body(session, body, "point", point.body())?;
        parts.point = Some(point);
    }
    if let Some(choice) = &asked.to_plane {
        let target = places::plane(document, evaluation, choice)?;
        off_body(session, body, target.body())?;
        parts.to_plane = Some(target);
    }
    if let Some(choice) = &asked.to_axis {
        let target = places::axis(document, evaluation, choice)?;
        off_body(session, body, target.body())?;
        parts.to_axis = Some(target);
    }
    if let Some(choice) = &asked.to_point {
        let target = places::point(document, evaluation, choice)?;
        off_body(session, body, target.body())?;
        parts.to_point = Some(target);
    }
    if let Some(on) = &asked.to_round {
        let target = round_target(session, on)?;
        off_body(session, body, Some(target.body))?;
        parts.to_round = Some(target);
    }
    Ok(parts)
}

pub fn mate_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Mating,
    base: Option<&Mate>,
) -> Result<Built, ToolError> {
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        signed_length(session, "distance", distance)
    })?;
    let angle = parsed(session, asked.angle.as_ref(), |session, angle| {
        turn(session, "angle", angle)
    })?;
    session.evaluate(work)?;
    let body = body_of(session, asked.body.as_ref(), base.map(|base| base.body))?;
    let kept = base
        .filter(|base| base.body == body)
        .map(|base| parts_of(&base.pair))
        .unwrap_or_default();
    let mut parts = asked_parts(session, body, &asked, kept)?;
    if distance.is_some() {
        parts.distance = distance;
    }
    if angle.is_some() {
        parts.angle = angle;
    }
    let pair = paired(parts).ok_or(MateError::NoPair)?;
    if let Some(base) = base
        && mem::discriminant(&base.pair) != mem::discriminant(&pair)
    {
        return Err(MateError::KindKept {
            kind: kind_words(&base.pair),
        }
        .into());
    }
    let mate = Mate {
        body,
        pair,
        flipped: asked
            .flipped
            .or_else(|| base.map(|base| base.flipped))
            .unwrap_or(false),
    };
    let label = body_label(session.document(), body);
    let kind = kind_words(&mate.pair);
    Ok(
        Built::new(FeatureKind::Mate(mate), mate_tools::NAME, asked.name)
            .telling("body", json!(label))
            .telling("mate", json!(kind)),
    )
}

pub fn mate(session: &mut Session, work: &Work<'_>, asked: Mating) -> Result<Answer, ToolError> {
    let built = mate_built(session, work, asked, None)?;
    added(session, work, built)
}
