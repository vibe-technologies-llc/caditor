use std::collections::{BTreeMap, VecDeque};

use caditor_geometry::Point2;
use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use super::{ProfileLoop, Region};
use crate::tolerance::{LINEAR_RESOLUTION, SamplingTolerance};

type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

const WELD_SHARE: f64 = 1e-7;
const WELD_TOLERANCES: f64 = 4.0;

#[derive(Clone, Debug, Default, PartialEq)]
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
        samples.pop();
        for point in samples {
            if !polygon
                .last()
                .is_some_and(|last| last.distance(point) <= LINEAR_RESOLUTION)
            {
                polygon.push(point);
            }
        }
    }
    while polygon.len() > 1
        && matches!((polygon.first(), polygon.last()), (Some(first), Some(last)) if first.distance(*last) <= LINEAR_RESOLUTION)
    {
        polygon.pop();
    }
    polygon
}

struct Welded {
    reach: f64,
    cells: BTreeMap<(i64, i64), Vec<(Point2, FixedVertexHandle)>>,
}

impl Welded {
    fn of(region: &Region) -> Self {
        let extent = region.bounds().map_or(0.0, |bounds| bounds.size().length());
        Self {
            reach: WELD_TOLERANCES * (extent * WELD_SHARE).max(LINEAR_RESOLUTION),
            cells: BTreeMap::new(),
        }
    }

    fn cell(&self, point: Point2) -> (i64, i64) {
        let along = |coordinate: f64| {
            let scaled = (coordinate / self.reach).floor();
            if scaled.is_finite() { scaled as i64 } else { 0 }
        };
        (along(point.x), along(point.y))
    }

    fn vertex(&mut self, cdt: &mut Cdt, point: Point2) -> Option<FixedVertexHandle> {
        let (column, row) = self.cell(point);

        let near = (-1..=1)
            .flat_map(|across| (-1..=1).map(move |up| (column + across, row + up)))
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .find(|(placed, _)| placed.distance(point) <= self.reach)
            .map(|(_, handle)| *handle);

        if near.is_some() {
            return near;
        }

        let handle = cdt.insert(PlanePoint::new(point.x, point.y)).ok()?;

        self.cells
            .entry((column, row))
            .or_default()
            .push((point, handle));
        Some(handle)
    }
}

pub(super) fn triangulate(region: &Region, tolerance: &SamplingTolerance) -> Option<RegionMesh> {
    let mut cdt = Cdt::new();
    let mut welded = Welded::of(region);
    let mut bounded = false;

    for profile_loop in region.loops() {
        let polygon = loop_polygon(profile_loop, tolerance);
        let mut ring = Vec::with_capacity(polygon.len());

        for point in polygon {
            let handle = welded.vertex(&mut cdt, point)?;
            if ring.last() != Some(&handle) {
                ring.push(handle);
            }
        }
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            continue;
        }
        for (index, from) in ring.iter().enumerate() {
            let to = ring.get((index + 1) % ring.len())?;

            if from == to || cdt.exists_constraint(*from, *to) {
                continue;
            }
            cdt.add_constraint_and_split(*from, *to, |crossing| crossing);
        }
        bounded = true;
    }
    if !bounded {
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
