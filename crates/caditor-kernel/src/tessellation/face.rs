use std::collections::VecDeque;

use caditor_geometry::{Aabb2, Point2, Vector3};
use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use crate::{
    coordinates::distance_to_segment,
    curve::Curve,
    sense::Sense,
    surface::Surface,
    tessellation::{
        EdgeSampling, Mesh, MeshVertex, TessellationError,
        density::{Density, density},
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

type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

#[derive(Debug, Clone, Copy, PartialEq)]
struct BoundaryPoint {
    uv: Point2,
    position: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LocalPoint {
    uv: Point2,
    position: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scaled {
    origin: Point2,
    u_scale: f64,
    v_scale: f64,
}

impl Scaled {
    fn map(&self, uv: Point2) -> Point2 {
        let offset = uv - self.origin;
        Point2::new(snap(offset.x * self.u_scale), snap(offset.y * self.v_scale))
    }
}

fn snap(value: f64) -> f64 {
    if value.abs() < TINY_COORDINATE {
        0.0
    } else {
        value
    }
}

pub(crate) fn triangulate(
    solid: &Solid,
    face_id: FaceId,
    samplings: &[EdgeSampling],
    tolerance: &SamplingTolerance,
    mesh: &mut Mesh,
) -> Result<(), TessellationError> {
    let face = solid
        .face(face_id)
        .ok_or(TessellationError::MissingEntity)?;
    let surface = face.surface();
    let loops = boundary_loops(solid, face, samplings)?;
    let Some(bounds) = Aabb2::from_points(loops.iter().flatten().map(|point| point.uv)) else {
        return Ok(());
    };
    let density = density(surface, bounds, tolerance);
    let steps = Point2::new(
        bounds.size().x / density.u_segments as f64,
        bounds.size().y / density.v_segments as f64,
    );
    let loops: Vec<Vec<BoundaryPoint>> = loops
        .into_iter()
        .map(|points| close_gaps(points, steps))
        .collect();
    let scaled = Scaled {
        origin: bounds.min(),
        u_scale: density.u_scale,
        v_scale: density.v_scale,
    };
    let mut triangulation = FaceTriangulation::new(face_id, scaled);
    for points in &loops {
        triangulation.add_loop(points)?;
    }
    for uv in grid_points(&loops, bounds, &density, &scaled) {
        triangulation.add_interior(uv)?;
    }
    triangulation.emit(surface, face.sense(), mesh)
}

pub(crate) fn pole_edge_segments(
    solid: &Solid,
    face_id: FaceId,
    tolerance: &SamplingTolerance,
) -> Result<Vec<(EdgeId, usize)>, TessellationError> {
    let face = solid
        .face(face_id)
        .ok_or(TessellationError::MissingEntity)?;
    let surface = face.surface();
    let poles = surface.poles();
    if poles.is_empty() {
        return Ok(Vec::new());
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
        return Ok(Vec::new());
    };
    let rows = density(surface, bounds, tolerance).v_segments;
    let row_height = bounds.size().y / rows as f64;
    if row_height.is_nan() || row_height <= 0.0 {
        return Ok(Vec::new());
    }
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
        let rise = (pcurve.end().y - pcurve.start().y).abs();
        let pieces = (rise / row_height).round();
        if pieces.is_finite() && pieces >= 2.0 {
            segments.push((coedge.edge(), (pieces as usize).min(MAX_POLE_EDGE_PIECES)));
        }
    }
    Ok(segments)
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
    Ok(loops)
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

fn close_gaps(points: Vec<BoundaryPoint>, steps: Point2) -> Vec<BoundaryPoint> {
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
        let gap = (next.uv - point.uv).abs();
        let pieces = [(gap.x, steps.x), (gap.y, steps.y)]
            .into_iter()
            .filter(|(_, step)| *step > 0.0)
            .map(|(length, step)| (length / step).ceil())
            .fold(1.0, f64::max)
            .min(MAX_GAP_PIECES) as usize;
        for piece in 1..pieces {
            closed.push(BoundaryPoint {
                uv: point.uv.lerp(next.uv, piece as f64 / pieces as f64),
                position: point.position,
            });
        }
    }
    closed
}

fn grid_points(
    loops: &[Vec<BoundaryPoint>],
    bounds: Aabb2,
    density: &Density,
    scaled: &Scaled,
) -> Vec<Point2> {
    let (columns, rows) = (density.u_segments, density.v_segments);
    if columns < 2 || rows < 2 {
        return Vec::new();
    }
    let size = bounds.size();
    let cell = Point2::new(
        size.x * scaled.u_scale / columns as f64,
        size.y * scaled.v_scale / rows as f64,
    );
    if cell.x <= 0.0 || cell.y <= 0.0 {
        return Vec::new();
    }
    let clearance = GRID_CLEARANCE * cell.x.min(cell.y);
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
        for column in clamp_cell(low.x / cell.x, columns)..=clamp_cell(high.x / cell.x, columns) {
            for row in clamp_cell(low.y / cell.y, rows)..=clamp_cell(high.y / cell.y, rows) {
                if let Some(bucket) = buckets.get_mut(row * columns + column) {
                    bucket.push(index);
                }
            }
        }
    }
    let mut points = Vec::new();
    for row in 1..rows {
        for column in 1..columns {
            let local = Point2::new(column as f64 * cell.x, row as f64 * cell.y);
            let near = (column - 1..=column)
                .flat_map(|column| (row - 1..=row).map(move |row| row * columns + column))
                .filter_map(|bucket| buckets.get(bucket))
                .flatten()
                .filter_map(|segment| segments.get(*segment))
                .any(|(start, end)| distance_to_segment(local, *start, *end) < clearance);
            if !near {
                points.push(Point2::new(
                    bounds.min().x + size.x * column as f64 / columns as f64,
                    bounds.min().y + size.y * row as f64 / rows as f64,
                ));
            }
        }
    }
    points
}

struct FaceTriangulation {
    face: FaceId,
    scaled: Scaled,
    cdt: Cdt,
    points: Vec<LocalPoint>,
    slots: Vec<usize>,
}

impl FaceTriangulation {
    fn new(face: FaceId, scaled: Scaled) -> Self {
        Self {
            face,
            scaled,
            cdt: Cdt::default(),
            points: Vec::new(),
            slots: Vec::new(),
        }
    }

    fn insert(
        &mut self,
        point: LocalPoint,
    ) -> Result<(FixedVertexHandle, bool), TessellationError> {
        let mapped = self.scaled.map(point.uv);
        let handle = self
            .cdt
            .insert(PlanePoint::new(mapped.x, mapped.y))
            .map_err(|_| TessellationError::Triangulation(self.face))?;
        if handle.index() == self.slots.len() {
            self.slots.push(self.points.len());
            self.points.push(point);
            return Ok((handle, true));
        }
        let existing = self
            .slots
            .get(handle.index())
            .and_then(|slot| self.points.get(*slot))
            .ok_or(TessellationError::Triangulation(self.face))?;
        if existing.position != point.position {
            return Err(TessellationError::DuplicateBoundaryPoint(self.face));
        }
        Ok((handle, false))
    }

    fn add_loop(&mut self, points: &[BoundaryPoint]) -> Result<(), TessellationError> {
        let mut handles = Vec::with_capacity(points.len());
        for point in points {
            let (handle, _) = self.insert(LocalPoint {
                uv: point.uv,
                position: Some(point.position),
            })?;
            handles.push(handle);
        }
        let count = handles.len();
        for (index, from) in handles.iter().enumerate() {
            let Some(to) = handles.get((index + 1) % count) else {
                continue;
            };
            if from == to || self.cdt.exists_constraint(*from, *to) {
                continue;
            }
            if self.cdt.try_add_constraint(*from, *to).is_empty() {
                return Err(TessellationError::SelfIntersectingBoundary(self.face));
            }
        }
        Ok(())
    }

    fn add_interior(&mut self, uv: Point2) -> Result<(), TessellationError> {
        self.insert(LocalPoint { uv, position: None }).map(|_| ())
    }

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
            let local = face
                .vertices()
                .map(|vertex| self.slots.get(vertex.fix().index()).copied());
            let [Some(a), Some(b), Some(c)] = local else {
                return Err(TessellationError::Triangulation(self.face));
            };
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
