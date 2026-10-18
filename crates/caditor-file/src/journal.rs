use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_document::{Document, Editor, FeatureKind, Picture, Transaction};
use serde::{Deserialize, Serialize};

use crate::{
    binary::{
        ChunkKind, EARLIER_BUILD_JOURNAL_MAGIC, EncodeError, FileDigest, JOURNAL_MAGIC, Piece,
        parse, push_packed_for_recovery, start_file, value,
    },
    configurations::{ConfigurationsRecord, configurations_record},
    format::{
        FeatureRecord, ImportTexts, Lenient, NamedValuesRecord, NextIdsRecord, ParameterRecord,
        PrincipalRecord, PropertiesRecord, Record, RollbackRecord, SuppressedRecord,
        TransactionRecord, ViewsRecord, feature_records, named_values_record, next_ids_record,
        parameter_record, principal_record, properties_record, restore_transaction_with,
        rollback_record, suppressed_record, transaction_record_sharing, views_record,
    },
    load::{Loaded, Parts, assemble},
    os,
    reference_images::Pictures,
    selection_sets::{SelectionSetsRecord, selection_sets_record},
};

pub(crate) const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum JournalEntry {
    Apply(Transaction),
    Undo(Transaction),
    Redo(Transaction),
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalHeader {
    file: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_disk: Option<FileDigest>,
    loaded_with_problems: bool,
    folded: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Logged {
    Entry(JournalEntry),
    UndoLast,
    RedoNext,
}

#[derive(Debug, Serialize, Deserialize)]
struct RebasedSnapshotRecord {
    snapshot: SnapshotRecord,
    folded: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    properties: Option<PropertiesRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    views: Option<ViewsRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    named_values: Option<NamedValuesRecord>,
    selection_sets: Option<SelectionSetsRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configurations: Option<ConfigurationsRecord>,
}

pub(crate) struct JournalHead<'a> {
    pub file: Option<&'a Path>,
    pub on_disk: Option<&'a FileDigest>,
    pub loaded_with_problems: bool,
    pub folded: usize,
}

#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn encode_journal(
    head: &JournalHead<'_>,
    base: &Document,
    entries: &[Logged],
) -> Result<Vec<u8>, EncodeError> {
    encode_journal_noting(head, base, entries, &mut WrittenPictures::default())
}

pub(crate) fn encode_journal_noting(
    head: &JournalHead<'_>,
    base: &Document,
    entries: &[Logged],
    pictures: &mut WrittenPictures,
) -> Result<Vec<u8>, EncodeError> {
    *pictures = WrittenPictures::of(base);
    let header = JournalHeader {
        file: head.file.map(|file| os::path_bytes(file.as_os_str())),
        on_disk: head.on_disk.cloned(),
        loaded_with_problems: head.loaded_with_problems,
        folded: u64::try_from(head.folded).unwrap_or(u64::MAX),
    };
    let mut bytes = start_file(&JOURNAL_MAGIC, JOURNAL_VERSION);
    push_packed_for_recovery(
        &mut bytes,
        ChunkKind::JournalHeader,
        &value::to_bytes(&header)?,
    )?;
    let snapshot = snapshot_record(base);
    if head.folded == 0 {
        push_packed_for_recovery(
            &mut bytes,
            ChunkKind::Snapshot,
            &value::to_bytes(&snapshot)?,
        )?;
    } else {
        let rebased = RebasedSnapshotRecord {
            snapshot,
            folded: u64::try_from(head.folded).unwrap_or(u64::MAX),
        };
        push_packed_for_recovery(
            &mut bytes,
            ChunkKind::RebasedSnapshot,
            &value::to_bytes(&rebased)?,
        )?;
    }
    for entry in entries {
        bytes.extend_from_slice(&encode_entry_sharing(entry, pictures)?);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WrittenPictures(BTreeSet<String>);

impl WrittenPictures {
    fn of(document: &Document) -> Self {
        Self(
            pictures_of(document)
                .map(|picture| picture.digest().hex())
                .collect(),
        )
    }
}

fn pictures_of(document: &Document) -> impl Iterator<Item = Arc<Picture>> + '_ {
    document
        .features()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::ReferenceImage(image) => Some(Arc::clone(&image.picture)),
            _ => None,
        })
}

#[cfg(test)]
pub(crate) fn encode_entry(entry: &Logged) -> Result<Vec<u8>, EncodeError> {
    encode_entry_sharing(entry, &mut WrittenPictures::default())
}

