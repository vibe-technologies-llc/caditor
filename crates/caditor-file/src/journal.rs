use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use caditor_document::{Document, Editor, Transaction};
use serde::{Deserialize, Serialize};

use crate::{
    binary::{ChunkKind, EncodeError, JOURNAL_MAGIC, Piece, parse, push_packed, start_file, value},
    format::{
        FeatureRecord, Lenient, NextIdsRecord, ParameterRecord, PrincipalRecord, Record,
        RollbackRecord, SuppressedRecord, TransactionRecord, feature_record, next_ids_record,
        parameter_record, principal_record, restore_transaction, rollback_record,
        suppressed_record, transaction_record,
    },
    load::{Parts, assemble},
};

const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEntry {
    Apply(Transaction),
    Undo(Transaction),
    Redo(Transaction),
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalHeader {
    file: Option<String>,
    #[serde(default)]
    file_bytes: Option<Vec<u8>>,
    #[serde(default)]
    loaded_with_problems: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotRecord {
    parameters: Vec<Lenient<ParameterRecord>>,
    features: Vec<Lenient<FeatureRecord>>,
    next_ids: NextIdsRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    principal: Option<PrincipalRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    suppressed: Option<SuppressedRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback: Option<RollbackRecord>,
}

pub(crate) fn encode_journal(
    file: Option<&Path>,
    loaded_with_problems: bool,
    base: &Document,
    entries: &[JournalEntry],
) -> Result<Vec<u8>, EncodeError> {
    let header = JournalHeader {
        file: file.and_then(Path::to_str).map(str::to_owned),
        file_bytes: file.map(|file| file.as_os_str().as_bytes().to_vec()),
        loaded_with_problems,
    };
    let mut bytes = start_file(&JOURNAL_MAGIC, JOURNAL_VERSION);
    push_packed(
        &mut bytes,
        ChunkKind::JournalHeader,
        &value::to_bytes(&header)?,
    )?;
    push_packed(
        &mut bytes,
        ChunkKind::Snapshot,
        &value::to_bytes(&snapshot_record(base))?,
    )?;
    for entry in entries {
        bytes.extend_from_slice(&encode_entry(entry)?);
    }
    Ok(bytes)
}

pub(crate) fn encode_entry(entry: &JournalEntry) -> Result<Vec<u8>, EncodeError> {
    let (kind, transaction) = match entry {
        JournalEntry::Apply(transaction) => (ChunkKind::Apply, transaction),
        JournalEntry::Undo(transaction) => (ChunkKind::Undo, transaction),
        JournalEntry::Redo(transaction) => (ChunkKind::Redo, transaction),
    };
    let mut bytes = Vec::new();
    push_packed(
        &mut bytes,
        kind,
        &value::to_bytes(&transaction_record(transaction))?,
    )?;
    Ok(bytes)
}

fn snapshot_record(document: &Document) -> SnapshotRecord {
    SnapshotRecord {
        parameters: document
            .parameters()
            .iter()
            .map(|parameter| Lenient::Read(parameter_record(parameter)))
            .collect(),
        features: document
            .features()
            .map(|feature| Lenient::Read(feature_record(feature)))
            .collect(),
        next_ids: next_ids_record(document),
        principal: principal_record(document),
        suppressed: suppressed_record(document),
        rollback: rollback_record(document),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JournalContents {
    pub file: Option<PathBuf>,
    pub loaded_with_problems: bool,
    pub base: Document,
    pub issues: Vec<String>,
    pub entries: Vec<JournalEntry>,
    pub unreadable_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the recovery journal is damaged")]
pub(crate) struct DamagedJournal;

pub(crate) fn decode_journal(bytes: &[u8]) -> Result<JournalContents, DamagedJournal> {
    let container = parse(bytes, &JOURNAL_MAGIC)
        .filter(|container| container.version <= JOURNAL_VERSION)
        .ok_or(DamagedJournal)?;
    let mut pieces = container.pieces.iter();
    let header: JournalHeader = next_of_kind(&mut pieces, ChunkKind::JournalHeader)?;
    let snapshot: SnapshotRecord = next_of_kind(&mut pieces, ChunkKind::Snapshot)?;

    let mut issues = Vec::new();
    let base = assemble(snapshot_parts(snapshot, &mut issues), &mut issues);
    let mut entries = Vec::new();
    let mut unreadable_entries = 0;
    for piece in pieces {
        let entry = (unreadable_entries == 0)
            .then(|| decode_entry(piece))
            .flatten();
        match entry {
            Some(entry) => entries.push(entry),
            None => unreadable_entries += 1,
        }
    }
    let file = match header.file_bytes {
        Some(bytes) => Some(PathBuf::from(OsString::from_vec(bytes))),
        None => header.file.map(PathBuf::from),
    };
    Ok(JournalContents {
        file,
        loaded_with_problems: header.loaded_with_problems,
        base,
        issues,
        entries,
        unreadable_entries,
    })
}

fn next_of_kind<'a, T: for<'de> Deserialize<'de>>(
    pieces: &mut impl Iterator<Item = &'a Piece<'a>>,
    kind: ChunkKind,
) -> Result<T, DamagedJournal> {
    let Some(Piece::Chunk(chunk)) = pieces.next() else {
        return Err(DamagedJournal);
    };
    if chunk.kind != Some(kind) {
        return Err(DamagedJournal);
    }
    let content = chunk.unpack(None).map_err(|_| DamagedJournal)?;
    value::from_bytes(&content).map_err(|_| DamagedJournal)
}

fn decode_entry(piece: &Piece<'_>) -> Option<JournalEntry> {
    let Piece::Chunk(chunk) = piece else {
        return None;
    };
    let content = chunk.unpack(None).ok()?;
    let record: TransactionRecord = value::from_bytes(&content).ok()?;
    let transaction = restore_transaction(record)?;
    match chunk.kind? {
        ChunkKind::Apply => Some(JournalEntry::Apply(transaction)),
        ChunkKind::Undo => Some(JournalEntry::Undo(transaction)),
        ChunkKind::Redo => Some(JournalEntry::Redo(transaction)),
        ChunkKind::Head
        | ChunkKind::Record
        | ChunkKind::VersionInfo
        | ChunkKind::VersionData
        | ChunkKind::JournalHeader
        | ChunkKind::Snapshot
        | ChunkKind::Padding => None,
    }
}

fn snapshot_parts(snapshot: SnapshotRecord, issues: &mut Vec<String>) -> Parts {
    let mut parts = Parts {
        next_ids: Some(snapshot.next_ids),
        hidden_principal: snapshot
            .principal
            .map(|principal| principal.hidden)
            .unwrap_or_default(),
        suppressed: snapshot
            .suppressed
            .map(|suppressed| suppressed.features)
            .unwrap_or_default(),
        rollback: snapshot.rollback.map(|rollback| rollback.before),
        ..Parts::default()
    };
    for parameter in snapshot.parameters {
        match parameter {
            Lenient::Read(parameter) => parts.add(Record::Parameter(parameter)),
            Lenient::Unreadable(_) => {
                issues.push("A parameter in the recovered model was damaged.".to_owned());
            }
        }
    }
    for feature in snapshot.features {
        match feature {
            Lenient::Read(feature) => parts.add(Record::Feature(Box::new(feature))),
            Lenient::Unreadable(_) => {
                issues.push("A feature in the recovered model was damaged.".to_owned());
            }
        }
    }
    parts
}

#[derive(Debug, Clone)]
pub(crate) struct Replayed {
    pub editor: Editor,
    pub entries: Vec<JournalEntry>,
    pub stopped_early: bool,
}

pub(crate) fn replay(base: Document, entries: Vec<JournalEntry>) -> Replayed {
    let mut editor = Editor::new(base);
    let mut replayed = Vec::with_capacity(entries.len());
    let mut stopped_early = false;
    for entry in entries {
        let applied = match &entry {
            JournalEntry::Apply(transaction) => editor.apply(transaction.clone()).is_ok(),
            JournalEntry::Undo(transaction) if editor.next_undo() == Some(transaction) => {
                editor.undo().is_ok_and(|undone| undone.is_some())
            }
            JournalEntry::Redo(transaction) if editor.next_redo() == Some(transaction) => {
                editor.redo().is_ok_and(|redone| redone.is_some())
            }
            JournalEntry::Undo(transaction) | JournalEntry::Redo(transaction) => {
                editor.apply(transaction.clone()).is_ok()
            }
        };
        if !applied {
            stopped_early = true;
            break;
        }
        replayed.push(entry);
    }
    Replayed {
        editor,
        entries: replayed,
        stopped_early,
    }
}
