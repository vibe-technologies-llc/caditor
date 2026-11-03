use thiserror::Error;

use super::{TANGENT_FACE_ANGLE, blend_chain, edge_faces, face_normal};
use crate::topology::{EdgeId, Solid};

const TURN_SAMPLES: [f64; 9] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
const SAME_TURN: f64 = 1e-5;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ChordError {
    #[error("no edge is chosen")]
    NoEdges,
    #[error("edge {0:?} is not part of the solid")]
    MissingEdge(EdgeId),
    #[error("edge {0:?} does not lie between two faces")]
    Unsupported(EdgeId),
    #[error("the faces meet smoothly at edge {0:?}")]
    Smooth(EdgeId),
    #[error("the faces at edge {0:?} meet back to back")]
    BackToBack(EdgeId),
    #[error("the faces at edge {0:?} turn by a different angle along it")]
    VaryingTurn(EdgeId),
    #[error("the faces at edges {first:?} and {second:?} turn by different angles")]
    DifferingTurns { first: EdgeId, second: EdgeId },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Turn {
    Smooth,
    Constant(f64),
}

fn turn_along(solid: &Solid, edge: EdgeId) -> Result<Turn, ChordError> {
    let definition = solid.edge(edge).ok_or(ChordError::MissingEdge(edge))?;
    let faces = edge_faces(solid, edge);
    let [(first, _), (second, _)] = faces.as_slice() else {
        return Err(ChordError::Unsupported(edge));
    };
    if first == second {
        return Err(ChordError::Unsupported(edge));
    }
    let interval = definition.interval();
    let turns = TURN_SAMPLES
        .iter()
        .map(|fraction| {
            let at = definition.curve().point(interval.at(*fraction));
            let a = face_normal(solid, *first, at).ok_or(ChordError::Unsupported(edge))?;
            let b = face_normal(solid, *second, at).ok_or(ChordError::Unsupported(edge))?;
            Ok(a.angle_between(b))
        })
        .collect::<Result<Vec<f64>, ChordError>>()?;
    let smooth = |turn: &f64| *turn <= TANGENT_FACE_ANGLE;
    if turns.iter().all(smooth) {
        return Ok(Turn::Smooth);
    }
    let Some(&reference) = turns.first() else {
        return Err(ChordError::Unsupported(edge));
    };
    if turns
        .iter()
        .any(|turn| (turn - reference).abs() > SAME_TURN)
    {
        return Err(ChordError::VaryingTurn(edge));
    }
    if std::f64::consts::PI - reference <= TANGENT_FACE_ANGLE {
        return Err(ChordError::BackToBack(edge));
    }
    Ok(Turn::Constant(reference))
}

pub fn chord_fillet_radius(solid: &Solid, edges: &[EdgeId], chord: f64) -> Result<f64, ChordError> {
    if edges.is_empty() {
        return Err(ChordError::NoEdges);
    }
    let chain = match blend_chain(solid, edges) {
        chain if chain.is_empty() => edges.to_vec(),
        chain => chain,
    };
    let mut found: Option<(EdgeId, f64)> = None;
    let mut first_smooth = None;
    for edge in chain {
        match turn_along(solid, edge)? {
            Turn::Smooth => {
                first_smooth.get_or_insert(edge);
            }
            Turn::Constant(turn) => match found {
                None => found = Some((edge, turn)),
                Some((first, reference)) if (turn - reference).abs() > SAME_TURN => {
                    return Err(ChordError::DifferingTurns {
                        first,
                        second: edge,
                    });
                }
                Some(_) => {}
            },
        }
    }
    match (found, first_smooth) {
        (Some((_, turn)), _) => Ok(chord / (2.0 * (turn / 2.0).sin())),
        (None, Some(edge)) => Err(ChordError::Smooth(edge)),
        (None, None) => Err(ChordError::NoEdges),
    }
}
