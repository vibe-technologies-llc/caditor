use std::{
    borrow::Cow,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use ahash::AHashMap;
use caditor_document::Document;
use serde::{Deserialize, Serialize};

use super::{
    Chunk, ChunkKind, Codec, MODEL_MAGIC, PackError, Piece, has_magic, parse, push_packed,
    push_packed_after,
    retention::retained,
    start_file,
    value::{self, ValueError, push_varint, read_varint},
};
use crate::{
    format::{
        FORMAT_VERSION, Lenient, Record, Unreadable, feature_record, next_ids_record,
        parameter_record,
    },
    load::{LoadError, Loaded, Parts, assemble, describe_unreadable_record, newer_version},
};

const KEYFRAME_SPACING: usize = 8;
const MAX_DECOMPRESSED: usize = 1 << 31;

struct Budget {
    remaining: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            remaining: MAX_DECOMPRESSED,
        }
    }
}

impl Budget {
    fn unpack(&mut self, chunk: &Chunk<'_>, newer: Option<&[u8]>) -> Option<Vec<u8>> {
        self.remaining = self.remaining.checked_sub(chunk.content_length())?;
        chunk.unpack(newer).ok()
    }
}

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
    version: u32,
    damaged: usize,
    head: Option<StateRecord>,
    records: Vec<Chunk<'a>>,
    versions: Vec<StoredVersion<'a>>,
    foreign: Vec<Chunk<'a>>,
}

impl<'a> Parsed<'a> {
    fn of(bytes: &'a [u8]) -> Option<Self> {
        let container = parse(bytes, &MODEL_MAGIC)?;
        let mut parsed = Self {
            version: container.version,
            damaged: container.damaged(),
            head: None,
            records: Vec::new(),
            versions: Vec::new(),
            foreign: Vec::new(),
        };
        let mut pending_info = None;
        for piece in &container.pieces {
            let Piece::Chunk(chunk) = *piece else {
                pending_info = None;
                continue;
            };
            let info = pending_info.take();
            match chunk.kind {
                Some(ChunkKind::Head) if parsed.head.is_none() => {
                    parsed.head = state_record(&chunk);
                }
                Some(ChunkKind::Record) => parsed.records.push(chunk),
                Some(ChunkKind::VersionInfo) => {
                    pending_info = state_record(&chunk).map(|record| StoredInfo { record, chunk });
                }
                Some(ChunkKind::VersionData) => {
                    parsed.versions.push(StoredVersion { info, data: chunk });
                }
                Some(
                    ChunkKind::Head
                    | ChunkKind::JournalHeader
                    | ChunkKind::Snapshot
                    | ChunkKind::Apply
                    | ChunkKind::Undo
                    | ChunkKind::Redo,
                ) => {}
                None => {
                    parsed.foreign.push(chunk);
                    pending_info = info;
                }
            }
        }
        Some(parsed)
    }

    fn record_contents(&self, budget: &mut Budget) -> Vec<Option<Vec<u8>>> {
        self.records
            .iter()
            .map(|chunk| budget.unpack(chunk, None))
            .collect()
    }

    fn snapshot_held(&self, contents: &[Option<Vec<u8>>]) -> Option<Vec<u8>> {
        let head = self.head.as_ref()?;
        let contents: Option<Vec<&[u8]>> = contents.iter().map(Option::as_deref).collect();
        let snapshot = snapshot_of(contents?);
        head.holds(&snapshot).then_some(snapshot)
    }

