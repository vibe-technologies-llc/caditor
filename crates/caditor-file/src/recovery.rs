use std::{
    cmp::Reverse,
    collections::BTreeSet,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use caditor_document::{Document, Editor};

use crate::{
    journal::{JournalEntry, decode_journal, replay},
    load::load,
    lock::{Location, in_use, location, lock_existing},
    paths::{self, JOURNAL_EXTENSION},
    read::read_open,
    save::sync_parent,
};

#[derive(Debug, Clone)]
pub struct Recovered {
    pub journal: PathBuf,
    pub file: Option<PathBuf>,
    pub loaded_with_problems: bool,
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
    SetAside(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unreadable {
    Keep,
    SetAside,
}

pub fn inspect(journal: &Path) -> io::Result<Inspection> {
    inspect_as(journal, Unreadable::Keep)
}

fn inspect_as(journal: &Path, unreadable: Unreadable) -> io::Result<Inspection> {
    let Some(file) = lock_existing(journal)? else {
        return Ok(Inspection::InUse);
    };
    let bytes = read_open(&file)?;
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok();

    let Ok(contents) = decode_journal(&bytes) else {
        log::warn!("{} is damaged and cannot be recovered", journal.display());
        return match unreadable {
            Unreadable::Keep => Ok(Inspection::Damaged),
            Unreadable::SetAside => set_aside_locked(&file, journal),
        };
    };
    let replayed = replay(contents.base.clone(), contents.entries);
    let document = replayed.editor.document();
    let unapplied = contents.unreadable_entries + usize::from(replayed.stopped_early);
    let unchanged = || document.same_content(&contents.base);
    let already_saved = || {
        contents
            .file
            .as_deref()
            .and_then(|file| load(file).ok())
            .is_some_and(|loaded| loaded.issues.is_empty() && loaded.document == *document)
    };
    if unapplied == 0 && (unchanged() || already_saved()) {
        return remove_locked(&file, journal);
    }

    let mut issues = contents.issues;
    if unapplied > 0 {
        issues.push(
            "The most recent changes were damaged or made by a newer version of caditor and could \
             not be recovered; everything before them was."
                .to_owned(),
        );
    }
    Ok(Inspection::Recoverable(Box::new(Recovered {
        journal: journal.to_path_buf(),
        file: contents.file,
        loaded_with_problems: contents.loaded_with_problems,
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
            Ok(
                Inspection::Removed
                | Inspection::InUse
                | Inspection::Damaged
                | Inspection::SetAside(_),
            ) => None,
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
    SetAside(Vec<PathBuf>),
}

pub fn journal_for(file: &Path, recovery_dir: Option<&Path>) -> FileJournal {
    let mut set_aside = Vec::new();
    for journal in paths::journals_for(file, recovery_dir) {
        if !journal.is_file() {
            continue;
        }
        match inspect_as(&journal, Unreadable::SetAside) {
            Ok(Inspection::Recoverable(mut recovered)) => {
                recovered.file = Some(file.to_path_buf());
                return FileJournal::Recoverable(recovered);
            }
            Ok(Inspection::InUse) => return FileJournal::InUse,
            Ok(Inspection::SetAside(kept)) => set_aside.push(kept),
            Ok(Inspection::Damaged) => {}
            Ok(Inspection::Removed) => {}
            Err(error) => log::warn!("could not inspect {}: {error}", journal.display()),
        }
    }
    if set_aside.is_empty() {
        FileJournal::None
    } else {
        FileJournal::SetAside(set_aside)
    }
}

pub fn describe_set_aside(kept: &Path) -> String {
    let name = kept.file_name().map_or_else(
        || kept.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let place = kept
        .parent()
        .map(|dir| format!(" in {}", dir.display()))
        .unwrap_or_default();
    format!(
        "Unsaved changes from an earlier session could not be read, perhaps because a newer \
         version of caditor wrote them. They were kept as “{name}”{place}, and the model opened \
         as it was last saved."
    )
}

fn set_aside_locked(file: &File, journal: &Path) -> io::Result<Inspection> {
    match location(file, journal)? {
        Location::Gone => Ok(Inspection::Removed),
        Location::Replaced => Ok(Inspection::InUse),
        Location::Here => {
            let kept = paths::unreadable_journal(journal);
            fs::rename(journal, &kept)?;
            if let Err(error) = sync_parent(&kept) {
                log::warn!("could not sync the folder of {}: {error}", kept.display());
            }
            Ok(Inspection::SetAside(kept))
        }
    }
}

pub fn discard(journal: &Path) -> io::Result<()> {
    let file = lock_existing(journal)?.ok_or_else(in_use)?;
    match remove_locked(&file, journal)? {
        Inspection::Removed => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            "the changes are open in another caditor window",
        )),
    }
}

fn remove_locked(file: &File, journal: &Path) -> io::Result<Inspection> {
    match location(file, journal)? {
        Location::Gone => Ok(Inspection::Removed),
        Location::Replaced => Ok(Inspection::InUse),
        Location::Here => {
            fs::remove_file(journal)?;
            Ok(Inspection::Removed)
        }
    }
}
