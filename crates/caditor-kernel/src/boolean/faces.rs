use std::collections::{BTreeMap, BTreeSet};

use crate::{
    boolean::{
        BooleanError, FaceKey, Input, Operand,
        imprint::Arrangement,
        trace::{Chart, Fragment, HalfEdge, fit_loop, group, trace},
    },
    sense::Sense,
    topology::{Face, Solid},
};

#[derive(Debug, Clone)]
pub(super) struct SplitFace {
    pub key: FaceKey,
    pub fragments: Vec<Fragment>,
}

pub(super) fn split(
    input: &Input,
    arrangement: &Arrangement,
) -> Result<Vec<SplitFace>, BooleanError> {
    let mut split = Vec::new();
    for operand in Operand::BOTH {
        let solid = input.solid(operand);
        for (id, face) in solid.faces() {
            let key = FaceKey { operand, face: id };
            let center = input
                .bounds(key)
                .map(|bounds| bounds.uv.center())
                .ok_or(BooleanError::Split)?;
            let chart = Chart {
                surface: face.surface(),
                sense: face.sense(),
                center,
            };
            let half_edges = half_edges(solid, arrangement, key, face)?;
            let loops = trace(arrangement, &chart, &half_edges)?
                .into_iter()
                .map(|members| fit_loop(arrangement, &chart, members))
                .collect::<Result<Vec<_>, _>>()?;
            split.push(SplitFace {
                key,
                fragments: group(&chart, loops)?,
            });
        }
    }
    Ok(split)
}

fn half_edges(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Result<Vec<HalfEdge>, BooleanError> {
    let mut boundary = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id).ok_or(BooleanError::Split)?;
        for coedge_id in face_loop.coedges() {
            let coedge = solid.coedge(*coedge_id).ok_or(BooleanError::Split)?;
            let mut pieces = arrangement.edge_pieces(key.operand, coedge.edge()).to_vec();
            if !coedge.sense().is_same() {
                pieces.reverse();
            }
            for piece in pieces {
                let data = arrangement.piece(piece).ok_or(BooleanError::Split)?;
                let start = if coedge.sense().is_same() {
                    data.interval.start()
                } else {
                    data.interval.end()
                };
                let (representative, relation) = arrangement.representative(piece);
                boundary.push(HalfEdge::new(
                    representative,
                    coedge.sense().combined(relation),
                    Some(coedge.pcurve().uv_at(start)),
                ));
            }
        }
    }
    let on_boundary: BTreeSet<usize> = boundary.iter().map(|half_edge| half_edge.piece).collect();
    let mut cuts: BTreeSet<usize> = arrangement
        .cuts(key)
        .iter()
        .map(|piece| arrangement.representative(*piece).0)
        .filter(|piece| !on_boundary.contains(piece))
        .collect();
    prune_dangling(arrangement, &boundary, &mut cuts);
    let mut half_edges = boundary;
    for piece in cuts {
        half_edges.push(HalfEdge::new(piece, Sense::Same, None));
        half_edges.push(HalfEdge::new(piece, Sense::Reversed, None));
    }
    Ok(half_edges)
}

fn prune_dangling(arrangement: &Arrangement, boundary: &[HalfEdge], cuts: &mut BTreeSet<usize>) {
    loop {
        let mut incidence: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let pieces = boundary
            .iter()
            .map(|half_edge| half_edge.piece)
            .chain(cuts.iter().copied());
        for piece in pieces {
            if let Some(data) = arrangement.piece(piece) {
                incidence.entry(data.start).or_default().insert(piece);
                incidence.entry(data.end).or_default().insert(piece);
            }
        }
        let dangling: Vec<usize> = cuts
            .iter()
            .copied()
            .filter(|piece| {
                arrangement.piece(*piece).is_some_and(|data| {
                    !data.is_closed()
                        && [data.start, data.end].iter().any(|vertex| {
                            incidence.get(vertex).is_none_or(|around| around.len() < 2)
                        })
                })
            })
            .collect();
        if dangling.is_empty() {
            return;
        }
        for piece in dangling {
            cuts.remove(&piece);
        }
    }
}
