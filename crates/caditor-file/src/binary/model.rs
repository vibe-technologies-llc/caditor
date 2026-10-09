use std::{
    borrow::Cow,
    ops::Range,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use caditor_document::{CancelToken, Document};
use serde::{Deserialize, Serialize};

use super::{
    CHUNK_HEADER_LENGTH, Chunk, ChunkKind, Codec, MODEL_MAGIC, PackError, Piece, UnpackError,
    damaged, parse, push_packed, push_packed_after, push_padding,
    retention::{self, retained},
    salvage, start_file,
    value::{self, ValueError, push_varint, read_varint},
};
use crate::{
    configurations::configurations_record,
    format::{
        FORMAT_VERSION, Lenient, Record, Unreadable, feature_records, named_values_record,
        next_ids_record, parameter_record, principal_record, properties_record, rollback_record,
        suppressed_record, views_record,
    },
    load::{
        LoadError, Loaded, Parts, assemble, describe_unpack_failure, describe_unreadable_record,
        newer_version,
    },
    selection_sets::selection_sets_record,
    untrusted::UntrustedMap,
};

const KEYFRAME_SPACING: usize = 8;
pub const MAX_DECOMPRESSED: usize = 1 << 31;
const LARGEST_FILE: usize = 2 << 30;
const SHARED_BLOCK: usize = 4096;
const WORTH_SHARING: usize = 256 << 10;

fn largest_file() -> usize {
    #[cfg(test)]
    if let Some(largest) = super::testing::largest_file() {
        return largest;
    }
    LARGEST_FILE
}

fn room_for_history() -> usize {
    largest_file() / 4 * 3
}

fn max_decompressed() -> usize {
    #[cfg(test)]
    if let Some(limit) = super::testing::max_decompressed() {
        return limit;
    }
    MAX_DECOMPRESSED
}

struct Budget {
    remaining: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            remaining: max_decompressed(),
        }
    }
}

impl Budget {
    fn unpack(&mut self, chunk: &Chunk<'_>, newer: Option<&[u8]>) -> Unpacked {
        self.remaining = self
            .remaining
            .checked_sub(chunk.content_length())
            .ok_or(UnpackError::OverBudget)?;
        chunk.unpack(newer)
    }
}

fn unpack_beside(chunk: &Chunk<'_>, newer: Option<&[u8]>, held: usize) -> Unpacked {
    held.checked_add(chunk.content_length())
        .filter(|live| *live <= max_decompressed())
        .ok_or(UnpackError::OverBudget)?;
    chunk.unpack(newer)
}

type Unpacked = Result<Vec<u8>, UnpackError>;

struct RecordDigest {
    hasher: blake3::Hasher,
    whole: bool,
}

impl Default for RecordDigest {
    fn default() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
            whole: true,
        }
    }
}

impl RecordDigest {
    fn add(&mut self, content: &Unpacked) {
        let Ok(content) = content else {
            self.whole = false;
            return;
        };
        let mut length = Vec::new();
        push_varint(&mut length, content.len() as u64);
        self.hasher.update(&length);
        self.hasher.update(content);
    }

