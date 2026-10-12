use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Point3};
use thiserror::Error;

use super::{
    general::{CHORD_FIT, Rounded, Stations},
    loft::{self, Segment},
    station::{Cross, on_both},
};
use crate::{
    bspline::BSpline,
    curve::{BSplineCurve, Curve, IntersectionCurve},
    error::GeometryError,
    interrupt::Interrupted,
    interval::Interval,
    naming::FaceName,
    sense::Sense,
    surface::{BSplineSurface, Surface},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
    topology::{
        BuildError, CoedgeId, EdgeId, FaceId, Solid, SolidBuilder, ValidationError, VertexId,
        fit_pcurve,
    },
};

const CUBIC: usize = 3;
const ARC_KNOTS: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
const ON_SURFACE: f64 = 10.0 * LINEAR_RESOLUTION;
const ALONG_ROW: f64 = 10.0 * CHORD_FIT;
const TRACE_SAMPLES: usize = 24;
const MOVE_ITERATIONS: usize = 32;
const SETTLED_STEP: f64 = 1e-3 * LINEAR_RESOLUTION;
const ON_ROW: f64 = 1e-2 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum RoundingError {
    #[error("a rounded face could not be shaped: {0}")]
    Geometry(#[from] GeometryError),
    #[error("a face of the blend could not be matched to the edge it rounds")]
    Unmatched,
    #[error("the rounded faces meet where no rounded edge can be traced between them")]
    Untraced,
    #[error("the rounded solid could not be rebuilt: {0}")]
    Rebuild(#[from] BuildError),
    #[error("the rounded solid is invalid: {0}")]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

pub(super) fn foot_rows(stations: &Stations) -> Result<[BSplineCurve; 2], GeometryError> {
    let row = |index: usize| -> Result<BSplineCurve, GeometryError> {
        let points: Vec<Point3> = stations
            .crosses
            .iter()
            .filter_map(|cross| cross.feet.get(index).map(|foot| foot.point))
            .collect();
        BSpline::interpolating_at(CUBIC, &stations.params, &points)
    };
    Ok([row(0)?, row(1)?])
}

pub(super) fn arc_rows(stations: &Stations) -> Result<(BSplineCurve, BSplineCurve), GeometryError> {
    let middles: Vec<(Point3, f64)> = stations
        .crosses
        .iter()
        .map(|cross| cross.arc().ok_or(GeometryError::ZeroDirection))
        .collect::<Result<_, _>>()?;
    let scaled: Vec<Point3> = middles
        .iter()
        .map(|(point, weight)| *point * *weight)
        .collect();
    let weights: Vec<Point3> = middles
        .iter()
        .map(|(_, weight)| Point3::new(*weight, 0.0, 0.0))
        .collect();
    Ok((
        BSpline::interpolating_at(CUBIC, &stations.params, &scaled)?,
        BSpline::interpolating_at(CUBIC, &stations.params, &weights)?,
    ))
}

fn fillet_surface(stations: &Stations) -> Result<Surface, GeometryError> {
    let [first, second] = foot_rows(stations)?;
    let (scaled, weights) = arc_rows(stations)?;
    let columns = first.control_points().len();
    let middle_weights: Vec<f64> = weights
        .control_points()
        .iter()
        .map(|point| point.x)
        .collect();
    if !middle_weights
        .iter()
        .all(|weight| weight.is_finite() && *weight > 0.0)
    {
        return Err(GeometryError::Weight(
            middle_weights.iter().copied().fold(f64::INFINITY, f64::min),
        ));
    }
    let middle: Vec<Point3> = scaled
        .control_points()
        .iter()
        .zip(&middle_weights)
        .map(|(point, weight)| *point / *weight)
        .collect();
    let points: Vec<Point3> = first
        .control_points()
        .iter()
        .chain(&middle)
        .chain(second.control_points())
        .copied()
        .collect();
    let all_weights: Vec<f64> = std::iter::repeat_n(1.0, columns)
        .chain(middle_weights)
        .chain(std::iter::repeat_n(1.0, columns))
        .collect();
    Ok(BSplineSurface::new(
        CUBIC,
        2,
        first.knots().to_vec(),
        ARC_KNOTS.to_vec(),
        columns,
        points,
        Some(all_weights),
    )?
    .into())
}

fn arc_curve(cross: &Cross) -> Result<BSplineCurve, GeometryError> {
    let (middle, weight) = cross.arc().ok_or(GeometryError::ZeroDirection)?;
    BSpline::rational(
        2,
        ARC_KNOTS.to_vec(),
        vec![cross.feet[0].point, middle, cross.feet[1].point],
        vec![1.0, weight, 1.0],
    )
}

fn chord_surface(chord: &Segment) -> Result<Surface, GeometryError> {
    let [first, second, _] = loft::rows(chord).map_err(|_| GeometryError::ZeroDirection)?;
    loft::ruled(&first, &second).map_err(|_| GeometryError::ZeroDirection)
}

struct Target {
    name: FaceName,
    chord: Surface,
    fillet: Surface,
    feet: [Curve; 2],
    foot_names: [BTreeSet<FaceName>; 2],
}

struct Joint {
    chord: [Point3; 2],
    arc: Curve,
}

fn side_names(solid: &Solid, faces: &[FaceId]) -> BTreeSet<FaceName> {
    faces
        .iter()
        .filter_map(|face| solid.face(*face).map(|face| face.name()))
        .collect()
}

fn targets(
    original: &Solid,
    rounded: &[Rounded],
) -> Result<(Vec<Target>, Vec<Joint>), GeometryError> {
    let mut targets = Vec::new();
    let mut joints = Vec::new();
    for chain in rounded {
        for (stations, chord) in chain.runs.iter().zip(&chain.chords) {
            let [first, second] = foot_rows(stations)?;
            targets.push(Target {
                name: stations.name.name,
                chord: chord_surface(chord)?,
                fillet: fillet_surface(stations)?,
                feet: [first.into(), second.into()],
                foot_names: [
                    side_names(original, &stations.sides.candidates[0]),
                    side_names(original, &stations.sides.candidates[1]),
                ],
            });
            let ends = [
                (stations.crosses.first(), chord.outlines.first()),
                (stations.crosses.last(), chord.outlines.last()),
            ];
            for (cross, shape) in ends {
                let (Some(cross), Some(shape)) = (cross, shape) else {
                    return Err(GeometryError::ZeroDirection);
                };
                joints.push(Joint {
                    chord: shape.chord,
                    arc: arc_curve(cross)?.into(),
                });
            }
        }
    }
    Ok((targets, joints))
}

fn face_edges(solid: &Solid, face: FaceId) -> Vec<EdgeId> {
    solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
        .collect()
}

fn edge_samples(solid: &Solid, edge: EdgeId, count: usize) -> Vec<Point3> {
    solid.edge(edge).map_or_else(Vec::new, |edge| {
        edge.interval()
            .split(count)
            .map(|parameter| edge.curve().point(parameter))
            .collect()
    })
}

fn matching_target(solid: &Solid, face: FaceId, targets: &[Target]) -> Option<usize> {
    let name = solid.face(face)?.name();
    let points: Vec<Point3> = face_edges(solid, face)
        .into_iter()
        .flat_map(|edge| edge_samples(solid, edge, 4))
        .collect();
    targets
        .iter()
        .enumerate()
        .filter(|(_, target)| target.name == name)
        .map(|(index, target)| {
            let farthest = points
                .iter()
                .map(|point| target.chord.distance(*point))
                .fold(0.0, f64::max);
            (index, farthest)
        })
        .filter(|(_, farthest)| *farthest <= ON_SURFACE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

fn faces_of(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect()
}

fn segment_distance(point: Point3, ends: [Point3; 2]) -> f64 {
    let along = ends[1] - ends[0];
    let length = along.length_squared();
    if length <= 0.0 {
        return point.distance(ends[0]);
    }
    let fraction = ((point - ends[0]).dot(along) / length).clamp(0.0, 1.0);
    point.distance(ends[0] + along * fraction)
}

fn restricted(curve: &Curve, from: Point3, to: Point3) -> Option<(Curve, Interval)> {
    let domain = curve.domain().clipped(MAX_SIZE);
    let start = curve.closest_parameter(from, domain);
    let end = curve.closest_parameter(to, domain);
    if curve.point(start).distance(from) > LINEAR_RESOLUTION
        || curve.point(end).distance(to) > LINEAR_RESOLUTION
    {
        return None;
    }
    if start < end {
        return Some((curve.clone(), Interval::new(start, end)?));
    }
    let reversed = curve.reversed();
    Interval::new(
        curve.reversed_parameter(start),
        curve.reversed_parameter(end),
    )
    .map(|interval| (reversed, interval))
}

type Moved = BTreeMap<VertexId, Point3>;

fn ends_of(solid: &Solid, moved: &Moved, edge: EdgeId) -> Result<[Point3; 2], RoundingError> {
    let definition = solid.edge(edge).ok_or(RoundingError::Untraced)?;
    let at = |vertex: VertexId| {
        moved
            .get(&vertex)
            .copied()
            .or_else(|| solid.vertex(vertex).map(|vertex| vertex.point()))
            .ok_or(RoundingError::Untraced)
    };
    Ok([at(definition.start())?, at(definition.end())?])
}

fn traced(
    solid: &Solid,
    edge: EdgeId,
    surfaces: [&Surface; 2],
    ends: [Point3; 2],
) -> Result<(Curve, Interval), RoundingError> {
    let definition = solid.edge(edge).ok_or(RoundingError::Untraced)?;
    let [start, end] = ends;
    let interval = definition.interval();
    let count = TRACE_SAMPLES;
    let mut points = Vec::with_capacity(count + 1);
    points.push(start);
    for index in 1..count {
        let parameter = interval.start() + interval.length() * index as f64 / count as f64;
        let derivatives = definition.curve().evaluate(parameter);
        let across = derivatives
            .first
            .try_normalize()
            .ok_or(RoundingError::Untraced)?;
        points.push(on_both(surfaces, derivatives.point, across).ok_or(RoundingError::Untraced)?);
    }
    points.push(end);
    let curve = IntersectionCurve::through(
        [surfaces[0].clone(), surfaces[1].clone()],
        &points,
        definition.is_closed(),
    )
    .ok_or(RoundingError::Untraced)?;
    let domain = curve.domain();
    let fits = curve.point(domain.start()).distance(start) <= LINEAR_RESOLUTION
        && curve.point(domain.end()).distance(end) <= LINEAR_RESOLUTION;
    if !fits {
        return Err(RoundingError::Untraced);
    }
    Ok((Curve::Intersection(curve), domain))
}

fn distance_to(curve: &Curve, point: Point3) -> f64 {
    let domain = curve.domain().clipped(MAX_SIZE);
    curve
        .point(curve.closest_parameter(point, domain))
        .distance(point)
}

struct Foot<'a> {
    row: &'a Curve,
    face: FaceId,
}

fn foot_of<'a>(
    solid: &Solid,
    edge: EdgeId,
    swapped: &BTreeMap<FaceId, usize>,
    targets: &'a [Target],
) -> Option<Foot<'a>> {
    let faces = faces_of(solid, edge);
    let [first, second] = faces.as_slice() else {
        return None;
    };
    let (index, face) = match (swapped.get(first), swapped.get(second)) {
        (Some(index), None) => (index, *second),
        (None, Some(index)) => (index, *first),
        _ => return None,
    };
    let target = targets.get(*index)?;
    let name = solid.face(face)?.name();
    let definition = solid.edge(edge)?;
    let middle = definition.curve().point(definition.interval().middle());
    target
        .feet
        .iter()
        .zip(&target.foot_names)
        .filter(|(_, names)| names.contains(&name))
        .map(|(row, _)| (row, distance_to(row, middle)))
        .filter(|(_, distance)| *distance <= ALONG_ROW)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(row, _)| Foot { row, face })
}

struct Rounding<'a> {
    swapped: BTreeMap<FaceId, usize>,
    targets: &'a [Target],
    joints: &'a [Joint],
    feet: BTreeMap<EdgeId, Foot<'a>>,
}

