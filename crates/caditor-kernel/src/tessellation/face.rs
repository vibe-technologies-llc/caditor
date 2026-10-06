use std::collections::{BTreeMap, BTreeSet, VecDeque};

use caditor_geometry::{Aabb2, Point2, Vector3};
use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use crate::{
    coordinates::distance_to_segment,
    curve::Curve,
    interrupt::{self, Interrupted},
    sense::Sense,
    surface::Surface,
    tessellation::{
        EdgeSampling, Mesh, MeshVertex, POLL_EVERY, TessellationError,
        density::{Density, density},
        insertion::insertion_order,
    },
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{EdgeId, Face, FaceId, Solid},
};

const UV_MERGE: f64 = 1e-9;
const JOINT_PARAMETER_GAP: f64 = 1e-6;
const GRID_CLEARANCE: f64 = 0.3;
const TINY_COORDINATE: f64 = 1e-30;
const MAX_GAP_PIECES: f64 = 4096.0;
const NORMAL_NUDGE: f64 = 1e-3;
const MAX_POLE_EDGE_PIECES: usize = 1024;
const PARALLEL_ENDS: f64 = 1e-6;

type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

#[derive(Debug, Clone, Copy, PartialEq)]
struct BoundaryPoint {
    uv: Point2,
    position: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EdgeEnd {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EndSegment {
    edge: EdgeId,
    end: EdgeEnd,
    at: BoundaryPoint,
    toward: Point2,
    length: f64,
}

impl EndSegment {
    fn runs_along(&self, other: &Self) -> bool {
        let (along, beside) = (self.toward - self.at.uv, other.toward - other.at.uv);
        along.dot(beside) > 0.0
            && along.perp_dot(beside).abs() <= PARALLEL_ENDS * along.length() * beside.length()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LocalPoint {
    uv: Point2,
    position: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scaled<'a> {
    Cells(&'a Density),
    Speeds {
        origin: Point2,
        u_scale: f64,
        v_scale: f64,
    },
}

impl<'a> Scaled<'a> {
    fn of(bounds: Aabb2, density: &'a Density) -> Self {
        if density.gridded() {
            Self::Cells(density)
        } else {
            Self::Speeds {
                origin: bounds.min(),
                u_scale: density.u.speed(),
                v_scale: density.v.speed(),
            }
        }
    }

    fn map(&self, uv: Point2) -> Point2 {
        let mapped = match self {
            Self::Cells(density) => density.index(uv),
            Self::Speeds {
                origin,
                u_scale,
                v_scale,
            } => {
                let offset = uv - *origin;
                Point2::new(offset.x * u_scale, offset.y * v_scale)
            }
        };
        Point2::new(snap(mapped.x), snap(mapped.y))
    }
}

fn snap(value: f64) -> f64 {
    if value.abs() < TINY_COORDINATE {
        0.0
    } else {
        value
    }
}

pub(crate) struct Budget {
    pub limit: usize,
    pub tolerance: SamplingTolerance,
    pub density: Option<Density>,
}

pub(crate) fn triangulate(
    solid: &Solid,
    face_id: FaceId,
    samplings: &[EdgeSampling],
    budget: &Budget,
    mesh: &mut Mesh,
) -> Result<(), TessellationError> {
    let tolerance = &budget.tolerance;
    let face = solid
        .face(face_id)
        .ok_or(TessellationError::MissingEntity)?;
    let surface = face.surface();
    let loops = boundary_loops(solid, face, samplings)?;
    let Some(bounds) = Aabb2::from_points(loops.iter().flatten().map(|point| point.uv)) else {
        return Ok(());
    };
    let density = budget
        .density
        .clone()
        .unwrap_or_else(|| density(surface, bounds, tolerance));
    let loops: Vec<Vec<BoundaryPoint>> = loops
        .into_iter()
        .map(|points| close_gaps(points, &density))
        .collect();
    let scaled = Scaled::of(bounds, &density);
    let mut points = FacePoints::new(face_id, scaled);
    for boundary in &loops {
        points.add_loop(boundary)?;
    }
    let grid = grid_points(&loops, &scaled)?;
    if mesh.positions.len().saturating_add(grid.len()) > budget.limit {
        return Err(TessellationError::TooLarge);
    }
    for (index, uv) in grid.into_iter().enumerate() {
        if index.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        points.add_interior(uv)?;
    }
    points.triangulate()?.emit(surface, face.sense(), mesh)
}

pub(crate) struct PoleSampling {
    pub density: Density,
    pub edges: Vec<(EdgeId, usize)>,
}

pub(crate) fn pole_sampling(
    solid: &Solid,
    face_id: FaceId,
    tolerance: &SamplingTolerance,
) -> Result<Option<PoleSampling>, TessellationError> {
    let face = solid
        .face(face_id)
        .ok_or(TessellationError::MissingEntity)?;
    let surface = face.surface();
    let poles = surface.poles();
    if poles.is_empty() {
        return Ok(None);
    }
    let mut coedges = Vec::new();
    for loop_id in face.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(TessellationError::MissingEntity)?;
        for coedge_id in face_loop.coedges() {
            coedges.push(
                solid
                    .coedge(*coedge_id)
                    .ok_or(TessellationError::MissingEntity)?,
            );
        }
    }
    let uvs = coedges
        .iter()
        .flat_map(|coedge| coedge.pcurve().samples().iter().map(|sample| sample.uv));
    let Some(bounds) = Aabb2::from_points(uvs) else {
        return Ok(None);
    };
    let density = density(surface, bounds, tolerance);
    let mut segments = Vec::new();
    for coedge in coedges {
        let edge = solid
            .edge(coedge.edge())
            .ok_or(TessellationError::MissingEntity)?;
        if !matches!(edge.curve(), Curve::Line(_)) {
            continue;
        }
        let at_pole = [edge.start(), edge.end()].into_iter().any(|vertex| {
            solid.vertex(vertex).is_some_and(|vertex| {
                poles
                    .iter()
                    .any(|pole| pole.point.distance(vertex.point()) <= LINEAR_RESOLUTION)
            })
        });
        if !at_pole {
            continue;
        }
        let pcurve = coedge.pcurve();
        let rows = density.v.index(pcurve.end().y) - density.v.index(pcurve.start().y);
        let pieces = rows.abs().round();
        if pieces.is_finite() && pieces >= 2.0 {
            segments.push((coedge.edge(), (pieces as usize).min(MAX_POLE_EDGE_PIECES)));
        }
    }
    Ok(Some(PoleSampling {
        density,
        edges: segments,
    }))
}

fn boundary_loops(
    solid: &Solid,
    face: &Face,
    samplings: &[EdgeSampling],
) -> Result<Vec<Vec<BoundaryPoint>>, TessellationError> {
    let surface = face.surface();
    let mut loops = Vec::with_capacity(face.loops().len());
    for loop_id in face.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(TessellationError::MissingEntity)?;
        let mut points: Vec<BoundaryPoint> = Vec::new();
        for coedge_id in face_loop.coedges() {
            let coedge = solid
                .coedge(*coedge_id)
                .ok_or(TessellationError::MissingEntity)?;
            let sampling = samplings
                .get(coedge.edge().index())
                .ok_or(TessellationError::MissingEntity)?;
            let count = sampling.parameters.len();
            let mut order: Vec<usize> = (0..count).collect();
            if coedge.sense() == Sense::Reversed {
                order.reverse();
            }
            let pcurve = coedge.pcurve();
            for (step, index) in order.into_iter().enumerate() {
                if step.is_multiple_of(POLL_EVERY) {
                    interrupt::check()?;
                }
                let (Some(parameter), Some(point), Some(position)) = (
                    sampling.parameters.get(index),
                    sampling.points.get(index),
                    sampling.positions.get(index),
                ) else {
                    return Err(TessellationError::MissingEntity);
                };
                let uv = if step == 0 {
                    pcurve.start()
                } else if step + 1 == count {
                    pcurve.end()
                } else {
                    surface.project(*point, Some(pcurve.uv_at(*parameter)))
                };
                push_distinct(
                    surface,
                    &mut points,
                    BoundaryPoint {
                        uv,
                        position: *position,
                    },
                );
            }
        }
        if points.len() > 1
            && let (Some(first), Some(last)) = (points.first(), points.last())
            && (coincide(first, last) || joined(surface, last, first))
        {
            points.pop();
        }
        loops.push(points);
    }
    share_revisited_vertices(surface, &mut loops);
    Ok(loops)
}

pub(crate) fn overlapping_ends(
    solid: &Solid,
    face_id: FaceId,
    samplings: &[EdgeSampling],
) -> Result<BTreeSet<(EdgeId, EdgeEnd)>, TessellationError> {
    let face = solid
        .face(face_id)
        .ok_or(TessellationError::MissingEntity)?;
    let surface = face.surface();
    let mut at_vertex: BTreeMap<u32, Vec<EndSegment>> = BTreeMap::new();
    for loop_id in face.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(TessellationError::MissingEntity)?;
        for coedge_id in face_loop.coedges() {
            let coedge = solid
                .coedge(*coedge_id)
                .ok_or(TessellationError::MissingEntity)?;
            let sampling = samplings
                .get(coedge.edge().index())
                .ok_or(TessellationError::MissingEntity)?;
            let pcurve = coedge.pcurve();
            let last = sampling.parameters.len().saturating_sub(1);
            for (end, index, neighbour) in [
                (EdgeEnd::Start, 0, 1),
                (EdgeEnd::End, last, last.saturating_sub(1)),
            ] {
                let (Some(point), Some(position), Some(next), Some(parameter)) = (
                    sampling.points.get(index),
                    sampling.positions.get(index),
                    sampling.points.get(neighbour),
                    sampling.parameters.get(neighbour),
                ) else {
                    return Err(TessellationError::MissingEntity);
                };
                let uv = if (end == EdgeEnd::Start) == coedge.sense().is_same() {
                    pcurve.start()
                } else {
                    pcurve.end()
                };
                at_vertex.entry(*position).or_default().push(EndSegment {
                    edge: coedge.edge(),
                    end,
                    at: BoundaryPoint {
                        uv,
                        position: *position,
                    },
                    toward: surface.project(*next, Some(pcurve.uv_at(*parameter))),
                    length: point.distance(*next),
                });
            }
        }
    }
    let mut longer = BTreeSet::new();
    for ends in at_vertex.values() {
        for (index, first) in ends.iter().enumerate() {
            for second in ends.iter().skip(index + 1) {
                let distinct = (first.edge, first.end) != (second.edge, second.end);
                let together =
                    coincide(&first.at, &second.at) || joined(surface, &first.at, &second.at);
                if distinct && together && first.runs_along(second) {
                    let split = if second.length > first.length {
                        second
                    } else {
                        first
                    };
                    longer.insert((split.edge, split.end));
                }
            }
        }
    }
    Ok(longer)
}

fn share_revisited_vertices(surface: &Surface, loops: &mut [Vec<BoundaryPoint>]) {
    let mut seen: BTreeMap<u32, Vec<Point2>> = BTreeMap::new();
    for point in loops.iter_mut().flatten() {
        let earlier = seen.entry(point.position).or_default();
        let same = earlier.iter().copied().find(|uv| {
            let first = BoundaryPoint {
                uv: *uv,
                position: point.position,
            };
            coincide(&first, point) || joined(surface, &first, point)
        });
        match same {
            Some(uv) => point.uv = uv,
            None => earlier.push(point.uv),
        }
    }
}

fn coincide(a: &BoundaryPoint, b: &BoundaryPoint) -> bool {
    a.position == b.position
        && (a.uv - b.uv).abs().max_element() <= UV_MERGE * (1.0 + a.uv.abs().max_element())
}

fn joined(surface: &Surface, a: &BoundaryPoint, b: &BoundaryPoint) -> bool {
    if a.position != b.position {
        return false;
    }
    let gap = b.uv - a.uv;
    let limit = JOINT_PARAMETER_GAP * (1.0 + a.uv.abs().max_element());
    if gap.abs().max_element() > limit {
        return false;
    }
    let derivatives = surface.evaluate(a.uv.x, a.uv.y);
    gap.x.abs() * derivatives.du.length() + gap.y.abs() * derivatives.dv.length()
        <= LINEAR_RESOLUTION
}

fn push_distinct(surface: &Surface, points: &mut Vec<BoundaryPoint>, point: BoundaryPoint) {
    if points
        .last()
        .is_none_or(|last| !coincide(last, &point) && !joined(surface, last, &point))
    {
        points.push(point);
    }
}

fn close_gaps(points: Vec<BoundaryPoint>, density: &Density) -> Vec<BoundaryPoint> {
    let count = points.len();
    let mut closed = Vec::with_capacity(count);
    for (index, point) in points.iter().enumerate() {
        closed.push(*point);
        let Some(next) = points.get((index + 1) % count.max(1)) else {
            continue;
        };
        if next.position != point.position || coincide(point, next) {
            continue;
        }
        let (from, to) = (density.index(point.uv), density.index(next.uv));
        let pieces = [to.x - from.x, to.y - from.y]
            .into_iter()
            .filter(|cells| cells.is_finite())
            .map(|cells| cells.abs().ceil())
            .fold(1.0, f64::max)
            .min(MAX_GAP_PIECES) as usize;
        for piece in 1..pieces {
            closed.push(BoundaryPoint {
                uv: density.at(from.lerp(to, piece as f64 / pieces as f64)),
                position: point.position,
            });
        }
    }
    closed
}

fn grid_points(loops: &[Vec<BoundaryPoint>], scaled: &Scaled) -> Result<Vec<Point2>, Interrupted> {
    let Scaled::Cells(density) = scaled else {
        return Ok(Vec::new());
    };
    let (columns, rows) = (density.u.segments(), density.v.segments());
    let segments: Vec<(Point2, Point2)> = loops
        .iter()
        .flat_map(|points| {
            let count = points.len();
            points.iter().enumerate().filter_map(move |(index, point)| {
                let next = points.get((index + 1) % count)?;
                Some((scaled.map(point.uv), scaled.map(next.uv)))
            })
        })
        .collect();
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); columns * rows];
    let clamp_cell = |value: f64, cells: usize| {
        let index = value.floor();
        if index.is_finite() && index > 0.0 {
            (index as usize).min(cells - 1)
        } else {
            0
        }
    };
    for (index, (start, end)) in segments.iter().enumerate() {
        let (low, high) = (start.min(*end), start.max(*end));
        for column in clamp_cell(low.x, columns)..=clamp_cell(high.x, columns) {
            for row in clamp_cell(low.y, rows)..=clamp_cell(high.y, rows) {
                if let Some(bucket) = buckets.get_mut(row * columns + column) {
                    bucket.push(index);
                }
            }
        }
    }
    let mut points = Vec::new();
    for row in 1..rows {
        interrupt::check()?;
        let Some(v) = density.v.line(row) else {
            continue;
        };
        for column in 1..columns {
            let local = Point2::new(column as f64, row as f64);
            let near = (column - 1..=column)
                .flat_map(|column| (row - 1..=row).map(move |row| row * columns + column))
                .filter_map(|bucket| buckets.get(bucket))
                .flatten()
                .filter_map(|segment| segments.get(*segment))
                .any(|(start, end)| distance_to_segment(local, *start, *end) < GRID_CLEARANCE);
            if !near && let Some(u) = density.u.line(column) {
                points.push(Point2::new(u, v));
            }
        }
    }
    Ok(points)
}

