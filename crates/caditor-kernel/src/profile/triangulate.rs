use std::collections::VecDeque;

use caditor_geometry::Point2;
use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use super::{ProfileLoop, Region};
use crate::tolerance::SamplingTolerance;

type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionMesh {
    pub points: Vec<Point2>,
    pub triangles: Vec<[u32; 3]>,
}

impl RegionMesh {
    pub fn anchor(&self) -> Option<Point2> {
        self.triangles
            .iter()
            .filter_map(|triangle| {
                let [a, b, c] = triangle.map(|index| self.points.get(index as usize).copied());
                Some([a?, b?, c?])
            })
            .max_by(|first, second| doubled_area(*first).total_cmp(&doubled_area(*second)))
            .map(|[a, b, c]| (a + b + c) / 3.0)
    }
}

fn doubled_area([a, b, c]: [Point2; 3]) -> f64 {
    (b - a).perp_dot(c - a).abs()
}

pub(super) fn loop_polygon(
    profile_loop: &ProfileLoop,
    tolerance: &SamplingTolerance,
) -> Vec<Point2> {
    let mut polygon: Vec<Point2> = Vec::new();
    for piece in profile_loop.pieces() {
        let mut samples: Vec<Point2> = piece
            .curve()
            .sample(piece.range(), tolerance)
            .into_iter()
            .map(|sample| sample.point)
            .collect();
        if piece.is_reversed() {
            samples.reverse();
        }
        for point in samples {
            if polygon.last() != Some(&point) {
                polygon.push(point);
            }
        }
    }
    if polygon.len() > 1 && polygon.first() == polygon.last() {
        polygon.pop();
    }
    polygon
}

pub(super) fn triangulate(region: &Region, tolerance: &SamplingTolerance) -> Option<RegionMesh> {
    let mut cdt = Cdt::new();
    let mut handles: Vec<FixedVertexHandle> = Vec::new();
    for profile_loop in region.loops() {
        let polygon = loop_polygon(profile_loop, tolerance);
        let mut ring = Vec::with_capacity(polygon.len());
        for point in polygon {
            let handle = cdt.insert(PlanePoint::new(point.x, point.y)).ok()?;
            if ring.last() != Some(&handle) {
                ring.push(handle);
            }
        }
        if ring.len() < 3 {
            continue;
        }
        for (index, from) in ring.iter().enumerate() {
            let to = ring.get((index + 1) % ring.len())?;
            if from == to || cdt.exists_constraint(*from, *to) {
                continue;
            }
            if cdt.try_add_constraint(*from, *to).is_empty() {
                return None;
            }
        }
        handles.extend(ring);
    }
    if handles.is_empty() {
        return None;
    }

    let inside = inside_faces(&cdt);
    let points = cdt
        .vertices()
        .map(|vertex| Point2::new(vertex.position().x, vertex.position().y))
        .collect();
    let triangles = cdt
        .inner_faces()
        .filter(|face| inside.get(face.fix().index()).copied().unwrap_or(false))
        .filter_map(|face| {
            let [a, b, c] = face
                .vertices()
                .map(|vertex| u32::try_from(vertex.fix().index()).ok());
            Some([a?, b?, c?])
        })
        .collect();
    Some(RegionMesh { points, triangles })
}

fn inside_faces(cdt: &Cdt) -> Vec<bool> {
    let mut parity: Vec<Option<bool>> = vec![None; cdt.num_all_faces()];
    let mut pending: VecDeque<FixedFaceHandle<InnerTag>> = VecDeque::new();
    for face in cdt.inner_faces() {
        for edge in face.adjacent_edges() {
            if !edge.rev().face().is_outer() {
                continue;
            }
            let crossing = cdt.is_constraint_edge(edge.fix().as_undirected());
            if let Some(slot) = parity.get_mut(face.fix().index())
                && slot.is_none()
            {
                *slot = Some(crossing);
                pending.push_back(face.fix());
            }
        }
    }
    while let Some(fixed) = pending.pop_front() {
        let face = cdt.face(fixed);
        let here = parity
            .get(fixed.index())
            .copied()
            .flatten()
            .unwrap_or(false);
        for edge in face.adjacent_edges() {
            let Some(neighbour) = edge.rev().face().as_inner() else {
                continue;
            };
            let crossing = cdt.is_constraint_edge(edge.fix().as_undirected());
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
