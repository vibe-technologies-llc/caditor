use std::{
    cmp::Reverse,
    collections::BTreeSet,
    fs::{self, File, TryLockError},
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

use caditor_document::{Document, Editor};

use crate::{
    journal::{JournalEntry, decode_journal, replay},
    load::load,
    paths::{self, JOURNAL_EXTENSION},
};

#[derive(Debug, Clone)]
pub struct Recovered {
    pub journal: PathBuf,
    pub file: Option<PathBuf>,
    pub base: Document,
    pub entries: Vec<JournalEntry>,
    pub editor: Editor,
    pub issues: Vec<String>,
    pub modified: Option<SystemTime>,
}

impl Recovered {
    pub fn changes(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone)]
pub enum Inspection {
    Recoverable(Box<Recovered>),
    Removed,
    InUse,
    Damaged,
}

pub fn inspect(journal: &Path) -> io::Result<Inspection> {
    let mut file = File::open(journal)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Inspection::InUse),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok();

    let Ok(contents) = decode_journal(&bytes) else {
        log::warn!("{} is damaged and cannot be recovered", journal.display());
        return Ok(Inspection::Damaged);
    };
    let replayed = replay(contents.base.clone(), contents.entries);
    let document = replayed.editor.document();
    let unchanged = replayed.entries.is_empty() || *document == contents.base;
    let already_saved = || {
        contents
            .file
            .as_deref()
            .and_then(|file| load(file).ok())
            .is_some_and(|loaded| loaded.issues.is_empty() && loaded.document == *document)
    };
    if unchanged || already_saved() {
        fs::remove_file(journal)?;
        return Ok(Inspection::Removed);
    }

    let mut issues = contents.issues;
    let unapplied = contents.unreadable_entries + usize::from(replayed.stopped_early);
    if unapplied > 0 {
        issues.push(
            "The most recent changes were damaged and could not be recovered; everything before \
             them was."
                .to_owned(),
        );
    }
    Ok(Inspection::Recoverable(Box::new(Recovered {
        journal: journal.to_path_buf(),
        file: contents.file,
        base: contents.base,
        entries: replayed.entries,
        editor: replayed.editor,
        issues,
        modified,
    })))
}

pub fn scan(recovery_dir: Option<&Path>, recent: &[PathBuf]) -> Vec<Recovered> {
    let in_recovery_dir = recovery_dir
        .and_then(|dir| fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == JOURNAL_EXTENSION)
        });
    let next_to_recent = recent
        .iter()
        .filter_map(|file| paths::adjacent_journal(file));
    let candidates: BTreeSet<PathBuf> = in_recovery_dir
        .chain(next_to_recent)
        .filter(|path| path.is_file())
        .collect();

    let mut recovered: Vec<Recovered> = candidates
        .iter()
        .filter_map(|journal| match inspect(journal) {
            Ok(Inspection::Recoverable(recovered)) => Some(*recovered),
            Ok(Inspection::Removed | Inspection::InUse | Inspection::Damaged) => None,
            Err(error) => {
                log::warn!("could not inspect {}: {error}", journal.display());
                None
            }
        })
        .collect();
    recovered.sort_by_key(|recovered| Reverse(recovered.modified));
    recovered
}

#[derive(Debug, Clone)]
pub enum FileJournal {
    None,
    InUse,
    Recoverable(Box<Recovered>),
}

pub fn journal_for(file: &Path, recovery_dir: Option<&Path>) -> FileJournal {
    for journal in paths::journals_for(file, recovery_dir) {
        if !journal.is_file() {
            continue;
        }
        match inspect(&journal) {
            Ok(Inspection::Recoverable(mut recovered)) => {
                recovered.file = Some(file.to_path_buf());
                return FileJournal::Recoverable(recovered);
            }
            Ok(Inspection::InUse) => return FileJournal::InUse,
            Ok(Inspection::Removed | Inspection::Damaged) => {}
            Err(error) => log::warn!("could not inspect {}: {error}", journal.display()),
        }
    }
    FileJournal::None
}

pub fn discard(journal: &Path) -> io::Result<()> {
    let file = File::open(journal)?;
    file.try_lock().map_err(io::Error::from)?;
    fs::remove_file(journal)
}