pub(crate) fn encode_entry_sharing(
    entry: &Logged,
    pictures: &mut WrittenPictures,
) -> Result<Vec<u8>, EncodeError> {
    let mut bytes = Vec::new();
    let (kind, transaction) = match entry {
        Logged::Entry(JournalEntry::Apply(transaction)) => (ChunkKind::Apply, transaction),
        Logged::Entry(JournalEntry::Undo(transaction)) => (ChunkKind::Undo, transaction),
        Logged::Entry(JournalEntry::Redo(transaction)) => (ChunkKind::Redo, transaction),
        Logged::UndoLast => {
            push_packed_for_recovery(&mut bytes, ChunkKind::UndoLast, &[])?;
            return Ok(bytes);
        }
        Logged::RedoNext => {
            push_packed_for_recovery(&mut bytes, ChunkKind::RedoNext, &[])?;
            return Ok(bytes);
        }
    };
    push_packed_for_recovery(
        &mut bytes,
        kind,
        &value::to_bytes(&transaction_record_sharing(transaction, &mut pictures.0))?,
    )?;
    Ok(bytes)
}

pub(crate) struct Mirror {
    editor: Option<Editor>,
}

impl Mirror {
    pub fn new(base: Document) -> Self {
        Self {
            editor: Some(Editor::new(base)),
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.editor.as_ref().map(Editor::document)
    }

    pub fn log(&mut self, entry: JournalEntry) -> Logged {
        let Some(editor) = &mut self.editor else {
            return Logged::Entry(entry);
        };
        let by_reference = match &entry {
            JournalEntry::Undo(transaction) if editor.next_undo() == Some(transaction) => editor
                .undo()
                .is_ok_and(|undone| undone.is_some())
                .then_some(Logged::UndoLast),
            JournalEntry::Redo(transaction) if editor.next_redo() == Some(transaction) => editor
                .redo()
                .is_ok_and(|redone| redone.is_some())
                .then_some(Logged::RedoNext),
            JournalEntry::Apply(transaction)
            | JournalEntry::Undo(transaction)
            | JournalEntry::Redo(transaction) => editor
                .apply(transaction.clone())
                .is_ok()
                .then(|| Logged::Entry(entry.clone())),
        };
        by_reference.unwrap_or_else(|| {
            log::warn!("the recovery journal stopped following the edits; it keeps them whole");
            self.editor = None;
            Logged::Entry(entry)
        })
    }
}

fn snapshot_record(document: &Document) -> SnapshotRecord {
    SnapshotRecord {
        parameters: document
            .parameters()
            .iter()
            .map(|parameter| Lenient::Read(parameter_record(parameter)))
            .collect(),
        features: feature_records(document).map(Lenient::Read).collect(),
        next_ids: next_ids_record(document),
        principal: principal_record(document),
        suppressed: suppressed_record(document),
        rollback: rollback_record(document),
        properties: properties_record(document),
        views: views_record(document),
        named_values: named_values_record(document),
        selection_sets: selection_sets_record(document),
        configurations: configurations_record(document),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JournalContents {
    pub file: Option<PathBuf>,
    pub on_disk: Option<FileDigest>,
    pub loaded_with_problems: bool,
    pub base: Document,
    pub folded: usize,
    pub issues: Vec<String>,
    pub entries: Vec<Logged>,
    pub unreadable_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum UnreadableJournal {
    #[error("the recovery journal is damaged")]
    Damaged,
    #[error("the recovery journal was written by a newer version of caditor")]
    FromNewerVersion,
    #[error("the recovery journal was written by an early development build of caditor")]
    FromEarlierBuild,
}

#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn decode_journal(bytes: &[u8]) -> Result<JournalContents, UnreadableJournal> {
    decode_journal_with(bytes, |_, _| None)
}

pub(crate) fn decode_journal_with(
    bytes: &[u8],
    saved_state: impl FnOnce(&Path, &FileDigest) -> Option<Loaded>,
) -> Result<JournalContents, UnreadableJournal> {
    if bytes.starts_with(&EARLIER_BUILD_JOURNAL_MAGIC) {
        return Err(UnreadableJournal::FromEarlierBuild);
    }
    let container = parse(bytes, &JOURNAL_MAGIC).ok_or(UnreadableJournal::Damaged)?;
    if container.version > JOURNAL_VERSION {
        return Err(UnreadableJournal::FromNewerVersion);
    }
    let mut pieces = container.pieces.iter();
    let header: JournalHeader = content_of(pieces.next(), ChunkKind::JournalHeader)?;
    let file = header
        .file
        .map(|bytes| PathBuf::from(os::path_from_bytes(bytes)));
    let mut issues = Vec::new();
    let mut folded = 0;
    let base = match pieces.next() {
        Some(Piece::Chunk(chunk)) if chunk.kind == Some(ChunkKind::RebasedSnapshot) => {
            let rebased: RebasedSnapshotRecord =
                content_of(Some(&Piece::Chunk(*chunk)), ChunkKind::RebasedSnapshot)?;
            folded = usize::try_from(rebased.folded).unwrap_or(usize::MAX);
            assemble(snapshot_parts(rebased.snapshot, &mut issues), &mut issues)
        }
        Some(Piece::Damaged) if header.folded == 0 => {
            let saved = file
                .as_deref()
                .zip(header.on_disk.as_ref())
                .and_then(|(file, on_disk)| saved_state(file, on_disk))
                .ok_or(UnreadableJournal::Damaged)?;
            issues.extend(saved.issues);
            issues.push(
                "The recovered copy of the model was damaged, so the changes were replayed onto \
                 the model as it was last saved."
                    .to_owned(),
            );
            saved.document
        }
        piece => {
            let snapshot = content_of(piece, ChunkKind::Snapshot)?;
            assemble(snapshot_parts(snapshot, &mut issues), &mut issues)
        }
    };
    let mut entries = Vec::new();
    let mut unreadable_entries = 0;
    let mut texts = ImportTexts::with_pictures(Pictures::of(pictures_of(&base)));
    for piece in pieces {
        let entry = (unreadable_entries == 0)
            .then(|| decode_entry(piece, &mut texts))
            .flatten();
        match entry {
            Some(entry) => entries.push(entry),
            None => unreadable_entries += 1,
        }
    }
    Ok(JournalContents {
        file,
        on_disk: header.on_disk,
        loaded_with_problems: header.loaded_with_problems,
        base,
        folded,
        issues,
        entries,
        unreadable_entries,
    })
}

fn content_of<T: for<'de> Deserialize<'de>>(
    piece: Option<&Piece<'_>>,
    kind: ChunkKind,
) -> Result<T, UnreadableJournal> {
    let Some(Piece::Chunk(chunk)) = piece else {
        return Err(UnreadableJournal::Damaged);
    };
    if chunk.kind != Some(kind) {
        return Err(UnreadableJournal::Damaged);
    }
    let content = chunk.unpack(None).map_err(|_| UnreadableJournal::Damaged)?;
    value::from_bytes(&content).map_err(|_| UnreadableJournal::Damaged)
}

fn decode_entry(piece: &Piece<'_>, texts: &mut ImportTexts) -> Option<Logged> {
    let Piece::Chunk(chunk) = piece else {
        return None;
    };
    let entry = match chunk.kind? {
        ChunkKind::UndoLast => return Some(Logged::UndoLast),
        ChunkKind::RedoNext => return Some(Logged::RedoNext),
        ChunkKind::Apply => JournalEntry::Apply,
        ChunkKind::Undo => JournalEntry::Undo,
        ChunkKind::Redo => JournalEntry::Redo,
        ChunkKind::Head
        | ChunkKind::Record
        | ChunkKind::VersionInfo
        | ChunkKind::VersionData
        | ChunkKind::JournalHeader
        | ChunkKind::Snapshot
        | ChunkKind::RebasedSnapshot
        | ChunkKind::Preview
        | ChunkKind::Padding => return None,
    };
    let content = chunk.unpack(None).ok()?;
    let record: TransactionRecord = value::from_bytes(&content).ok()?;
    Some(Logged::Entry(entry(restore_transaction_with(
        record, texts,
    )?)))
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
        properties: snapshot.properties,
        views: snapshot.views,
        named_values: snapshot.named_values,
        selection_sets: snapshot.selection_sets,
        configurations: snapshot.configurations,
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

pub(crate) fn replay(base: Document, entries: Vec<Logged>) -> Replayed {
    let mut editor = Editor::new(base);
    let mut replayed = Vec::with_capacity(entries.len());
    let mut stopped_early = false;
    for logged in entries {
        let Some(entry) = replay_one(&mut editor, logged) else {
            stopped_early = true;
            break;
        };
        replayed.push(entry);
    }
    Replayed {
        editor,
        entries: replayed,
        stopped_early,
    }
}

fn replay_one(editor: &mut Editor, logged: Logged) -> Option<JournalEntry> {
    let applied = match &logged {
        Logged::UndoLast => {
            let undone = editor.next_undo().cloned()?;
            let entry = JournalEntry::Undo(undone);
            return editor
                .undo()
                .is_ok_and(|undone| undone.is_some())
                .then_some(entry);
        }
        Logged::RedoNext => {
            let redone = editor.next_redo().cloned()?;
            let entry = JournalEntry::Redo(redone);
            return editor
                .redo()
                .is_ok_and(|redone| redone.is_some())
                .then_some(entry);
        }
        Logged::Entry(JournalEntry::Apply(transaction)) => {
            editor.apply(transaction.clone()).is_ok()
        }
        Logged::Entry(JournalEntry::Undo(transaction))
            if editor.next_undo() == Some(transaction) =>
        {
            editor.undo().is_ok_and(|undone| undone.is_some())
        }
        Logged::Entry(JournalEntry::Redo(transaction))
            if editor.next_redo() == Some(transaction) =>
        {
            editor.redo().is_ok_and(|redone| redone.is_some())
        }
        Logged::Entry(JournalEntry::Undo(transaction) | JournalEntry::Redo(transaction)) => {
            editor.apply(transaction.clone()).is_ok()
        }
    };
    match logged {
        Logged::Entry(entry) if applied => Some(entry),
        Logged::Entry(_) | Logged::UndoLast | Logged::RedoNext => None,
    }
}