fn new_curve(
    solid: &Solid,
    edge: EdgeId,
    rounding: &Rounding<'_>,
    moved: &Moved,
) -> Result<(Curve, Interval), RoundingError> {
    let Rounding {
        swapped,
        targets,
        joints,
        feet,
    } = rounding;
    let definition = solid.edge(edge).ok_or(RoundingError::Untraced)?;
    let ends = ends_of(solid, moved, edge)?;
    let [start, end] = ends;
    let middle = definition.curve().point(definition.interval().middle());
    let faces = faces_of(solid, edge);
    let [first, second] = faces.as_slice() else {
        return Err(RoundingError::Untraced);
    };
    let surface_of = |face: FaceId| -> Option<&Surface> {
        match swapped.get(&face) {
            Some(index) => targets.get(*index).map(|target| &target.fillet),
            None => solid.face(face).map(|face| face.surface()),
        }
    };
    match (swapped.get(first), swapped.get(second)) {
        (Some(a), Some(b)) if a == b => Err(RoundingError::Untraced),
        (Some(_), Some(_)) => {
            if let Some(joint) = joints
                .iter()
                .find(|joint| segment_distance(middle, joint.chord) <= ON_SURFACE)
                && let Some(found) = restricted(&joint.arc, start, end)
            {
                return Ok(found);
            }
            let (Some(a), Some(b)) = (surface_of(*first), surface_of(*second)) else {
                return Err(RoundingError::Untraced);
            };
            traced(solid, edge, [a, b], ends)
        }
        (Some(index), None) | (None, Some(index)) => {
            let other = if swapped.contains_key(first) {
                *second
            } else {
                *first
            };
            let target = targets.get(*index).ok_or(RoundingError::Unmatched)?;
            if let Some(foot) = feet.get(&edge)
                && let Some(found) = restricted(foot.row, start, end)
            {
                return Ok(found);
            }
            if let Some(joint) = joints
                .iter()
                .find(|joint| segment_distance(middle, joint.chord) <= ON_SURFACE)
                && let Some(found) = restricted(&joint.arc, start, end)
                && solid.face(other).is_some_and(|face| {
                    face.surface().distance(found.0.point(found.1.middle())) <= ON_SURFACE
                })
            {
                return Ok(found);
            }
            let other_surface = solid
                .face(other)
                .map(|face| face.surface())
                .ok_or(RoundingError::Untraced)?;
            traced(solid, edge, [&target.fillet, other_surface], ends)
        }
        (None, None) => Err(RoundingError::Untraced),
    }
}

