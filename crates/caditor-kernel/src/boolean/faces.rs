use std::collections::{BTreeMap, BTreeSet};

use crate::{
    boolean::{
        BooleanError, FaceKey, Input, Operand,
        imprint::Arrangement,
        trace::{Chart, Coedge, Fragment, HalfEdge, TracedLoop, fit_loop, group, snap_ends, trace},
        untangle::{sharpen, untangle},
    },
    interrupt,
    sense::Sense,
    surface::Surface,
    topology::{Face, LoopId, Solid, continues, fit_pcurve, signed_area},
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
    let shared = arrangement.shared_pieces();
    let mut split = Vec::new();
    for operand in Operand::BOTH
        .into_iter()
        .filter(|operand| input.traces(*operand))
    {
        let solid = input.solid(operand);
        for (id, face) in solid.faces() {
            interrupt::check()?;
            let key = FaceKey { operand, face: id };
            let divided = split_face(input, arrangement, &shared, key, face).map_err(|error| {
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
    shared: &BTreeSet<usize>,
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
    let Boundary { loops, cuts } = boundary(solid, arrangement, key, face)?;
    if cuts.is_empty()
        && let Some(fragment) = passed_through(solid, arrangement, key, face)
    {
        return Ok(SplitFace {
            key,
            fragments: vec![fragment],
            untouched: true,
        });
    }
    let quiet = if input.carrying {
        quiet_loops(solid, arrangement, shared, key, face, &loops, &cuts)
    } else {
        BTreeMap::new()
    };
    let mut half_edges: Vec<HalfEdge> = loops
        .into_iter()
        .enumerate()
        .filter(|(position, _)| !quiet.contains_key(position))
        .flat_map(|(_, members)| members)
        .collect();
    for piece in cuts {
        half_edges.push(HalfEdge::new(piece, Sense::Same, None));
        half_edges.push(HalfEdge::new(piece, Sense::Reversed, None));
    }
    let mut loops = trace(arrangement, &chart, &half_edges)?
        .into_iter()
        .map(|members| fit_loop(arrangement, &chart, members))
        .collect::<Result<Vec<_>, _>>()?;
    loops.extend(quiet.into_values());
    untangle(arrangement, &chart, &mut loops)?;
    let mut fragments = group(&chart, loops)?;
    sharpen(arrangement, &chart, &mut fragments)?;
    Ok(SplitFace {
        key,
        fragments,
        untouched: false,
    })
}

fn quiet_loops(
    solid: &Solid,
    arrangement: &Arrangement,
    shared: &BTreeSet<usize>,
    key: FaceKey,
    face: &Face,
    loops: &[Vec<HalfEdge>],
    cuts: &BTreeSet<usize>,
) -> BTreeMap<usize, TracedLoop> {
    let mut leaving: BTreeMap<usize, usize> = BTreeMap::new();
    let boundary_starts = loops
        .iter()
        .flatten()
        .filter_map(|half_edge| start_vertex(arrangement, half_edge));
    let cut_ends = cuts
        .iter()
        .filter_map(|piece| arrangement.piece(*piece))
        .flat_map(|piece| [piece.start, piece.end]);
    for vertex in boundary_starts.chain(cut_ends) {
        *leaving.entry(vertex).or_default() += 1;
    }
    face.loops()
        .iter()
        .zip(loops)
        .enumerate()
        .filter(|(_, (loop_id, members))| {
            unsplit(solid, arrangement, shared, key, **loop_id)
                && alone(arrangement, &leaving, members)
        })
        .filter_map(|(position, (loop_id, _))| {
            let carried = carried_loop(solid, arrangement, key, face.surface(), *loop_id)?;
            Some((position, carried))
        })
        .collect()
}

fn start_vertex(arrangement: &Arrangement, half_edge: &HalfEdge) -> Option<usize> {
    Some(
        arrangement
            .piece(half_edge.piece)?
            .start_of(half_edge.sense),
    )
}

fn unsplit(
    solid: &Solid,
    arrangement: &Arrangement,
    shared: &BTreeSet<usize>,
    key: FaceKey,
    loop_id: LoopId,
) -> bool {
    solid.face_loop(loop_id).is_some_and(|face_loop| {
        face_loop.coedges().iter().all(|coedge| {
            solid.coedge(*coedge).is_some_and(|coedge| {
                let [piece] = arrangement.edge_pieces(key.operand, coedge.edge()) else {
                    return false;
                };
                arrangement.representative(*piece).0 == *piece && !shared.contains(piece)
            })
        })
    })
}

fn alone(
    arrangement: &Arrangement,
    leaving: &BTreeMap<usize, usize>,
    members: &[HalfEdge],
) -> bool {
    let mut own: BTreeMap<usize, usize> = BTreeMap::new();
    for half_edge in members {
        let Some(vertex) = start_vertex(arrangement, half_edge) else {
            return false;
        };
        *own.entry(vertex).or_default() += 1;
    }
    own.iter()
        .all(|(vertex, count)| leaving.get(vertex) == Some(count))
}

fn passed_through(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Option<Fragment> {
    let loops = face
        .loops()
        .iter()
        .map(|loop_id| carried_loop(solid, arrangement, key, face.surface(), *loop_id))
        .collect::<Option<Vec<_>>>()?;
    Some(Fragment { loops })
}

fn carried_loop(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    surface: &Surface,
    loop_id: LoopId,
) -> Option<TracedLoop> {
    let face_loop = solid.face_loop(loop_id)?;
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
    (closed && traced.area.is_finite() && traced.area != 0.0).then_some(traced)
}

struct Boundary {
    loops: Vec<Vec<HalfEdge>>,
    cuts: BTreeSet<usize>,
}

fn boundary(
    solid: &Solid,
    arrangement: &Arrangement,
    key: FaceKey,
    face: &Face,
) -> Result<Boundary, BooleanError> {
    let mut loops = Vec::with_capacity(face.loops().len());
    for loop_id in face.loops() {
        let face_loop = solid.face_loop(*loop_id).ok_or_else(BooleanError::split)?;
        let mut members = Vec::new();
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
                members.push(HalfEdge::new(
                    representative,
                    coedge.sense().combined(relation),
                    Some(coedge.pcurve().uv_at(start)),
                ));
            }
        }
        loops.push(members);
    }
    let on_boundary: BTreeSet<usize> = loops
        .iter()
        .flatten()
        .map(|half_edge| half_edge.piece)
        .collect();
    let mut cuts: BTreeSet<usize> = arrangement
        .cuts(key)
        .iter()
        .map(|piece| arrangement.representative(*piece).0)
        .filter(|piece| !on_boundary.contains(piece))
        .collect();
    prune_dangling(arrangement, loops.iter().flatten(), &mut cuts);
    Ok(Boundary { loops, cuts })
}

fn prune_dangling<'a>(
    arrangement: &Arrangement,
    boundary: impl Iterator<Item = &'a HalfEdge>,
    cuts: &mut BTreeSet<usize>,
) {
    let mut incidence: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let pieces = boundary
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