    fn matches(&self, head: Option<&StateRecord>) -> bool {
        self.whole
            && head.is_some_and(|head| head.digest == self.hasher.finalize().to_hex().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileDigest(pub(crate) String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedState {
    pub saved_at: SystemTime,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub index: usize,
    pub state: SavedState,
    pub available: bool,
    pub kept: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    pub current: Option<SavedState>,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StateRecord {
    saved_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    digest: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    kept: bool,
}

fn seconds_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl StateRecord {
    fn new(saved_at: SystemTime, label: Option<&str>, snapshot: &[u8]) -> Self {
        Self {
            saved_at: seconds_since_epoch(saved_at),
            label: label.map(str::to_owned),
            digest: digest(snapshot),
            kept: false,
        }
    }

    fn state(&self) -> SavedState {
        SavedState {
            saved_at: UNIX_EPOCH
                .checked_add(Duration::from_secs(self.saved_at))
                .unwrap_or(UNIX_EPOCH),
            label: self.label.clone(),
        }
    }

    fn holds(&self, snapshot: &[u8]) -> bool {
        self.digest == digest(snapshot)
    }
}

fn digest(snapshot: &[u8]) -> String {
    blake3::hash(snapshot).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq)]
struct StoredInfo<'a> {
    record: StateRecord,
    chunk: Chunk<'a>,
}

#[derive(Debug, Clone, PartialEq)]
struct StoredVersion<'a> {
    info: Option<StoredInfo<'a>>,
    data: Chunk<'a>,
    after_damage: bool,
}

impl StoredVersion<'_> {
    fn is_keyframe(&self) -> bool {
        self.data.codec != Some(Codec::ZstdAfterNewer)
    }

    fn holds(&self, snapshot: &[u8]) -> bool {
        self.info
            .as_ref()
            .is_some_and(|info| info.record.holds(snapshot))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Parsed<'a> {
    version: Option<u32>,
    damaged: usize,
    head: Option<StateRecord>,
    head_chunk: Option<Chunk<'a>>,
    records: Vec<Chunk<'a>>,
    versions: Vec<StoredVersion<'a>>,
    foreign: Vec<Chunk<'a>>,
}

impl<'a> Parsed<'a> {
    fn of(bytes: &'a [u8]) -> Option<Self> {
        let (version, pieces) = match parse(bytes, &MODEL_MAGIC) {
            Some(container) => (Some(container.version), container.pieces),
            None => (None, salvage(bytes, ChunkKind::Head)?),
        };
        let mut parsed = Self {
            version,
            damaged: damaged(&pieces),
            head: None,
            head_chunk: None,
            records: Vec::new(),
            versions: Vec::new(),
            foreign: Vec::new(),
        };
        let mut pending_info = None;
        let mut damage_since_data = false;
        for piece in &pieces {
            let Piece::Chunk(chunk) = *piece else {
                pending_info = None;
                damage_since_data = true;
                continue;
            };
            let info = pending_info.take();
            match chunk.kind {
                Some(ChunkKind::Head) if parsed.head.is_none() => {
                    parsed.head = state_record(&chunk);
                    parsed.head_chunk = parsed.head.is_some().then_some(chunk);
                }
                Some(ChunkKind::Record) => parsed.records.push(chunk),
                Some(ChunkKind::VersionInfo) => {
                    pending_info = state_record(&chunk).map(|record| StoredInfo { record, chunk });
                }
                Some(ChunkKind::VersionData) => {
                    parsed.versions.push(StoredVersion {
                        info,
                        data: chunk,
                        after_damage: std::mem::take(&mut damage_since_data),
                    });
                }
                Some(ChunkKind::Padding) => pending_info = info,
                Some(
                    ChunkKind::Head
                    | ChunkKind::JournalHeader
                    | ChunkKind::Snapshot
                    | ChunkKind::Apply
                    | ChunkKind::Undo
                    | ChunkKind::Redo
                    | ChunkKind::UndoLast
                    | ChunkKind::RedoNext
                    | ChunkKind::RebasedSnapshot,
                ) => {}
                None => {
                    parsed.foreign.push(chunk);
                    pending_info = info;
                }
            }
        }
        Some(parsed)
    }

    fn records_digest(&self) -> RecordDigest {
        let mut budget = Budget::default();
        let mut digest = RecordDigest::default();
        for chunk in &self.records {
            digest.add(&budget.unpack(chunk, None));
        }
        digest
    }

    fn prior_records(&self) -> PriorRecords<'a> {
        let mut budget = Budget::default();
        let mut prior = PriorRecords::default();
        for chunk in &self.records {
            let content = budget.unpack(chunk, None);
            prior.digest.add(&content);
            let Ok(content) = content else {
                continue;
            };
            push_varint(&mut prior.snapshot, content.len() as u64);
            let start = prior.snapshot.len();
            prior.snapshot.extend_from_slice(&content);
            if let Some(understood) = understood_digest(&content) {
                prior.kept.insert(
                    understood,
                    KeptRecord {
                        content: start..prior.snapshot.len(),
                        stored: chunk.whole,
                    },
                );
            }
        }
        prior
    }

    fn head_snapshot(&self, budget: &mut Budget) -> Option<Vec<u8>> {
        let head = self.head.as_ref()?;
        let mut snapshot = Vec::new();
        for chunk in &self.records {
            let content = budget.unpack(chunk, None).ok()?;
            push_varint(&mut snapshot, content.len() as u64);
            snapshot.extend_from_slice(&content);
        }
        head.holds(&snapshot).then_some(snapshot)
    }

    fn is_damaged(&self, digest: &RecordDigest) -> bool {
        self.version.is_none() || self.damaged > 0 || !digest.matches(self.head.as_ref())
    }

    fn rebuild(&self, index: usize) -> Option<Vec<u8>> {
        let from = self.keyframe_at_or_before(index);
        let mut newer = if from == 0 {
            self.head_snapshot(&mut Budget::default())
        } else {
            None
        };
        for version in self.versions.get(from..=index)? {
            let held = newer.as_ref().map_or(0, Vec::len);
            newer = unpack_beside(&version.data, newer.as_deref(), held).ok();
        }
        newer
    }

    fn verified(&self, index: usize) -> Option<Vec<u8>> {
        let version = self.versions.get(index)?;
        self.rebuild(index)
            .filter(|snapshot| version.holds(snapshot))
    }

    fn rebuildable(&self) -> Vec<bool> {
        let mut newer_rebuilds = self
            .versions
            .first()
            .is_some_and(|first| !first.is_keyframe())
            && self.records_digest().matches(self.head.as_ref());
        let mut newer_in_doubt = false;
        let mut rebuildable = Vec::with_capacity(self.versions.len());
        for (index, version) in self.versions.iter().enumerate() {
            let in_doubt = !version.is_keyframe() && (version.after_damage || newer_in_doubt);
            let rebuilds = version.data.codec.is_some()
                && (version.is_keyframe()
                    || newer_rebuilds && (!in_doubt || self.rebuilds_despite_damage(index)));
            rebuildable.push(rebuilds);
            newer_rebuilds = rebuilds;
            newer_in_doubt = rebuilds && in_doubt && version.info.is_none();
        }
        rebuildable
    }

    fn rebuilds_despite_damage(&self, index: usize) -> bool {
        match self.versions.get(index).map(|version| &version.info) {
            Some(Some(_)) => self.verified(index).is_some(),
            Some(None) => self.rebuild(index).is_some(),
            None => false,
        }
    }

    fn keyframe_at_or_before(&self, index: usize) -> usize {
        self.versions
            .iter()
            .take(index.saturating_add(1))
            .rposition(StoredVersion::is_keyframe)
            .unwrap_or(0)
    }

    fn leading_deltas(&self) -> usize {
        self.versions
            .iter()
            .take_while(|version| !version.is_keyframe())
            .count()
    }
}

fn understood_digest(content: &[u8]) -> Option<[u8; blake3::OUT_LEN]> {
    let Ok(Lenient::Read(record)) = value::from_bytes::<Lenient<Record>>(content) else {
        return None;
    };
    let understood = value::to_bytes(&record).ok()?;
    Some(*blake3::hash(&understood).as_bytes())
}

#[derive(Default)]
struct PriorRecords<'a> {
    snapshot: Vec<u8>,
    digest: RecordDigest,
    kept: UntrustedMap<[u8; blake3::OUT_LEN], KeptRecord<'a>>,
}

