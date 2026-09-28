mod model;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
pub(crate) mod value;

use caditor_zstd::{Level, ZstdError};
use xxhash_rust::xxh3::Xxh3;

pub(crate) use self::model::{EncodeError, decode, encode, history, load_version, save_bytes};
pub use self::model::{History, SavedState, Version};

pub(crate) type Magic = [u8; 8];

pub(crate) const MODEL_MAGIC: Magic = [0x89, b'C', b'A', b'D', b'\r', b'\n', 0x1a, b'\n'];
pub(crate) const JOURNAL_MAGIC: Magic = [0x89, b'C', b'J', b'L', b'\r', b'\n', 0x1a, b'\n'];
const SYNC: [u8; 4] = *b"CDCK";
const VERSION_LENGTH: usize = 4;
const CHUNK_HEADER_LENGTH: usize = 24;
const CHECKED_HEADER: std::ops::Range<usize> = 4..16;
const MAX_CONTENT: usize = 1 << 28;
const HASHING_ALLOWANCE: usize = 4;
const LEVEL: Level = Level::BALANCED;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ChunkKind {
    Head = 1,
    Record = 2,
    VersionInfo = 3,
    VersionData = 4,
    JournalHeader = 5,
    Snapshot = 6,
    Apply = 7,
    Undo = 8,
    Redo = 9,
}

impl ChunkKind {
    const ALL: [Self; 9] = [
        Self::Head,
        Self::Record,
        Self::VersionInfo,
        Self::VersionData,
        Self::JournalHeader,
        Self::Snapshot,
        Self::Apply,
        Self::Undo,
        Self::Redo,
    ];

