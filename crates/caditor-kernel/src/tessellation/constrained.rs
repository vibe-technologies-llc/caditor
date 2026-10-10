use std::{
    collections::{BTreeSet, VecDeque},
    ops::Range,
};

use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use crate::{
    interrupt,
    tessellation::{POLL_EVERY, TessellationError, insertion::insertion_order},
    topology::FaceId,
};

const MIN_HOLE_CORNERS: usize = 4;
const HOLE_CLEARANCE: f64 = 1e-6;

pub(super) type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Splitting {
    Allowed,
    Refused,
}

pub(super) struct Built {
    pub cdt: Cdt,
    helpers: usize,
    members: Vec<usize>,
    handles: Vec<Option<FixedVertexHandle>>,
}

impl Built {
    pub(super) fn handle(&self, member: usize) -> Option<FixedVertexHandle> {
        self.handles.get(member).copied().flatten()
    }

    pub(super) fn member(&self, vertex: FixedVertexHandle) -> Option<usize> {
        vertex
            .index()
            .checked_sub(self.helpers)
            .and_then(|local| self.members.get(local))
            .copied()
    }

    pub(super) fn corners(&self, face: FixedFaceHandle<InnerTag>) -> Option<[usize; 3]> {
        let [a, b, c] = self
            .cdt
            .face(face)
            .vertices()
            .map(|vertex| self.member(vertex.fix()));
        Some([a?, b?, c?])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Hole {
    pub centre: PlanePoint<f64>,
    pub low: f64,
    pub high: f64,
    pub segments: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Outline {
    pub segments: Vec<(usize, usize)>,
    pub holes: Vec<Hole>,
}

pub(super) fn outline(mapped: &[PlanePoint<f64>], loops: &[Vec<usize>]) -> Outline {
    let mut seen = BTreeSet::new();
    let mut segments = Vec::new();
    let mut ranges = Vec::with_capacity(loops.len());
    for indices in loops {
        let start = segments.len();
        let corners = indices.len();
        for (index, from) in indices.iter().enumerate() {
            let Some(to) = indices.get((index + 1) % corners) else {
                continue;
            };
            if from != to && seen.insert((*from.min(to), *from.max(to))) {
                segments.push((*from, *to));
            }
        }
        ranges.push(start..segments.len());
    }
    Outline {
        holes: holes(mapped, loops, &ranges),
        segments,
    }
}

fn holes(mapped: &[PlanePoint<f64>], loops: &[Vec<usize>], ranges: &[Range<usize>]) -> Vec<Hole> {
    let Some(outer) = loops.first().map(|outer| signed_area(mapped, outer)) else {
        return Vec::new();
    };
    if !outer.is_normal() {
        return Vec::new();
    }
    loops
        .iter()
        .zip(ranges)
        .skip(1)
        .filter_map(|(indices, range)| hole(mapped, indices, range.clone(), -outer.signum()))
        .collect()
}

fn signed_area(mapped: &[PlanePoint<f64>], indices: &[usize]) -> f64 {
    let corners: Vec<PlanePoint<f64>> = indices
        .iter()
        .filter_map(|index| mapped.get(*index).copied())
        .collect();
    let Some(origin) = corners.first().copied() else {
        return 0.0;
    };
    corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .map(|(a, b)| (a.x - origin.x) * (b.y - origin.y) - (a.y - origin.y) * (b.x - origin.x))
        .sum::<f64>()
        * 0.5
}

fn hole(
    mapped: &[PlanePoint<f64>],
    indices: &[usize],
    segments: Range<usize>,
    turning: f64,
) -> Option<Hole> {
    let corners: Vec<PlanePoint<f64>> = indices
        .iter()
        .map(|index| mapped.get(*index).copied())
        .collect::<Option<_>>()?;
    if corners.len() < MIN_HOLE_CORNERS {
        return None;
    }
    let count = corners.len() as f64;
    let centre = corners.iter().fold(PlanePoint::new(0.0, 0.0), |sum, at| {
        PlanePoint::new(sum.x + at.x / count, sum.y + at.y / count)
    });
    for (a, b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let (along, towards) = ([b.x - a.x, b.y - a.y], [centre.x - a.x, centre.y - a.y]);
        let side = (along[0] * towards[1] - along[1] * towards[0]) * turning;
        let scale = along[0].hypot(along[1]) * towards[0].hypot(towards[1]);
        if side <= HOLE_CLEARANCE * scale || !side.is_finite() {
            return None;
        }
    }
    let (low, high) = corners
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), at| {
            (low.min(at.x), high.max(at.x))
        });
    Some(Hole {
        centre,
        low,
        high,
        segments,
    })
}

pub(super) fn build(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    members: Vec<usize>,
    constraints: &[(usize, usize)],
    splitting: Splitting,
    helpers: &[PlanePoint<f64>],
) -> Result<Built, TessellationError> {
    let mut chosen = Vec::with_capacity(members.len());
    for member in &members {
        chosen.push(
            *mapped
                .get(*member)
                .ok_or(TessellationError::Triangulation(face))?,
        );
    }
    let order = insertion_order(&chosen);
    let mut cdt = Cdt::new();
    for (inserted, helper) in helpers.iter().enumerate() {
        let handle = cdt
            .insert(*helper)
            .map_err(|_| TessellationError::Triangulation(face))?;
        if handle.index() != inserted {
            return Err(TessellationError::Triangulation(face));
        }
    }
    let mut handles: Vec<Option<FixedVertexHandle>> = vec![None; mapped.len()];
    let mut inserted_members = Vec::with_capacity(order.len());
    for (inserted, local) in order.iter().enumerate() {
        if inserted.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        let (Some(point), Some(member)) = (chosen.get(*local), members.get(*local)) else {
            return Err(TessellationError::Triangulation(face));
        };
        let handle = cdt
            .insert(*point)
            .map_err(|_| TessellationError::Triangulation(face))?;
        let slot = handles
            .get_mut(*member)
            .ok_or(TessellationError::Triangulation(face))?;
        if handle.index() != helpers.len() + inserted || slot.is_some() {
            return Err(TessellationError::Triangulation(face));
        }
        *slot = Some(handle);
        inserted_members.push(*member);
    }
    let handle = |index: usize| {
        handles
            .get(index)
            .copied()
            .flatten()
            .ok_or(TessellationError::Triangulation(face))
    };
    for (index, (from, to)) in constraints.iter().enumerate() {
        if index.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        let (from, to) = (handle(*from)?, handle(*to)?);
        if from == to || cdt.exists_constraint(from, to) {
            continue;
        }
        let added = cdt.try_add_constraint(from, to);
        match (added.len(), splitting) {
            (0, _) => return Err(TessellationError::SelfIntersectingBoundary(face)),
            (1, _) | (_, Splitting::Allowed) => {}
            (_, Splitting::Refused) => return Err(TessellationError::Triangulation(face)),
        }
    }
    Ok(Built {
        cdt,
        helpers: helpers.len(),
        members: inserted_members,
        handles,
    })
}

pub(super) fn whole(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let segments = &outline.segments;
    let helpers: Vec<PlanePoint<f64>> = outline.holes.iter().map(|hole| hole.centre).collect();
    if !helpers.is_empty() {
        match whole_with(face, mapped, segments, &helpers) {
            Err(TessellationError::Triangulation(_)) => {}
            settled => return settled,
        }
    }
    whole_with(face, mapped, segments, &[])
}

fn whole_with(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    segments: &[(usize, usize)],
    helpers: &[PlanePoint<f64>],
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let members = (0..mapped.len()).collect();
    let built = build(face, mapped, members, segments, Splitting::Allowed, helpers)?;
    inside_triangles(face, &built)
}

pub(super) fn inside_triangles(
    face: FaceId,
    built: &Built,
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let inside = inside_from_outside(&built.cdt);
    let mut triangles = Vec::new();
    for fixed in built.cdt.fixed_inner_faces() {
        if inside.get(fixed.index()).copied().unwrap_or(false) {
            triangles.push(
                built
                    .corners(fixed)
                    .ok_or(TessellationError::Triangulation(face))?,
            );
        }
    }
    Ok(triangles)
}

pub(super) fn inside_from_outside(cdt: &Cdt) -> Vec<bool> {
    let mut parity: Vec<Option<bool>> = vec![None; cdt.num_all_faces()];
    let mut pending: VecDeque<FixedFaceHandle<InnerTag>> = VecDeque::new();
    for face in cdt.inner_faces() {
        for edge in face.adjacent_edges() {
            if !edge.rev().face().is_outer() {
                continue;
            }
            let inside = cdt.is_constraint_edge(edge.fix().as_undirected());
            if let Some(slot) = parity.get_mut(face.fix().index())
                && slot.is_none()
            {
                *slot = Some(inside);
                pending.push_back(face.fix());
            }
        }
    }
    spread_parity(cdt, &mut parity, pending, |_| true);
    parity
        .into_iter()
        .map(|inside| inside.unwrap_or(false))
        .collect()
}

pub(super) fn spread_parity(
    cdt: &Cdt,
    parity: &mut [Option<bool>],
    mut pending: VecDeque<FixedFaceHandle<InnerTag>>,
    reaches: impl Fn(FixedFaceHandle<InnerTag>) -> bool,
) {
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
            if !reaches(neighbour.fix()) {
                continue;
            }
            let crossing = cdt.is_constraint_edge(edge.fix().as_undirected());
            if let Some(slot) = parity.get_mut(neighbour.fix().index())
                && slot.is_none()
            {
                *slot = Some(here != crossing);
                pending.push_back(neighbour.fix());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;

    fn square_with(holes: &[Vec<(f64, f64)>]) -> (Vec<PlanePoint<f64>>, Vec<Vec<usize>>) {
        let mut mapped: Vec<PlanePoint<f64>> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| PlanePoint::new(x, y))
            .to_vec();
        let mut loops = vec![vec![0, 1, 2, 3]];
        for hole in holes {
            let first = mapped.len();
            mapped.extend(hole.iter().map(|(x, y)| PlanePoint::new(*x, *y)));
            loops.push((first..mapped.len()).collect());
        }
        (mapped, loops)
    }

    fn circle(centre: (f64, f64), radius: f64, turn: f64) -> Vec<(f64, f64)> {
        (0..48)
            .map(|step| {
                let angle = turn * TAU * f64::from(step) / 48.0;
                (
                    centre.0 + radius * angle.cos(),
                    centre.1 + radius * angle.sin(),
                )
            })
            .collect()
    }

    fn area(mapped: &[PlanePoint<f64>], triangles: &[[usize; 3]]) -> f64 {
        triangles
            .iter()
            .map(|[a, b, c]| {
                let (a, b, c) = (mapped[*a], mapped[*b], mapped[*c]);
                0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x))
            })
            .sum()
    }

    #[test]
    fn a_hole_seen_whole_from_its_centre_gets_a_helper_that_no_kept_triangle_uses() {
        let face = FaceId::from_index(0).unwrap();
        let notched = [
            (6.0, 2.0),
            (6.0, 8.0),
            (7.0, 8.0),
            (7.0, 3.0),
            (8.0, 3.0),
            (8.0, 8.0),
            (9.0, 8.0),
            (9.0, 2.0),
        ];
        let (mapped, loops) = square_with(&[circle((3.0, 5.0), 1.5, -1.0), notched.to_vec()]);

        let outline = outline(&mapped, &loops);
        let helped = whole(face, &mapped, &outline).unwrap();
        let plain = Outline {
            segments: outline.segments.clone(),
            holes: Vec::new(),
        };
        let unhelped = whole(face, &mapped, &plain).unwrap();

        let [hole] = outline.holes.as_slice() else {
            panic!("{:?}", outline.holes);
        };
        assert!((hole.centre.x - 3.0).abs() < 1e-12 && (hole.centre.y - 5.0).abs() < 1e-12);
        assert!((hole.low - 1.5).abs() < 1e-12 && (hole.high - 4.5).abs() < 1e-12);
        assert_eq!(hole.segments, 4..52);
        assert_eq!(helped.len(), unhelped.len());
        assert!(helped.iter().flatten().all(|corner| *corner < mapped.len()));
        assert!((area(&mapped, &helped) - area(&mapped, &unhelped)).abs() < 1e-9);
    }

    #[test]
    fn a_loop_turning_with_the_outer_one_gets_no_helper() {
        let (mapped, loops) = square_with(&[circle((5.0, 5.0), 2.0, 1.0)]);

        assert!(outline(&mapped, &loops).holes.is_empty());
        assert!(outline(&mapped, &loops[..1]).holes.is_empty());
    }
}
