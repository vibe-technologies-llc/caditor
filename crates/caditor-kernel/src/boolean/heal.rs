use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb2, Point3};

use crate::{
    boolean::{
        BooleanError, TOLERANCE,
        imprint::{Arrangement, Piece},
        select::KeptFace,
        trace::{Chart, Coedge, HalfEdge, TracedLoop, fit_loop, group, snap_ends, trace},
    },
    curve::Curve,
    interval::Interval,
    sense::Sense,
    surface::Surface,
    topology::fit_pcurve,
};

const PARAMETER_MATCH: f64 = 1e-9;
const PERIOD_SLACK: f64 = 1e-9;

type Uses = BTreeMap<usize, Vec<(usize, Sense)>>;

fn uses(faces: &[KeptFace]) -> Uses {
    let mut uses: Uses = BTreeMap::new();
    for (index, face) in faces.iter().enumerate() {
        for coedge in face
            .fragment
            .loops
            .iter()
            .flat_map(|traced| &traced.coedges)
        {
            uses.entry(coedge.half_edge.piece)
                .or_default()
                .push((index, coedge.half_edge.sense));
        }
    }
    uses
}

pub(super) fn check_closed(faces: &[KeptFace]) -> Result<(), BooleanError> {
    for list in uses(faces).values() {
        let forward = list.iter().filter(|(_, sense)| sense.is_same()).count();
        let backward = list.len() - forward;
        if forward != backward {
            return Err(BooleanError::Open);
        }
        if forward > 1 {
            return Err(BooleanError::NonManifold);
        }
    }
    Ok(())
}

pub(super) fn heal(
    arrangement: &mut Arrangement,
    kept: Vec<KeptFace>,
) -> Result<Vec<KeptFace>, BooleanError> {
    check_closed(&kept)?;
    let mut faces = merge_faces(arrangement, kept);
    heal_edges(arrangement, &mut faces)?;
    Ok(faces)
}

fn continuous(first: &KeptFace, second: &KeptFace) -> bool {
    if first.key == second.key {
        return first.sense == second.sense;
    }
    first
        .surface
        .same_surface(&second.surface)
        .is_some_and(|relation| relation.combined(first.sense) == second.sense)
}

fn root(parents: &[usize], mut item: usize) -> usize {
    while let Some(parent) = parents.get(item).copied() {
        if parent == item {
            break;
        }
        item = parent;
    }
    item
}