struct FacePoints<'a> {
    face: FaceId,
    scaled: Scaled<'a>,
    points: Vec<LocalPoint>,
    mapped: Vec<PlanePoint<f64>>,
    by_place: BTreeMap<[u64; 2], usize>,
    loops: Vec<Vec<usize>>,
}

impl<'a> FacePoints<'a> {
    fn new(face: FaceId, scaled: Scaled<'a>) -> Self {
        Self {
            face,
            scaled,
            points: Vec::new(),
            mapped: Vec::new(),
            by_place: BTreeMap::new(),
            loops: Vec::new(),
        }
    }

    fn insert(&mut self, point: LocalPoint) -> Result<usize, TessellationError> {
        let mapped = self.scaled.map(point.uv);
        let place = [mapped.x.to_bits(), mapped.y.to_bits()];
        if let Some(&index) = self.by_place.get(&place) {
            let existing = self
                .points
                .get(index)
                .ok_or(TessellationError::Triangulation(self.face))?;
            if existing.position != point.position {
                return Err(TessellationError::DuplicateBoundaryPoint(self.face));
            }
            return Ok(index);
        }
        let index = self.points.len();
        self.by_place.insert(place, index);
        self.points.push(point);
        self.mapped.push(PlanePoint::new(mapped.x, mapped.y));
        Ok(index)
    }

