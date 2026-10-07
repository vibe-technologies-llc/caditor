mod arrangement;
mod culprits;
mod error;
mod geometry;
mod intersect;
mod reference;
mod region;
mod source;
#[cfg(test)]
mod tests;
mod triangulate;

use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Point2};

use self::{
    arrangement::Arrangement,
    culprits::culprits,
    region::{base_keys, lumps, shared_keys, with_keys},
};
pub use self::{
    error::{Neighbour, OpenEnd, ProfileError},
    reference::{BoundaryPiece, RegionMatch, RegionReference, ResolvedRegions, resolve_regions},
    triangulate::RegionMesh,
};
use crate::{
    curve2::Curve2, interrupt, interval::Interval, naming::Digest, tolerance::SamplingTolerance,
};

const REGION_KEY: u8 = 0x30;
const REGION_TIEBREAK: u8 = 0x31;
const PIECE_ID: u8 = 0x40;
const BOUND_START: u8 = 0;
const BOUND_END: u8 = 1;
const BOUND_CUT: u8 = 2;
const BOUND_CROSSING: u8 = 3;
const SIDE_LEFT: u8 = 0;
const SIDE_RIGHT: u8 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileShape {
    Line {
        start: Point2,
        end: Point2,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc {
        center: Point2,
        start: Point2,
        end: Point2,
    },
    Spline {
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<Point2>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileCurve {
    pub entity: u64,
    pub shape: ProfileShape,
}

impl ProfileCurve {
    pub fn new(entity: u64, shape: ProfileShape) -> Self {
        Self { entity, shape }
    }

    pub fn line(entity: u64, start: Point2, end: Point2) -> Self {
        Self::new(entity, ProfileShape::Line { start, end })
    }

    pub fn circle(entity: u64, center: Point2, radius: f64) -> Self {
        Self::new(entity, ProfileShape::Circle { center, radius })
    }

    pub fn arc(entity: u64, center: Point2, start: Point2, end: Point2) -> Self {
        Self::new(entity, ProfileShape::Arc { center, start, end })
    }

    pub fn curve(&self) -> Result<(Curve2, Interval), ProfileError> {
        let source = source::Source::from_curve(self)?;
        Ok((source.curve, source.range))
    }

    pub fn spline(
        entity: u64,
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<Point2>,
    ) -> Self {
        Self::new(
            entity,
            ProfileShape::Spline {
                degree,
                knots,
                control_points,
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn byte(self) -> u8 {
        match self {
            Self::Left => SIDE_LEFT,
            Self::Right => SIDE_RIGHT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PieceBound {
    Start,
    End,
    Cut { entities: Vec<u64>, occurrence: u32 },
    Crossing { entities: Vec<u64>, occurrence: u32 },
}

impl PieceBound {
    fn write(&self, digest: &mut Digest) {
        let (tag, entities, occurrence) = match self {
            Self::Start => return digest.byte(BOUND_START),
            Self::End => return digest.byte(BOUND_END),
            Self::Cut {
                entities,
                occurrence,
            } => (BOUND_CUT, entities, occurrence),
            Self::Crossing {
                entities,
                occurrence,
            } => (BOUND_CROSSING, entities, occurrence),
        };
        digest.byte(tag);
        digest.count(entities.len());
        for entity in entities {
            digest.u64(*entity);
        }
        digest.u32(*occurrence);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PieceId {
    entity: u64,
    start: PieceBound,
    end: PieceBound,
}

impl PieceId {
    pub fn new(entity: u64, start: PieceBound, end: PieceBound) -> Self {
        Self { entity, start, end }
    }

    pub fn entity(&self) -> u64 {
        self.entity
    }

    pub fn start(&self) -> &PieceBound {
        &self.start
    }

    pub fn end(&self) -> &PieceBound {
        &self.end
    }

    pub(crate) fn write(&self, digest: &mut Digest) {
        digest.u64(self.entity);
        self.start.write(digest);
        self.end.write(digest);
    }

    fn digest(&self) -> u128 {
        let mut digest = Digest::new(PIECE_ID);
        self.write(&mut digest);
        digest.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionKey(u128);

impl RegionKey {
    pub fn from_digest(digest: u128) -> Self {
        Self(digest)
    }

    pub fn digest(self) -> u128 {
        self.0
    }

    fn of_sides<'a>(pieces: impl IntoIterator<Item = &'a Piece>) -> Self {
        let sides: BTreeSet<(u64, Side)> = pieces
            .into_iter()
            .map(|piece| (piece.id.entity, piece.side()))
            .collect();
        let mut digest = Digest::new(REGION_KEY);
        digest.count(sides.len());
        for (entity, side) in sides {
            digest.u64(entity);
            digest.byte(side.byte());
        }
        Self(digest.finish())
    }

    fn tiebroken<'a>(self, pieces: impl IntoIterator<Item = &'a Piece>) -> Self {
        let identities: BTreeSet<(u128, Side)> = pieces
            .into_iter()
            .map(|piece| (piece.id.digest(), piece.side()))
            .collect();
        let mut digest = Digest::new(REGION_TIEBREAK);
        digest.u128(self.0);
        digest.count(identities.len());
        for (identity, side) in identities {
            digest.u128(identity);
            digest.byte(side.byte());
        }
        Self(digest.finish())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    id: PieceId,
    curve: Curve2,
    range: Interval,
    reversed: bool,
}

impl Piece {
    pub fn id(&self) -> &PieceId {
        &self.id
    }

    pub fn entity(&self) -> u64 {
        self.id.entity
    }

    pub fn curve(&self) -> &Curve2 {
        &self.curve
    }

    pub fn range(&self) -> Interval {
        self.range
    }

    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    pub fn side(&self) -> Side {
        if self.reversed {
            Side::Right
        } else {
            Side::Left
        }
    }

    pub fn start_parameter(&self) -> f64 {
        if self.reversed {
            self.range.end()
        } else {
            self.range.start()
        }
    }

    pub fn end_parameter(&self) -> f64 {
        if self.reversed {
            self.range.start()
        } else {
            self.range.end()
        }
    }

    pub fn start(&self) -> Point2 {
        self.curve.point(self.start_parameter())
    }

    pub fn end(&self) -> Point2 {
        self.curve.point(self.end_parameter())
    }

    pub fn bounds(&self) -> Aabb2 {
        self.curve.bounding_box(self.range)
    }

    pub(crate) fn with_curve(&self, curve: Curve2, range: Interval) -> Self {
        Self {
            id: self.id.clone(),
            curve,
            range,
            reversed: self.reversed,
        }
    }

    fn signed_area(&self) -> f64 {
        let area = geometry::area_under(&self.curve, self.range);
        if self.reversed { -area } else { area }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileLoop {
    pieces: Vec<Piece>,
}

impl ProfileLoop {
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn signed_area(&self) -> f64 {
        self.pieces.iter().map(Piece::signed_area).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    key: RegionKey,
    depth: usize,
    outer: ProfileLoop,
    holes: Vec<ProfileLoop>,
}

impl Region {
    pub fn key(&self) -> RegionKey {
        self.key
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn outer(&self) -> &ProfileLoop {
        &self.outer
    }

    pub fn holes(&self) -> &[ProfileLoop] {
        &self.holes
    }

    pub fn loops(&self) -> impl Iterator<Item = &ProfileLoop> {
        std::iter::once(&self.outer).chain(&self.holes)
    }

    pub fn pieces(&self) -> impl Iterator<Item = &Piece> {
        self.loops()
            .flat_map(|profile_loop| profile_loop.pieces.iter())
    }

    pub fn entities(&self) -> Vec<u64> {
        let entities: BTreeSet<u64> = self.pieces().map(Piece::entity).collect();
        entities.into_iter().collect()
    }

    pub fn area(&self) -> f64 {
        self.loops().map(ProfileLoop::signed_area).sum()
    }

    pub fn bounds(&self) -> Option<Aabb2> {
        self.pieces().map(Piece::bounds).reduce(Aabb2::union)
    }

    pub fn polygons(&self, tolerance: &SamplingTolerance) -> Vec<Vec<Point2>> {
        self.loops()
            .map(|profile_loop| triangulate::loop_polygon(profile_loop, tolerance))
            .collect()
    }

    pub fn triangulate(&self, tolerance: &SamplingTolerance) -> Option<RegionMesh> {
        triangulate::triangulate(self, tolerance)
    }

    pub fn contains(&self, point: Point2) -> bool {
        let segments = self
            .pieces()
            .map(|piece| (&piece.curve, piece.range, piece.reversed));
        geometry::winding(segments, point) != 0
    }

    pub fn anchor(&self) -> Option<Point2> {
        let extent = self.bounds()?.size().length();
        self.triangulate(&SamplingTolerance::for_extent(extent))?
            .anchor()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    EvenDepth,
    Regions(Vec<RegionKey>),
}

#[derive(Debug, Clone)]
pub struct Profile {
    arrangement: Arrangement,
    regions: Vec<Region>,
    region_faces: Vec<BTreeSet<usize>>,
    ambiguous: BTreeSet<RegionKey>,
}

impl Profile {
    pub fn new(curves: &[ProfileCurve]) -> Result<Self, ProfileError> {
        let arrangement = match Arrangement::new(curves) {
            Err(ProfileError::Unresolved { entities }) if entities.is_empty() => {
                let entities = culprits(curves, |subset| {
                    matches!(
                        Arrangement::new(subset),
                        Err(ProfileError::Unresolved { .. })
                    )
                });
                interrupt::check()?;
                return Err(ProfileError::Unresolved { entities });
            }
            built => built?,
        };
        let mut drafts = Vec::with_capacity(arrangement.face_count());
        for face in 0..arrangement.face_count() {
            interrupt::check()?;
            drafts.extend(
                lumps(&arrangement, &BTreeSet::from([face]))
                    .into_iter()
                    .next(),
            );
        }
        let ambiguous = shared_keys(&base_keys(&drafts));
        let (regions, region_faces) = with_keys(drafts, &ambiguous).into_iter().unzip();
        Ok(Self {
            arrangement,
            regions,
            region_faces,
            ambiguous,
        })
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn region(&self, key: RegionKey) -> Option<&Region> {
        self.regions.iter().find(|region| region.key == key)
    }

    pub fn tolerance(&self) -> f64 {
        self.arrangement.tolerance()
    }

    pub fn even_depth(&self) -> Vec<RegionKey> {
        self.regions
            .iter()
            .filter(|region| region.depth % 2 == 0)
            .map(|region| region.key)
            .collect()
    }

    pub fn select(&self, selection: &Selection) -> Result<Vec<Region>, ProfileError> {
        if self.regions.is_empty() {
            return Err(ProfileError::NoClosedProfile {
                open_ends: self.arrangement.open_ends(),
            });
        }
        let keys = match selection {
            Selection::EvenDepth => self.even_depth(),
            Selection::Regions(keys) => keys.clone(),
        };
        if keys.is_empty() {
            return Err(ProfileError::EmptySelection);
        }
        let mut faces = BTreeSet::new();
        for key in keys {
            let region_faces = self
                .regions
                .iter()
                .position(|region| region.key == key)
                .and_then(|index| self.region_faces.get(index))
                .ok_or(ProfileError::MissingRegion(key))?;
            faces.extend(region_faces.iter().copied());
        }
        Ok(with_keys(lumps(&self.arrangement, &faces), &self.ambiguous)
            .into_iter()
            .map(|(region, _)| region)
            .collect())
    }
}