impl PriorRecords<'_> {
    fn head<'p>(&'p self, parsed: &'p Parsed<'_>) -> Option<(&'p StateRecord, &'p [u8])> {
        let head = parsed.head.as_ref()?;
        self.digest
            .matches(Some(head))
            .then_some((head, self.snapshot.as_slice()))
    }
}

struct NewRecords<'a> {
    snapshot: Vec<u8>,
    records: Vec<RecordToWrite<'a>>,
}

impl<'a> NewRecords<'a> {
    fn of(document: &Document, prior: Option<&PriorRecords<'a>>) -> Result<Self, ValueError> {
        let mut new = Self {
            snapshot: Vec::new(),
            records: Vec::new(),
        };
        for record in document_records(document) {
            let record = record?;
            let kept = prior.and_then(|prior| {
                let kept = prior.kept.get(blake3::hash(&record).as_bytes())?;
                Some((kept, prior.snapshot.get(kept.content.clone())?))
            });
            match kept {
                Some((kept, content)) => {
                    push_varint(&mut new.snapshot, content.len() as u64);
                    new.snapshot.extend_from_slice(content);
                    new.records.push(RecordToWrite::Kept(kept.stored));
                }
                None => {
                    push_varint(&mut new.snapshot, record.len() as u64);
                    let start = new.snapshot.len();
                    new.snapshot.extend_from_slice(&record);
                    new.records
                        .push(RecordToWrite::Fresh(start..new.snapshot.len()));
                }
            }
        }
        Ok(new)
    }

    fn push_chunks(&self, bytes: &mut Vec<u8>) -> Result<(), PackError> {
        for record in &self.records {
            match record {
                RecordToWrite::Kept(stored) => bytes.extend_from_slice(stored),
                RecordToWrite::Fresh(content) => push_packed(
                    bytes,
                    ChunkKind::Record,
                    self.snapshot.get(content.clone()).unwrap_or_default(),
                )?,
            }
        }
        Ok(())
    }
}

fn state_record(chunk: &Chunk<'_>) -> Option<StateRecord> {
    let content = chunk.unpack(None).ok()?;
    value::from_bytes(&content).ok()
}

fn document_records(document: &Document) -> impl Iterator<Item = Result<Vec<u8>, ValueError>> + '_ {
    let parameters = document
        .parameters()
        .iter()
        .map(|parameter| Record::Parameter(parameter_record(parameter)));
    let features = feature_records(document).map(|feature| Record::Feature(Box::new(feature)));
    parameters
        .chain(features)
        .chain(principal_record(document).map(Record::Principal))
        .chain(suppressed_record(document).map(Record::Suppressed))
        .chain(rollback_record(document).map(Record::Rollback))
        .chain(properties_record(document).map(Record::Properties))
        .chain(views_record(document).map(Record::Views))
        .chain(named_values_record(document).map(Record::NamedValues))
        .chain(selection_sets_record(document).map(Record::SelectionSets))
        .chain(configurations_record(document).map(Record::Configurations))
        .chain(std::iter::once(Record::NextIds(next_ids_record(document))))
        .map(|record| value::to_bytes(&record))
}

#[cfg(test)]
fn snapshot_of<'r>(records: impl IntoIterator<Item = &'r [u8]>) -> Vec<u8> {
    let mut snapshot = Vec::new();
    for record in records {
        push_varint(&mut snapshot, record.len() as u64);
        snapshot.extend_from_slice(record);
    }
    snapshot
}

fn split_snapshot(snapshot: &[u8]) -> Option<Vec<&[u8]>> {
    let mut records = Vec::new();
    let mut position = 0;
    while position < snapshot.len() {
        let length = usize::try_from(read_varint(snapshot, &mut position)?).ok()?;
        let end = position.checked_add(length)?;
        records.push(snapshot.get(position..end)?);
        position = end;
    }
    Some(records)
}