    fn add_loop(&mut self, points: &[BoundaryPoint]) -> Result<(), TessellationError> {
        let mut indices = Vec::with_capacity(points.len());
        for (index, point) in points.iter().enumerate() {
            if index.is_multiple_of(POLL_EVERY) {
                interrupt::check()?;
            }
            indices.push(self.insert(LocalPoint {
                uv: point.uv,
                position: Some(point.position),
            })?);
        }
        self.loops.push(indices);
        Ok(())
    }

    fn add_interior(&mut self, uv: Point2) -> Result<(), TessellationError> {
        self.insert(LocalPoint { uv, position: None }).map(|_| ())
    }

    fn triangulate(self) -> Result<FaceTriangulation, TessellationError> {
        let order = insertion_order(&self.mapped);
        let mut cdt = Cdt::new();
        let mut handles: Vec<Option<FixedVertexHandle>> = vec![None; self.points.len()];
        for (inserted, index) in order.iter().enumerate() {
            if inserted.is_multiple_of(POLL_EVERY) {
                interrupt::check()?;
            }
            let (Some(point), Some(slot)) = (self.mapped.get(*index), handles.get_mut(*index))
            else {
                return Err(TessellationError::Triangulation(self.face));
            };
            let handle = cdt
                .insert(*point)
                .map_err(|_| TessellationError::Triangulation(self.face))?;
            if handle.index() != inserted {
                return Err(TessellationError::Triangulation(self.face));
            }
            *slot = Some(handle);
        }
        let handle = |index: &usize| {
            handles
                .get(*index)
                .copied()
                .flatten()
                .ok_or(TessellationError::Triangulation(self.face))
        };
        for indices in &self.loops {
            let corners = indices.len();
            for (index, from) in indices.iter().enumerate() {
                if index.is_multiple_of(POLL_EVERY) {
                    interrupt::check()?;
                }
                let Some(to) = indices.get((index + 1) % corners) else {
                    continue;
                };
                let (from, to) = (handle(from)?, handle(to)?);
                if from == to || cdt.exists_constraint(from, to) {
                    continue;
                }
                if cdt.try_add_constraint(from, to).is_empty() {
                    return Err(TessellationError::SelfIntersectingBoundary(self.face));
                }
            }
        }
        let mut points = Vec::with_capacity(order.len());
        for index in &order {
            points.push(
                *self
                    .points
                    .get(*index)
                    .ok_or(TessellationError::Triangulation(self.face))?,
            );
        }
        Ok(FaceTriangulation {
            face: self.face,
            cdt,
            points,
        })
    }
}

