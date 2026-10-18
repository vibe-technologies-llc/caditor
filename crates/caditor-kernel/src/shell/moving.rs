use std::collections::BTreeSet;

use caditor_geometry::RigidTransform;
use thiserror::Error;

use super::{
    Carried, OFFSET_TOLERANCE, Offsets, ShellError,
    collapse::loop_edges,
    inner::{Naming, inner_solid},
};
use crate::{
    interrupt::{self, Interrupted},
    tolerance::LINEAR_RESOLUTION,
    topology::{CrossingCheck, EdgeId, FaceId, Solid, VertexId},
};

const TRAVEL_SAMPLES: usize = 8;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum MoveError {
    #[error("the faces would not move by more than 0.000001 mm")]
    Unmoved,
    #[error("no face is to be moved")]
    NothingToMove,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("face {0:?} cannot be moved")]
    UnsupportedFace(FaceId),
    #[error("the faces next to the moved ones cannot follow edge {0:?}")]
    UnsupportedEdge(EdgeId),
    #[error("face {0:?} would vanish")]
    Vanishes(FaceId),
    #[error("the faces cannot meet at vertex {0:?}")]
    Corner(VertexId),
    #[error("the face along edge {0:?} would shrink to nothing")]
    EdgeCollapses(EdgeId),
    #[error(
        "the moved faces would not form a valid body, perhaps by passing through another part of it"
    )]
    Invalid {
        faces: Vec<FaceId>,
        edge: Option<EdgeId>,
    },
    #[error("faces {first:?} and {second:?} would cross")]
    Crosses { first: FaceId, second: FaceId },
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<ShellError> for MoveError {
    fn from(error: ShellError) -> Self {
        match error {
            ShellError::MissingFace(face) => Self::MissingFace(face),
            ShellError::UnsupportedFace(face)
            | ShellError::CurvedOpening(face)
            | ShellError::TooCurved(face) => Self::UnsupportedFace(face),
            ShellError::UnsupportedEdge(edge) => Self::UnsupportedEdge(edge),
            ShellError::Corner(vertex) => Self::Corner(vertex),
            ShellError::EdgeCollapses(edge) => Self::EdgeCollapses(edge),
            ShellError::ClosesBesideClosing { face, .. } => Self::Vanishes(face),
            ShellError::Walls { faces, edge } => Self::Invalid { faces, edge },
            ShellError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            ShellError::InvalidThickness
            | ShellError::InvalidFaceThickness(_)
            | ShellError::OpenedFaceThickness(_)
            | ShellError::TooThick
            | ShellError::Opening(_)
            | ShellError::Overhang { .. }
            | ShellError::Voids(_)
            | ShellError::Boolean(_) => Self::Invalid {
                faces: Vec::new(),
                edge: None,
            },
        }
    }
}

pub fn move_faces(
    solid: &Solid,
    faces: &[FaceId],
    motion: &RigidTransform,
) -> Result<Solid, MoveError> {
    move_each(
        solid,
        &[Carried {
            faces,
            motion: *motion,
        }],
    )
}

pub(super) fn move_each(solid: &Solid, groups: &[Carried<'_>]) -> Result<Solid, MoveError> {
    carried(solid, groups).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn travel(solid: &Solid, faces: &[FaceId], motion: &RigidTransform) -> f64 {
    let edges: BTreeSet<EdgeId> = faces
        .iter()
        .flat_map(|face| loop_edges(solid, *face))
        .collect();
    edges
        .into_iter()
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            edge.interval()
                .split(TRAVEL_SAMPLES)
                .map(|parameter| edge.curve().point(parameter))
                .collect::<Vec<_>>()
        })
        .map(|point| motion.apply_point(point).distance(point))
        .fold(0.0, f64::max)
}

fn carried(solid: &Solid, groups: &[Carried<'_>]) -> Result<Solid, MoveError> {
    if groups.iter().all(|group| group.faces.is_empty()) {
        return Err(MoveError::NothingToMove);
    }
    for face in groups.iter().flat_map(|group| group.faces) {
        solid.face(*face).ok_or(MoveError::MissingFace(*face))?;
    }
    let reach = groups
        .iter()
        .map(|group| travel(solid, group.faces, &group.motion))
        .fold(0.0, f64::max);
    if !reach.is_finite() || reach <= LINEAR_RESOLUTION {
        return Err(MoveError::Unmoved);
    }
    let offsets = Offsets::rigid(solid, groups, reach.max(OFFSET_TOLERANCE));
    let inner = inner_solid(&offsets, Naming::Kept)?;
    if let Some(face) = inner.dropped.iter().next() {
        return Err(MoveError::Vanishes(*face));
    }
    if let CrossingCheck::Crossing(crossing) = inner.solid.find_crossing()? {
        let [first, second] = crossing.faces;
        return Err(MoveError::Crosses { first, second });
    }
    Ok(inner.solid)
}
