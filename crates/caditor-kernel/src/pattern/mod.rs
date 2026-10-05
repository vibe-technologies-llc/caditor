#[cfg(test)]
mod tests;

use caditor_geometry::Similarity;
use thiserror::Error;

use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    interrupt::{self, Interrupted},
    naming::{FaceCopy, FaceName},
    topology::{Solid, TransformError},
};

const ORIGINAL: [u32; 2] = [0, 0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternCopy {
    pub index: [u32; 2],
    pub placement: Similarity,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PatternError {
    #[error("copy {copy:?} could not be placed: {error}")]
    Placement {
        copy: [u32; 2],
        error: TransformError,
    },
    #[error("the copies {copies:?} could not be joined: {error}")]
    Union {
        copies: Vec<[u32; 2]>,
        error: BooleanError,
    },
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

struct Part {
    solid: Solid,
    copies: Vec<[u32; 2]>,
}

pub fn pattern(solid: &Solid, copies: &[PatternCopy], feature: u64) -> Result<Solid, PatternError> {
    let mut parts = Vec::with_capacity(copies.len() + 1);
    parts.push(Part {
        solid: solid.clone(),
        copies: vec![ORIGINAL],
    });
    for copy in copies {
        interrupt::check()?;
        parts.push(Part {
            solid: placed(solid, copy, feature)?,
            copies: vec![copy.index],
        });
    }
    while parts.len() > 1 {
        parts = joined_in_pairs(parts)?;
    }
    Ok(parts.pop().map_or_else(|| solid.clone(), |part| part.solid))
}

fn placed(solid: &Solid, copy: &PatternCopy, feature: u64) -> Result<Solid, PatternError> {
    let moved = solid.mapped(&copy.placement).map_err(|error| match error {
        TransformError::Cancelled(interrupted) => PatternError::Cancelled(interrupted),
        error => PatternError::Placement {
            copy: copy.index,
            error,
        },
    })?;
    let made = FaceCopy {
        pattern: feature,
        index: copy.index,
    };
    Ok(moved
        .with_face_origins(|origin| origin.copied(made))
        .with_face_names(|original| FaceName::pattern(feature, copy.index, original)))
}

fn joined_in_pairs(parts: Vec<Part>) -> Result<Vec<Part>, PatternError> {
    let mut joined = Vec::with_capacity(parts.len().div_ceil(2));
    let mut remaining = parts.into_iter();
    while let Some(first) = remaining.next() {
        interrupt::check()?;
        match remaining.next() {
            Some(second) => joined.push(union(first, second)?),
            None => joined.push(first),
        }
    }
    Ok(joined)
}

fn union(first: Part, second: Part) -> Result<Part, PatternError> {
    let copies: Vec<[u32; 2]> = first.copies.into_iter().chain(second.copies).collect();
    let solid = match boolean(&first.solid, &second.solid, BooleanOperation::Union) {
        Ok(solid) => solid,
        Err(BooleanError::Cancelled(interrupted)) => return Err(interrupted.into()),
        Err(error @ BooleanError::NonManifold(_)) => separate_shells(&first.solid, &second.solid)
            .ok_or(PatternError::Union {
            copies: copies.clone(),
            error,
        })?,
        Err(error) => return Err(PatternError::Union { copies, error }),
    };
    Ok(Part { solid, copies })
}

fn separate_shells(first: &Solid, second: &Solid) -> Option<Solid> {
    let beside = first.beside(second)?;
    beside.validate().ok()?;
    Some(beside)
}