    fn from_byte(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| *kind as u8 == byte)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Codec {
    Stored = 0,
    Zstd = 1,
    ZstdAfterNewer = 2,
}

impl Codec {
    fn from_byte(byte: u8) -> Option<Self> {
        [Self::Stored, Self::Zstd, Self::ZstdAfterNewer]
            .into_iter()
            .find(|codec| *codec as u8 == byte)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chunk<'a> {
    pub kind: Option<ChunkKind>,
    pub codec: Option<Codec>,
    content_length: usize,
    payload: &'a [u8],
    pub whole: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum UnpackError {
    #[error("the chunk is compressed in a way this version cannot read")]
    UnknownCodec,
    #[error("the chunk needs the newer version it was stored against")]
    MissingNewer,
    #[error("the chunk could not be decompressed: {0}")]
    Zstd(#[from] ZstdError),
    #[error("the chunk holds {actual} bytes instead of {expected}")]
    WrongLength { expected: usize, actual: usize },
}

impl Chunk<'_> {
    pub fn content_length(&self) -> usize {
        self.content_length
    }

    pub fn unpack(&self, newer: Option<&[u8]>) -> Result<Vec<u8>, UnpackError> {
        let content = match self.codec {
            Some(Codec::Stored) => self.payload.to_vec(),
            Some(Codec::Zstd) => caditor_zstd::decompress(self.payload, self.content_length)?,
            Some(Codec::ZstdAfterNewer) => caditor_zstd::decompress_after(
                self.payload,
                newer.ok_or(UnpackError::MissingNewer)?,
                self.content_length,
            )?,
            None => return Err(UnpackError::UnknownCodec),
        };
        if content.len() != self.content_length {
            return Err(UnpackError::WrongLength {
                expected: self.content_length,
                actual: content.len(),
            });
        }
        Ok(content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Chunk(Chunk<'a>),
    Damaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Container<'a> {
    pub version: u32,
    pub pieces: Vec<Piece<'a>>,
}

impl<'a> Container<'a> {
    pub fn chunks(&self) -> impl Iterator<Item = Chunk<'a>> + '_ {
        self.pieces.iter().filter_map(|piece| match piece {
            Piece::Chunk(chunk) => Some(*chunk),
            Piece::Damaged => None,
        })
    }

    pub fn damaged(&self) -> usize {
        self.pieces
            .iter()
            .filter(|piece| **piece == Piece::Damaged)
            .count()
    }
}

pub(crate) fn has_magic(bytes: &[u8], magic: &Magic) -> bool {
    bytes.starts_with(magic)
}

pub(crate) fn parse<'a>(bytes: &'a [u8], magic: &Magic) -> Option<Container<'a>> {
    let rest = bytes.strip_prefix(magic)?;
    let version = u32::from_le_bytes(rest.get(..VERSION_LENGTH)?.try_into().ok()?);
    let body = rest.get(VERSION_LENGTH..)?;
    let mut pieces = Vec::new();
    let mut position = 0;
    let mut hashing_budget = body.len().saturating_mul(HASHING_ALLOWANCE);
    while position < body.len() {
        match read_chunk(body, position, &mut hashing_budget) {
            Ok((chunk, next)) => {
                pieces.push(Piece::Chunk(chunk));
                position = next;
            }
            Err(Rejected::Chunk) => {
                if pieces.last() != Some(&Piece::Damaged) {
                    pieces.push(Piece::Damaged);
                }
                position = next_sync(body, position + 1);
            }
            Err(Rejected::OverBudget) => {
                if pieces.last() != Some(&Piece::Damaged) {
                    pieces.push(Piece::Damaged);
                }
                break;
            }
        }
    }
    Some(Container { version, pieces })
}

fn next_sync(body: &[u8], from: usize) -> usize {
    body.get(from..)
        .and_then(|rest| rest.windows(SYNC.len()).position(|window| window == SYNC))
        .map_or(body.len(), |offset| from + offset)
}

enum Rejected {
    Chunk,
    OverBudget,
}

fn read_chunk<'a>(
    body: &'a [u8],
    position: usize,
    hashing_budget: &mut usize,
) -> Result<(Chunk<'a>, usize), Rejected> {
    let (chunk, end, header, checksum) = chunk_at(body, position).ok_or(Rejected::Chunk)?;
    *hashing_budget = hashing_budget
        .checked_sub(chunk.payload.len())
        .ok_or(Rejected::OverBudget)?;
    if checksum_of(header, chunk.payload) != checksum {
        return Err(Rejected::Chunk);
    }
    Ok((chunk, end))
}

fn chunk_at(body: &[u8], position: usize) -> Option<(Chunk<'_>, usize, &[u8], u64)> {
    let header = body.get(position..position.checked_add(CHUNK_HEADER_LENGTH)?)?;
    if header.get(..SYNC.len())? != SYNC {
        return None;
    }
    let kind = *header.get(4)?;
    let codec = *header.get(5)?;
    let stored_length = u32::from_le_bytes(header.get(8..12)?.try_into().ok()?) as usize;
    let content_length = u32::from_le_bytes(header.get(12..16)?.try_into().ok()?) as usize;
    let checksum = u64::from_le_bytes(header.get(16..24)?.try_into().ok()?);
    if content_length > MAX_CONTENT {
        return None;
    }
    let payload_start = position + CHUNK_HEADER_LENGTH;
    let end = payload_start.checked_add(stored_length)?;
    let payload = body.get(payload_start..end)?;
    Some((
        Chunk {
            kind: ChunkKind::from_byte(kind),
            codec: Codec::from_byte(codec),
            content_length,
            payload,
            whole: body.get(position..end)?,
        },
        end,
        header.get(CHECKED_HEADER)?,
        checksum,
    ))
}

fn checksum_of(header: &[u8], payload: &[u8]) -> u64 {
    let mut hasher = Xxh3::new();
    hasher.update(header);
    hasher.update(payload);
    hasher.digest()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PackError {
    #[error("a part of the model is too large to store")]
    TooLarge,
    #[error("a part of the model could not be compressed: {0}")]
    Zstd(#[from] ZstdError),
}

pub(crate) fn start_file(magic: &Magic, version: u32) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes
}

pub(crate) fn push_packed(
    bytes: &mut Vec<u8>,
    kind: ChunkKind,
    content: &[u8],
) -> Result<(), PackError> {
    let compressed = caditor_zstd::compress(content, LEVEL)?;
    if compressed.len() < content.len() {
        push_chunk(bytes, kind, Codec::Zstd, content.len(), &compressed)
    } else {
        push_chunk(bytes, kind, Codec::Stored, content.len(), content)
    }
}

pub(crate) fn push_packed_after(
    bytes: &mut Vec<u8>,
    kind: ChunkKind,
    content: &[u8],
    newer: &[u8],
) -> Result<(), PackError> {
    let compressed = caditor_zstd::compress_after(content, newer, LEVEL)?;
    push_chunk(
        bytes,
        kind,
        Codec::ZstdAfterNewer,
        content.len(),
        &compressed,
    )
}

fn push_chunk(
    bytes: &mut Vec<u8>,
    kind: ChunkKind,
    codec: Codec,
    content_length: usize,
    payload: &[u8],
) -> Result<(), PackError> {
    if content_length > MAX_CONTENT {
        return Err(PackError::TooLarge);
    }
    let stored = u32::try_from(payload.len()).map_err(|_| PackError::TooLarge)?;
    let content = u32::try_from(content_length).map_err(|_| PackError::TooLarge)?;
    let mut header = [0_u8; CHUNK_HEADER_LENGTH];
    let fields = [
        SYNC.as_slice(),
        &[kind as u8, codec as u8, 0, 0],
        &stored.to_le_bytes(),
        &content.to_le_bytes(),
    ]
    .concat();
    for (slot, byte) in header.iter_mut().zip(fields) {
        *slot = byte;
    }
    let checksum = checksum_of(header.get(CHECKED_HEADER).unwrap_or_default(), payload);
    for (slot, byte) in header.iter_mut().skip(16).zip(checksum.to_le_bytes()) {
        *slot = byte;
    }
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    Ok(())
}