pub(crate) fn encode(document: &Document) -> Result<Vec<u8>, EncodeError> {
    Ok(encode_over(document, None, SystemTime::now(), None)?.bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub shared: Vec<Shared>,
    pub digest: FileDigest,
    pub previous_damaged: bool,
    pub dropped_for_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shared {
    pub at: usize,
    pub from: usize,
    pub length: usize,
}

impl Shared {
    fn within(at: usize, from: usize, length: usize, previous_length: usize) -> Option<Self> {
        let start = from.checked_next_multiple_of(SHARED_BLOCK)?;
        let end = from.checked_add(length)?;
        let end = if end == previous_length {
            end
        } else {
            end - end % SHARED_BLOCK
        };
        let skipped = start - from;
        (end > start).then(|| Self {
            at: at + skipped,
            from: start,
            length: end - start,
        })
    }
}

enum Part<'a> {
    Fresh(Vec<u8>),
    Kept { from: usize, bytes: &'a [u8] },
}

impl Part<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Fresh(bytes) => bytes.len(),
            Self::Kept { bytes, .. } => bytes.len(),
        }
    }

    fn shareable_from(&self, worth: usize) -> Option<usize> {
        match self {
            Self::Kept { from, bytes } if bytes.len() >= worth => Some(*from),
            Self::Fresh(_) | Self::Kept { .. } => None,
        }
    }
}

fn worth_sharing() -> usize {
    #[cfg(test)]
    if let Some(length) = super::testing::worth_sharing() {
        return length;
    }
    WORTH_SHARING
}

struct Section<'a> {
    previous: &'a [u8],
    parts: Vec<Part<'a>>,
    fresh: Vec<u8>,
}

impl<'a> Section<'a> {
    fn new(previous: &'a [u8]) -> Self {
        Self {
            previous,
            parts: Vec::new(),
            fresh: Vec::new(),
        }
    }

    fn fresh(&mut self) -> &mut Vec<u8> {
        &mut self.fresh
    }

    fn len(&self) -> usize {
        self.parts.iter().map(Part::len).sum::<usize>() + self.fresh.len()
    }

    fn keep(&mut self, stored: &'a [u8]) {
        let Some(from) = offset_within(self.previous, stored) else {
            self.fresh.extend_from_slice(stored);
            return;
        };
        self.end_fresh();
        if let Some(Part::Kept {
            from: start,
            bytes: kept,
        }) = self.parts.last_mut()
            && *start + kept.len() == from
            && let Some(joined) = self.previous.get(*start..from + stored.len())
        {
            *kept = joined;
            return;
        }
        self.parts.push(Part::Kept {
            from,
            bytes: stored,
        });
    }

    fn end_fresh(&mut self) {
        if !self.fresh.is_empty() {
            self.parts
                .push(Part::Fresh(std::mem::take(&mut self.fresh)));
        }
    }

    fn place(mut self, bytes: &mut Vec<u8>, shared: &mut Vec<Shared>) -> Result<(), PackError> {
        self.end_fresh();
        let worth = worth_sharing();
        let first = self
            .parts
            .iter()
            .enumerate()
            .find_map(|(index, part)| Some((index, part.shareable_from(worth)?)));
        if let Some((first, from)) = first {
            let before: usize = self.parts.iter().take(first).map(Part::len).sum();
            pad_to(bytes, from.wrapping_sub(before))?;
        }
        for part in self.parts {
            if let Some(from) = part.shareable_from(worth) {
                pad_to(bytes, from)?;
                shared.extend(Shared::within(
                    bytes.len(),
                    from,
                    part.len(),
                    self.previous.len(),
                ));
            }
            match part {
                Part::Fresh(fresh) => bytes.extend_from_slice(&fresh),
                Part::Kept { bytes: kept, .. } => bytes.extend_from_slice(kept),
            }
        }
        Ok(())
    }
}

fn pad_to(bytes: &mut Vec<u8>, congruent_to: usize) -> Result<(), PackError> {
    let behind = congruent_to.wrapping_sub(bytes.len()) % SHARED_BLOCK;
    let padding = match behind {
        0 => return Ok(()),
        short if short < CHUNK_HEADER_LENGTH => short + SHARED_BLOCK,
        enough => enough,
    };
    push_padding(bytes, padding)
}

