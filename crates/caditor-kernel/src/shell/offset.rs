use thiserror::Error;

use super::{
    Offsets, ShellError,
    inner::{Naming, inner_solid},
};
use crate::{
    interrupt::{self, Interrupted},
    tolerance::LINEAR_RESOLUTION,
    topology::{CrossingCheck, EdgeId, FaceId, Solid, VertexId},
};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum OffsetError {
    #[error("the distance is not a finite number above 0.000001 mm")]
    InvalidDistance,
    #[error("no face is to be moved")]
    NothingToMove,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("face {0:?} is curved in a way that cannot be offset")]
    UnsupportedFace(FaceId),
    #[error("the faces next to the moved ones cannot follow edge {0:?}")]
    UnsupportedEdge(EdgeId),
    #[error("face {0:?} would vanish")]
    Vanishes(FaceId),
    #[error("face {0:?} curves more tightly than the distance")]
    TooCurved(FaceId),
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

impl From<ShellError> for OffsetError {
    fn from(error: ShellError) -> Self {
        match error {
            ShellError::InvalidThickness => Self::InvalidDistance,
            ShellError::MissingFace(face) => Self::MissingFace(face),
            ShellError::UnsupportedFace(face) => Self::UnsupportedFace(face),
            ShellError::UnsupportedEdge(edge) => Self::UnsupportedEdge(edge),
            ShellError::TooCurved(face) => Self::TooCurved(face),
            ShellError::Corner(vertex) => Self::Corner(vertex),
            ShellError::EdgeCollapses(edge) => Self::EdgeCollapses(edge),
            ShellError::ClosesBesideClosing { face, .. } => Self::Vanishes(face),
            ShellError::Walls { faces, edge } => Self::Invalid { faces, edge },
            ShellError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            ShellError::TooThick
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

pub fn offset_faces(solid: &Solid, faces: &[FaceId], distance: f64) -> Result<Solid, OffsetError> {
    moved(solid, faces, distance).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn moved(solid: &Solid, faces: &[FaceId], distance: f64) -> Result<Solid, OffsetError> {
    if !distance.is_finite() || distance.abs() <= LINEAR_RESOLUTION {
        return Err(OffsetError::InvalidDistance);
    }
    if faces.is_empty() {
        return Err(OffsetError::NothingToMove);
    }
    for face in faces {
        solid.face(*face).ok_or(OffsetError::MissingFace(*face))?;
    }
    let offsets = Offsets::moving(solid, faces, -distance);
    let inner = inner_solid(&offsets, Naming::Kept)?;
    if let Some(face) = inner.dropped.iter().next() {
        return Err(OffsetError::Vanishes(*face));
    }
    let crossing = inner.solid.find_crossing()?;
    if let CrossingCheck::Crossing(crossing) = crossing {
        let [first, second] = crossing.faces;
        return Err(OffsetError::Crosses { first, second });
    }
    Ok(inner.solid)
}
