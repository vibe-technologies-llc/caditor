use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use caditor_document::Document;

use crate::{
    binary::FileDigest,
    journal::{JournalEntry, JournalHead, Logged, Mirror, encode_entry, encode_journal},
    lock::{holds, install, locked_elsewhere, remove_held, remove_unheld},
    os, paths,
    reason::WriteFailure,
    recovery::{mark_journal, unmark_journal},
    save::{
        self, KeepVersionError, SaveError, SaveOptions, remove_orphaned_temporaries, sync_parent,
        temporary_sibling,
    },
};

const PREDECESSOR_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const REBASE_JOURNAL_AFTER: u64 = 32 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageConfig {
    pub recovery_dir: Option<PathBuf>,
    pub rebase_journal_after: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            recovery_dir: None,
            rebase_journal_after: REBASE_JOURNAL_AFTER,
        }
    }
}

pub struct Start {
    pub file: Option<PathBuf>,
    pub on_disk: Option<FileDigest>,
    pub loaded_with_problems: bool,
    pub base: Document,
    pub folded: usize,
    pub entries: Vec<JournalEntry>,
    pub replaces: Option<PathBuf>,
    pub after: Option<Closing>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveRequest {
    pub ticket: u64,
    pub document: Document,
    pub path: PathBuf,
    pub keep_original: bool,
    pub label: Option<String>,
    pub replace_outside_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepRequest {
    pub path: PathBuf,
    pub index: usize,
    pub kept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalFailure {
    #[error("{0}")]
    Writing(WriteFailure),
    #[error("the recovery file was moved or replaced")]
    Moved,
    #[error("there is no folder to keep it in")]
    NoFolder,
    #[error("the model could not be converted for the recovery journal")]
    Unconvertible,
    #[error("the recovery file is in use by another caditor window")]
    InUseElsewhere,
    #[error("the background writer could not start")]
    WriterUnavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Saved {
        ticket: u64,
        path: PathBuf,
        backup: Option<PathBuf>,
        dropped_for_size: usize,
        digest: FileDigest,
    },
    ChangedOnDisk {
        ticket: u64,
        path: PathBuf,
    },
    VersionKept {
        path: PathBuf,
        index: usize,
        kept: bool,
    },
    KeepFailed {
        path: PathBuf,
        error: KeepVersionError,
    },
    SaveFailed {
        ticket: u64,
        path: PathBuf,
        error: SaveError,
    },
    JournalFailed {
        failure: JournalFailure,
    },
    JournalRestored,
    Rebased {
        entries: usize,
        base: Document,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the storage worker stopped")]
pub struct StorageStopped;

enum Command {
    Record(JournalEntry),
    Save(SaveRequest),
    Keep(KeepRequest),
    Flush(SyncSender<()>),
    Close { discard: bool, done: SyncSender<()> },
}

pub struct Storage {
    commands: Sender<Command>,
    reports: Receiver<Report>,
}

impl Storage {
    pub fn spawn(
        config: StorageConfig,
        start: Start,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let (commands, queue) = mpsc::channel();
        let (sender, reports) = mpsc::channel();
        thread::Builder::new()
            .name("storage".to_owned())
            .spawn(move || {
                let Start {
                    file,
                    on_disk,
                    loaded_with_problems,
                    base,
                    folded,
                    entries,
                    replaces,
                    after,
                } = start;
                if let Some(predecessor) = after
                    && !predecessor.wait(PREDECESSOR_TIMEOUT)
                {
                    log::warn!("the previous storage worker did not finish in time");
                }
                let mut mirror = Mirror::new(base.clone());
                let entries = entries.into_iter().map(|entry| mirror.log(entry)).collect();
                let mut worker = Worker {
                    untitled: config.recovery_dir.as_deref().map(paths::untitled_journal),
                    recovery_dir: config.recovery_dir,
                    rebase_after: config.rebase_journal_after,
                    file,
                    on_disk,
                    loaded_with_problems,
                    base,
                    folded,
                    entries,
                    mirror,
                    written: 0,
                    appended: 0,
                    replaces,
                    journal: None,
                    protected: false,
                    failure_reported: false,
                    next_retry: Instant::now(),
                    reports: sender,
                    wake: Box::new(wake),
                    unsynced: false,
                };
                worker.rewrite();
                worker.run(&queue);
            })?;
        Ok(Self { commands, reports })
    }

    pub fn record(&self, entry: JournalEntry) -> Result<(), StorageStopped> {
        self.send(Command::Record(entry))
    }

    pub fn save(&self, request: SaveRequest) -> Result<(), StorageStopped> {
        self.send(Command::Save(request))
    }

    pub fn keep_version(&self, request: KeepRequest) -> Result<(), StorageStopped> {
        self.send(Command::Keep(request))
    }

    pub fn flusher(&self) -> Flusher {
        Flusher(self.commands.clone())
    }

    pub fn poll(&self) -> Result<Vec<Report>, StorageStopped> {
        let mut reports = Vec::new();
        loop {
            match self.reports.try_recv() {
                Ok(report) => reports.push(report),
                Err(TryRecvError::Empty) => return Ok(reports),
                Err(TryRecvError::Disconnected) if reports.is_empty() => {
                    return Err(StorageStopped);
                }
                Err(TryRecvError::Disconnected) => return Ok(reports),
            }
        }
    }

    pub fn close(self, discard_journal: bool) -> Closing {
        let (done, finished) = mpsc::sync_channel(1);
        let sent = self.send(Command::Close {
            discard: discard_journal,
            done,
        });
        Closing(sent.ok().map(|()| finished))
    }

    fn send(&self, command: Command) -> Result<(), StorageStopped> {
        self.commands.send(command).map_err(|_| StorageStopped)
    }
}

#[derive(Clone)]
pub struct Flusher(Sender<Command>);

impl Flusher {
    pub fn flush(&self, timeout: Duration) -> bool {
        let (done, flushed) = mpsc::sync_channel(1);
        self.0.send(Command::Flush(done)).is_ok() && flushed.recv_timeout(timeout).is_ok()
    }
}

pub struct Closing(Option<Receiver<()>>);

impl Closing {
    pub fn finished(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|finished| !matches!(finished.try_recv(), Err(TryRecvError::Empty)))
    }

    pub fn wait(self, timeout: Duration) -> bool {
        self.0.is_some_and(|finished| {
            !matches!(
                finished.recv_timeout(timeout),
                Err(RecvTimeoutError::Timeout)
            )
        })
    }
}

struct OpenJournal {
    path: PathBuf,
    file: File,
}

struct Worker {
    recovery_dir: Option<PathBuf>,
    untitled: Option<PathBuf>,
    rebase_after: u64,
    file: Option<PathBuf>,
    on_disk: Option<FileDigest>,
    loaded_with_problems: bool,
    base: Document,
    folded: usize,
    entries: Vec<Logged>,
    mirror: Mirror,
    written: u64,
    appended: u64,
    replaces: Option<PathBuf>,
    journal: Option<OpenJournal>,
    protected: bool,
    failure_reported: bool,
    next_retry: Instant,
    reports: Sender<Report>,
    wake: Box<dyn Fn() + Send>,
    unsynced: bool,
}

enum Flow {
    Continue,
    Stop,
}

impl Worker {
    fn run(&mut self, queue: &Receiver<Command>) {
        loop {
            let received = if self.protected || !self.has_place() {
                queue.recv().map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                queue.recv_timeout(self.next_retry.saturating_duration_since(Instant::now()))
            };
            match received {
                Ok(command) => {
                    let mut next = Some(command);
                    while let Some(command) = next.take() {
                        if let Flow::Stop = self.handle(command) {
                            return;
                        }
                        next = queue.try_recv().ok();
                    }
                    self.sync();
                    if self.retry_due() {
                        self.retry();
                    }
                    if self.rebase_due() {
                        self.rebase();
                    }
                }
                Err(RecvTimeoutError::Timeout) => self.retry(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.sync();
    }

    fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::Record(entry) => self.append(entry),
            Command::Save(request) => self.save(request),
            Command::Keep(request) => self.keep_version(request),
            Command::Flush(done) => {
                self.sync();
                if !self.protected {
                    self.retry();
                }
                let _ = done.send(());
            }
            Command::Close { discard, done } => {
                self.sync();
                if let Some(journal) = self.journal.take()
                    && discard
                {
                    self.remove_own(&journal);
                }
                let _ = done.send(());
                (self.wake)();
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    fn append(&mut self, entry: JournalEntry) {
        let logged = self.mirror.log(entry);
        let written = match &mut self.journal {
            Some(journal) if self.protected => encode_entry(&logged)
                .map_err(io::Error::other)
                .and_then(|chunk| {
                    journal.file.write_all(&chunk)?;
                    Ok(chunk.len())
                }),
            Some(_) | None => Ok(0),
        };
        self.entries.push(logged);
        match written {
            Ok(length) => {
                self.appended = self
                    .appended
                    .saturating_add(u64::try_from(length).unwrap_or(u64::MAX));
                self.unsynced = self.protected;
            }
            Err(error) => self.lose_protection(&error),
        }
    }

    fn rebase_due(&self) -> bool {
        self.protected
            && !self.entries.is_empty()
            && self.mirror.document().is_some()
            && self.appended > self.rebase_after.max(self.written)
    }

    fn rebase(&mut self) {
        let Some(document) = self.mirror.document().cloned() else {
            return;
        };
        let entries = self.entries.len();
        self.folded = self.folded.saturating_add(entries);
        self.entries.clear();
        self.base = document.clone();
        self.mirror = Mirror::new(document.clone());
        self.rewrite();
        self.report(Report::Rebased {
            entries,
            base: document,
        });
    }

    fn sync(&mut self) {
        if !self.unsynced {
            return;
        }
        self.unsynced = false;
        let Some(journal) = &self.journal else {
            return;
        };
        if let Err(error) = journal.file.sync_data() {
            self.lose_protection(&error);
        } else if !holds(&journal.file, &journal.path) {
            log::warn!(
                "{} was moved or replaced while this window wrote to it",
                journal.path.display()
            );
            self.fail(JournalFailure::Moved);
        }
    }

    fn lose_protection(&mut self, error: &io::Error) {
        log::warn!("the recovery journal could not be written: {error}");
        self.fail(JournalFailure::Writing(WriteFailure::of(error)));
    }

    fn fail(&mut self, failure: JournalFailure) {
        self.protected = false;
        self.unsynced = false;
        self.next_retry = Instant::now() + RETRY_INTERVAL;
        if !self.failure_reported {
            self.failure_reported = true;
            self.report(Report::JournalFailed { failure });
        }
    }

    fn retry(&mut self) {
        self.rewrite();
        if self.protected && self.failure_reported {
            self.failure_reported = false;
            self.report(Report::JournalRestored);
        }
    }

    fn save(&mut self, request: SaveRequest) {
        self.sync();
        if self.file.as_deref() != Some(request.path.as_path())
            && self.opened_elsewhere(&request.path)
        {
            self.report(Report::SaveFailed {
                ticket: request.ticket,
                path: request.path,
                error: SaveError::OpenInAnotherWindow,
            });
            return;
        }
        let options = SaveOptions {
            keep_original: request.keep_original,
            history_from: self.file.as_deref(),
            label: request.label.as_deref(),
            unless_changed_from: self
                .on_disk
                .as_ref()
                .filter(|_| !request.replace_outside_changes),
        };
        let report = match save::save_with(&request.document, &request.path, &options) {
            Ok(saved) => {
                self.file = Some(request.path.clone());
                self.on_disk = Some(saved.digest.clone());
                self.loaded_with_problems = false;
                self.mirror = Mirror::new(request.document.clone());
                self.base = request.document;
                self.folded = 0;
                self.entries.clear();
                self.rewrite();
                Report::Saved {
                    ticket: request.ticket,
                    path: request.path,
                    backup: saved.backup,
                    dropped_for_size: saved.dropped_for_size,
                    digest: saved.digest,
                }
            }
            Err(SaveError::ChangedOnDisk) => Report::ChangedOnDisk {
                ticket: request.ticket,
                path: request.path,
            },
            Err(error) => Report::SaveFailed {
                ticket: request.ticket,
                path: request.path,
                error,
            },
        };
        self.report(report);
    }

    fn keep_version(&mut self, request: KeepRequest) {
        self.sync();
        let KeepRequest { path, index, kept } = request;
        let result = if self.file.as_deref() == Some(path.as_path()) {
            save::set_version_kept(&path, index, kept, self.on_disk.as_ref())
        } else {
            Err(KeepVersionError::NotTheOpenModel)
        };
        self.report(match result {
            Ok(()) => Report::VersionKept { path, index, kept },
            Err(error) => Report::KeepFailed { path, error },
        });
    }

    fn opened_elsewhere(&self, file: &Path) -> bool {
        paths::journals_for(file, self.recovery_dir.as_deref())
            .iter()
            .filter(|candidate| !self.owns(candidate))
            .any(|candidate| locked_elsewhere(candidate))
    }

    fn owns(&self, path: &Path) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|journal| journal.path == path && holds(&journal.file, path))
    }

    fn journal_path(&self) -> Option<&Path> {
        self.journal.as_ref().map(|journal| journal.path.as_path())
    }

    fn rewrite(&mut self) {
        let candidates = self.journal_candidates();
        if candidates.is_empty() {
            self.fail(JournalFailure::NoFolder);
            return;
        }
        let head = JournalHead {
            file: self.file.as_deref(),
            on_disk: self.on_disk.as_ref(),
            loaded_with_problems: self.loaded_with_problems,
            folded: self.folded,
        };
        let contents = match encode_journal(&head, &self.base, &self.entries) {
            Ok(contents) => contents,
            Err(error) => {
                log::error!("could not encode the recovery journal: {error}");
                self.fail(JournalFailure::Unconvertible);
                return;
            }
        };
        let permissions = self
            .file
            .as_deref()
            .and_then(|file| fs::metadata(file).ok())
            .and_then(|metadata| os::inherited_permissions(&metadata));
        let mut failure = None;
        for candidate in candidates.iter().cloned() {
            let own = self
                .journal
                .as_ref()
                .filter(|journal| journal.path == candidate && holds(&journal.file, &candidate))
                .map(|journal| &journal.file);
            if own.is_none() && locked_elsewhere(&candidate) {
                log::warn!("{} belongs to another caditor window", candidate.display());
                continue;
            }
            let written = write_locked(
                &candidate,
                &contents,
                permissions.as_ref(),
                self.recovery_dir.as_deref(),
                own,
            );
            match written {
                Ok(file) => {
                    self.mark_adjacent(&candidate);
                    let previous = self.journal.replace(OpenJournal {
                        path: candidate,
                        file,
                    });
                    if let Some(previous) = previous
                        && self.journal_path() != Some(previous.path.as_path())
                    {
                        self.remove_own(&previous);
                    }
                    if let Some(replaced) = self.replaces.take()
                        && self.journal_path() != Some(replaced.as_path())
                    {
                        remove_unheld(&replaced);
                    }
                    self.protected = true;
                    self.unsynced = false;
                    self.written = u64::try_from(contents.len()).unwrap_or(u64::MAX);
                    self.appended = 0;
                    return;
                }
                Err(error) => {
                    log::warn!("could not create {}: {error}", candidate.display());
                    failure = Some(error);
                }
            }
        }
        if let Some(stale) = self
            .journal
            .take_if(|previous| !candidates.contains(&previous.path))
        {
            self.remove_own(&stale);
        }
        self.fail(failure.map_or(JournalFailure::InUseElsewhere, |error| {
            JournalFailure::Writing(WriteFailure::of(&error))
        }));
    }

    fn remove_own(&self, journal: &OpenJournal) {
        if !holds(&journal.file, &journal.path) {
            return;
        }
        remove_held(&journal.file, &journal.path);
        if let Some(recovery_dir) = self.recovery_dir.as_deref()
            && !journal.path.starts_with(recovery_dir)
        {
            unmark_journal(&journal.path, recovery_dir);
        }
    }

    fn mark_adjacent(&self, journal: &Path) {
        let Some(recovery_dir) = self.recovery_dir.as_deref() else {
            return;
        };
        if journal.starts_with(recovery_dir) {
            return;
        }
        if let Err(error) = mark_journal(journal, recovery_dir) {
            log::warn!(
                "could not remember where {} is kept: {error}",
                journal.display()
            );
        }
    }

    fn retry_due(&self) -> bool {
        !self.protected && self.has_place() && Instant::now() >= self.next_retry
    }

    fn has_place(&self) -> bool {
        self.file.is_some() || self.untitled.is_some()
    }

    fn journal_candidates(&self) -> Vec<PathBuf> {
        match &self.file {
            Some(file) => paths::journals_for(file, self.recovery_dir.as_deref()),
            None => self.untitled.iter().cloned().collect(),
        }
    }

    fn report(&self, report: Report) {
        if self.reports.send(report).is_ok() {
            (self.wake)();
        }
    }
}

fn write_locked(
    path: &Path,
    contents: &[u8],
    permissions: Option<&Permissions>,
    recovery_dir: Option<&Path>,
    own: Option<&File>,
) -> io::Result<File> {
    if let Some(dir) = recovery_dir
        && path.starts_with(dir)
    {
        fs::create_dir_all(dir)?;
    }
    let temporary = temporary_sibling(path)?;
    let written = write_locked_then_rename(&temporary, path, contents, permissions, own);
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn write_locked_then_rename(
    temporary: &Path,
    path: &Path,
    contents: &[u8],
    permissions: Option<&Permissions>,
    own: Option<&File>,
) -> io::Result<File> {
    let mut file =
        os::hidden(os::private(OpenOptions::new().write(true).create_new(true))).open(temporary)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions.clone())?;
    }
    file.try_lock().map_err(io::Error::from)?;
    file.write_all(contents)?;
    file.sync_all()?;
    install(temporary, path, own)?;
    if let Err(error) = sync_parent(path) {
        log::warn!(
            "the folder of {} could not be synced after the journal was renamed into place: \
             {error}",
            path.display()
        );
    }
    remove_orphaned_temporaries(path);
    Ok(file)
}