fn offset_within(whole: &[u8], part: &[u8]) -> Option<usize> {
    let offset = part.as_ptr().addr().checked_sub(whole.as_ptr().addr())?;
    (offset.checked_add(part.len())? <= whole.len()).then_some(offset)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum EncodeError {
    #[error("the model could not be converted for saving: {0}")]
    Value(#[from] ValueError),
    #[error("{0}")]
    Pack(#[from] PackError),
    #[error("an earlier version could not be rebuilt in the memory available to thin the history")]
    HistoryTooLarge,
    #[error("the model takes {size} bytes, more than the {largest} a model file can hold")]
    ModelTooLarge { size: usize, largest: usize },
}

#[derive(Debug, Clone)]
struct KeptRecord<'a> {
    content: Range<usize>,
    stored: &'a [u8],
}

enum RecordToWrite<'a> {
    Kept(&'a [u8]),
    Fresh(Range<usize>),
}

#[cfg(test)]
pub(crate) fn save_bytes(
    document: &Document,
    previous: Option<&[u8]>,
    now: SystemTime,
    label: Option<&str>,
) -> Result<Vec<u8>, EncodeError> {
    Ok(encode_over(document, previous, now, label)?.bytes)
}

pub(crate) fn encode_over(
    document: &Document,
    previous: Option<&[u8]>,
    now: SystemTime,
    label: Option<&str>,
) -> Result<Encoded, EncodeError> {
    let prior = previous.and_then(Parsed::of);
    let prior_records = prior.as_ref().map(Parsed::prior_records);
    let read_prior = prior.as_ref().zip(prior_records.as_ref());
    let previous_damaged = match (read_prior, previous) {
        (Some((prior, records)), _) => prior.is_damaged(&records.digest),
        (None, Some(bytes)) => !bytes.iter().all(u8::is_ascii_whitespace),
        (None, None) => false,
    };
    let prior_head = read_prior.and_then(|(prior, records)| records.head(prior));
    let new = NewRecords::of(document, prior_records.as_ref())?;
    if new.snapshot.len() > max_decompressed() {
        return Err(EncodeError::ModelTooLarge {
            size: new.snapshot.len(),
            largest: max_decompressed(),
        });
    }
    let snapshot = new.snapshot.as_slice();
    let unchanged = prior_head.is_some_and(|(info, _)| info.holds(snapshot));
    let head = match prior_head {
        Some((info, _)) if unchanged => info.clone(),
        _ => StateRecord::new(now, label, snapshot),
    };

    let mut bytes = start_file(&MODEL_MAGIC, FORMAT_VERSION);
    push_packed(&mut bytes, ChunkKind::Head, &value::to_bytes(&head)?)?;
    new.push_chunks(&mut bytes)?;
    for foreign in prior.iter().flat_map(|prior| &prior.foreign) {
        if !foreign.must_understand() {
            bytes.extend_from_slice(foreign.whole);
        }
    }
    if bytes.len() > largest_file() {
        return Err(EncodeError::ModelTooLarge {
            size: bytes.len(),
            largest: largest_file(),
        });
    }
    let room = room_for_history().saturating_sub(bytes.len());
    let mut cut = 0;
    let (versions, written) = loop {
        let mut versions = Section::new(previous.unwrap_or_default());
        let written = write_versions(
            &mut versions,
            prior.as_ref(),
            prior_head,
            !unchanged,
            snapshot,
            now,
            cut,
        )?;
        let excess = versions.len().saturating_sub(room);
        if excess == 0 || written.kept.is_empty() {
            break (versions, written);
        }
        cut += oldest_holding(&written.kept, excess).max(1);
    };
    let mut shared = Vec::new();
    versions.place(&mut bytes, &mut shared)?;
    if bytes.len() > largest_file() {
        return Err(EncodeError::ModelTooLarge {
            size: bytes.len(),
            largest: largest_file(),
        });
    }
    Ok(Encoded {
        bytes,
        shared,
        digest: FileDigest(head.digest),
        previous_damaged,
        dropped_for_size: written.cut_listed,
    })
}

fn oldest_holding(kept: &[usize], excess: usize) -> usize {
    let mut freed = 0;
    let mut count = 0;
    for size in kept.iter().rev() {
        if freed >= excess {
            break;
        }
        freed += size;
        count += 1;
    }
    count
}

struct Written {
    kept: Vec<usize>,
    cut_listed: usize,
}

pub(crate) fn head_digest(bytes: &[u8]) -> Option<FileDigest> {
    Parsed::of(bytes)?.head.map(|head| FileDigest(head.digest))
}

pub(crate) fn reads_back(bytes: &[u8], digest: &FileDigest) -> bool {
    let Some(parsed) = Parsed::of(bytes) else {
        return false;
    };
    parsed.damaged == 0
        && parsed
            .head
            .as_ref()
            .is_some_and(|head| head.digest == digest.0)
        && parsed.records_digest().matches(parsed.head.as_ref())
}

enum Candidate<'p, 'a> {
    Replaced {
        info: &'p StateRecord,
        snapshot: &'p [u8],
        whole: bool,
    },
    Stored {
        index: usize,
        version: &'p StoredVersion<'a>,
    },
}

impl Candidate<'_, '_> {
    fn saved_at(&self) -> Option<u64> {
        match self {
            Self::Replaced { info, .. } => Some(info.saved_at),
            Self::Stored { version, .. } => version.info.as_ref().map(|info| info.record.saved_at),
        }
    }

    fn is_kept(&self) -> bool {
        match self {
            Self::Replaced { info, .. } => info.kept,
            Self::Stored { version, .. } => {
                version.info.as_ref().is_some_and(|info| info.record.kept)
            }
        }
    }

    fn for_retention(&self) -> retention::Candidate {
        retention::Candidate {
            saved_at: self.saved_at(),
            kept: self.is_kept(),
        }
    }

    fn is_keyframe(&self) -> bool {
        match self {
            Self::Replaced { whole, .. } => *whole,
            Self::Stored { version, .. } => version.is_keyframe(),
        }
    }
}

fn thinned<'p, 'a>(
    prior: Option<&'p Parsed<'a>>,
    prior_head: Option<(&'p StateRecord, &'p [u8])>,
    adds_version: bool,
    now: SystemTime,
    cut: usize,
) -> (Vec<(Candidate<'p, 'a>, bool)>, usize) {
    let stored = prior.map_or(&[][..], |prior| prior.versions.as_slice());
    let leading_deltas = prior.map_or(0, Parsed::leading_deltas);
    let unreachable = if prior_head.is_some() {
        0
    } else {
        leading_deltas
    };
    let replaced = prior_head
        .filter(|_| adds_version)
        .map(|(info, old)| Candidate::Replaced {
            info,
            snapshot: old,
            whole: leading_deltas + 1 >= KEYFRAME_SPACING,
        });
    let candidates: Vec<Candidate<'p, 'a>> = replaced
        .into_iter()
        .chain(
            stored
                .iter()
                .enumerate()
                .skip(unreachable)
                .map(|(index, version)| Candidate::Stored { index, version }),
        )
        .collect();
    let mut keep = if adds_version {
        let for_retention: Vec<retention::Candidate> =
            candidates.iter().map(Candidate::for_retention).collect();
        retained(&for_retention, seconds_since_epoch(now))
    } else {
        vec![true; candidates.len()]
    };
    let mut cut_listed = 0;
    let oldest_kept = keep
        .iter_mut()
        .zip(&candidates)
        .rev()
        .filter(|(keep, _)| **keep)
        .take(cut);
    for (keep, candidate) in oldest_kept {
        *keep = false;
        cut_listed += usize::from(candidate.saved_at().is_some());
    }
    (candidates.into_iter().zip(keep).collect(), cut_listed)
}

fn needing_content(
    stored: &[StoredVersion<'_>],
    candidates: &[(Candidate<'_, '_>, bool)],
) -> Vec<bool> {
    let mut needed = vec![false; stored.len()];
    let mut after_thinning = false;
    for (candidate, keep) in candidates {
        if !keep {
            after_thinning = true;
            continue;
        }
        if let Candidate::Stored { index, version } = candidate
            && after_thinning
            && !version.is_keyframe()
        {
            let start = stored
                .iter()
                .take(index.saturating_add(1))
                .rposition(StoredVersion::is_keyframe)
                .unwrap_or(0);
            for slot in needed.iter_mut().take(index.saturating_add(1)).skip(start) {
                *slot = true;
            }
        }
        after_thinning = false;
    }
    needed
}

#[derive(Debug, Clone)]
enum Content<'p> {
    Borrowed(&'p [u8]),
    Decoded(Rc<Vec<u8>>),
}

impl Content<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Decoded(bytes) => bytes,
        }
    }

    fn decoded_length(&self) -> usize {
        match self {
            Self::Borrowed(_) => 0,
            Self::Decoded(bytes) => bytes.len(),
        }
    }
}

