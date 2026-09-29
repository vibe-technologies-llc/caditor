use thiserror::Error;

use crate::{error::GeometryError, interrupt::Interrupted, profile::RegionKey};

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProfileError {
    #[error("the sketch has no closed profile")]
    NoClosedProfile,
    #[error("no region of the sketch is chosen")]
    EmptySelection,
    #[error("a chosen region no longer exists in the sketch")]
    MissingRegion(RegionKey),
    #[error("curve {entity} appears more than once")]
    DuplicateEntity { entity: u64 },
    #[error("curve {entity} has no length")]
    Degenerate { entity: u64 },
    #[error("curve {entity} cannot be used: {error}")]
    InvalidCurve { entity: u64, error: GeometryError },
    #[error("curve {entity} runs back over itself")]
    SelfOverlap { entity: u64 },
    #[error("curves {first} and {second} run along each other and cannot be told apart")]
    Overlap { first: u64, second: u64 },
    #[error("curves {entities:?} cross each other too often to be split into regions")]
    TooIntricate { entities: Vec<u64> },
    #[error("curves {entities:?} could not be split into closed regions")]
    Unresolved { entities: Vec<u64> },
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl ProfileError {
    pub fn unresolved() -> Self {
        Self::Unresolved {
            entities: Vec::new(),
        }
    }

    pub fn entities(&self) -> Vec<u64> {
        match self {
            Self::DuplicateEntity { entity }
            | Self::Degenerate { entity }
            | Self::InvalidCurve { entity, .. }
            | Self::SelfOverlap { entity } => vec![*entity],
            Self::Overlap { first, second } => vec![*first, *second],
            Self::TooIntricate { entities } | Self::Unresolved { entities } => entities.clone(),
            Self::NoClosedProfile
            | Self::EmptySelection
            | Self::MissingRegion(_)
            | Self::Cancelled(_) => Vec::new(),
        }
    }
}