fn merge_faces(arrangement: &Arrangement, faces: Vec<KeptFace>) -> Vec<KeptFace> {
    let mut parents: Vec<usize> = (0..faces.len()).collect();
    for list in uses(&faces).values() {
        let [(first, _), (second, _)] = list.as_slice() else {
            continue;
        };
        if first == second {
            continue;
        }
        let (Some(a), Some(b)) = (faces.get(*first), faces.get(*second)) else {
            continue;
        };
        if continuous(a, b) {
            let (low, high) = {
                let (a, b) = (root(&parents, *first), root(&parents, *second));
                (a.min(b), a.max(b))
            };
            if let Some(slot) = parents.get_mut(high) {
                *slot = low;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..faces.len() {
        groups.entry(root(&parents, index)).or_default().push(index);
    }
    let mut merged = Vec::with_capacity(faces.len());
    for members in groups.values() {
        let combined = if members.len() > 1 {
            merge_group(arrangement, &faces, members)
        } else {
            None
        };
        match combined {
            Some(result) => merged.extend(result),
            None => merged.extend(
                members
                    .iter()
                    .filter_map(|index| faces.get(*index))
                    .cloned(),
            ),
        }
    }
    merged
}

fn merge_group(
    arrangement: &Arrangement,
    faces: &[KeptFace],
    members: &[usize],
) -> Option<Vec<KeptFace>> {
    let lead = faces.get(*members.first()?)?;
    let mut owners: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for member in members {
        for coedge in faces
            .get(*member)?
            .fragment
            .loops
            .iter()
            .flat_map(|l| &l.coedges)
        {
            owners
                .entry(coedge.half_edge.piece)
                .or_default()
                .insert(*member);
        }
    }
    let mut half_edges = Vec::new();
    let lead_index = *members.first()?;
    for member in members {
        let face = faces.get(*member)?;
        for coedge in face.fragment.loops.iter().flat_map(|l| &l.coedges) {
            let internal = owners
                .get(&coedge.half_edge.piece)
                .is_some_and(|set| set.len() > 1);
            if internal {
                continue;
            }
            let hint = (*member == lead_index).then(|| coedge.pcurve.start());
            half_edges.push(HalfEdge::new(
                coedge.half_edge.piece,
                coedge.half_edge.sense,
                hint,
            ));
        }
    }
    let center = Aabb2::from_points(
        lead.fragment
            .loops
            .iter()
            .flat_map(|traced| traced.polygon()),
    )?
    .center();
    let chart = Chart {
        surface: &lead.surface,
        sense: lead.sense,
        center,
    };
    let loops = trace(arrangement, &chart, &half_edges)
        .ok()?
        .into_iter()
        .map(|members| fit_loop(arrangement, &chart, members))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let fragments = group(&chart, loops).ok()?;
    Some(
        fragments
            .into_iter()
            .map(|fragment| KeptFace {
                fragment,
                ..lead.clone()
            })
            .collect(),
    )
}

fn incidence(arrangement: &Arrangement, faces: &[KeptFace]) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut around: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for coedge in faces
        .iter()
        .flat_map(|face| face.fragment.loops.iter())
        .flat_map(|traced| &traced.coedges)
    {
        if let Some(piece) = arrangement.piece(coedge.half_edge.piece) {
            around
                .entry(piece.start)
                .or_default()
                .insert(coedge.half_edge.piece);
            around
                .entry(piece.end)
                .or_default()
                .insert(coedge.half_edge.piece);
        }
    }
    around
}

fn face_users(uses: &Uses, piece: usize) -> Vec<usize> {
    let mut users: Vec<usize> = uses
        .get(&piece)
        .map(|list| list.iter().map(|(face, _)| *face).collect())
        .unwrap_or_default();
    users.sort_unstable();
    users.dedup();
    users
}

struct Joints {
    around: BTreeMap<usize, BTreeSet<usize>>,
    candidates: BTreeSet<usize>,
    uses: Uses,
}

impl Joints {
    fn new(arrangement: &Arrangement, faces: &[KeptFace]) -> Self {
        let around = incidence(arrangement, faces);
        let candidates = around
            .iter()
            .filter(|(_, pieces)| pieces.len() == 2)
            .map(|(vertex, _)| *vertex)
            .collect();
        Self {
            around,
            candidates,
            uses: uses(faces),
        }
    }

    fn next(&mut self) -> Option<(usize, usize, usize)> {
        let vertex = self.candidates.pop_first()?;
        let mut pieces = self.around.get(&vertex)?.iter().copied();
        Some((vertex, pieces.next()?, pieces.next()?))
    }

    fn joined(&mut self, arrangement: &Arrangement, joint: &Joint) {
        self.around.remove(&joint.vertex);
        let replaced: Vec<usize> = joint
            .pieces
            .iter()
            .filter_map(|piece| arrangement.piece(*piece))
            .flat_map(|piece| [piece.start, piece.end])
            .filter(|vertex| *vertex != joint.vertex)
            .collect();
        for vertex in replaced {
            let Some(pieces) = self.around.get_mut(&vertex) else {
                continue;
            };
            for piece in joint.pieces {
                pieces.remove(&piece);
            }
            pieces.insert(joint.merged);
            if pieces.len() == 2 {
                self.candidates.insert(vertex);
            } else {
                self.candidates.remove(&vertex);
            }
        }
        let [first, second] = joint.pieces;
        let inherited = self.uses.remove(&first).unwrap_or_default();
        self.uses.remove(&second);
        self.uses.insert(joint.merged, inherited);
    }
}

fn heal_edges(arrangement: &mut Arrangement, faces: &mut [KeptFace]) -> Result<(), BooleanError> {
    let mut joints = Joints::new(arrangement, faces);
    while let Some((vertex, first, second)) = joints.next() {
        let users = face_users(&joints.uses, first);
        let joined = (users == face_users(&joints.uses, second))
            .then(|| join(arrangement, vertex, first, second))
            .flatten();
        let Some(piece) = joined else {
            continue;
        };
        let merged = arrangement.add_piece(piece);
        let joint = Joint {
            vertex,
            pieces: [first, second],
            merged,
        };
        for index in &users {
            let Some(face) = faces.get_mut(*index) else {
                continue;
            };
            for traced in &mut face.fragment.loops {
                let mut replaced = false;
                while replace_pair(arrangement, &face.surface, traced, &joint)? {
                    replaced = true;
                }
                if replaced {
                    snap_ends(arrangement, &face.surface, &mut traced.coedges);
                }
            }
        }
        joints.joined(arrangement, &joint);
    }
    Ok(())
}

struct Joint {
    vertex: usize,
    pieces: [usize; 2],
    merged: usize,
}

fn replace_pair(
    arrangement: &Arrangement,
    surface: &Surface,
    traced: &mut TracedLoop,
    joint: &Joint,
) -> Result<bool, BooleanError> {
    let Joint {
        vertex,
        pieces,
        merged,
    } = *joint;
    let (curve, piece) = arrangement.curve(merged).ok_or(BooleanError::Split)?;
    let count = traced.coedges.len();
    let position = (0..count).find(|index| {
        let (Some(current), Some(next)) = (
            traced.coedges.get(*index),
            traced.coedges.get((index + 1) % count),
        ) else {
            return false;
        };
        let ends_here = arrangement
            .piece(current.half_edge.piece)
            .is_some_and(|data| data.end_of(current.half_edge.sense) == vertex);
        current.half_edge.piece != next.half_edge.piece
            && ends_here
            && pieces.contains(&current.half_edge.piece)
            && pieces.contains(&next.half_edge.piece)
    });
    let Some(position) = position else {
        return Ok(false);
    };
    let Some(current) = traced.coedges.get(position) else {
        return Ok(false);
    };
    let (old_curve, old_piece) = arrangement
        .curve(current.half_edge.piece)
        .ok_or(BooleanError::Split)?;
    let probe = old_piece.interval.middle();
    let travel = old_curve.evaluate(probe).first * current.half_edge.sense.sign();
    let on_merged = curve.closest_parameter(old_curve.point(probe), piece.interval);
    let sense = Sense::from_sign(travel.dot(curve.evaluate(on_merged).first));
    let pcurve = fit_pcurve(
        surface,
        curve,
        piece.interval,
        sense,
        Some(current.pcurve.start()),
    )
    .map_err(|_| BooleanError::Split)?;
    let coedge = Coedge {
        half_edge: HalfEdge::new(merged, sense, Some(pcurve.start())),
        pcurve,
    };
    let following = (position + 1) % count;
    if following == 0 {
        traced.coedges.remove(position);
        if let Some(slot) = traced.coedges.first_mut() {
            *slot = coedge;
        }
    } else {
        traced.coedges.remove(following);
        if let Some(slot) = traced.coedges.get_mut(position) {
            *slot = coedge;
        }
    }
    Ok(true)
}

fn far_end(piece: &Piece, vertex: usize) -> usize {
    if piece.start == vertex {
        piece.end
    } else {
        piece.start
    }
}

fn join(arrangement: &Arrangement, vertex: usize, first: usize, second: usize) -> Option<Piece> {
    let a = *arrangement.piece(first)?;
    let b = *arrangement.piece(second)?;
    if a.is_closed() || b.is_closed() {
        return None;
    }
    same_source(arrangement, vertex, &a, &b)
        .or_else(|| same_source(arrangement, vertex, &b, &a))
        .or_else(|| extended(arrangement, vertex, &a, &b))
}

fn same_source(arrangement: &Arrangement, vertex: usize, a: &Piece, b: &Piece) -> Option<Piece> {
    if a.source != b.source || a.end != vertex || b.start != vertex {
        return None;
    }
    let curve = &arrangement.source(a.source)?.curve;
    let gap = b.interval.start() - a.interval.end();
    let shift = match curve.period() {
        Some(period) => -(gap / period).round() * period,
        None => 0.0,
    };
    let residual = gap + shift;
    if residual.abs() > PARAMETER_MATCH * (1.0 + a.interval.end().abs()) {
        return None;
    }
    let interval = Interval::new(a.interval.start(), a.interval.end() + b.interval.length())?;
    fits_period(curve, interval).then_some(Piece {
        source: a.source,
        interval,
        start: a.start,
        end: b.end,
    })
}

fn fits_period(curve: &Curve, interval: Interval) -> bool {
    curve
        .period()
        .is_none_or(|period| interval.length() <= period * (1.0 + PERIOD_SLACK))
}

fn extended(arrangement: &Arrangement, vertex: usize, a: &Piece, b: &Piece) -> Option<Piece> {
    let curve = &arrangement.source(a.source)?.curve;
    let other = &arrangement.source(b.source)?.curve;
    let scale = match (curve, other) {
        (Curve::Line(_), Curve::Line(_)) => 1.0,
        (Curve::Circle(circle), Curve::Circle(_)) => circle.radius(),
        _ => return None,
    };
    if on_curve(curve, other.point(b.interval.middle())) > TOLERANCE {
        return None;
    }
    let step = other.length(b.interval) / scale;
    let target = arrangement.point(far_end(b, vertex))?;
    let (interval, start, end) = if a.end == vertex {
        (
            Interval::new(a.interval.start(), a.interval.end() + step)?,
            a.start,
            far_end(b, vertex),
        )
    } else if a.start == vertex {
        (
            Interval::new(a.interval.start() - step, a.interval.end())?,
            far_end(b, vertex),
            a.end,
        )
    } else {
        return None;
    };
    let reached = if a.end == vertex {
        curve.point(interval.end())
    } else {
        curve.point(interval.start())
    };
    (reached.distance(target) <= TOLERANCE && fits_period(curve, interval)).then_some(Piece {
        source: a.source,
        interval,
        start,
        end,
    })
}

fn on_curve(curve: &Curve, point: Point3) -> f64 {
    let parameter = match curve {
        Curve::Line(line) => line.parameter_of(point),
        _ => curve.closest_parameter(point, Interval::FULL_TURN),
    };
    curve.point(parameter).distance(point)
}