struct Rebuild<'p> {
    needed: Vec<bool>,
    newer: Option<Content<'p>>,
    held: usize,
    starved: bool,
}

impl<'p> Rebuild<'p> {
    fn new(
        stored: &[StoredVersion<'_>],
        candidates: &[(Candidate<'_, '_>, bool)],
        prior_head: Option<(&'p StateRecord, &'p [u8])>,
        snapshot: &[u8],
    ) -> Self {
        Self {
            needed: needing_content(stored, candidates),
            newer: prior_head.map(|(_, old)| Content::Borrowed(old)),
            held: snapshot
                .len()
                .saturating_add(prior_head.map_or(0, |(_, old)| old.len())),
            starved: false,
        }
    }

    fn advance(&mut self, candidate: &Candidate<'p, '_>) -> Option<&Content<'p>> {
        let content = match candidate {
            Candidate::Replaced { snapshot, .. } => {
                self.starved = false;
                Some(Content::Borrowed(snapshot))
            }
            Candidate::Stored { index, version } if self.needed.get(*index) == Some(&true) => {
                let newer = self.newer.as_ref();
                let held = self
                    .held
                    .saturating_add(newer.map_or(0, Content::decoded_length));
                match unpack_beside(&version.data, newer.map(Content::bytes), held) {
                    Ok(content) => {
                        self.starved = false;
                        Some(Content::Decoded(Rc::new(content)))
                    }
                    Err(UnpackError::MissingNewer) => None,
                    Err(UnpackError::OverBudget | UnpackError::OutOfMemory) => {
                        self.starved = true;
                        None
                    }
                    Err(_) => {
                        self.starved = false;
                        None
                    }
                }
            }
            Candidate::Stored { .. } => {
                self.starved = false;
                None
            }
        };
        self.newer = content;
        self.newer.as_ref()
    }
}

fn is_rewritten(candidate: &Candidate<'_, '_>, after_thinning: bool) -> bool {
    after_thinning
        && matches!(candidate, Candidate::Stored { version, .. } if !version.is_keyframe())
}

fn keep_what_cannot_be_rewritten<'p>(
    stored: &[StoredVersion<'_>],
    candidates: &mut [(Candidate<'p, '_>, bool)],
    prior_head: Option<(&'p StateRecord, &'p [u8])>,
    snapshot: &[u8],
) {
    let mut rebuild = Rebuild::new(stored, candidates, prior_head, snapshot);
    let mut dropped_from = None;
    for position in 0..candidates.len() {
        let Some((candidate, keep)) = candidates.get(position) else {
            break;
        };
        let keep = *keep;
        let rebuilt = rebuild.advance(candidate).is_some();
        if !keep {
            dropped_from.get_or_insert(position);
            continue;
        }
        let Some(start) = dropped_from.take() else {
            continue;
        };
        if is_rewritten(candidate, true) && !rebuilt && rebuild.starved {
            for (_, keep) in candidates.get_mut(start..position).unwrap_or_default() {
                *keep = true;
            }
        }
    }
}