fn edges_at_vertices(solid: &Solid) -> BTreeMap<VertexId, Vec<EdgeId>> {
    let mut found: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
    for (id, edge) in solid.edges() {
        found.entry(edge.start()).or_default().push(id);
        if edge.end() != edge.start() {
            found.entry(edge.end()).or_default().push(id);
        }
    }
    found
}

fn foot_parameter(curve: &Curve, point: Point3, from: f64) -> f64 {
    let mut parameter = from;
    for _ in 0..MOVE_ITERATIONS {
        let derivatives = curve.evaluate(parameter);
        let speed = derivatives.first.length_squared();
        if !speed.is_finite() || speed <= f64::MIN_POSITIVE {
            break;
        }
        let step = (point - derivatives.point).dot(derivatives.first) / speed;
        parameter += step;
        if step.abs() * speed.sqrt() <= SETTLED_STEP {
            break;
        }
    }
    parameter
}

fn crossing(row: &Curve, guide: &Curve, starts: [f64; 2], face: &Surface) -> Option<[f64; 2]> {
    let [mut along, mut on_guide_at] = starts;
    let mut hint: Option<Point2> = None;
    for _ in 0..MOVE_ITERATIONS {
        let on_row = row.evaluate(along);
        on_guide_at = foot_parameter(guide, on_row.point, on_guide_at);
        let on_guide = guide.evaluate(on_guide_at);
        let uv = face.project(on_guide.point, hint);
        hint = Some(uv);
        let across = face
            .normal(uv.x, uv.y)?
            .cross(on_guide.first)
            .try_normalize()?;
        let rate = on_row.first.dot(across);
        if !rate.is_finite() || rate.abs() <= f64::MIN_POSITIVE {
            return None;
        }
        let step = (on_row.point - on_guide.point).dot(across) / rate;
        along -= step;
        if step.abs() * on_row.first.length() <= SETTLED_STEP {
            break;
        }
    }
    let on_row = row.point(along);
    on_guide_at = foot_parameter(guide, on_row, on_guide_at);
    let within = row.domain().clipped(MAX_SIZE).contains(along);
    let met = on_row.distance(guide.point(on_guide_at)) <= LINEAR_RESOLUTION;
    (within && met).then_some([along, on_guide_at])
}

