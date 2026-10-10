use std::collections::BTreeSet;

use caditor_geometry::{Point3, Vector3};

use super::{
    BlendError, BlendShape, Blended, End, EndPlane, Ends, Named, SquareEnd, Surroundings, Tool,
    beside_vertex,
    chain::{Chain, Run},
    cut_ends, edge_faces, end_at, end_planes_at, face_normal,
    loft::{self, Loft, Outline, Segment},
    named,
    section::Side,
    station::{Cross, Missed, Request, Sides, cross, on_both},
    tangent_faces,
};
use crate::{
    curve::BSplineCurve,
    interval::Interval,
    naming::FaceName,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid, SolidClassifier, VertexId},
};

const OVERHANG: f64 = 0.5;
const APEX_REACH: f64 = 2.0;
const FIRST_STATIONS: usize = 8;
const MAX_STATIONS: usize = 1024;
const OUTLINE_FIT: f64 = 1e-6;
const FOOT_FIT: f64 = 2e-7;
const ARC_FIT: f64 = 1e-5;
const SWITCH_SAMPLES: usize = 16;
const SWITCH_BISECTIONS: usize = 48;
const SHORTEST_PIECE: f64 = 10.0 * LINEAR_RESOLUTION;
const JOINT_GAP: f64 = 10.0 * LINEAR_RESOLUTION;
const BREAK_OFFSET: f64 = 1e-4;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RunSides {
    pub run: Run,
    pub faces: [FaceId; 2],
    pub same: [bool; 2],
    pub candidates: [Vec<FaceId>; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Stations {
    pub sides: RunSides,
    pub params: Vec<f64>,
    pub crosses: Vec<Cross>,
    pub name: Named,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rounded {
    pub runs: Vec<Stations>,
    pub convex: bool,
}

pub(super) struct Context<'a> {
    pub around: &'a Surroundings<'a>,
    pub classifier: &'a SolidClassifier<'a>,
    pub shape: BlendShape,
    pub feature: u64,
    pub measured_on: &'a dyn Fn([FaceId; 2]) -> Side,
}

pub(super) struct Planned {
    pub tool: Tool,
    pub rounded: Option<Rounded>,
}

fn unsupported(edge: EdgeId) -> BlendError {
    BlendError::Unsupported(edge)
}

fn ordered_sides(
    solid: &Solid,
    run: Run,
    previous: Option<&RunSides>,
) -> Result<RunSides, BlendError> {
    let faces = edge_faces(solid, run.edge);
    let [(first, first_same), (second, second_same)] = faces.as_slice() else {
        return Err(unsupported(run.edge));
    };
    if first == second {
        return Err(unsupported(run.edge));
    }
    let mut sides = RunSides {
        run,
        faces: [*first, *second],
        same: [*first_same, *second_same],
        candidates: [
            tangent_faces(solid, &[*first]),
            tangent_faces(solid, &[*second]),
        ],
    };
    if let Some(previous) = previous {
        let kept =
            previous.candidates[0].contains(first) || previous.candidates[1].contains(second);
        let swapped =
            previous.candidates[0].contains(second) || previous.candidates[1].contains(first);
        if !kept && swapped {
            sides.faces.swap(0, 1);
            sides.same.swap(0, 1);
            sides.candidates.swap(0, 1);
        } else if !kept {
            return Err(unsupported(run.edge));
        }
    }
    for (candidates, own) in sides.candidates.iter_mut().zip(sides.faces) {
        candidates.retain(|face| *face != own);
        candidates.insert(0, own);
    }
    Ok(sides)
}

fn sides_of(solid: &Solid, chain: &Chain) -> Result<Vec<RunSides>, BlendError> {
    let mut found: Vec<RunSides> = Vec::with_capacity(chain.runs.len());
    for run in &chain.runs {
        let sides = ordered_sides(solid, *run, found.last())?;
        found.push(sides);
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Station {
    point: Point3,
    tangent: Vector3,
    along: Vector3,
}

fn faces_tangent(
    solid: &Solid,
    faces: [FaceId; 2],
    point: Point3,
    toward: Vector3,
) -> Option<Vector3> {
    let crossing = face_normal(solid, faces[0], point)?
        .cross(face_normal(solid, faces[1], point)?)
        .try_normalize()?;
    Some(if crossing.dot(toward) < 0.0 {
        -crossing
    } else {
        crossing
    })
}

fn edge_station(solid: &Solid, sides: &RunSides, parameter: f64) -> Option<Station> {
    let definition = solid.edge(sides.run.edge)?;
    let derivatives = definition.curve().evaluate(parameter);
    let along = derivatives.first.try_normalize()?;
    let forward = if sides.run.to >= sides.run.from {
        along
    } else {
        -along
    };
    let tangent = faces_tangent(solid, sides.faces, derivatives.point, forward).unwrap_or(forward);
    Some(Station {
        point: derivatives.point,
        tangent,
        along,
    })
}

fn blended_station(
    solid: &Solid,
    sides: &RunSides,
    fraction: f64,
    ends: [Vector3; 2],
) -> Option<Station> {
    let Run { from, to, .. } = sides.run;
    let at = |fraction: f64| from + (to - from) * fraction;
    let own = edge_station(solid, sides, at(fraction))?;
    let start = edge_station(solid, sides, at(0.0))?.tangent;
    let end = edge_station(solid, sides, at(1.0))?.tangent;
    let tangent = (own.tangent + (ends[0] - start) * (1.0 - fraction) + (ends[1] - end) * fraction)
        .try_normalize()?;
    Some(Station { tangent, ..own })
}

fn extended_station(
    solid: &Solid,
    sides: &RunSides,
    from: &Station,
    distance: f64,
) -> Option<Station> {
    let surfaces = [
        solid.face(sides.faces[0])?.surface(),
        solid.face(sides.faces[1])?.surface(),
    ];
    let guess = from.point + from.tangent * distance;
    let point = on_both(surfaces, guess, from.tangent)?;
    let tangent = faces_tangent(solid, sides.faces, point, from.tangent)?;
    let along = tangent * from.tangent.dot(from.along).signum();
    Some(Station {
        point,
        tangent,
        along,
    })
}

fn inward(solid: &Solid, sides: &RunSides, station: &Station) -> Option<[Vector3; 2]> {
    let mut found = [Vector3::ZERO; 2];
    for ((slot, face), same) in found.iter_mut().zip(sides.faces).zip(sides.same) {
        let normal = face_normal(solid, face, station.point)?;
        *slot = normal
            .cross(if same { station.along } else { -station.along })
            .try_normalize()?;
    }
    Some(found)
}

fn station_cross(
    context: &Context<'_>,
    sides: &RunSides,
    extra: Option<&RunSides>,
    convex: bool,
    station: &Station,
) -> Result<Cross, BlendError> {
    let solid = context.around.solid;
    let edge = sides.run.edge;
    let inward = inward(solid, sides, station).ok_or(unsupported(edge))?;
    let joined = |index: usize| -> Vec<FaceId> {
        let mut faces = sides.candidates.get(index).cloned().unwrap_or_default();
        if let Some(extra) = extra.and_then(|extra| extra.candidates.get(index)) {
            let more: Vec<FaceId> = extra
                .iter()
                .filter(|face| !faces.contains(face))
                .copied()
                .collect();
            faces.extend(more);
        }
        faces
    };
    let candidates = [joined(0), joined(1)];
    let request = Request {
        point: station.point,
        tangent: station.tangent,
        inward,
        shape: context.shape,
        measured_on: (context.measured_on)(sides.faces),
    };
    let sided = Sides {
        solid,
        classifier: context.classifier,
        candidates: [&candidates[0], &candidates[1]],
        convex,
    };
    cross(&sided, &request).map_err(|missed| match missed {
        Missed::Short => BlendError::TooLarge(edge),
        Missed::Angle => BlendError::AngleMisses(edge),
    })
}

pub(super) fn reach(cross: &Cross) -> f64 {
    let feet = cross
        .feet
        .iter()
        .map(|foot| foot.point.distance(cross.point))
        .fold(0.0, f64::max);
    cross
        .center
        .map_or(feet, |center| feet.max(center.distance(cross.point)))
}

pub(super) fn outline(cross: &Cross, convex: bool) -> Option<Outline> {
    let [first, second] = cross.feet.map(|foot| foot.point);
    let across = second - first;
    let length = across.length();
    let unit = across.try_normalize()?;
    let bisector = (cross.feet[0].normal + cross.feet[1].normal).try_normalize()?;
    let side = if convex { 1.0 } else { -1.0 };
    Some(Outline {
        chord: [
            first - unit * (OVERHANG * length),
            second + unit * (OVERHANG * length),
        ],
        apex: cross.point + bisector * (side * APEX_REACH * reach(cross)),
    })
}

fn convexity(solid: &Solid, sides: &RunSides) -> Option<bool> {
    let station = edge_station(solid, sides, sides.run.from)?;
    let inward = inward(solid, sides, &station)?;
    let normal = face_normal(solid, sides.faces[1], station.point)?;
    Some(normal.dot(inward[0]) < 0.0)
}

pub(super) fn chain_convexity(solid: &Solid, chain: &Chain) -> Option<bool> {
    let sides = sides_of(solid, chain).ok()?;
    let first = convexity(solid, sides.first()?)?;
    sides
        .iter()
        .all(|sides| convexity(solid, sides) == Some(first))
        .then_some(first)
}

fn deviation(row: &BSplineCurve, at: f64, point: Point3) -> f64 {
    row.point(at).distance(point)
}

fn fitted(stations: &Stations, middles: &[(f64, Cross)], convex: bool) -> bool {
    let outlines: Option<Vec<Outline>> = stations
        .crosses
        .iter()
        .map(|cross| outline(cross, convex))
        .collect();
    let Some(outlines) = outlines else {
        return false;
    };
    let segment = Segment {
        params: stations.params.clone(),
        outlines,
        name: stations.name,
    };
    let Ok([first, second, _]) = loft::rows(&segment) else {
        return false;
    };
    let rounding = stations.crosses.iter().any(|cross| cross.center.is_some());
    let feet = super::rounding::foot_rows(stations).ok();
    let arcs = super::rounding::arc_rows(stations).ok();
    middles.iter().all(|(at, cross)| {
        let Some(exact) = outline(cross, convex) else {
            return false;
        };
        let chord =
            deviation(&first, *at, exact.chord[0]).max(deviation(&second, *at, exact.chord[1]));
        if !rounding {
            return chord <= OUTLINE_FIT;
        }
        let (Some([foot_first, foot_second]), Some((scaled, weights)), Some((middle, _))) =
            (&feet, &arcs, cross.arc())
        else {
            return false;
        };
        let foot = deviation(foot_first, *at, cross.feet[0].point).max(deviation(
            foot_second,
            *at,
            cross.feet[1].point,
        ));
        let interpolated = scaled.point(*at) / weights.point(*at).x;
        chord <= FOOT_FIT && foot <= FOOT_FIT && interpolated.distance(middle) <= ARC_FIT
    })
}

struct Piece {
    sides: RunSides,
    tangents: [Vector3; 2],
    extended: [f64; 2],
}

fn run_stations(
    context: &Context<'_>,
    piece: &Piece,
    convex: bool,
    past_ends: &[EndPlane],
) -> Result<Stations, BlendError> {
    let solid = context.around.solid;
    let sides = &piece.sides;
    let edge = sides.run.edge;
    let definition = solid.edge(edge).ok_or(BlendError::MissingEdge(edge))?;
    let name = Named {
        name: FaceName::blend(context.feature, definition.name()),
        origin: Some(context.shape.origin(context.feature)),
    };
    let Run { from, to, .. } = sides.run;
    let span = (to - from).abs();
    let speed = |parameter: f64| definition.curve().evaluate(parameter).first.length();
    let [before, after] = piece.extended;
    let reach_before = speed(from) * span;
    let reach_after = speed(to) * span;
    let low = if before > 0.0 && reach_before > 0.0 {
        -before / reach_before
    } else {
        0.0
    };
    let high = if after > 0.0 && reach_after > 0.0 {
        1.0 + after / reach_after
    } else {
        1.0
    };
    let start = blended_station(solid, sides, 0.0, piece.tangents).ok_or(unsupported(edge))?;
    let end = blended_station(solid, sides, 1.0, piece.tangents).ok_or(unsupported(edge))?;
    let compute = |fraction: f64| -> Result<Cross, BlendError> {
        let station = if fraction < 0.0 {
            let mut backward = start;
            backward.tangent = -start.tangent;
            let mut found = extended_station(solid, sides, &backward, -fraction * reach_before)
                .ok_or(BlendError::TooLarge(edge))?;
            found.tangent = -found.tangent;
            found
        } else if fraction > 1.0 {
            extended_station(solid, sides, &end, (fraction - 1.0) * reach_after)
                .ok_or(BlendError::TooLarge(edge))?
        } else {
            blended_station(solid, sides, fraction, piece.tangents).ok_or(unsupported(edge))?
        };
        let found = station_cross(context, sides, None, convex, &station)?;
        let beyond = fraction <= 0.0 || fraction >= 1.0;
        let fits = beyond
            || found.inside.iter().zip(found.feet).all(|(inside, foot)| {
                *inside || past_ends.iter().any(|end| end.passed_by(foot.point))
            });
        if fits {
            Ok(found)
        } else {
            Err(BlendError::TooLarge(edge))
        }
    };
    let fraction_at = |index: usize, count: usize| low + (high - low) * index as f64 / count as f64;
    let mut count = FIRST_STATIONS;
    let mut crosses: Vec<Cross> = (0..=count)
        .map(|index| compute(fraction_at(index, count)))
        .collect::<Result<_, _>>()?;
    let last = *crosses.last().ok_or(BlendError::NoEdges)?;
    loop {
        crate::interrupt::check()?;
        let params: Vec<f64> = (0..=count)
            .map(|index| span * fraction_at(index, count))
            .collect();
        let stations = Stations {
            sides: sides.clone(),
            params,
            crosses: crosses.clone(),
            name,
        };
        let mut middles = Vec::with_capacity(count);
        for index in 0..count {
            let fraction = fraction_at(2 * index + 1, 2 * count);
            middles.push((span * fraction, compute(fraction)?));
        }
        if fitted(&stations, &middles, convex) {
            return Ok(stations);
        }
        if count * 2 > MAX_STATIONS {
            return Err(BlendError::Intricate(edge));
        }
        let mut doubled = Vec::with_capacity(2 * count + 1);
        for (cross, (_, middle)) in crosses.iter().zip(&middles) {
            doubled.push(*cross);
            doubled.push(*middle);
        }
        doubled.push(last);
        crosses = doubled;
        count *= 2;
    }
}

fn feet_faces(cross: &Cross) -> [FaceId; 2] {
    cross.feet.map(|foot| foot.face)
}

fn forced(sides: &RunSides, from: f64, to: f64, faces: [FaceId; 2]) -> RunSides {
    RunSides {
        run: Run {
            edge: sides.run.edge,
            from,
            to,
        },
        faces: sides.faces,
        same: sides.same,
        candidates: faces.map(|face| vec![face]),
    }
}

fn pieces(
    context: &Context<'_>,
    sides: &RunSides,
    convex: bool,
    ends: [&Cross; 2],
) -> Result<Vec<(RunSides, [Vector3; 2])>, BlendError> {
    let solid = context.around.solid;
    let edge = sides.run.edge;
    let Run { from, to, .. } = sides.run;
    let at = |fraction: f64| from + (to - from) * fraction;
    let tangents = [ends[0].tangent, ends[1].tangent];
    let station_at =
        |fraction: f64| blended_station(solid, sides, fraction, tangents).ok_or(unsupported(edge));
    let compute = |fraction: f64| -> Result<Cross, BlendError> {
        station_cross(context, sides, None, convex, &station_at(fraction)?)
    };
    let length = solid
        .edge(edge)
        .and_then(|definition| {
            Interval::new(from.min(to), from.max(to)).map(|range| definition.curve().length(range))
        })
        .unwrap_or(0.0);
    let mut samples = Vec::with_capacity(SWITCH_SAMPLES + 1);
    samples.push((0.0, *ends[0]));
    for index in 1..SWITCH_SAMPLES {
        let fraction = index as f64 / SWITCH_SAMPLES as f64;
        samples.push((fraction, compute(fraction)?));
    }
    samples.push((1.0, *ends[1]));
    let mut breaks: Vec<f64> = Vec::new();
    for pair in samples.windows(2) {
        let [(low_fraction, low), (high_fraction, high)] = pair else {
            continue;
        };
        if feet_faces(low) == feet_faces(high) {
            continue;
        }
        let (low_fraction_start, high_fraction_end) = (low_fraction, high_fraction);
        let (mut low_fraction, mut high_fraction) = (*low_fraction, *high_fraction);
        let wanted = feet_faces(low);
        for _ in 0..SWITCH_BISECTIONS {
            let middle = 0.5 * (low_fraction + high_fraction);
            if feet_faces(&compute(middle)?) == wanted {
                low_fraction = middle;
            } else {
                high_fraction = middle;
            }
        }
        let switched = 0.5 * (low_fraction + high_fraction);
        let left = forced(sides, at(0.0), at(1.0), wanted);
        let crossing = onto_shared_edge(
            solid,
            wanted,
            feet_faces(high),
            [*low_fraction_start, *high_fraction_end],
            |fraction| station_cross(context, &left, None, convex, &station_at(fraction)?),
        )?
        .unwrap_or(switched);
        let rounding = matches!(context.shape, BlendShape::Fillet { .. });
        let offset = if length > 0.0 && !rounding {
            BREAK_OFFSET / length
        } else {
            0.0
        };
        breaks.push(crossing + offset);
    }
    let closest = if length > 0.0 {
        SHORTEST_PIECE / length
    } else {
        1.0
    };
    let mut bounds = vec![0.0];
    for fraction in breaks {
        let last = bounds.last().copied().unwrap_or(0.0);
        if fraction - last > closest && 1.0 - fraction > closest {
            bounds.push(fraction);
        }
    }
    bounds.push(1.0);
    let mut found = Vec::with_capacity(bounds.len());
    for window in bounds.windows(2) {
        let [low, high] = window else {
            continue;
        };
        let middle = compute(0.5 * (low + high))?;
        let piece = forced(sides, at(*low), at(*high), feet_faces(&middle));
        found.push((
            piece,
            [station_at(*low)?.tangent, station_at(*high)?.tangent],
        ));
    }
    Ok(found)
}

fn shared_edge(solid: &Solid, first: FaceId, second: FaceId) -> Option<EdgeId> {
    solid.edges().map(|(id, _)| id).find(|edge| {
        let faces: Vec<FaceId> = edge_faces(solid, *edge)
            .into_iter()
            .map(|(face, _)| face)
            .collect();
        faces.contains(&first) && faces.contains(&second)
    })
}

fn across_edge(solid: &Solid, edge: EdgeId, face: FaceId, point: Point3) -> Option<f64> {
    let definition = solid.edge(edge)?;
    let curve = definition.curve();
    let parameter = curve.closest_parameter(point, definition.interval());
    let derivatives = curve.evaluate(parameter);
    let across = face_normal(solid, face, derivatives.point)?
        .cross(derivatives.first)
        .try_normalize()?;
    Some((point - derivatives.point).dot(across))
}

fn onto_shared_edge(
    solid: &Solid,
    before: [FaceId; 2],
    after: [FaceId; 2],
    bracket: [f64; 2],
    cross_at: impl Fn(f64) -> Result<Cross, BlendError>,
) -> Result<Option<f64>, BlendError> {
    let mut changed = before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (old, new))| **old != *new);
    let (Some((side, (old, new))), None) = (changed.next(), changed.next()) else {
        return Ok(None);
    };
    let Some(edge) = shared_edge(solid, *old, new) else {
        return Ok(None);
    };
    let signed = |fraction: f64| -> Result<Option<f64>, BlendError> {
        let found = cross_at(fraction)?;
        Ok(found
            .feet
            .get(side)
            .and_then(|foot| across_edge(solid, edge, *old, foot.point)))
    };
    let [mut low, mut high] = bracket;
    let (Some(low_value), Some(high_value)) = (signed(low)?, signed(high)?) else {
        return Ok(None);
    };
    if low_value.signum() == high_value.signum() {
        return Ok(None);
    }
    for _ in 0..SWITCH_BISECTIONS {
        let middle = 0.5 * (low + high);
        let Some(value) = signed(middle)? else {
            return Ok(None);
        };
        if value.signum() == low_value.signum() {
            low = middle;
        } else {
            high = middle;
        }
    }
    Ok(Some(0.5 * (low + high)))
}

fn met(before: &Cross, after: &Cross) -> Option<Cross> {
    let gap = before
        .feet
        .iter()
        .zip(&after.feet)
        .map(|(a, b)| a.point.distance(b.point))
        .chain(before.center.zip(after.center).map(|(a, b)| a.distance(b)))
        .fold(before.point.distance(after.point), f64::max);
    if gap > JOINT_GAP {
        return None;
    }
    let mut joined = *after;
    joined.point = before.point.midpoint(after.point);
    joined.tangent = (before.tangent + after.tangent).try_normalize()?;
    for (foot, earlier) in joined.feet.iter_mut().zip(&before.feet) {
        foot.point = foot.point.midpoint(earlier.point);
        foot.normal = (foot.normal + earlier.normal).try_normalize()?;
    }
    joined.center = before.center.zip(after.center).map(|(a, b)| a.midpoint(b));
    Some(joined)
}

fn join_runs(runs: &mut [Stations], closed: bool) -> Result<(), BlendError> {
    let count = runs.len();
    let pairs = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    for index in 0..pairs {
        let next = (index + 1) % count;
        let (Some(before), Some(after), Some(edge)) = (
            runs.get(index).and_then(|run| run.crosses.last()).copied(),
            runs.get(next).and_then(|run| run.crosses.first()).copied(),
            runs.get(next).map(|run| run.sides.run.edge),
        ) else {
            continue;
        };
        let joined = met(&before, &after).ok_or(BlendError::Intricate(edge))?;
        if let Some(slot) = runs.get_mut(index).and_then(|run| run.crosses.last_mut()) {
            let mut ending = joined;
            for (foot, earlier) in ending.feet.iter_mut().zip(&before.feet) {
                foot.face = earlier.face;
            }
            *slot = ending;
        }
        if let Some(slot) = runs.get_mut(next).and_then(|run| run.crosses.first_mut()) {
            *slot = joined;
        }
    }
    Ok(())
}

fn end_vertex(solid: &Solid, run: &Run, at_start: bool) -> Option<VertexId> {
    let definition = solid.edge(run.edge)?;
    let parameter = if at_start { run.from } else { run.to };
    let interval = definition.interval();
    if (parameter - interval.start()).abs() <= (parameter - interval.end()).abs() {
        Some(definition.start())
    } else {
        Some(definition.end())
    }
}

fn joint_crosses(
    context: &Context<'_>,
    chain: &Chain,
    all_sides: &[RunSides],
    convex: bool,
) -> Result<Vec<Cross>, BlendError> {
    let solid = context.around.solid;
    let mut joints = Vec::with_capacity(all_sides.len() + 1);
    for (index, sides) in all_sides.iter().enumerate() {
        let previous = if index == 0 {
            chain.closed.then(|| all_sides.last()).flatten()
        } else {
            all_sides.get(index - 1)
        };
        let mut station =
            edge_station(solid, sides, sides.run.from).ok_or(unsupported(sides.run.edge))?;
        if let Some(previous) = previous
            && let Some(before) = edge_station(solid, previous, previous.run.to)
        {
            station.tangent = (station.tangent + before.tangent)
                .try_normalize()
                .unwrap_or(station.tangent);
        }
        joints.push(station_cross(context, sides, previous, convex, &station)?);
    }
    if !chain.closed {
        let last = all_sides.last().ok_or(BlendError::NoEdges)?;
        let station = edge_station(solid, last, last.run.to).ok_or(unsupported(last.run.edge))?;
        joints.push(station_cross(context, last, None, convex, &station)?);
    }
    Ok(joints)
}

pub(super) fn plan(context: &Context<'_>, chain: &Chain) -> Result<Planned, BlendError> {
    let solid = context.around.solid;
    let topology = context.around.topology;
    let first_run = chain.runs.first().ok_or(BlendError::NoEdges)?;
    let all_sides = sides_of(solid, chain)?;
    let first_sides = all_sides.first().ok_or(BlendError::NoEdges)?;
    let last_sides = all_sides.last().ok_or(BlendError::NoEdges)?;
    let convex = convexity(solid, first_sides).ok_or(unsupported(first_run.edge))?;
    if all_sides
        .iter()
        .any(|sides| convexity(solid, sides) != Some(convex))
    {
        return Err(unsupported(first_run.edge));
    }
    let joints = joint_crosses(context, chain, &all_sides, convex)?;
    let blended = |sides: &RunSides| Blended {
        edge: sides.run.edge,
        faces: sides.faces,
        convex,
    };
    let end_vertices = if chain.closed {
        [None, None]
    } else {
        [
            end_vertex(solid, &first_sides.run, true),
            end_vertex(solid, &last_sides.run, false),
        ]
    };
    let past_ends: Vec<EndPlane> = [
        (first_sides, end_vertices[0]),
        (last_sides, end_vertices[1]),
    ]
    .into_iter()
    .filter_map(|(sides, vertex)| Some(end_planes_at(solid, topology, &blended(sides), vertex?)))
    .flatten()
    .collect();
    let mut ends = Ends {
        at: [End::Flush, End::Flush],
        faces: [None, None],
        beside: [Vec::new(), Vec::new()],
    };
    let mut extended = [0.0; 2];
    if !chain.closed {
        let own: BTreeSet<FaceId> = first_sides
            .faces
            .into_iter()
            .chain(last_sides.faces)
            .collect();
        let first_cross = *joints.first().ok_or(BlendError::NoEdges)?;
        let last_cross = *joints.last().ok_or(BlendError::NoEdges)?;
        for (slot, (sides, vertex, cross)) in [
            (first_sides, end_vertices[0], first_cross),
            (last_sides, end_vertices[1], last_cross),
        ]
        .into_iter()
        .enumerate()
        {
            let refused = BlendError::UnsupportedEnd {
                edge: sides.run.edge,
                vertex,
            };
            let vertex = vertex.ok_or(refused.clone())?;
            let (end, face) = end_at(
                context.around,
                &blended(sides),
                vertex,
                reach(&cross),
                SquareEnd::Past,
            )?;
            if matches!(end, End::Setback(_)) {
                return Err(refused);
            }
            if let (Some(at), Some(faces), Some(beside), Some(length)) = (
                ends.at.get_mut(slot),
                ends.faces.get_mut(slot),
                ends.beside.get_mut(slot),
                extended.get_mut(slot),
            ) {
                *at = end;
                *faces = face;
                *beside = beside_vertex(solid, topology, &own, vertex);
                *length = end.extension().max(0.0);
            }
        }
    }
    let mut runs = Vec::with_capacity(all_sides.len());
    let last_index = all_sides.len().saturating_sub(1);
    for (index, sides) in all_sides.iter().enumerate() {
        let (Some(start), Some(end)) = (joints.get(index), joints.get((index + 1) % joints.len()))
        else {
            return Err(BlendError::NoEdges);
        };
        let split = pieces(context, sides, convex, [start, end])?;
        let piece_count = split.len();
        for (piece_index, (piece_sides, tangents)) in split.into_iter().enumerate() {
            let before = if index == 0 && piece_index == 0 {
                extended[0]
            } else {
                0.0
            };
            let after = if index == last_index && piece_index + 1 == piece_count {
                extended[1]
            } else {
                0.0
            };
            let piece = Piece {
                sides: piece_sides,
                tangents,
                extended: [before, after],
            };
            runs.push(run_stations(context, &piece, convex, &past_ends)?);
        }
    }
    join_runs(&mut runs, chain.closed)?;
    let largest = runs
        .iter()
        .flat_map(|run| run.crosses.iter().map(reach))
        .fold(0.0, f64::max);
    let own = Named {
        name: runs
            .first()
            .map_or(FaceName::default(), |run| run.name.name),
        origin: Some(context.shape.origin(context.feature)),
    };
    let segments: Vec<Segment> = runs
        .iter()
        .map(|run| {
            let outlines: Option<Vec<Outline>> = run
                .crosses
                .iter()
                .map(|cross| outline(cross, convex))
                .collect();
            Some(Segment {
                params: run.params.clone(),
                outlines: outlines?,
                name: run.name,
            })
        })
        .collect::<Option<_>>()
        .ok_or(BlendError::TooLarge(first_run.edge))?;
    let caps = [
        named(solid, ends.faces[0], own),
        named(solid, ends.faces[1], own),
    ];
    let lofted = loft::loft(&Loft {
        segments: &segments,
        closed: chain.closed,
        sides: [
            named(solid, Some(first_sides.faces[0]), own),
            named(solid, Some(first_sides.faces[1]), own),
        ],
        caps,
    })
    .map_err(|error| BlendError::Loft {
        error,
        edge: Some(first_run.edge),
    })?;
    let shaped = cut_ends(
        lofted,
        solid,
        &blended(first_sides),
        &ends,
        caps,
        largest,
        context.feature,
    )
    .map_err(|error| error.at(Some(first_run.edge)))?;
    let rounded =
        matches!(context.shape, BlendShape::Fillet { .. }).then(|| Rounded { runs, convex });
    Ok(Planned {
        tool: Tool {
            solid: shaped,
            convex,
            edge: Some(first_run.edge),
        },
        rounded,
    })
}
