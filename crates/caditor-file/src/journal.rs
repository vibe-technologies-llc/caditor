use std::path::{Path, PathBuf};

use caditor_document::{Document, Editor, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{
    format::{
        FeatureRecord, Lenient, NextIdsRecord, ParameterRecord, TransactionRecord, feature_record,
        next_ids_record, parameter_record, restore_transaction, transaction_record,
    },
    load::{Parts, assemble},
};

const JOURNAL_FORMAT: &str = "caditor-journal";
const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEntry {
    Apply(Transaction),
    Undo(Transaction),
    Redo(Transaction),
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalHeader {
    format: String,
    version: u32,
    file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EntryRecord {
    Snapshot(SnapshotRecord),
    Apply(TransactionRecord),
    Undo(TransactionRecord),
    Redo(TransactionRecord),
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotRecord {
    parameters: Vec<Lenient<ParameterRecord>>,
    features: Vec<Lenient<FeatureRecord>>,
    next_ids: NextIdsRecord,
}

#[derive(Debug, Deserialize)]
struct Line<'a> {
    crc: String,
    #[serde(borrow)]
    entry: &'a RawValue,
}

pub(crate) fn encode_journal(
    file: Option<&Path>,
    base: &Document,
    entries: &[JournalEntry],
) -> Result<String, serde_json::Error> {
    let header = JournalHeader {
        format: JOURNAL_FORMAT.to_owned(),
        version: JOURNAL_VERSION,
        file: file.and_then(Path::to_str).map(str::to_owned),
    };
    let mut text = serde_json::to_string(&header)?;
    text.push('\n');
    text.push_str(&encode_line(&EntryRecord::Snapshot(snapshot_record(base)))?);
    for entry in entries {
        text.push_str(&encode_entry(entry)?);
    }
    Ok(text)
}

pub(crate) fn encode_entry(entry: &JournalEntry) -> Result<String, serde_json::Error> {
    encode_line(&match entry {
        JournalEntry::Apply(transaction) => EntryRecord::Apply(transaction_record(transaction)),
        JournalEntry::Undo(transaction) => EntryRecord::Undo(transaction_record(transaction)),
        JournalEntry::Redo(transaction) => EntryRecord::Redo(transaction_record(transaction)),
    })
}

fn encode_line(record: &EntryRecord) -> Result<String, serde_json::Error> {
    let entry = serde_json::to_string(record)?;
    let crc = crc32fast::hash(entry.as_bytes());
    Ok(format!("{{\"crc\":\"{crc:08x}\",\"entry\":{entry}}}\n"))
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
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JournalContents {
    pub file: Option<PathBuf>,
    pub base: Document,
    pub issues: Vec<String>,
    pub entries: Vec<JournalEntry>,
    pub unreadable_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the recovery journal is damaged")]
pub(crate) struct DamagedJournal;

pub(crate) fn decode_journal(bytes: &[u8]) -> Result<JournalContents, DamagedJournal> {
    let mut lines = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace));
    let header: JournalHeader = lines
        .next()
        .and_then(|line| serde_json::from_slice(line).ok())
        .filter(|header: &JournalHeader| {
            header.format == JOURNAL_FORMAT && header.version <= JOURNAL_VERSION
        })
        .ok_or(DamagedJournal)?;
    let Some(EntryRecord::Snapshot(snapshot)) = lines.next().and_then(decode_line) else {
        return Err(DamagedJournal);
    };

    let mut issues = Vec::new();
    let base = assemble(snapshot_parts(snapshot, &mut issues), &mut issues);
    let mut entries = Vec::new();
    let mut unreadable_entries = 0;
    for line in lines {
        let entry = (unreadable_entries == 0)
            .then(|| decode_line(line))
            .flatten()
            .and_then(|record| match record {
                EntryRecord::Snapshot(_) => None,
                EntryRecord::Apply(record) => restore_transaction(record).map(JournalEntry::Apply),
                EntryRecord::Undo(record) => restore_transaction(record).map(JournalEntry::Undo),
                EntryRecord::Redo(record) => restore_transaction(record).map(JournalEntry::Redo),
            });
        match entry {
            Some(entry) => entries.push(entry),
            None => unreadable_entries += 1,
        }
    }
    Ok(JournalContents {
        file: header.file.map(PathBuf::from),
        base,
        issues,
        entries,
        unreadable_entries,
    })
}

fn decode_line(line: &[u8]) -> Option<EntryRecord> {
    let line: Line<'_> = serde_json::from_slice(line).ok()?;
    let entry = line.entry.get();
    let expected = u32::from_str_radix(&line.crc, 16).ok()?;
    if crc32fast::hash(entry.as_bytes()) != expected {
        return None;
    }
    serde_json::from_str(entry).ok()
}

fn snapshot_parts(snapshot: SnapshotRecord, issues: &mut Vec<String>) -> Parts {
    let mut parts = Parts {
        next_ids: Some(snapshot.next_ids),
        ..Parts::default()
    };
    for parameter in snapshot.parameters {
        match parameter {
            Lenient::Read(parameter) => parts.parameters.push(parameter),
            Lenient::Unreadable(_) => {
                issues.push("A parameter in the recovered model was damaged.".to_owned());
            }
        }
    }
    for feature in snapshot.features {
        match feature {
            Lenient::Read(feature) => parts.features.push(feature),
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
