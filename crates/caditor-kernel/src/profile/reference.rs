use std::collections::BTreeSet;

use caditor_geometry::Point2;

use crate::profile::{Piece, ProfileError, Region, RegionKey, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundaryPiece {
    pub entity: u64,
    pub side: Side,
    pub piece: u128,
}

impl BoundaryPiece {
    fn of(piece: &Piece) -> Self {
        Self {
            entity: piece.entity(),
            side: piece.side(),
            piece: piece.id().digest(),
        }
    }

    fn entity_side(self) -> (u64, Side) {
        (self.entity, self.side)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionReference {
    key: RegionKey,
    boundary: BTreeSet<BoundaryPiece>,
    anchor: Option<Point2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionMatch {
    Same(RegionKey),
    Healed(RegionKey),
    Gone,
}

impl RegionMatch {
    pub fn key(self) -> Option<RegionKey> {
        match self {
            Self::Same(key) | Self::Healed(key) => Some(key),
            Self::Gone => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Likeness {
    shared_pieces: usize,
    holds_anchor: bool,
    shared_sides: usize,
}

impl RegionReference {
    pub fn new(key: RegionKey, boundary: BTreeSet<BoundaryPiece>, anchor: Option<Point2>) -> Self {
        Self {
            key,
            boundary,
            anchor,
        }
    }

    pub fn of_key(key: RegionKey) -> Self {
        Self::new(key, BTreeSet::new(), None)
    }

    pub fn capture(region: &Region, anchor: Option<Point2>) -> Self {
        Self::new(
            region.key(),
            region.pieces().map(BoundaryPiece::of).collect(),
            anchor,
        )
    }

    pub fn key(&self) -> RegionKey {
        self.key
    }

    pub fn boundary(&self) -> &BTreeSet<BoundaryPiece> {
        &self.boundary
    }

    pub fn anchor(&self) -> Option<Point2> {
        self.anchor
    }

    pub fn resolve<'a>(
        &self,
        regions: impl IntoIterator<Item = &'a Region>,
    ) -> Result<RegionMatch, ProfileError> {
        let sides: BTreeSet<(u64, Side)> = self
            .boundary
            .iter()
            .map(|piece| piece.entity_side())
            .collect();

        let mut likenesses: Vec<(Likeness, RegionKey)> = Vec::new();
        for region in regions {
            if region.key() == self.key {
                return Ok(RegionMatch::Same(self.key));
            }
            let boundary: BTreeSet<BoundaryPiece> =
                region.pieces().map(BoundaryPiece::of).collect();
            let shared_sides = boundary
                .iter()
                .map(|piece| piece.entity_side())
                .collect::<BTreeSet<_>>()
                .intersection(&sides)
                .count();
            if shared_sides == 0 {
                continue;
            }
            let likeness = Likeness {
                shared_pieces: boundary.intersection(&self.boundary).count(),
                holds_anchor: self.anchor.is_some_and(|anchor| region.contains(anchor)),
                shared_sides,
            };
            likenesses.push((likeness, region.key()));
        }

        let Some(best) = likenesses.iter().map(|(likeness, _)| *likeness).max() else {
            return Ok(RegionMatch::Gone);
        };
        let candidates: Vec<RegionKey> = likenesses
            .into_iter()
            .filter(|(likeness, _)| *likeness == best)
            .map(|(_, key)| key)
            .collect();
        match candidates.as_slice() {
            [only] => Ok(RegionMatch::Healed(*only)),
            _ => Err(ProfileError::AmbiguousRegion {
                region: self.key,
                candidates,
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedRegions {
    pub keys: Vec<RegionKey>,
    pub healed: Vec<(RegionKey, RegionKey)>,
    pub gone: Vec<RegionKey>,
}

pub fn resolve_regions<'a>(
    references: &[RegionReference],
    regions: impl IntoIterator<Item = &'a Region> + Clone,
) -> Result<ResolvedRegions, ProfileError> {
    let mut resolved = ResolvedRegions::default();
    for reference in references {
        let found = match reference.resolve(regions.clone())? {
            RegionMatch::Same(key) => key,
            RegionMatch::Healed(key) => {
                resolved.healed.push((reference.key(), key));
                key
            }
            RegionMatch::Gone => {
                resolved.gone.push(reference.key());
                continue;
            }
        };
        if !resolved.keys.contains(&found) {
            resolved.keys.push(found);
        }
    }
    match (resolved.keys.is_empty(), references.first()) {
        (true, Some(first)) => Err(ProfileError::MissingRegion(first.key())),
        _ => Ok(resolved),
    }
}