struct Guide<'a> {
    curve: &'a Curve,
    parameter: f64,
    edge: Option<(EdgeId, bool)>,
}

fn guide_at<'a>(
    solid: &'a Solid,
    vertex: VertexId,
    point: Point3,
    foot: &Foot<'a>,
    around: &[EdgeId],
    touched: &BTreeSet<EdgeId>,
    rows: &BTreeMap<EdgeId, Foot<'a>>,
) -> Option<Guide<'a>> {
    let untouched: Vec<EdgeId> = around
        .iter()
        .filter(|edge| !touched.contains(edge))
        .copied()
        .collect();
    match untouched.as_slice() {
        [edge] => {
            let definition = solid.edge(*edge)?;
            if definition.start() == definition.end()
                || !faces_of(solid, *edge).contains(&foot.face)
            {
                return None;
            }
            let at_start = definition.start() == vertex;
            let interval = definition.interval();
            Some(Guide {
                curve: definition.curve(),
                parameter: if at_start {
                    interval.start()
                } else {
                    interval.end()
                },
                edge: Some((*edge, at_start)),
            })
        }
        [] => around
            .iter()
            .filter_map(|edge| rows.get(edge))
            .find(|other| other.face == foot.face && !std::ptr::eq(other.row, foot.row))
            .map(|other| Guide {
                curve: other.row,
                parameter: other
                    .row
                    .closest_parameter(point, other.row.domain().clipped(MAX_SIZE)),
                edge: None,
            }),
        _ => None,
    }
}

