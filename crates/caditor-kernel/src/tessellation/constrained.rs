use std::collections::{BTreeSet, VecDeque};

use spade::{
    ConstrainedDelaunayTriangulation, Point2 as PlanePoint, Triangulation,
    handles::{FixedFaceHandle, FixedVertexHandle, InnerTag},
};

use crate::{
    interrupt,
    tessellation::{POLL_EVERY, TessellationError, insertion::insertion_order},
    topology::FaceId,
};

pub(super) type Cdt = ConstrainedDelaunayTriangulation<PlanePoint<f64>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Splitting {
    Allowed,
    Refused,
}

pub(super) struct Built {
    pub cdt: Cdt,
    pub members: Vec<usize>,
}

impl Built {
    pub(super) fn corners(&self, face: FixedFaceHandle<InnerTag>) -> Option<[usize; 3]> {
        let [a, b, c] = self
            .cdt
            .face(face)
            .vertices()
            .map(|vertex| self.members.get(vertex.fix().index()).copied());
        Some([a?, b?, c?])
    }
}

pub(super) fn segments(loops: &[Vec<usize>]) -> Vec<(usize, usize)> {
    let mut seen = BTreeSet::new();
    let mut segments = Vec::new();
    for indices in loops {
        let corners = indices.len();
        for (index, from) in indices.iter().enumerate() {
            let Some(to) = indices.get((index + 1) % corners) else {
                continue;
            };
            if from != to && seen.insert((*from.min(to), *from.max(to))) {
                segments.push((*from, *to));
            }
        }
    }
    segments
}

pub(super) fn build(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    members: Vec<usize>,
    constraints: &[(usize, usize)],
    splitting: Splitting,
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
        if handle.index() != inserted || slot.is_some() {
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
        members: inserted_members,
    })
}

pub(super) fn whole(
    face: FaceId,
    mapped: &[PlanePoint<f64>],
    segments: &[(usize, usize)],
) -> Result<Vec<[usize; 3]>, TessellationError> {
    let members = (0..mapped.len()).collect();
    let built = build(face, mapped, members, segments, Splitting::Allowed)?;
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