fn write_versions<'a>(
    section: &mut Section<'a>,
    prior: Option<&Parsed<'a>>,
    prior_head: Option<(&StateRecord, &[u8])>,
    adds_version: bool,
    snapshot: &[u8],
    now: SystemTime,
    cut: usize,
) -> Result<Written, EncodeError> {
    let stored = prior.map_or(&[][..], |prior| prior.versions.as_slice());
    let (mut candidates, cut_listed) = thinned(prior, prior_head, adds_version, now, cut);
    let mut written = Written {
        kept: Vec::new(),
        cut_listed,
    };
    keep_what_cannot_be_rewritten(stored, &mut candidates, prior_head, snapshot);
    let mut rebuild = Rebuild::new(stored, &candidates, prior_head, snapshot);
    let mut base = Some(Content::Borrowed(snapshot));
    let mut after_thinning = false;
    let mut thinned_keyframe = false;
    for (candidate, keep) in &candidates {
        let content = rebuild.advance(candidate).cloned();
        if !keep {
            after_thinning = true;
            thinned_keyframe |= candidate.is_keyframe();
            continue;
        }
        let before = section.len();
        match candidate {
            Candidate::Replaced {
                info,
                snapshot: old,
                whole,
            } => {
                let bytes = section.fresh();
                push_packed(bytes, ChunkKind::VersionInfo, &value::to_bytes(info)?)?;
                if *whole {
                    push_packed(bytes, ChunkKind::VersionData, old)?;
                } else {
                    push_packed_after(bytes, ChunkKind::VersionData, old, snapshot)?;
                }
            }
            Candidate::Stored { version, .. } => {
                if let Some(info) = &version.info {
                    section.keep(info.chunk.whole);
                }
                let rewritten = is_rewritten(candidate, after_thinning);
                match (rewritten, content.as_ref(), base.as_ref()) {
                    (true, Some(content), Some(base)) if !thinned_keyframe => {
                        push_packed_after(
                            section.fresh(),
                            ChunkKind::VersionData,
                            content.bytes(),
                            base.bytes(),
                        )?;
                    }
                    (true, Some(content), _) => {
                        push_packed(section.fresh(), ChunkKind::VersionData, content.bytes())?;
                    }
                    (true, None, _) if rebuild.starved => {
                        return Err(EncodeError::HistoryTooLarge);
                    }
                    (true, None, _) | (false, ..) => section.keep(version.data.whole),
                }
            }
        }
        written.kept.push(section.len() - before);
        base = content;
        after_thinning = false;
        thinned_keyframe = false;
    }
    Ok(written)
}

#[cfg(test)]
pub(crate) fn file_from_records(version: u32, records: &[Vec<u8>]) -> Result<Vec<u8>, EncodeError> {
    let snapshot = snapshot_of(records.iter().map(Vec::as_slice));
    let head = StateRecord::new(SystemTime::now(), None, &snapshot);
    let mut bytes = start_file(&MODEL_MAGIC, version);
    push_packed(&mut bytes, ChunkKind::Head, &value::to_bytes(&head)?)?;
    for record in records {
        push_packed(&mut bytes, ChunkKind::Record, record)?;
    }
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Loaded, LoadError> {
    decode_cancellable(bytes, &CancelToken::never())
}

pub(crate) fn decode_cancellable(bytes: &[u8], cancel: &CancelToken) -> Result<Loaded, LoadError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(LoadError::Empty);
    }
    let parsed = Parsed::of(bytes).ok_or(LoadError::NotAModel)?;
    let mut issues = Vec::new();
    match parsed.version {
        Some(version) if version > FORMAT_VERSION => issues.push(newer_version(version)),
        Some(_) => {}
        None => issues.push(
            "The start of the file is damaged, so it no longer reads as a caditor model. Its parts \
             still pass their checks and were loaded from them; saving writes a sound start again."
                .to_owned(),
        ),
    }
    if parsed.foreign.iter().any(Chunk::must_understand) {
        issues.push(
            "This model holds something a newer version of caditor needs and this version cannot \
             read, so it was left out; saving here will not keep it."
                .to_owned(),
        );
    }
    if parsed.damaged > 0 {
        issues.push(match parsed.damaged {
            1 => {
                "A damaged part of the file was skipped; anything it held was left out.".to_owned()
            }
            count => format!(
                "{count} damaged parts of the file were skipped; anything they held was left out."
            ),
        });
    }
    let mut budget = Budget::default();
    let mut digest = RecordDigest::default();
    let mut record_issues = Vec::new();
    let records = parsed
        .records
        .iter()
        .take_while(|_| !cancel.is_cancelled())
        .map(|chunk| {
            let content = budget.unpack(chunk, None);
            digest.add(&content);
            content.map(Cow::Owned)
        });
    let parts = read_records(records, &mut record_issues);
    if cancel.is_cancelled() {
        return Err(LoadError::Cancelled);
    }
    if parsed.damaged == 0 && digest.whole && !digest.matches(parsed.head.as_ref()) {
        issues.push(
            "The file ends early, probably because it was not copied or synced completely: \
             parts of the model saved in it are missing. Everything that remained was loaded."
                .to_owned(),
        );
    }
    issues.extend(record_issues);
    let document = assemble(parts, &mut issues);
    Ok(Loaded {
        document,
        issues,
        digest: parsed.head.map(|head| FileDigest(head.digest)),
    })
}