    fn kept_records<'b>(&self, contents: &'b [Option<Vec<u8>>]) -> AHashMap<Vec<u8>, KeptRecord<'b>>
    where
        'a: 'b,
    {
        self.records
            .iter()
            .zip(contents)
            .filter_map(|(chunk, content)| {
                let content = content.as_deref()?;
                let Ok(Lenient::Read(record)) = value::from_bytes::<Lenient<Record>>(content)
                else {
                    return None;
                };
                let understood = value::to_bytes(&record).ok()?;
                Some((
                    understood,
                    KeptRecord {
                        content,
                        stored: chunk.whole,
                    },
                ))
            })
            .collect()
    }

    fn head_snapshot(&self, budget: &mut Budget) -> Option<Vec<u8>> {
        let head = self.head.as_ref()?;
        let mut snapshot = Vec::new();
        for chunk in &self.records {
            let content = budget.unpack(chunk, None)?;
            push_varint(&mut snapshot, content.len() as u64);
            snapshot.extend_from_slice(&content);
        }
        head.holds(&snapshot).then_some(snapshot)
    }

    fn holds_every_record(&self, contents: &[Option<Vec<u8>>]) -> bool {
        let Some(head) = &self.head else {
            return false;
        };
        let mut hasher = blake3::Hasher::new();
        for content in contents {
            let Some(content) = content else {
                return false;
            };
            let mut length = Vec::new();
            push_varint(&mut length, content.len() as u64);
            hasher.update(&length);
            hasher.update(content);
        }
        head.digest == hasher.finalize().to_hex().to_string()
    }

    fn walk_versions(
        &self,
        from: usize,
        until: usize,
        mut visit: impl FnMut(usize, Option<&[u8]>),
    ) {
        let mut budget = Budget::default();
        let mut newer = if from == 0 {
            self.head_snapshot(&mut budget)
        } else {
            None
        };
        let versions = self.versions.iter().enumerate();
        let count = until.saturating_add(1).saturating_sub(from);
        for (index, version) in versions.skip(from).take(count) {
            let snapshot = budget.unpack(&version.data, newer.as_deref());
            let verified = snapshot
                .as_deref()
                .filter(|snapshot| version.holds(snapshot));
            visit(index, verified);
            newer = snapshot;
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

fn state_record(chunk: &Chunk<'_>) -> Option<StateRecord> {
    let content = chunk.unpack(None).ok()?;
    value::from_bytes(&content).ok()
}

fn document_records(document: &Document) -> Result<Vec<Vec<u8>>, ValueError> {
    let parameters = document
        .parameters()
        .iter()
        .map(|parameter| Record::Parameter(parameter_record(parameter)));
    let features = document
        .features()
        .map(|feature| Record::Feature(feature_record(feature)));
    parameters
        .chain(features)
        .chain(std::iter::once(Record::NextIds(next_ids_record(document))))
        .map(|record| value::to_bytes(&record))
        .collect()
}

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
    save_bytes(document, None, SystemTime::now(), None)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EncodeError {
    #[error("the model could not be converted for saving: {0}")]
    Value(#[from] ValueError),
    #[error("{0}")]
    Pack(#[from] PackError),
}

#[derive(Debug, Clone, Copy)]
struct KeptRecord<'a> {
    content: &'a [u8],
    stored: &'a [u8],
}

enum RecordToWrite<'a> {
    Kept(KeptRecord<'a>),
    Fresh(Vec<u8>),
}

impl RecordToWrite<'_> {
    fn content(&self) -> &[u8] {
        match self {
            Self::Kept(kept) => kept.content,
            Self::Fresh(content) => content,
        }
    }
}

pub(crate) fn save_bytes(
    document: &Document,
    previous: Option<&[u8]>,
    now: SystemTime,
    label: Option<&str>,
) -> Result<Vec<u8>, EncodeError> {
    let prior = previous.and_then(Parsed::of);
    let prior_contents = prior
        .as_ref()
        .map(|prior| prior.record_contents(&mut Budget::default()))
        .unwrap_or_default();
    let prior_head = prior
        .as_ref()
        .and_then(|prior| prior.head.clone().zip(prior.snapshot_held(&prior_contents)));
    let kept = prior
        .as_ref()
        .map(|prior| prior.kept_records(&prior_contents))
        .unwrap_or_default();
    let records: Vec<RecordToWrite<'_>> = document_records(document)?
        .into_iter()
        .map(|record| match kept.get(&record) {
            Some(kept) => RecordToWrite::Kept(*kept),
            None => RecordToWrite::Fresh(record),
        })
        .collect();
    let snapshot = snapshot_of(records.iter().map(RecordToWrite::content));
    let unchanged = prior_head
        .as_ref()
        .is_some_and(|(info, _)| info.holds(&snapshot));
    let head = match &prior_head {
        Some((info, _)) if unchanged => info.clone(),
        _ => StateRecord::new(now, label, &snapshot),
    };

    let mut bytes = start_file(&MODEL_MAGIC, FORMAT_VERSION);
    push_packed(&mut bytes, ChunkKind::Head, &value::to_bytes(&head)?)?;
    for record in &records {
        match record {
            RecordToWrite::Kept(kept) => bytes.extend_from_slice(kept.stored),
            RecordToWrite::Fresh(content) => push_packed(&mut bytes, ChunkKind::Record, content)?,
        }
    }
    for foreign in prior.iter().flat_map(|prior| &prior.foreign) {
        if !foreign.must_understand() {
            bytes.extend_from_slice(foreign.whole);
        }
    }
    write_versions(
        &mut bytes,
        prior.as_ref(),
        prior_head.as_ref(),
        !unchanged,
        &snapshot,
        now,
    )?;
    Ok(bytes)
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

    fn is_keyframe(&self) -> bool {
        match self {
            Self::Replaced { whole, .. } => *whole,
            Self::Stored { version, .. } => version.is_keyframe(),
        }
    }
}

fn thinned<'p, 'a>(
    prior: Option<&'p Parsed<'a>>,
    prior_head: Option<&'p (StateRecord, Vec<u8>)>,
    adds_version: bool,
    now: SystemTime,
) -> Vec<(Candidate<'p, 'a>, bool)> {
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
    let keep = if adds_version {
        let saved_at: Vec<Option<u64>> = candidates.iter().map(Candidate::saved_at).collect();
        retained(&saved_at, seconds_since_epoch(now))
    } else {
        vec![true; candidates.len()]
    };
    candidates.into_iter().zip(keep).collect()
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

fn write_versions(
    bytes: &mut Vec<u8>,
    prior: Option<&Parsed<'_>>,
    prior_head: Option<&(StateRecord, Vec<u8>)>,
    adds_version: bool,
    snapshot: &[u8],
    now: SystemTime,
) -> Result<(), EncodeError> {
    let stored = prior.map_or(&[][..], |prior| prior.versions.as_slice());
    let candidates = thinned(prior, prior_head, adds_version, now);
    let needed = needing_content(stored, &candidates);
    let mut budget = Budget::default();
    let mut previous: Option<Cow<'_, [u8]>> =
        prior_head.map(|(_, old)| Cow::Borrowed(old.as_slice()));
    let mut base: Option<Cow<'_, [u8]>> = Some(Cow::Borrowed(snapshot));
    let mut after_thinning = false;
    let mut thinned_keyframe = false;
    for (candidate, keep) in &candidates {
        let content = match candidate {
            Candidate::Replaced { snapshot: old, .. } => Some(Cow::Borrowed(*old)),
            Candidate::Stored { index, version } => needed
                .get(*index)
                .copied()
                .unwrap_or(false)
                .then(|| budget.unpack(&version.data, previous.as_deref()))
                .flatten()
                .map(Cow::Owned),
        };
        if !keep {
            after_thinning = true;
            thinned_keyframe |= candidate.is_keyframe();
            previous = content;
            continue;
        }
        match candidate {
            Candidate::Replaced {
                info,
                snapshot: old,
                whole,
            } => {
                push_packed(bytes, ChunkKind::VersionInfo, &value::to_bytes(info)?)?;
                if *whole {
                    push_packed(bytes, ChunkKind::VersionData, old)?;
                } else {
                    push_packed_after(bytes, ChunkKind::VersionData, old, snapshot)?;
                }
            }
            Candidate::Stored { version, .. } => {
                if let Some(info) = &version.info {
                    bytes.extend_from_slice(info.chunk.whole);
                }
                let rewritten = (after_thinning && !version.is_keyframe())
                    .then_some(content.as_deref())
                    .flatten();
                match (rewritten, base.as_deref()) {
                    (Some(content), Some(base)) if !thinned_keyframe => {
                        push_packed_after(bytes, ChunkKind::VersionData, content, base)?;
                    }
                    (Some(content), _) => push_packed(bytes, ChunkKind::VersionData, content)?,
                    (None, _) => bytes.extend_from_slice(version.data.whole),
                }
            }
        }
        base.clone_from(&content);
        previous = content;
        after_thinning = false;
        thinned_keyframe = false;
    }
    Ok(())
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
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(LoadError::Empty);
    }
    if !has_magic(bytes, &MODEL_MAGIC) {
        return Err(LoadError::NotAModel);
    }
    let parsed = Parsed::of(bytes).ok_or(LoadError::NotAModel)?;
    let mut issues = Vec::new();
    if parsed.version > FORMAT_VERSION {
        issues.push(newer_version(parsed.version));
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
    let contents: Vec<Option<Vec<u8>>> = parsed
        .records
        .iter()
        .map(|chunk| budget.unpack(chunk, None))
        .collect();
    if parsed.damaged == 0 && !parsed.holds_every_record(&contents) {
        issues.push(
            "The file ends early, probably because it was not copied or synced completely: \
             parts of the model saved in it are missing. Everything that remained was loaded."
                .to_owned(),
        );
    }
    let records = contents.into_iter().map(|content| content.map(Cow::Owned));
    let parts = read_records(records, &mut issues);
    let document = assemble(parts, &mut issues);
    Ok(Loaded { document, issues })
}

fn read_records<'a>(
    records: impl Iterator<Item = Option<Cow<'a, [u8]>>>,
    issues: &mut Vec<String>,
) -> Parts {
    let mut parts = Parts::default();
    for (index, content) in records.enumerate() {
        let place = format!("Record {}", index + 1);
        match content.map(|content| value::from_bytes::<Lenient<Record>>(&content)) {
            Some(Ok(Lenient::Read(record))) => parts.add(record),
            Some(Ok(Lenient::Unreadable(value))) => {
                let item = Unreadable(&value);
                parts.remember_lost(&item);
                issues.push(describe_unreadable_record(&place, &item));
            }
            Some(Err(_)) | None => issues.push(format!("{place} is damaged and was left out.")),
        }
    }
    parts
}

pub(crate) fn history(bytes: &[u8]) -> History {
    let Some(parsed) = Parsed::of(bytes) else {
        return History::default();
    };
    let mut available = vec![false; parsed.versions.len()];
    parsed.walk_versions(0, usize::MAX, |index, snapshot| {
        if let Some(slot) = available.get_mut(index) {
            *slot = snapshot.is_some();
        }
    });
    History {
        current: parsed.head.as_ref().map(StateRecord::state),
        versions: parsed
            .versions
            .iter()
            .zip(available)
            .enumerate()
            .filter_map(|(index, (version, available))| {
                Some(Version {
                    index,
                    state: version.info.as_ref()?.record.state(),
                    available,
                })
            })
            .collect(),
    }
}

pub(crate) fn load_version(bytes: &[u8], index: usize) -> Result<Loaded, LoadError> {
    let parsed = Parsed::of(bytes).ok_or(LoadError::NotAModel)?;
    let mut wanted = None;
    parsed.walk_versions(
        parsed.keyframe_at_or_before(index),
        index,
        |at, snapshot| {
            if at == index {
                wanted = snapshot.map(<[u8]>::to_vec);
            }
        },
    );
    let snapshot = wanted.ok_or(LoadError::VersionUnavailable)?;
    let records = split_snapshot(&snapshot).ok_or(LoadError::VersionUnavailable)?;
    let mut issues = Vec::new();
    let parts = read_records(
        records
            .into_iter()
            .map(|record| Some(Cow::Borrowed(record))),
        &mut issues,
    );
    let document = assemble(parts, &mut issues);
    Ok(Loaded { document, issues })
}
