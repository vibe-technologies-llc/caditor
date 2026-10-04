use caditor_geometry::Point2;
use thiserror::Error;

use crate::{error::GeometryError, interrupt::Interrupted, profile::RegionKey};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Neighbour {
    pub entity: u64,
    pub gap: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenEnd {
    pub entity: u64,
    pub point: Point2,
    pub nearest: Option<Neighbour>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProfileError {
    #[error("the sketch has no closed profile")]
    NoClosedProfile { open_ends: Vec<OpenEnd> },
    #[error("no region of the sketch is chosen")]
    EmptySelection,
    #[error("a chosen region no longer exists in the sketch")]
    MissingRegion(RegionKey),
    #[error("a chosen region changed so that it matches {} regions of the sketch", candidates.len())]
    AmbiguousRegion {
        region: RegionKey,
        candidates: Vec<RegionKey>,
    },
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
            Self::NoClosedProfile { open_ends } => {
                let mut entities: Vec<u64> = open_ends
                    .iter()
                    .flat_map(|end| [Some(end.entity), end.nearest.map(|near| near.entity)])
                    .flatten()
                    .collect();
                entities.sort_unstable();
                entities.dedup();
                entities
            }
            Self::EmptySelection
            | Self::MissingRegion(_)
            | Self::AmbiguousRegion { .. }
            | Self::Cancelled(_) => Vec::new(),
        }
    }
}