#[derive(Default)]
struct Moves {
    vertices: Moved,
    edges: BTreeMap<EdgeId, (Curve, Interval)>,
}

fn onto_rows(
    solid: &Solid,
    feet: &BTreeMap<EdgeId, Foot<'_>>,
    touched: &BTreeSet<EdgeId>,
) -> Moves {
    let around = edges_at_vertices(solid);
    let mut moves = Moves::default();
    for (edge, foot) in feet {
        let Some(definition) = solid.edge(*edge) else {
            continue;
        };
        for vertex in [definition.start(), definition.end()] {
            if moves.vertices.contains_key(&vertex) {
                continue;
            }
            let Some(point) = solid.vertex(vertex).map(|vertex| vertex.point()) else {
                continue;
            };
            let domain = foot.row.domain().clipped(MAX_SIZE);
            let along = foot.row.closest_parameter(point, domain);
            if foot.row.point(along).distance(point) <= ON_ROW {
                continue;
            }
            let (Some(edges), Some(face)) = (around.get(&vertex), solid.face(foot.face)) else {
                continue;
            };
            let Some(guide) = guide_at(solid, vertex, point, foot, edges, touched, feet) else {
                continue;
            };
            let Some([along, on_guide]) = crossing(
                foot.row,
                guide.curve,
                [along, guide.parameter],
                face.surface(),
            ) else {
                continue;
            };
            if let Some((guided, at_start)) = guide.edge {
                let Some(guided_definition) = solid.edge(guided) else {
                    continue;
                };
                let (curve, interval) = moves.edges.get(&guided).cloned().unwrap_or_else(|| {
                    (
                        guided_definition.curve().clone(),
                        guided_definition.interval(),
                    )
                });
                let reached = if at_start {
                    Interval::new(on_guide, interval.end())
                } else {
                    Interval::new(interval.start(), on_guide)
                };
                let Some(reached) = reached else {
                    continue;
                };
                moves.edges.insert(guided, (curve, reached));
            }
            moves.vertices.insert(vertex, foot.row.point(along));
        }
    }
    moves
}

fn new_sense(solid: &Solid, face: FaceId, fillet: &Surface) -> Option<Sense> {
    let definition = solid.face(face)?;
    let old = definition.surface();
    let sign = definition.sense().sign();
    let votes: f64 = face_edges(solid, face)
        .into_iter()
        .filter_map(|edge| edge_samples(solid, edge, 2).get(1).copied())
        .filter_map(|point| {
            let old_uv = old.project(point, None);
            let old_normal = old.normal(old_uv.x, old_uv.y)? * sign;
            let new_uv = fillet.project(point, None);
            let new_normal = fillet.normal(new_uv.x, new_uv.y)?;
            Some(new_normal.dot(old_normal))
        })
        .sum();
    (votes != 0.0).then_some(if votes > 0.0 {
        Sense::Same
    } else {
        Sense::Reversed
    })
}

struct Changes {
    faces: BTreeMap<FaceId, (Surface, Sense)>,
    edges: BTreeMap<EdgeId, (Curve, Interval)>,
    vertices: Moved,
}