struct FaceTriangulation {
    face: FaceId,
    cdt: Cdt,
    points: Vec<LocalPoint>,
}

impl FaceTriangulation {
    fn inside_faces(&self) -> Vec<bool> {
        let mut parity: Vec<Option<bool>> = vec![None; self.cdt.num_all_faces()];
        let mut pending: VecDeque<FixedFaceHandle<InnerTag>> = VecDeque::new();
        for face in self.cdt.inner_faces() {
            for edge in face.adjacent_edges() {
                if !edge.rev().face().is_outer() {
                    continue;
                }
                let inside = self.cdt.is_constraint_edge(edge.fix().as_undirected());
                if let Some(slot) = parity.get_mut(face.fix().index())
                    && slot.is_none()
                {
                    *slot = Some(inside);
                    pending.push_back(face.fix());
                }
            }
        }
        while let Some(fixed) = pending.pop_front() {
            let face = self.cdt.face(fixed);
            let here = parity
                .get(fixed.index())
                .copied()
                .flatten()
                .unwrap_or(false);
            for edge in face.adjacent_edges() {
                let Some(neighbour) = edge.rev().face().as_inner() else {
                    continue;
                };
                let crossing = self.cdt.is_constraint_edge(edge.fix().as_undirected());
                if let Some(slot) = parity.get_mut(neighbour.fix().index())
                    && slot.is_none()
                {
                    *slot = Some(here != crossing);
                    pending.push_back(neighbour.fix());
                }
            }
        }
        parity
            .into_iter()
            .map(|inside| inside.unwrap_or(false))
            .collect()
    }

