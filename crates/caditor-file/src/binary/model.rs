use std::time::{Duration, SystemTime, UNIX_EPOCH};

use caditor_document::Document;
use serde::{Deserialize, Serialize};

use super::{
    Chunk, ChunkKind, Codec, MODEL_MAGIC, PackError, has_magic, parse, push_packed,
    push_packed_after, start_file,
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

impl StateRecord {
    fn new(saved_at: SystemTime, label: Option<&str>, snapshot: &[u8]) -> Self {
        Self {
            saved_at: saved_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            label: label.map(str::to_owned),
            digest: digest(snapshot),
        }
    }

    fn state(&self) -> SavedState {
        SavedState {
            saved_at: UNIX_EPOCH + Duration::from_secs(self.saved_at),
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
struct StoredVersion<'a> {
    info: StateRecord,
    info_chunk: Chunk<'a>,
    data: Chunk<'a>,
}

impl StoredVersion<'_> {
    fn is_keyframe(&self) -> bool {
        self.data.codec != Some(Codec::ZstdAfterNewer)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Parsed<'a> {
    version: u32,
    damaged: usize,
    head: Option<StateRecord>,
    records: Vec<Chunk<'a>>,
    versions: Vec<StoredVersion<'a>>,
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
        };
        let mut pending_info = None;
        for chunk in container.chunks() {
            match chunk.kind {
                Some(ChunkKind::Head) if parsed.head.is_none() => {
                    parsed.head = state_record(&chunk);
                }
                Some(ChunkKind::Record) => parsed.records.push(chunk),
                Some(ChunkKind::VersionInfo) => {
                    pending_info = state_record(&chunk).map(|info| (info, chunk));
                }
                Some(ChunkKind::VersionData) => {
                    if let Some((info, info_chunk)) = pending_info.take() {
                        parsed.versions.push(StoredVersion {
                            info,
                            info_chunk,
                            data: chunk,
                        });
                    }
                }
                Some(
                    ChunkKind::Head
                    | ChunkKind::JournalHeader
                    | ChunkKind::Snapshot
                    | ChunkKind::Apply
                    | ChunkKind::Undo
                    | ChunkKind::Redo,
                )
                | None => {}
            }
        }
        Some(parsed)
    }

    fn record_contents(&self) -> Vec<Result<Vec<u8>, super::UnpackError>> {
        self.records
            .iter()
            .map(|chunk| chunk.unpack(None))
            .collect()
    }

    fn head_snapshot(&self) -> Option<Vec<u8>> {
        let head = self.head.as_ref()?;
        let contents: Option<Vec<Vec<u8>>> =
            self.record_contents().into_iter().map(Result::ok).collect();
        let snapshot = snapshot_of(&contents?);
        head.holds(&snapshot).then_some(snapshot)
    }

    fn version_snapshots(&self, until: usize) -> Vec<Option<Vec<u8>>> {
        let mut newer = self.head_snapshot();
        let mut snapshots = Vec::new();
        for version in self.versions.iter().take(until.saturating_add(1)) {
            let snapshot = version
                .data
                .unpack(newer.as_deref())
                .ok()
                .filter(|snapshot| version.info.holds(snapshot));
            snapshots.push(snapshot.clone());
            newer = snapshot;
        }
        snapshots
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

fn snapshot_of(records: &[Vec<u8>]) -> Vec<u8> {
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

pub(crate) fn save_bytes(
    document: &Document,
    previous: Option<&[u8]>,
    now: SystemTime,
    label: Option<&str>,
) -> Result<Vec<u8>, EncodeError> {
    let records = document_records(document)?;
    let snapshot = snapshot_of(&records);
    let prior = previous.and_then(Parsed::of);
    let prior_head = prior
        .as_ref()
        .and_then(|prior| prior.head.clone().zip(prior.head_snapshot()));
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
        push_packed(&mut bytes, ChunkKind::Record, record)?;
    }
    if !unchanged && let Some((info, old)) = &prior_head {
        push_packed(&mut bytes, ChunkKind::VersionInfo, &value::to_bytes(info)?)?;
        let leading_deltas = prior.as_ref().map_or(0, Parsed::leading_deltas);
        if leading_deltas + 1 >= KEYFRAME_SPACING {
            push_packed(&mut bytes, ChunkKind::VersionData, old)?;
        } else {
            push_packed_after(&mut bytes, ChunkKind::VersionData, old, &snapshot)?;
        }
    }
    if let Some(prior) = &prior {
        let unreachable = if prior_head.is_some() {
            0
        } else {
            prior.leading_deltas()
        };
        for version in prior.versions.iter().skip(unreachable) {
            bytes.extend_from_slice(version.info_chunk.whole);
            bytes.extend_from_slice(version.data.whole);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) fn file_from_records(version: u32, records: &[Vec<u8>]) -> Result<Vec<u8>, EncodeError> {
    let snapshot = snapshot_of(records);
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
    let contents = parsed.record_contents();
    let records = contents.iter().map(|content| content.as_deref().ok());
    let parts = read_records(records, &mut issues);
    let document = assemble(parts, &mut issues);
    Ok(Loaded { document, issues })
}

fn read_records<'a>(
    records: impl Iterator<Item = Option<&'a [u8]>>,
    issues: &mut Vec<String>,
) -> Parts {
    let mut parts = Parts::default();
    for (index, content) in records.enumerate() {
        let place = format!("Record {}", index + 1);
        match content.map(value::from_bytes::<Lenient<Record>>) {
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
    let snapshots = parsed.version_snapshots(usize::MAX);
    History {
        current: parsed.head.as_ref().map(StateRecord::state),
        versions: parsed
            .versions
            .iter()
            .zip(snapshots)
            .enumerate()
            .map(|(index, (version, snapshot))| Version {
                index,
                state: version.info.state(),
                available: snapshot.is_some(),
            })
            .collect(),
    }
}

pub(crate) fn load_version(bytes: &[u8], index: usize) -> Result<Loaded, LoadError> {
    let parsed = Parsed::of(bytes).ok_or(LoadError::NotAModel)?;
    let snapshot = parsed
        .version_snapshots(index)
        .into_iter()
        .nth(index)
        .flatten()
        .ok_or(LoadError::VersionUnavailable)?;
    let records = split_snapshot(&snapshot).ok_or(LoadError::VersionUnavailable)?;
    let mut issues = Vec::new();
    let parts = read_records(records.into_iter().map(Some), &mut issues);
    let document = assemble(parts, &mut issues);
    Ok(Loaded { document, issues })
}
