use std::collections::BTreeSet;

use caditor_geometry::Point2;

use crate::profile::{Piece, ProfileError, Region, RegionKey, Side};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct RegionReference {
    key: RegionKey,
    boundary: BTreeSet<BoundaryPiece>,
    anchor: Option<Point2>,
}

impl RegionReference {
    pub fn heap_size(&self) -> usize {
        self.boundary.len() * size_of::<BoundaryPiece>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

const ANCHOR_TIE_TOLERANCE: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Likeness {
    holds_anchor: bool,
    shared_pieces: usize,
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
        let regions: Vec<&Region> = regions.into_iter().collect();

        if regions.iter().any(|region| region.key() == self.key) {
            return Ok(RegionMatch::Same(self.key));
        }

        let sides: BTreeSet<(u64, Side)> = self
            .boundary
            .iter()
            .map(|piece| piece.entity_side())
            .collect();

        let mut likenesses: Vec<(Likeness, &Region)> = Vec::new();

        for region in regions {
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
                holds_anchor: self.anchor.is_some_and(|anchor| region.contains(anchor)),
                shared_pieces: boundary.intersection(&self.boundary).count(),
                shared_sides,
            };

            likenesses.push((likeness, region));
        }

        let Some(best) = likenesses.iter().map(|(likeness, _)| *likeness).max() else {
            return Ok(RegionMatch::Gone);
        };
        let tied: Vec<&Region> = likenesses
            .into_iter()
            .filter(|(likeness, _)| *likeness == best)
            .map(|(_, region)| region)
            .collect();

        if let [only] = tied.as_slice() {
            return Ok(RegionMatch::Healed(only.key()));
        }
        if let Some(anchor) = self.anchor
            && let Some(nearest) = nearest_to_anchor(&tied, anchor)
        {
            return Ok(RegionMatch::Healed(nearest));
        }
        Err(ProfileError::AmbiguousRegion {
            region: self.key,
            candidates: tied.into_iter().map(Region::key).collect(),
        })
    }
}

fn nearest_to_anchor(candidates: &[&Region], anchor: Point2) -> Option<RegionKey> {
    let scored: Vec<(f64, f64, RegionKey)> = candidates
        .iter()
        .map(|region| {
            let bounds = region.bounds()?;
            let clamped = Point2::new(
                anchor.x.clamp(bounds.min().x, bounds.max().x),
                anchor.y.clamp(bounds.min().y, bounds.max().y),
            );
            Some((
                clamped.distance(anchor),
                bounds.center().distance(anchor),
                region.key(),
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    let min_box = scored
        .iter()
        .map(|(box_distance, _, _)| *box_distance)
        .fold(f64::INFINITY, f64::min);
    let box_slack = ANCHOR_TIE_TOLERANCE * (1.0 + min_box);

    let near_box: Vec<(f64, RegionKey)> = scored
        .into_iter()
        .filter(|(box_distance, _, _)| *box_distance <= min_box + box_slack)
        .map(|(_, centre_distance, key)| (centre_distance, key))
        .collect();

    if let [(_, only)] = near_box.as_slice() {
        return Some(*only);
    }

    let min_centre = near_box
        .iter()
        .map(|(centre_distance, _)| *centre_distance)
        .fold(f64::INFINITY, f64::min);
    let centre_slack = ANCHOR_TIE_TOLERANCE * (1.0 + min_centre);

    let best: Vec<RegionKey> = near_box
        .into_iter()
        .filter(|(centre_distance, _)| *centre_distance <= min_centre + centre_slack)
        .map(|(_, key)| key)
        .collect();
    match best.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
