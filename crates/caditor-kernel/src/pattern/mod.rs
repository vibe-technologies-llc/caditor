#[cfg(test)]
mod tests;

use caditor_geometry::RigidTransform;
use thiserror::Error;

use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    error::GeometryError,
    interrupt::{self, Interrupted},
    naming::{FaceCopy, FaceName},
    topology::Solid,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternCopy {
    pub index: [u32; 2],
    pub placement: RigidTransform,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PatternError {
    #[error("copy {copy:?} could not be placed: {error}")]
    Placement {
        copy: [u32; 2],
        error: GeometryError,
    },
    #[error("the copies could not be joined: {0}")]
    Union(BooleanError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<BooleanError> for PatternError {
    fn from(error: BooleanError) -> Self {
        match error {
            BooleanError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            other => Self::Union(other),
        }
    }
}

pub fn pattern(solid: &Solid, copies: &[PatternCopy], feature: u64) -> Result<Solid, PatternError> {
    let mut parts = Vec::with_capacity(copies.len() + 1);
    parts.push(solid.clone());
    for copy in copies {
        interrupt::check()?;
        parts.push(placed(solid, copy, feature)?);
    }
    while parts.len() > 1 {
        parts = joined_in_pairs(parts)?;
    }
    Ok(parts.pop().unwrap_or_else(|| solid.clone()))
}

fn placed(solid: &Solid, copy: &PatternCopy, feature: u64) -> Result<Solid, PatternError> {
    let moved = solid
        .transformed(&copy.placement)
        .map_err(|error| PatternError::Placement {
            copy: copy.index,
            error,
        })?;
    let made = FaceCopy {
        pattern: feature,
        index: copy.index,
    };
    Ok(moved
        .with_face_origins(|origin| origin.copied(made))
        .with_face_names(|original| FaceName::pattern(feature, copy.index, original)))
}

fn joined_in_pairs(parts: Vec<Solid>) -> Result<Vec<Solid>, PatternError> {
    let mut joined = Vec::with_capacity(parts.len().div_ceil(2));
    let mut remaining = parts.into_iter();
    while let Some(first) = remaining.next() {
        interrupt::check()?;
        match remaining.next() {
            Some(second) => joined.push(boolean(&first, &second, BooleanOperation::Union)?),
            None => joined.push(first),
        }
    }
    Ok(joined)
}
