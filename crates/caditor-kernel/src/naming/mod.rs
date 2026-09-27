mod digest;
#[cfg(test)]
mod tests;

pub(crate) use self::digest::Digest;
use crate::profile::{PieceId, RegionKey};

const SIDE_FACE: u8 = 0x01;
const START_CAP: u8 = 0x02;
const END_CAP: u8 = 0x03;
const EDGE_BETWEEN: u8 = 0x10;
const EDGE_BETWEEN_AT: u8 = 0x11;
const SEAM_EDGE: u8 = 0x12;
const EDGE_OCCURRENCE: u8 = 0x13;
const VERTEX_OF_FACES: u8 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FaceName(u128);

impl FaceName {
    pub const NONE: Self = Self(0);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    pub fn from_digest(digest: u128) -> Self {
        Self(digest)
    }

    pub fn digest(self) -> u128 {
        self.0
    }

    pub fn side(feature: u64, piece: &PieceId) -> Self {
        let mut digest = Digest::new(SIDE_FACE);
        digest.u64(feature);
        piece.write(&mut digest);
        Self(digest.finish())
    }

    pub fn start_cap(feature: u64, region: RegionKey) -> Self {
        Self::cap(START_CAP, feature, region)
    }

    pub fn end_cap(feature: u64, region: RegionKey) -> Self {
        Self::cap(END_CAP, feature, region)
    }

    fn cap(tag: u8, feature: u64, region: RegionKey) -> Self {
        let mut digest = Digest::new(tag);
        digest.u64(feature);
        digest.u128(region.digest());
        Self(digest.finish())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VertexName(u128);

impl VertexName {
    pub fn from_digest(digest: u128) -> Self {
        Self(digest)
    }

    pub fn digest(self) -> u128 {
        self.0
    }

    pub fn of_faces(faces: impl IntoIterator<Item = FaceName>) -> Self {
        let mut faces: Vec<FaceName> = faces.into_iter().collect();
        faces.sort_unstable();
        faces.dedup();
        let mut digest = Digest::new(VERTEX_OF_FACES);
        digest.count(faces.len());
        for face in faces {
            digest.u128(face.0);
        }
        Self(digest.finish())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EdgeName(u128);

impl EdgeName {
    pub const NONE: Self = Self(0);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    pub fn from_digest(digest: u128) -> Self {
        Self(digest)
    }

    pub fn digest(self) -> u128 {
        self.0
    }

    pub fn between(first: FaceName, second: FaceName) -> Self {
        let (low, high) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        let mut digest = Digest::new(EDGE_BETWEEN);
        digest.u128(low.0);
        digest.u128(high.0);
        Self(digest.finish())
    }

    pub fn between_at(left: FaceName, right: FaceName, from: VertexName, to: VertexName) -> Self {
        let (left, right, from, to) = if from <= to {
            (left, right, from, to)
        } else {
            (right, left, to, from)
        };
        let mut digest = Digest::new(EDGE_BETWEEN_AT);
        digest.u128(left.0);
        digest.u128(right.0);
        digest.u128(from.0);
        digest.u128(to.0);
        Self(digest.finish())
    }

    pub fn seam(face: FaceName) -> Self {
        let mut digest = Digest::new(SEAM_EDGE);
        digest.u128(face.0);
        Self(digest.finish())
    }

    pub fn occurrence(base: Self, index: u32) -> Self {
        let mut digest = Digest::new(EDGE_OCCURRENCE);
        digest.u128(base.0);
        digest.u32(index);
        Self(digest.finish())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaceOrigin {
    Side { feature: u64, entity: u64 },
    StartCap { feature: u64 },
    EndCap { feature: u64 },
}

impl FaceOrigin {
    pub fn feature(&self) -> u64 {
        match self {
            Self::Side { feature, .. } | Self::StartCap { feature } | Self::EndCap { feature } => {
                *feature
            }
        }
    }

    pub fn entity(&self) -> Option<u64> {
        match self {
            Self::Side { entity, .. } => Some(*entity),
            Self::StartCap { .. } | Self::EndCap { .. } => None,
        }
    }
}