fn read_records<'a>(
    records: impl Iterator<Item = Result<Cow<'a, [u8]>, UnpackError>>,
    issues: &mut Vec<String>,
) -> Parts {
    let mut parts = Parts::default();
    for (index, content) in records.enumerate() {
        let place = format!("Record {}", index + 1);
        match content.map(|content| value::from_bytes::<Lenient<Record>>(&content)) {
            Ok(Ok(Lenient::Read(record))) => parts.add(record),
            Ok(Ok(Lenient::Unreadable(value))) => {
                let item = Unreadable(&value);
                parts.remember_lost(&item);
                issues.push(describe_unreadable_record(&place, &item));
            }
            Ok(Err(_)) => issues.push(format!("{place} is damaged and was left out.")),
            Err(error) => issues.push(describe_unpack_failure(&place, &error)),
        }
    }
    parts
}

pub(crate) fn history(bytes: &[u8]) -> History {
    let Some(parsed) = Parsed::of(bytes) else {
        return History::default();
    };
    let available = parsed.rebuildable();
    History {
        current: parsed.head.as_ref().map(StateRecord::state),
        versions: parsed
            .versions
            .iter()
            .zip(available)
            .enumerate()
            .filter_map(|(index, (version, available))| {
                let info = &version.info.as_ref()?.record;
                Some(Version {
                    index,
                    state: info.state(),
                    available,
                    kept: info.kept,
                })
            })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum KeepError {
    #[error("the file holds no listed version numbered {index}")]
    NoSuchVersion { index: usize },
    #[error("the file has damaged parts")]
    FileDamaged,
    #[error("the file holds something from a newer version of caditor")]
    FromNewerVersion,
    #[error("{0}")]
    Value(#[from] ValueError),
    #[error("{0}")]
    Pack(#[from] PackError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rewritten {
    pub bytes: Vec<u8>,
    pub digest: FileDigest,
}

pub(crate) fn with_version_kept(
    bytes: &[u8],
    index: usize,
    kept: bool,
) -> Result<Rewritten, KeepError> {
    let parsed = Parsed::of(bytes).ok_or(KeepError::FileDamaged)?;
    match parsed.version {
        Some(version) if version > FORMAT_VERSION => return Err(KeepError::FromNewerVersion),
        Some(_) => {}
        None => return Err(KeepError::FileDamaged),
    }
    if parsed.foreign.iter().any(Chunk::must_understand) {
        return Err(KeepError::FromNewerVersion);
    }
    if parsed.is_damaged(&parsed.records_digest()) {
        return Err(KeepError::FileDamaged);
    }
    let (Some(head), Some(head_chunk), Some(version)) =
        (&parsed.head, parsed.head_chunk, parsed.version)
    else {
        return Err(KeepError::FileDamaged);
    };
    let target = parsed
        .versions
        .get(index)
        .and_then(|stored| stored.info.as_ref())
        .ok_or(KeepError::NoSuchVersion { index })?;
    let changed_info = info_marked(target, kept)?;

    let mut rewritten = start_file(&MODEL_MAGIC, version);
    rewritten.extend_from_slice(head_chunk.whole);
    for record in &parsed.records {
        rewritten.extend_from_slice(record.whole);
    }
    for foreign in &parsed.foreign {
        rewritten.extend_from_slice(foreign.whole);
    }
    for (position, stored) in parsed.versions.iter().enumerate() {
        match &stored.info {
            Some(_) if position == index => rewritten.extend_from_slice(&changed_info),
            Some(info) => rewritten.extend_from_slice(info.chunk.whole),
            None => {}
        }
        rewritten.extend_from_slice(stored.data.whole);
    }
    Ok(Rewritten {
        bytes: rewritten,
        digest: FileDigest(head.digest.clone()),
    })
}

fn info_marked(info: &StoredInfo<'_>, kept: bool) -> Result<Vec<u8>, KeepError> {
    let content = info
        .chunk
        .unpack(None)
        .map_err(|_| KeepError::FileDamaged)?;
    let mut fields: serde_json::Map<String, serde_json::Value> = value::from_bytes(&content)?;
    if kept {
        fields.insert("kept".to_owned(), serde_json::Value::Bool(true));
    } else {
        fields.remove("kept");
    }
    let mut chunk = Vec::new();
    push_packed(
        &mut chunk,
        ChunkKind::VersionInfo,
        &value::to_bytes(&fields)?,
    )?;
    Ok(chunk)
}

pub(crate) fn load_version(bytes: &[u8], index: usize) -> Result<Loaded, LoadError> {
    let parsed = Parsed::of(bytes).ok_or(LoadError::NotAModel)?;
    let snapshot = parsed
        .verified(index)
        .ok_or(LoadError::VersionUnavailable)?;
    let records = split_snapshot(&snapshot).ok_or(LoadError::VersionUnavailable)?;
    let mut issues = Vec::new();
    let parts = read_records(
        records.into_iter().map(|record| Ok(Cow::Borrowed(record))),
        &mut issues,
    );
    let document = assemble(parts, &mut issues);
    Ok(Loaded {
        document,
        issues,
        digest: None,
    })
}
