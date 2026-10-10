use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::Point3;
use thiserror::Error;

use super::{
    general::{Rounded, Stations, outline},
    loft::{self, Outline},
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
        BuildError, CoedgeId, EdgeId, FaceId, Solid, SolidBuilder, ValidationError, fit_pcurve,
    },
};

const CUBIC: usize = 3;
const ARC_KNOTS: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
const ON_SURFACE: f64 = 10.0 * LINEAR_RESOLUTION;
const ALONG_ROW: f64 = 1e-5;
const TRACE_SAMPLES: usize = 24;

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

fn chord_surface(stations: &Stations, convex: bool) -> Result<Surface, GeometryError> {
    let outlines: Vec<Outline> = stations
        .crosses
        .iter()
        .map(|cross| outline(cross, convex).ok_or(GeometryError::ZeroDirection))
        .collect::<Result<_, _>>()?;
    let segment = loft::Segment {
        params: stations.params.clone(),
        outlines,
        name: stations.name,
    };
    let [first, second, _] = loft::rows(&segment).map_err(|_| GeometryError::ZeroDirection)?;
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
        for stations in &chain.runs {
            let [first, second] = foot_rows(stations)?;
            targets.push(Target {
                name: stations.name.name,
                chord: chord_surface(stations, chain.convex)?,
                fillet: fillet_surface(stations)?,
                feet: [first.into(), second.into()],
                foot_names: [
                    side_names(original, &stations.sides.candidates[0]),
                    side_names(original, &stations.sides.candidates[1]),
                ],
            });
            for cross in [stations.crosses.first(), stations.crosses.last()]
                .into_iter()
                .flatten()
            {
                let shape = outline(cross, chain.convex).ok_or(GeometryError::ZeroDirection)?;
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

fn traced(
    solid: &Solid,
    edge: EdgeId,
    surfaces: [&Surface; 2],
) -> Result<(Curve, Interval), RoundingError> {
    let definition = solid.edge(edge).ok_or(RoundingError::Untraced)?;
    let start = solid
        .vertex(definition.start())
        .ok_or(RoundingError::Untraced)?
        .point();
    let end = solid
        .vertex(definition.end())
        .ok_or(RoundingError::Untraced)?
        .point();
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

fn new_curve(
    solid: &Solid,
    edge: EdgeId,
    swapped: &BTreeMap<FaceId, usize>,
    targets: &[Target],
    joints: &[Joint],
) -> Result<(Curve, Interval), RoundingError> {
    let definition = solid.edge(edge).ok_or(RoundingError::Untraced)?;
    let start = solid
        .vertex(definition.start())
        .ok_or(RoundingError::Untraced)?
        .point();
    let end = solid
        .vertex(definition.end())
        .ok_or(RoundingError::Untraced)?
        .point();
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
            traced(solid, edge, [a, b])
        }
        (Some(index), None) | (None, Some(index)) => {
            let other = if swapped.contains_key(first) {
                *second
            } else {
                *first
            };
            let target = targets.get(*index).ok_or(RoundingError::Unmatched)?;
            let other_name = solid.face(other).map(|face| face.name());
            for (foot, names) in target.feet.iter().zip(&target.foot_names) {
                let named = other_name.is_some_and(|name| names.contains(&name));
                let domain = foot.domain().clipped(MAX_SIZE);
                let near = foot
                    .point(foot.closest_parameter(middle, domain))
                    .distance(middle)
                    <= ALONG_ROW;
                if named
                    && near
                    && let Some(found) = restricted(foot, start, end)
                {
                    return Ok(found);
                }
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
            traced(solid, edge, [&target.fillet, other_surface])
        }
        (None, None) => Err(RoundingError::Untraced),
    }
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
}

fn rebuilt(solid: &Solid, changes: &Changes) -> Result<Solid, RoundingError> {
    let mut builder = SolidBuilder::new();
    for (_, vertex) in solid.vertices() {
        builder.vertex(vertex.point())?;
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
    let mut edges = BTreeMap::new();
    for edge in touched {
        crate::interrupt::check()?;
        edges.insert(edge, new_curve(solid, edge, &swapped, &targets, &joints)?);
    }
    rebuilt(solid, &Changes { faces, edges })
}