    fn emit(
        self,
        surface: &Surface,
        sense: Sense,
        mesh: &mut Mesh,
    ) -> Result<(), TessellationError> {
        let inside = self.inside_faces();
        let mut corners: Vec<Option<u32>> = vec![None; self.points.len()];
        for face in self.cdt.inner_faces() {
            if !inside.get(face.fix().index()).copied().unwrap_or(false) {
                continue;
            }
            let [a, b, c] = face.vertices().map(|vertex| vertex.fix().index());
            let triangle = match sense {
                Sense::Same => [a, b, c],
                Sense::Reversed => [a, c, b],
            };
            let mut indices = [0u32; 3];
            for (slot, index) in indices.iter_mut().zip(triangle) {
                *slot = self.corner(index, &triangle, &mut corners, surface, sense, mesh)?;
            }
            let [pa, pb, pc] = indices.map(|corner| {
                mesh.vertices
                    .get(corner as usize)
                    .map(|vertex| vertex.position)
            });
            if pa == pb || pb == pc || pa == pc {
                continue;
            }
            mesh.triangles.push(indices);
        }
        Ok(())
    }

    fn corner(
        &self,
        index: usize,
        triangle: &[usize; 3],
        corners: &mut [Option<u32>],
        surface: &Surface,
        sense: Sense,
        mesh: &mut Mesh,
    ) -> Result<u32, TessellationError> {
        if let Some(corner) = corners.get(index).copied().flatten() {
            return Ok(corner);
        }
        let point = self
            .points
            .get(index)
            .ok_or(TessellationError::Triangulation(self.face))?;
        let position = match point.position {
            Some(position) => position,
            None => mesh.push_position(surface.point_at(point.uv))?,
        };
        let corner = mesh.push_vertex(MeshVertex {
            position,
            normal: vertex_normal(surface, point.uv, sense, &self.points, triangle),
        })?;
        if let Some(entry) = corners.get_mut(index) {
            *entry = Some(corner);
        }
        Ok(corner)
    }
}

fn vertex_normal(
    surface: &Surface,
    uv: Point2,
    sense: Sense,
    points: &[LocalPoint],
    triangle: &[usize; 3],
) -> Vector3 {
    let flip = sense.sign();
    if let Some(normal) = surface.normal(uv.x, uv.y) {
        return normal * flip;
    }
    let centroid = triangle
        .iter()
        .filter_map(|index| points.get(*index))
        .fold(Point2::ZERO, |sum, point| sum + point.uv)
        / 3.0;
    let nudged = uv.lerp(centroid, NORMAL_NUDGE);
    surface
        .normal(nudged.x, nudged.y)
        .map_or(Vector3::ZERO, |normal| normal * flip)
}
