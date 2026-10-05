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
    binary::FileDigest,
    journal::{JournalEntry, decode_journal, replay},
    load::load,
    lock::{Location, in_use, location, lock_existing},
    os,
    paths::{self, JOURNAL_EXTENSION, MARKER_EXTENSION},
    read::{read_file, read_open},
    save::{sweep_orphaned_temporaries, sync_parent, write_atomically},
};

#[derive(Debug, Clone)]
pub struct Recovered {
    pub journal: PathBuf,
    pub file: Option<PathBuf>,
    pub on_disk: Option<FileDigest>,
    pub loaded_with_problems: bool,
    pub base: Document,
    pub folded: usize,
    pub entries: Vec<JournalEntry>,
    pub editor: Editor,
    pub issues: Vec<String>,
    pub modified: Option<SystemTime>,
}

impl Recovered {
    pub fn changes(&self) -> usize {
        self.folded.saturating_add(self.entries.len())
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
    let unchanged = || contents.folded == 0 && document.same_content(&contents.base);
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
        on_disk: contents.on_disk,
        loaded_with_problems: contents.loaded_with_problems,
        base: contents.base,
        folded: contents.folded,
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
    let marked = recovery_dir.map(marked_journals).unwrap_or_default();
    prune_set_aside(recovery_dir, recent);
    if let Some(dir) = recovery_dir {
        sweep_orphaned_temporaries(dir);
    }
    let set_aside = set_aside_journals(recovery_dir, recent);
    let candidates: BTreeSet<PathBuf> = in_recovery_dir
        .chain(next_to_recent)
        .chain(marked)
        .chain(set_aside)
        .filter(|path| path.is_file())
        .collect();

    let mut recovered: Vec<Recovered> = candidates
        .iter()
        .filter_map(|journal| match inspect(journal) {
            Ok(Inspection::Recoverable(recovered)) => Some(*recovered),
            Ok(Inspection::Removed) => {
                if let Some(dir) = recovery_dir {
                    unmark_journal(journal, dir);
                }
                None
            }
            Ok(Inspection::InUse | Inspection::Damaged | Inspection::SetAside(_)) => None,
            Err(error) => {
                log::warn!("could not inspect {}: {error}", journal.display());
                None
            }
        })
        .collect();
    recovered.sort_by_key(|recovered| Reverse(recovered.modified));
    recovered
}

struct SetAside {
    path: PathBuf,
    at: u64,
}

fn set_aside_places(recovery_dir: Option<&Path>, recent: &[PathBuf]) -> Vec<SetAside> {
    let in_recovery_dir = recovery_dir
        .map(|dir| set_aside_in(dir, None))
        .unwrap_or_default();
    let next_to_recent = recent
        .iter()
        .filter_map(|file| paths::adjacent_journal(file))
        .flat_map(|journal| set_aside_from(&journal));
    in_recovery_dir.into_iter().chain(next_to_recent).collect()
}

fn set_aside_from(journal: &Path) -> Vec<SetAside> {
    let (Some(dir), Some(name)) = (journal.parent(), journal.file_name()) else {
        return Vec::new();
    };
    let mut prefix = name.to_os_string();
    prefix.push(".");
    set_aside_in(dir, Some(prefix.as_encoded_bytes()))
}

fn set_aside_in(dir: &Path, prefix: Option<&[u8]>) -> Vec<SetAside> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let named_as_wanted =
                prefix.is_none_or(|prefix| name.as_encoded_bytes().starts_with(prefix));
            let at = paths::set_aside_at(&name).filter(|_| named_as_wanted)?;
            Some(SetAside {
                path: entry.path(),
                at,
            })
        })
        .collect()
}

fn prune_set_aside(recovery_dir: Option<&Path>, recent: &[PathBuf]) {
    let now = paths::now_seconds();
    for set_aside in set_aside_places(recovery_dir, recent) {
        if now.saturating_sub(set_aside.at) <= paths::SET_ASIDE_KEPT_SECONDS {
            continue;
        }
        match fs::remove_file(&set_aside.path) {
            Ok(()) => log::info!(
                "removed {}, kept aside for too long",
                set_aside.path.display()
            ),
            Err(error) => log::warn!("could not remove {}: {error}", set_aside.path.display()),
        }
    }
}

fn set_aside_journals(recovery_dir: Option<&Path>, recent: &[PathBuf]) -> Vec<PathBuf> {
    set_aside_places(recovery_dir, recent)
        .into_iter()
        .map(|set_aside| set_aside.path)
        .collect()
}

pub(crate) fn mark_journal(journal: &Path, recovery_dir: &Path) -> io::Result<()> {
    let journal = std::path::absolute(journal)?;
    let marker = paths::journal_marker(&journal, recovery_dir);
    if marker.exists() {
        return Ok(());
    }
    fs::create_dir_all(recovery_dir)?;
    write_atomically(&marker, &os::path_bytes(journal.as_os_str()))
}

pub(crate) fn unmark_journal(journal: &Path, recovery_dir: &Path) {
    let Ok(journal) = std::path::absolute(journal) else {
        return;
    };
    let marker = paths::journal_marker(&journal, recovery_dir);
    match fs::remove_file(&marker) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("could not remove {}: {error}", marker.display()),
    }
}

fn marked_journals(recovery_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(recovery_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == MARKER_EXTENSION)
        })
        .filter_map(|marker| {
            let journal = PathBuf::from(os::path_from_bytes(read_file(&marker).ok()?));
            let usable = journal.is_absolute()
                && journal
                    .extension()
                    .is_some_and(|extension| extension == JOURNAL_EXTENSION)
                && paths::journal_marker(&journal, recovery_dir) == marker;
            match fs::symlink_metadata(&journal) {
                Ok(_) if usable => Some(journal),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Err(error) = fs::remove_file(&marker) {
                        log::warn!("could not remove {}: {error}", marker.display());
                    }
                    None
                }
                Ok(_) | Err(_) => None,
            }
        })
        .collect()
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
    if !set_aside.is_empty() {
        return FileJournal::SetAside(set_aside);
    }
    match readable_set_aside(file, recovery_dir) {
        Some(recovered) => FileJournal::Recoverable(recovered),
        None => FileJournal::None,
    }
}

fn readable_set_aside(file: &Path, recovery_dir: Option<&Path>) -> Option<Box<Recovered>> {
    let mut earlier: Vec<SetAside> = paths::journals_for(file, recovery_dir)
        .iter()
        .flat_map(|journal| set_aside_from(journal))
        .collect();
    earlier.sort_by_key(|set_aside| Reverse(set_aside.at));
    earlier
        .iter()
        .find_map(|set_aside| match inspect(&set_aside.path) {
            Ok(Inspection::Recoverable(mut recovered)) => {
                recovered.file = Some(file.to_path_buf());
                Some(recovered)
            }
            Ok(_) => None,
            Err(error) => {
                log::warn!("could not inspect {}: {error}", set_aside.path.display());
                None
            }
        })
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
         as it was last saved. For thirty days, a version of caditor that reads them offers to \
         restore them when it starts or opens this model."
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
