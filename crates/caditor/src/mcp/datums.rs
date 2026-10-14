use caditor_document::{
    AxisReference, Datum, DatumAxis, DatumFrame, DatumPlane, DatumPoint, Document, Evaluation,
    FaceTangent, FeatureKind, PlaneReference, PlaneRotation, PlaneThrough, PointBy, PointReference,
    PrincipalAxis, PrincipalPlane,
};
use caditor_expression::Expression;
use caditor_geometry::Vector3;
use caditor_kernel::Surface;
use serde::Deserialize;

use crate::{
    datum_tools,
    mcp::{
        features::{
            Amount, Built, FeatureError, added, needed, parsed, signed_length, triple, turn,
        },
        places::{self, AxisChoice, OnEdge, OnFace, PlaceError, PlaneChoice, PointChoice},
        references::body_label,
        session::{Session, Work},
        tools::{Answer, ToolError},
    },
    units::LengthUnit,
};

const FLAT_FACE: &str = "is flat; give a curved face to touch, or use it as a plane";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumPlaning {
    pub base: Option<PlaneChoice>,
    pub offset: Option<Amount>,
    pub turn_axis: Option<AxisChoice>,
    pub turn_angle: Option<Amount>,
    pub through_points: Option<[PointChoice; 3]>,
    pub midway: Option<[PlaneChoice; 2]>,
    pub lines: Option<[AxisChoice; 2]>,
    pub through_axis: Option<AxisChoice>,
    pub square_to_axis: Option<AxisChoice>,
    pub tangent_to: Option<OnFace>,
    pub square_to_edge: Option<OnEdge>,
    pub point: Option<PointChoice>,
    pub distance: Option<Amount>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumAxising {
    pub along: Option<AxisChoice>,
    pub planes: Option<[PlaneChoice; 2]>,
    pub through_points: Option<[PointChoice; 2]>,
    pub square_to_plane: Option<PlaneChoice>,
    pub square_to_face: Option<OnFace>,
    pub point: Option<PointChoice>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumPointing {
    pub at: Option<PointChoice>,
    pub offset: Option<[Amount; 3]>,
    pub lines: Option<[AxisChoice; 2]>,
    pub axis: Option<AxisChoice>,
    pub plane: Option<PlaneChoice>,
    pub planes: Option<[PlaneChoice; 3]>,
    pub along_edge: Option<OnEdge>,
    pub distance: Option<Amount>,
    pub middle_of: Option<OnEdge>,
    pub centre_of: Option<OnFace>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Framing {
    pub origin: Option<PointChoice>,
    pub x_axis: Option<AxisChoice>,
    pub plane: Option<PlaneChoice>,
    pub reverse_x: Option<bool>,
    pub reverse_z: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DatumError {
    #[error("place the datum {datum} one way: {first} and {second} are two different ways")]
    TwoWays {
        datum: &'static str,
        first: &'static str,
        second: &'static str,
    },
    #[error("{field} goes with {with}")]
    NotWith {
        field: &'static str,
        with: &'static str,
    },
    #[error(
        "a datum axis goes along an axis, where two planes meet, through two points, square to a plane or square to a curved face: give along, planes, through_points, square_to_plane or square_to_face"
    )]
    NoDatumAxis,
}

struct Way<W> {
    given: bool,
    way: W,
    field: &'static str,
}

fn chosen_way<W: Copy + PartialEq>(
    datum: &'static str,
    ways: &[Way<W>],
    kept: Option<W>,
) -> Result<Option<W>, DatumError> {
    let mut chosen: Option<&Way<W>> = None;
    for way in ways.iter().filter(|way| way.given) {
        match chosen {
            Some(first) if first.way != way.way => {
                return Err(DatumError::TwoWays {
                    datum,
                    first: first.field,
                    second: way.field,
                });
            }
            Some(_) => {}
            None => chosen = Some(way),
        }
    }
    Ok(chosen.map(|way| way.way).or(kept))
}

fn part<C, T>(
    asked: Option<&C>,
    kept: Option<T>,
    field: &'static str,
    resolve: impl FnOnce(&C) -> Result<T, ToolError>,
) -> Result<T, ToolError> {
    match asked {
        Some(choice) => resolve(choice),
        None => needed(kept, field),
    }
}

fn shared(
    given: bool,
    fits: bool,
    field: &'static str,
    with: &'static str,
) -> Result<(), DatumError> {
    if given && !fits {
        Err(DatumError::NotWith { field, with })
    } else {
        Ok(())
    }
}

fn zero_length() -> Expression {
    LengthUnit::Millimetre.default_length(0.0)
}

fn datum_built(datum: Datum, name: Option<String>) -> Built {
    let title = datum.title();
    Built::new(FeatureKind::Datum(datum), title, name)
}

fn curved_face(
    document: &Document,
    evaluation: &Evaluation,
    on: &OnFace,
    toward: PointReference,
) -> Result<FaceTangent, ToolError> {
    let (at, face) = places::face_at_bar(document, evaluation, on)?;
    let flat = face
        .resolve(at.state)
        .ok()
        .and_then(|found| at.state.face(found))
        .is_some_and(|found| matches!(found.surface(), Surface::Plane(_)));
    if flat {
        return Err(PlaceError::UnusableFace {
            body: body_label(document, at.body),
            face: on.face.clone(),
            reason: FLAT_FACE,
        }
        .into());
    }
    Ok(FaceTangent {
        body: at.body,
        face,
        toward,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaneWay {
    Offset,
    Points,
    Midway,
    Lines,
    ThroughAxis,
    SquareToAxis,
    Tangent,
    SquareToEdge,
}

fn plane_way(datum: &Datum) -> Option<PlaneWay> {
    Some(match datum {
        Datum::Plane(_) => PlaneWay::Offset,
        Datum::PlaneThrough(through) => match through {
            PlaneThrough::Points(_) => PlaneWay::Points,
            PlaneThrough::Midway(..) => PlaneWay::Midway,
            PlaneThrough::Lines(..) => PlaneWay::Lines,
            PlaneThrough::AxisAndPoint(..) => PlaneWay::ThroughAxis,
            PlaneThrough::NormalTo(..) => PlaneWay::SquareToAxis,
            PlaneThrough::Tangent(_) | PlaneThrough::TangentAt(_) => PlaneWay::Tangent,
            PlaneThrough::SquareToCurve(_) => PlaneWay::SquareToEdge,
        },
        Datum::Axis(_) | Datum::Point(_) | Datum::PointBy(_) | Datum::Frame(_) => return None,
    })
}

struct PlaneValues {
    offset: Option<Expression>,
    angle: Option<Expression>,
    distance: Option<Expression>,
}

fn offset_plane(
    session: &Session,
    asked: &DatumPlaning,
    values: PlaneValues,
    kept: Option<&DatumPlane>,
) -> Result<DatumPlane, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    let base = part(
        asked.base.as_ref(),
        kept.map(|kept| kept.base.clone()),
        "base",
        |choice| places::plane(document, evaluation, choice),
    )?;
    let kept_rotation = kept.and_then(|kept| kept.rotation.as_ref());
    let rotation = if asked.turn_axis.is_some() || values.angle.is_some() {
        let axis = match &asked.turn_axis {
            Some(choice) => Some(places::axis(document, evaluation, choice)?),
            None => kept_rotation.map(|kept| kept.axis.clone()),
        };
        let angle = values
            .angle
            .or_else(|| kept_rotation.map(|kept| kept.angle.clone()));
        let (Some(axis), Some(angle)) = (axis, angle) else {
            return Err(FeatureError::HalfAnAxisTurn.into());
        };
        Some(PlaneRotation { axis, angle })
    } else {
        kept_rotation.cloned()
    };
    Ok(DatumPlane {
        base,
        rotation,
        offset: values
            .offset
            .or_else(|| kept.map(|kept| kept.offset.clone()))
            .unwrap_or_else(zero_length),
    })
}

fn through_plane(
    session: &Session,
    asked: &DatumPlaning,
    way: PlaneWay,
    (distance, kept): (Option<Expression>, Option<&PlaneThrough>),
) -> Result<PlaneThrough, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    let plane = |choice: &PlaneChoice| places::plane(document, evaluation, choice);
    let axis = |choice: &AxisChoice| places::axis(document, evaluation, choice);
    let point = |choice: &PointChoice| places::point(document, evaluation, choice);
    let kept_axis_and_point = match kept {
        Some(PlaneThrough::AxisAndPoint(axis, point) | PlaneThrough::NormalTo(axis, point)) => {
            Some((axis.clone(), point.clone()))
        }
        _ => None,
    };
    let kept_tangent = match kept {
        Some(PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent)) => Some(tangent),
        _ => None,
    };
    Ok(match way {
        PlaneWay::Points => {
            let kept = match kept {
                Some(PlaneThrough::Points(points)) => Some(points.clone()),
                _ => None,
            };
            PlaneThrough::Points(part(
                asked.through_points.as_ref(),
                kept,
                "through_points",
                |[first, second, third]| Ok([point(first)?, point(second)?, point(third)?]),
            )?)
        }
        PlaneWay::Midway => {
            let kept = match kept {
                Some(PlaneThrough::Midway(first, second)) => Some((first.clone(), second.clone())),
                _ => None,
            };
            let (first, second) =
                part(asked.midway.as_ref(), kept, "midway", |[first, second]| {
                    Ok((plane(first)?, plane(second)?))
                })?;
            PlaneThrough::Midway(first, second)
        }
        PlaneWay::Lines => {
            let kept = match kept {
                Some(PlaneThrough::Lines(first, second)) => Some((first.clone(), second.clone())),
                _ => None,
            };
            let (first, second) = part(asked.lines.as_ref(), kept, "lines", |[first, second]| {
                Ok((axis(first)?, axis(second)?))
            })?;
            PlaneThrough::Lines(first, second)
        }
        PlaneWay::ThroughAxis | PlaneWay::SquareToAxis => {
            let (asked_axis, field) = if way == PlaneWay::ThroughAxis {
                (asked.through_axis.as_ref(), "through_axis")
            } else {
                (asked.square_to_axis.as_ref(), "square_to_axis")
            };
            let held = part(
                asked_axis,
                kept_axis_and_point.as_ref().map(|(axis, _)| axis.clone()),
                field,
                axis,
            )?;
            let at = part(
                asked.point.as_ref(),
                kept_axis_and_point.map(|(_, point)| point),
                "point",
                point,
            )?;
            if way == PlaneWay::ThroughAxis {
                PlaneThrough::AxisAndPoint(held, at)
            } else {
                PlaneThrough::NormalTo(held, at)
            }
        }
        PlaneWay::Tangent => {
            let toward = part(
                asked.point.as_ref(),
                kept_tangent.map(|kept| kept.toward.clone()),
                "point",
                point,
            )?;
            let tangent = match &asked.tangent_to {
                Some(on) => curved_face(document, evaluation, on, toward)?,
                None => FaceTangent {
                    toward,
                    ..needed(kept_tangent.map(|kept| (**kept).clone()), "tangent_to")?
                },
            };
            match kept {
                Some(PlaneThrough::Tangent(_)) => PlaneThrough::Tangent(Box::new(tangent)),
                _ => PlaneThrough::TangentAt(Box::new(tangent)),
            }
        }
        PlaneWay::SquareToEdge => {
            let kept = match kept {
                Some(PlaneThrough::SquareToCurve(station)) => Some(station.as_ref()),
                _ => None,
            };
            let distance = distance
                .or_else(|| kept.map(|kept| kept.distance.clone()))
                .unwrap_or_else(zero_length);
            let station = match &asked.square_to_edge {
                Some(on) => places::station(document, evaluation, on, distance)?,
                None => caditor_document::CurveStation {
                    distance,
                    ..needed(kept.cloned(), "square_to_edge")?
                },
            };
            PlaneThrough::SquareToCurve(Box::new(station))
        }
        PlaneWay::Offset => return Err(FeatureError::Missing { field: "base" }.into()),
    })
}

pub fn datum_plane_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumPlaning,
    base: Option<&Datum>,
) -> Result<Built, ToolError> {
    let values = PlaneValues {
        offset: parsed(session, asked.offset.as_ref(), |session, offset| {
            signed_length(session, "offset", offset)
        })?,
        angle: parsed(session, asked.turn_angle.as_ref(), |session, angle| {
            turn(session, "turn_angle", angle)
        })?,
        distance: parsed(session, asked.distance.as_ref(), |session, distance| {
            signed_length(session, "distance", distance)
        })?,
    };
    session.evaluate(work)?;
    let offset_given = asked.base.is_some()
        || asked.offset.is_some()
        || asked.turn_axis.is_some()
        || asked.turn_angle.is_some();
    let ways = [
        Way {
            given: offset_given,
            way: PlaneWay::Offset,
            field: "base",
        },
        Way {
            given: asked.through_points.is_some(),
            way: PlaneWay::Points,
            field: "through_points",
        },
        Way {
            given: asked.midway.is_some(),
            way: PlaneWay::Midway,
            field: "midway",
        },
        Way {
            given: asked.lines.is_some(),
            way: PlaneWay::Lines,
            field: "lines",
        },
        Way {
            given: asked.through_axis.is_some(),
            way: PlaneWay::ThroughAxis,
            field: "through_axis",
        },
        Way {
            given: asked.square_to_axis.is_some(),
            way: PlaneWay::SquareToAxis,
            field: "square_to_axis",
        },
        Way {
            given: asked.tangent_to.is_some(),
            way: PlaneWay::Tangent,
            field: "tangent_to",
        },
        Way {
            given: asked.square_to_edge.is_some(),
            way: PlaneWay::SquareToEdge,
            field: "square_to_edge",
        },
    ];
    let way = chosen_way("plane", &ways, base.and_then(plane_way))?.unwrap_or(PlaneWay::Offset);
    shared(
        asked.point.is_some(),
        matches!(
            way,
            PlaneWay::ThroughAxis | PlaneWay::SquareToAxis | PlaneWay::Tangent
        ),
        "point",
        "through_axis, square_to_axis or tangent_to",
    )?;
    shared(
        asked.distance.is_some(),
        way == PlaneWay::SquareToEdge,
        "distance",
        "square_to_edge",
    )?;
    let kept = base.filter(|base| plane_way(base) == Some(way));
    let datum = match way {
        PlaneWay::Offset => {
            let kept = match kept {
                Some(Datum::Plane(plane)) => Some(plane),
                _ => None,
            };
            Datum::Plane(offset_plane(session, &asked, values, kept)?)
        }
        _ => {
            let kept = match kept {
                Some(Datum::PlaneThrough(through)) => Some(through),
                _ => None,
            };
            Datum::PlaneThrough(through_plane(
                session,
                &asked,
                way,
                (values.distance, kept),
            )?)
        }
    };
    Ok(datum_built(datum, asked.name))
}

pub fn datum_plane(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumPlaning,
) -> Result<Answer, ToolError> {
    let built = datum_plane_built(session, work, asked, None)?;
    added(session, work, built)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxisWay {
    Along,
    Planes,
    Points,
    SquareToPlane,
    SquareToFace,
}

fn axis_way(axis: &DatumAxis) -> AxisWay {
    match axis {
        DatumAxis::Along(_) => AxisWay::Along,
        DatumAxis::Intersection(..) => AxisWay::Planes,
        DatumAxis::Points(..) => AxisWay::Points,
        DatumAxis::NormalTo(..) => AxisWay::SquareToPlane,
        DatumAxis::SquareToFace(_) => AxisWay::SquareToFace,
    }
}

pub fn datum_axis_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumAxising,
    base: Option<&DatumAxis>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let ways = [
        Way {
            given: asked.along.is_some(),
            way: AxisWay::Along,
            field: "along",
        },
        Way {
            given: asked.planes.is_some(),
            way: AxisWay::Planes,
            field: "planes",
        },
        Way {
            given: asked.through_points.is_some(),
            way: AxisWay::Points,
            field: "through_points",
        },
        Way {
            given: asked.square_to_plane.is_some(),
            way: AxisWay::SquareToPlane,
            field: "square_to_plane",
        },
        Way {
            given: asked.square_to_face.is_some(),
            way: AxisWay::SquareToFace,
            field: "square_to_face",
        },
    ];
    let way = chosen_way("axis", &ways, base.map(axis_way))?.ok_or(DatumError::NoDatumAxis)?;
    shared(
        asked.point.is_some(),
        matches!(way, AxisWay::SquareToPlane | AxisWay::SquareToFace),
        "point",
        "square_to_plane or square_to_face",
    )?;
    let kept = base.filter(|base| axis_way(base) == way);
    let plane = |choice: &PlaneChoice| places::plane(document, evaluation, choice);
    let point = |choice: &PointChoice| places::point(document, evaluation, choice);
    let axis = match way {
        AxisWay::Along => {
            let kept = match kept {
                Some(DatumAxis::Along(axis)) => Some(axis.clone()),
                _ => None,
            };
            DatumAxis::Along(part(asked.along.as_ref(), kept, "along", |choice| {
                places::axis(document, evaluation, choice)
            })?)
        }
        AxisWay::Planes => {
            let kept = match kept {
                Some(DatumAxis::Intersection(first, second)) => {
                    Some((first.clone(), second.clone()))
                }
                _ => None,
            };
            let (first, second) =
                part(asked.planes.as_ref(), kept, "planes", |[first, second]| {
                    Ok((plane(first)?, plane(second)?))
                })?;
            DatumAxis::Intersection(first, second)
        }
        AxisWay::Points => {
            let kept = match kept {
                Some(DatumAxis::Points(first, second)) => Some((first.clone(), second.clone())),
                _ => None,
            };
            let (first, second) = part(
                asked.through_points.as_ref(),
                kept,
                "through_points",
                |[first, second]| Ok((point(first)?, point(second)?)),
            )?;
            DatumAxis::Points(first, second)
        }
        AxisWay::SquareToPlane => {
            let kept = match kept {
                Some(DatumAxis::NormalTo(plane, point)) => Some((plane.clone(), point.clone())),
                _ => None,
            };
            let held = part(
                asked.square_to_plane.as_ref(),
                kept.as_ref().map(|(plane, _)| plane.clone()),
                "square_to_plane",
                plane,
            )?;
            let at = part(
                asked.point.as_ref(),
                kept.map(|(_, point)| point),
                "point",
                point,
            )?;
            DatumAxis::NormalTo(held, at)
        }
        AxisWay::SquareToFace => {
            let kept = match kept {
                Some(DatumAxis::SquareToFace(tangent)) => Some(tangent.as_ref()),
                _ => None,
            };
            let toward = part(
                asked.point.as_ref(),
                kept.map(|kept| kept.toward.clone()),
                "point",
                point,
            )?;
            let tangent = match &asked.square_to_face {
                Some(on) => curved_face(document, evaluation, on, toward)?,
                None => FaceTangent {
                    toward,
                    ..needed(kept.cloned(), "square_to_face")?
                },
            };
            DatumAxis::SquareToFace(Box::new(tangent))
        }
    };
    Ok(datum_built(Datum::Axis(axis), asked.name))
}

pub fn datum_axis(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumAxising,
) -> Result<Answer, ToolError> {
    let built = datum_axis_built(session, work, asked, None)?;
    added(session, work, built)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointWay {
    At,
    Lines,
    AxisAndPlane,
    Planes,
    Along,
    Middle,
    Centre,
}

fn point_way(datum: &Datum) -> Option<PointWay> {
    Some(match datum {
        Datum::Point(_) => PointWay::At,
        Datum::PointBy(by) => match by {
            PointBy::LinesCross(..) => PointWay::Lines,
            PointBy::AxisAndPlane(..) => PointWay::AxisAndPlane,
            PointBy::ThreePlanes(_) => PointWay::Planes,
            PointBy::Along(_) => PointWay::Along,
            PointBy::EdgeMiddle { .. } => PointWay::Middle,
            PointBy::FaceCentre { .. } => PointWay::Centre,
        },
        Datum::Plane(_) | Datum::PlaneThrough(_) | Datum::Axis(_) | Datum::Frame(_) => {
            return None;
        }
    })
}

fn placed_point(
    session: &Session,
    asked: &DatumPointing,
    offset: Option<[Expression; 3]>,
    kept: Option<&DatumPoint>,
) -> Result<DatumPoint, ToolError> {
    let base = match (&asked.at, kept) {
        (Some(choice), _) => places::point(session.document(), session.evaluation(), choice)?,
        (None, Some(kept)) => kept.base.clone(),
        (None, None) => PointReference::Origin,
    };
    Ok(DatumPoint {
        base,
        offset: offset
            .or_else(|| kept.map(|kept| kept.offset.clone()))
            .unwrap_or_else(|| [zero_length(), zero_length(), zero_length()]),
    })
}

fn point_by(
    session: &Session,
    asked: &DatumPointing,
    way: PointWay,
    (distance, kept): (Option<Expression>, Option<&PointBy>),
) -> Result<PointBy, ToolError> {
    let document = session.document();
    let evaluation = session.evaluation();
    let plane = |choice: &PlaneChoice| places::plane(document, evaluation, choice);
    let axis = |choice: &AxisChoice| places::axis(document, evaluation, choice);
    Ok(match way {
        PointWay::Lines => {
            let kept = match kept {
                Some(PointBy::LinesCross(first, second)) => Some((first.clone(), second.clone())),
                _ => None,
            };
            let (first, second) = part(asked.lines.as_ref(), kept, "lines", |[first, second]| {
                Ok((axis(first)?, axis(second)?))
            })?;
            PointBy::LinesCross(first, second)
        }
        PointWay::AxisAndPlane => {
            let kept = match kept {
                Some(PointBy::AxisAndPlane(axis, plane)) => Some((axis.clone(), plane.clone())),
                _ => None,
            };
            let held = part(
                asked.axis.as_ref(),
                kept.as_ref().map(|(axis, _)| axis.clone()),
                "axis",
                axis,
            )?;
            let crossed = part(
                asked.plane.as_ref(),
                kept.map(|(_, plane)| plane),
                "plane",
                plane,
            )?;
            PointBy::AxisAndPlane(held, crossed)
        }
        PointWay::Planes => {
            let kept = match kept {
                Some(PointBy::ThreePlanes(planes)) => Some(planes.clone()),
                _ => None,
            };
            PointBy::ThreePlanes(part(
                asked.planes.as_ref(),
                kept,
                "planes",
                |[first, second, third]| Ok([plane(first)?, plane(second)?, plane(third)?]),
            )?)
        }
        PointWay::Along => {
            let kept = match kept {
                Some(PointBy::Along(station)) => Some(station.as_ref()),
                _ => None,
            };
            let distance = distance
                .or_else(|| kept.map(|kept| kept.distance.clone()))
                .unwrap_or_else(zero_length);
            let station = match &asked.along_edge {
                Some(on) => places::station(document, evaluation, on, distance)?,
                None => caditor_document::CurveStation {
                    distance,
                    ..needed(kept.cloned(), "along_edge")?
                },
            };
            PointBy::Along(Box::new(station))
        }
        PointWay::Middle => {
            let kept = match kept {
                Some(by @ PointBy::EdgeMiddle { .. }) => Some(by.clone()),
                _ => None,
            };
            part(asked.middle_of.as_ref(), kept, "middle_of", |on| {
                let (at, edge) = places::edge_at_bar(document, evaluation, on)?;
                let edge =
                    caditor_kernel::EdgeReference::capture(at.state, edge).ok_or_else(|| {
                        PlaceError::NoEdge {
                            body: body_label(document, at.body),
                            edge: on.edge.clone(),
                        }
                    })?;
                Ok(PointBy::EdgeMiddle {
                    body: at.body,
                    edge: Box::new(edge),
                })
            })?
        }
        PointWay::Centre => {
            let kept = match kept {
                Some(by @ PointBy::FaceCentre { .. }) => Some(by.clone()),
                _ => None,
            };
            part(asked.centre_of.as_ref(), kept, "centre_of", |on| {
                let (at, face) = places::face_at_bar(document, evaluation, on)?;
                Ok(PointBy::FaceCentre {
                    body: at.body,
                    face,
                })
            })?
        }
        PointWay::At => return Err(FeatureError::Missing { field: "at" }.into()),
    })
}

pub fn datum_point_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumPointing,
    base: Option<&Datum>,
) -> Result<Built, ToolError> {
    let offset = triple(session, "offset", asked.offset.as_ref(), signed_length)?;
    let distance = parsed(session, asked.distance.as_ref(), |session, distance| {
        signed_length(session, "distance", distance)
    })?;
    session.evaluate(work)?;
    let ways = [
        Way {
            given: asked.at.is_some() || asked.offset.is_some(),
            way: PointWay::At,
            field: "at",
        },
        Way {
            given: asked.lines.is_some(),
            way: PointWay::Lines,
            field: "lines",
        },
        Way {
            given: asked.axis.is_some() || asked.plane.is_some(),
            way: PointWay::AxisAndPlane,
            field: "axis and plane",
        },
        Way {
            given: asked.planes.is_some(),
            way: PointWay::Planes,
            field: "planes",
        },
        Way {
            given: asked.along_edge.is_some(),
            way: PointWay::Along,
            field: "along_edge",
        },
        Way {
            given: asked.middle_of.is_some(),
            way: PointWay::Middle,
            field: "middle_of",
        },
        Way {
            given: asked.centre_of.is_some(),
            way: PointWay::Centre,
            field: "centre_of",
        },
    ];
    let way = chosen_way("point", &ways, base.and_then(point_way))?.unwrap_or(PointWay::At);
    shared(
        asked.distance.is_some(),
        way == PointWay::Along,
        "distance",
        "along_edge",
    )?;
    let kept = base.filter(|base| point_way(base) == Some(way));
    let datum = match (way, kept) {
        (PointWay::At, kept) => {
            let kept = match kept {
                Some(Datum::Point(point)) => Some(point),
                _ => None,
            };
            Datum::Point(placed_point(session, &asked, offset, kept)?)
        }
        (_, kept) => {
            let kept = match kept {
                Some(Datum::PointBy(by)) => Some(by),
                _ => None,
            };
            Datum::PointBy(point_by(session, &asked, way, (distance, kept))?)
        }
    };
    Ok(datum_built(datum, asked.name))
}

pub fn datum_point(
    session: &mut Session,
    work: &Work<'_>,
    asked: DatumPointing,
) -> Result<Answer, ToolError> {
    let built = datum_point_built(session, work, asked, None)?;
    added(session, work, built)
}

fn principal_normal(plane: &PlaneReference) -> Option<Vector3> {
    match plane {
        PlaneReference::Principal(plane) => Some(plane.plane().normal()),
        PlaneReference::Datum(_) | PlaneReference::Face(_) | PlaneReference::Frame { .. } => None,
    }
}

fn principal_direction(axis: &AxisReference) -> Option<Vector3> {
    match axis {
        AxisReference::Principal(axis) => Some(axis.direction()),
        AxisReference::Datum(_)
        | AxisReference::Edge { .. }
        | AxisReference::Face { .. }
        | AxisReference::Sketch { .. }
        | AxisReference::Frame { .. } => None,
    }
}

pub fn coordinate_system_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Framing,
    base: Option<&DatumFrame>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let origin = match (&asked.origin, base) {
        (Some(choice), _) => places::point(document, evaluation, choice)?,
        (None, Some(base)) => base.origin.clone(),
        (None, None) => PointReference::Origin,
    };
    let x_axis = match &asked.x_axis {
        Some(choice) => Some(places::axis(document, evaluation, choice)?),
        None => base.map(|base| base.x_axis.clone()),
    };
    let plane = match &asked.plane {
        Some(choice) => Some(places::plane(document, evaluation, choice)?),
        None => base.map(|base| base.plane.clone()),
    };
    let x_axis = x_axis.unwrap_or_else(|| {
        let normal = plane.as_ref().and_then(principal_normal);
        let flattest = datum_tools::least_steep(
            PrincipalAxis::ALL.map(|axis| (axis, axis.direction())),
            normal,
        );
        AxisReference::Principal(flattest.unwrap_or(PrincipalAxis::X))
    });
    let plane = plane.unwrap_or_else(|| {
        let along = principal_direction(&x_axis);
        let holding = datum_tools::least_steep(
            PrincipalPlane::ALL.map(|plane| (plane, plane.plane().normal())),
            along,
        );
        PlaneReference::Principal(holding.unwrap_or(PrincipalPlane::Xy))
    });
    let frame = DatumFrame {
        origin,
        x_axis,
        plane,
        reverse_x: asked
            .reverse_x
            .or_else(|| base.map(|base| base.reverse_x))
            .unwrap_or(false),
        reverse_z: asked
            .reverse_z
            .or_else(|| base.map(|base| base.reverse_z))
            .unwrap_or(false),
    };
    Ok(datum_built(Datum::Frame(Box::new(frame)), asked.name))
}

pub fn coordinate_system(
    session: &mut Session,
    work: &Work<'_>,
    asked: Framing,
) -> Result<Answer, ToolError> {
    let built = coordinate_system_built(session, work, asked, None)?;
    added(session, work, built)
}