fn rebuilt(solid: &Solid, changes: &Changes) -> Result<Solid, RoundingError> {
    let mut builder = SolidBuilder::new();
    for (id, vertex) in solid.vertices() {
        let point = changes
            .vertices
            .get(&id)
            .copied()
            .unwrap_or_else(|| vertex.point());
        builder.vertex(point)?;
    }
    for (id, edge) in solid.edges() {
        let (curve, interval) = changes
            .edges
            .get(&id)
            .cloned()
            .unwrap_or_else(|| (edge.curve().clone(), edge.interval()));
        let built = builder.edge(curve, interval, edge.start(), edge.end())?;
        builder.set_edge_name(built, edge.name())?;
    }
    let mut shells = BTreeMap::new();
    for (id, _) in solid.shells() {
        shells.insert(id, builder.shell()?);
    }
    for (id, face) in solid.faces() {
        let shell = *shells.get(&face.shell()).ok_or(RoundingError::Unmatched)?;
        let (surface, sense) = changes
            .faces
            .get(&id)
            .cloned()
            .unwrap_or_else(|| (face.surface().clone(), face.sense()));
        let built = builder.face(shell, surface.clone(), sense)?;
        builder.set_face_name(built, face.name())?;
        if let Some(origin) = face.origin() {
            builder.set_face_origin(built, origin)?;
        }
        let refit_all = changes.faces.contains_key(&id);
        for loop_id in face.loops() {
            let Some(face_loop) = solid.face_loop(*loop_id) else {
                continue;
            };
            let coedges: Vec<(CoedgeId, EdgeId, Sense)> = face_loop
                .coedges()
                .iter()
                .filter_map(|coedge| {
                    let data = solid.coedge(*coedge)?;
                    Some((*coedge, data.edge(), data.sense()))
                })
                .collect();
            if refit_all {
                let uses: Vec<(EdgeId, Sense)> = coedges
                    .iter()
                    .map(|(_, edge, sense)| (*edge, *sense))
                    .collect();
                builder.add_loop(built, &uses)?;
                continue;
            }
            let mut with_pcurves = Vec::with_capacity(coedges.len());
            for (coedge, edge, sense) in coedges {
                let data = solid.coedge(coedge).ok_or(RoundingError::Unmatched)?;
                let pcurve = match changes.edges.get(&edge) {
                    Some((curve, interval)) => fit_pcurve(
                        &surface,
                        curve,
                        *interval,
                        sense,
                        Some(data.pcurve().start()),
                    )
                    .map_err(|error| BuildError::pcurve(edge, error))?,
                    None => data.pcurve().clone(),
                };
                with_pcurves.push((edge, sense, pcurve));
            }
            builder.add_loop_with_pcurves(built, with_pcurves)?;
        }
    }
    Ok(builder.build()?)
}

pub(super) fn round(
    original: &Solid,
    solid: &Solid,
    rounded: &[Rounded],
) -> Result<Solid, RoundingError> {
    if rounded.is_empty() {
        return Ok(solid.clone());
    }
    let (targets, joints) = targets(original, rounded)?;
    let names: BTreeSet<FaceName> = targets.iter().map(|target| target.name).collect();
    let mut swapped = BTreeMap::new();
    let mut faces = BTreeMap::new();
    for (id, face) in solid.faces() {
        if !names.contains(&face.name()) {
            continue;
        }
        let Some(index) = matching_target(solid, id, &targets) else {
            continue;
        };
        let target = targets.get(index).ok_or(RoundingError::Unmatched)?;
        let sense = new_sense(solid, id, &target.fillet).ok_or(RoundingError::Unmatched)?;
        swapped.insert(id, index);
        faces.insert(id, (target.fillet.clone(), sense));
    }
    let touched: BTreeSet<EdgeId> = swapped
        .keys()
        .flat_map(|face| face_edges(solid, *face))
        .collect();
    let feet: BTreeMap<EdgeId, Foot<'_>> = touched
        .iter()
        .filter_map(|edge| Some((*edge, foot_of(solid, *edge, &swapped, &targets)?)))
        .collect();
    let Moves {
        vertices,
        edges: mut changed,
    } = onto_rows(solid, &feet, &touched);
    let rounding = Rounding {
        swapped,
        targets: &targets,
        joints: &joints,
        feet,
    };
    for edge in touched {
        crate::interrupt::check()?;
        changed.insert(edge, new_curve(solid, edge, &rounding, &vertices)?);
    }
    rebuilt(
        solid,
        &Changes {
            faces,
            edges: changed,
            vertices,
        },
    )
}
