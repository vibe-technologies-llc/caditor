use std::collections::{BTreeMap, BTreeSet};

use crate::{
    boolean::{
        BooleanError, FaceKey, Input, Operand,
        imprint::Arrangement,
        trace::{Chart, Coedge, Fragment, HalfEdge, TracedLoop, fit_loop, group, snap_ends, trace},
    },
    interrupt,
    sense::Sense,
    topology::{Face, Solid, continues, fit_pcurve, signed_area},
};

#[derive(Debug, Clone)]
pub(super) struct SplitFace {
    pub key: FaceKey,
    pub fragments: Vec<Fragment>,
    pub untouched: bool,
}

pub(super) fn split(
    input: &Input,
    arrangement: &Arrangement,
) -> Result<Vec<SplitFace>, BooleanError> {
    let mut split = Vec::new();
    for operand in Operand::BOTH {
        let solid = input.solid(operand);
        for (id, face) in solid.faces() {
            interrupt::check()?;
            let key = FaceKey { operand, face: id };
            let divided = split_face(input, arrangement, key, face).map_err(|error| {
                error.or_faces([key]).or_point(|| {
                    let center = input.bounds(key)?.uv.center();
                    Some(face.surface().point_at(center))
                })
            })?;
            split.push(divided);
        }
    }
    Ok(split)
}

fn split_face(
    input: &Input,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Result<SplitFace, BooleanError> {
    let solid = input.solid(key.operand);
    let center = input
        .bounds(key)
        .map(|bounds| bounds.uv.center())
        .ok_or_else(BooleanError::split)?;
    let chart = Chart {
        surface: face.surface(),
        sense: face.sense(),
        center,
    };
    let half_edges = half_edges(solid, arrangement, key, face)?;
    let cut = half_edges.iter().any(|half_edge| half_edge.hint.is_none());
    if !cut && let Some(fragment) = passed_through(solid, arrangement, key, face) {
        return Ok(SplitFace {
            key,
            fragments: vec![fragment],
            untouched: true,
        });
    }
    let loops = trace(arrangement, &chart, &half_edges)?
        .into_iter()
        .map(|members| fit_loop(arrangement, &chart, members))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SplitFace {
        key,
        fragments: group(&chart, loops)?,
        untouched: false,
    })
}

fn passed_through(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Option<Fragment> {
    let surface = face.surface();
    let mut loops = Vec::with_capacity(face.loops().len());
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id)?;
        let mut coedges = Vec::with_capacity(face_loop.coedges().len());
        for coedge_id in face_loop.coedges() {
            let coedge = solid.coedge(*coedge_id)?;
            let [piece] = arrangement.edge_pieces(key.operand, coedge.edge()) else {
                return None;
            };
            let (representative, relation) = arrangement.representative(*piece);
            let sense = coedge.sense().combined(relation);
            let pcurve = if representative == *piece {
                coedge.pcurve().clone()
            } else {
                let (curve, data) = arrangement.curve(representative)?;
                let hint = Some(coedge.pcurve().start());
                fit_pcurve(surface, curve, data.interval, sense, hint).ok()?
            };
            coedges.push(Coedge {
                half_edge: HalfEdge::new(representative, sense, Some(pcurve.start())),
                pcurve,
            });
        }
        snap_ends(arrangement, surface, &mut coedges);
        let count = coedges.len();
        let closed = coedges.iter().enumerate().all(|(index, coedge)| {
            coedges
                .get((index + 1) % count)
                .is_some_and(|next| continues(surface, coedge.pcurve.end(), next.pcurve.start()))
        });
        let mut traced = TracedLoop { coedges, area: 0.0 };
        traced.area = signed_area(&traced.polygon());
        if !closed || !traced.area.is_finite() || traced.area == 0.0 {
            return None;
        }
        loops.push(traced);
    }
    Some(Fragment { loops })
}

fn half_edges(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Result<Vec<HalfEdge>, BooleanError> {
    let mut boundary = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id).ok_or_else(BooleanError::split)?;
        for coedge_id in face_loop.coedges() {
            let coedge = solid.coedge(*coedge_id).ok_or_else(BooleanError::split)?;
            let mut pieces = arrangement.edge_pieces(key.operand, coedge.edge()).to_vec();
            if !coedge.sense().is_same() {
                pieces.reverse();
            }
            for piece in pieces {
                let data = arrangement.piece(piece).ok_or_else(BooleanError::split)?;
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
    let mut pending: Vec<usize> = cuts.iter().copied().collect();
    while let Some(piece) = pending.pop() {
        if !cuts.contains(&piece) {
            continue;
        }
        let Some(data) = arrangement.piece(piece) else {
            continue;
        };
        let ends = [data.start, data.end];
        let dangling = !data.is_closed()
            && ends
                .iter()
                .any(|vertex| incidence.get(vertex).is_none_or(|around| around.len() < 2));
        if !dangling {
            continue;
        }
        cuts.remove(&piece);
        for vertex in ends {
            if let Some(around) = incidence.get_mut(&vertex) {
                around.remove(&piece);
                pending.extend(around.iter().copied().filter(|other| cuts.contains(other)));
            }
        }
    }
}
