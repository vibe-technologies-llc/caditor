use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use caditor_document::Document;
use parking_lot::{Condvar, Mutex};

use crate::{
    binary::{FileDigest, Retention},
    journal::{
        JournalEntry, JournalHead, Logged, Mirror, WrittenPictures, encode_entry_sharing,
        encode_journal_noting,
    },
    load::VersionDigest,
    lock::{holds, install, locked_elsewhere, remove_held, remove_unheld},
    os, paths,
    preview::PreviewSlot,
    reason::WriteFailure,
    recovery::{mark_journal, unmark_journal},
    save::{
        self, KeepVersionError, SaveError, SaveOptions, Saved, remove_orphaned_temporaries,
        same_file_blocking, sync_parent, temporary_sibling,
    },
};

const PREDECESSOR_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const RETRY_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const RETRY_CEILING: Duration = Duration::from_secs(30);
const LONGEST_DOUBLING: u32 = 10;
const REBASE_JOURNAL_AFTER: u64 = 32 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageConfig {
    pub recovery_dir: Option<PathBuf>,
    pub rebase_journal_after: u64,
    #[cfg(test)]
    pub(crate) jobs: crate::tests::JobGate,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            recovery_dir: None,
            rebase_journal_after: REBASE_JOURNAL_AFTER,
            #[cfg(test)]
            jobs: crate::tests::JobGate::default(),
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

#[derive(Clone, Debug, PartialEq)]
pub struct SaveRequest {
    pub ticket: u64,
    pub document: Document,
    pub path: PathBuf,
    pub keep_original: bool,
    pub label: Option<String>,
    pub replace_outside_changes: bool,
    pub retention: Retention,
    pub preview: Option<PreviewSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepRequest {
    pub path: PathBuf,
    pub index: usize,
    pub digest: VersionDigest,
    pub kept: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the storage worker stopped")]
pub struct StorageStopped;

enum Command {
    Record(JournalEntry),
    Save(Box<SaveRequest>),
    Keep(KeepRequest),
    Flush(SyncSender<()>),
    Close {
        discard: bool,
        done: SyncSender<()>,
    },
    Finished(Box<Finished>),
    Released,
    #[cfg(test)]
    CopyJournal {
        to: PathBuf,
        done: SyncSender<io::Result<()>>,
    },
}

pub struct Storage {
    commands: Sender<Command>,
    reports: Receiver<Report>,
    handover: Handover,
    untitled: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub struct Handover(Arc<HandoverState>);

#[derive(Default)]
struct HandoverState {
    predecessor_gone: Mutex<bool>,
    changed: Condvar,
}

impl Handover {
    fn complete(&self) {
        *self.0.predecessor_gone.lock() = true;
        self.0.changed.notify_all();
    }

    pub fn wait(&self, timeout: Duration) -> bool {
        let mut gone = self.0.predecessor_gone.lock();

        self.0
            .changed
            .wait_while_for(&mut gone, |gone| !*gone, timeout);
        *gone
    }
}

impl Storage {
    pub fn spawn(
        config: StorageConfig,
        start: Start,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let (commands, queue) = mpsc::channel();
        let jobs = commands.clone();
        let (sender, reports) = mpsc::channel();
        let handover = Handover::default();
        let handed_over = handover.clone();
        let untitled = config.recovery_dir.as_deref().map(paths::untitled_journal);
        let worker_untitled = untitled.clone();

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
                handed_over.complete();

                let mut mirror = Mirror::new(base.clone());

                let entries = entries.into_iter().map(|entry| mirror.log(entry)).collect();
                let mut worker = Worker {
                    untitled: worker_untitled,
                    recovery_dir: config.recovery_dir,
                    rebase_after: config.rebase_journal_after,
                    file,
                    on_disk,
                    loaded_with_problems,
                    base,
                    folded,
                    entries,
                    mirror,
                    pictures: WrittenPictures::default(),
                    written: 0,
                    appended: 0,
                    replaces,
                    journal: None,
                    protected: false,
                    failure_reported: false,
                    next_retry: Instant::now(),
                    backoff: Backoff::default(),
                    reports: sender,
                    wake: Box::new(wake),
                    unsynced: false,
                    jobs: Some(jobs),
                    busy: None,
                    #[cfg(test)]
                    gate: config.jobs,
                };

                worker.rewrite();
                worker.run(&queue);
            })?;
        Ok(Self {
            commands,
            reports,
            handover,
            untitled,
        })
    }

    pub fn untitled_journal(&self) -> Option<&Path> {
        self.untitled.as_deref()
    }

    pub fn handover(&self) -> Handover {
        self.handover.clone()
    }

    pub fn record(&self, entry: JournalEntry) -> Result<(), StorageStopped> {
        self.send(Command::Record(entry))
    }

    pub fn save(&self, request: SaveRequest) -> Result<(), StorageStopped> {
        self.send(Command::Save(Box::new(request)))
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

impl Drop for Storage {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Released);
    }
}

#[cfg(feature = "test-support")]
impl Storage {
    pub fn stop(&self) {
        let (done, _finished) = mpsc::sync_channel(1);
        let _ = self.send(Command::Close {
            discard: false,
            done,
        });
        loop {
            let (done, _flushed) = mpsc::sync_channel(1);

            if self.send(Command::Flush(done)).is_err() {
                return;
            }
            thread::yield_now();
        }
    }
}

#[cfg(test)]
impl Storage {
    pub(crate) fn copy_journal(&self, to: &Path) -> io::Result<()> {
        let (done, copied) = mpsc::sync_channel(1);

        self.send(Command::CopyJournal {
            to: to.to_owned(),
            done,
        })
        .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        copied
            .recv()
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Backoff {
    failures: u32,
}

impl Backoff {
    pub(crate) fn next_delay(&mut self) -> Duration {
        let doublings = self.failures.min(LONGEST_DOUBLING);

        self.failures = self.failures.saturating_add(1);
        RETRY_INTERVAL
            .saturating_mul(1 << doublings)
            .min(RETRY_CEILING)
    }

    pub(crate) fn reset(&mut self) {
        self.failures = 0;
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
    pictures: WrittenPictures,
    written: u64,
    appended: u64,
    replaces: Option<PathBuf>,
    journal: Option<OpenJournal>,
    protected: bool,
    failure_reported: bool,
    next_retry: Instant,
    backoff: Backoff,
    reports: Sender<Report>,
    wake: Box<dyn Fn() + Send>,
    unsynced: bool,
    jobs: Option<Sender<Command>>,
    busy: Option<Busy>,
    #[cfg(test)]
    gate: crate::tests::JobGate,
}

#[derive(Default)]
struct Busy {
    since: Vec<JournalEntry>,
    waiting: VecDeque<Queued>,
}

enum Queued {
    Command(Command),
    Journaled(JournalEntry),
}

enum Job {
    Save(SaveJob),
    Keep {
        request: KeepRequest,
        unless_changed_from: Option<FileDigest>,
    },
    Rebase(Box<RebaseJob>),
}

struct RebaseJob {
    entries: usize,
    document: Document,
    head: OwnedHead,
    plan: JournalPlan,
}

struct SaveJob {
    request: Box<SaveRequest>,
    history_from: Option<PathBuf>,
    unless_changed_from: Option<FileDigest>,
    plan: JournalPlan,
}

enum Finished {
    Saved {
        request: Box<SaveRequest>,
        saved: Result<Saved, SaveError>,
        prepared: Option<PreparedJournal>,
    },
    Kept {
        request: KeepRequest,
        result: Result<(), KeepVersionError>,
    },
    Rebased {
        entries: usize,
        document: Document,
        prepared: Option<PreparedJournal>,
    },
    Stopped(Stopped),
}

enum Stopped {
    Save { ticket: u64, path: PathBuf },
    Keep { path: PathBuf },
    Rebase,
}

impl Job {
    fn stopped(&self) -> Stopped {
        match self {
            Self::Save(job) => Stopped::Save {
                ticket: job.request.ticket,
                path: job.request.path.clone(),
            },
            Self::Keep { request, .. } => Stopped::Keep {
                path: request.path.clone(),
            },
            Self::Rebase(_) => Stopped::Rebase,
        }
    }

    fn run_contained(self, before: impl FnOnce()) -> Finished {
        let stopped = self.stopped();
        let ran = panic::catch_unwind(AssertUnwindSafe(|| {
            before();
            self.run()
        }));
        ran.unwrap_or_else(|_| {
            log::error!("a background save, version change or journal rebase panicked");
            Finished::Stopped(stopped)
        })
    }

    fn run(self) -> Finished {
        match self {
            Self::Save(job) => job.run(),
            Self::Keep {
                request,
                unless_changed_from,
            } => {
                let result = save::set_version_kept(
                    &request.path,
                    request.index,
                    &request.digest,
                    request.kept,
                    unless_changed_from.as_ref(),
                );
                Finished::Kept { request, result }
            }
            Self::Rebase(job) => {
                let RebaseJob {
                    entries,
                    document,
                    head,
                    plan,
                } = *job;

                let prepared = plan.prepare(&head.borrowed(), &document, head.file.as_deref());
                Finished::Rebased {
                    entries,
                    document,
                    prepared,
                }
            }
        }
    }
}

impl SaveJob {
    fn run(self) -> Finished {
        let Self {
            request,
            history_from,
            unless_changed_from,
            plan,
        } = self;

        let options = SaveOptions {
            keep_original: request.keep_original,
            history_from: history_from.as_deref(),
            label: request.label.as_deref(),
            unless_changed_from: unless_changed_from.as_ref(),
            retention: request.retention,
            preview: request.preview.as_ref(),
        };
        let saved = save::save_with(&request.document, &request.path, &options);
        let prepared = saved.as_ref().ok().and_then(|saved| {
            let head = JournalHead {
                file: Some(&request.path),
                on_disk: Some(&saved.digest),
                loaded_with_problems: false,
                folded: 0,
            };
            plan.prepare(&head, &request.document, Some(&request.path))
        });
        Finished::Saved {
            request,
            saved,
            prepared,
        }
    }
}

struct OwnedHead {
    file: Option<PathBuf>,
    on_disk: Option<FileDigest>,
    loaded_with_problems: bool,
    folded: usize,
}

impl OwnedHead {
    fn borrowed(&self) -> JournalHead<'_> {
        JournalHead {
            file: self.file.as_deref(),
            on_disk: self.on_disk.as_ref(),
            loaded_with_problems: self.loaded_with_problems,
            folded: self.folded,
        }
    }
}

struct JournalPlan {
    candidates: Vec<PathBuf>,
    own: Option<PathBuf>,
    recovery_dir: Option<PathBuf>,
}

struct PreparedJournal {
    target: PathBuf,
    staged: Staged,
    length: usize,
    pictures: WrittenPictures,
}

impl JournalPlan {
    fn prepare(
        &self,
        head: &JournalHead<'_>,
        base: &Document,
        model: Option<&Path>,
    ) -> Option<PreparedJournal> {
        let mut pictures = WrittenPictures::default();

        let contents = match encode_journal_noting(head, base, &[], &mut pictures) {
            Ok(contents) => contents,
            Err(error) => {
                log::error!("could not encode the recovery journal: {error}");
                return None;
            }
        };
        let permissions = permissions_of(model);

        for candidate in &self.candidates {
            if self.own.as_ref() != Some(candidate) && locked_elsewhere(candidate) {
                continue;
            }
            match stage(
                candidate,
                &contents,
                permissions.as_ref(),
                self.recovery_dir.as_deref(),
            ) {
                Ok(staged) => {
                    return Some(PreparedJournal {
                        target: candidate.clone(),
                        staged,
                        length: contents.len(),
                        pictures,
                    });
                }
                Err(error) => log::warn!("could not prepare {}: {error}", candidate.display()),
            }
        }
        None
    }
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
            Command::Record(entry) => {
                self.note_recorded(&entry);
                self.append(entry);
            }
            Command::Flush(done) => {
                self.sync();
                if !self.protected {
                    self.retry();
                }

                let _ = done.send(());
            }
            Command::Finished(finished) => return self.finish(*finished),
            Command::Released => self.jobs = None,
            #[cfg(test)]
            Command::CopyJournal { to, done } => {
                self.sync();

                let _ = done.send(self.copy_journal(&to));
            }
            command @ (Command::Save(_) | Command::Keep(_) | Command::Close { .. })
                if self.busy.is_some() =>
            {
                if let Some(busy) = &mut self.busy {
                    busy.waiting.push_back(Queued::Command(command));
                }
            }
            Command::Save(request) => self.save(request),
            Command::Keep(request) => self.keep_version(request),
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

    fn note_recorded(&mut self, entry: &JournalEntry) {
        let Some(busy) = &mut self.busy else {
            return;
        };

        busy.since.push(entry.clone());
        if !busy.waiting.is_empty() {
            busy.waiting.push_back(Queued::Journaled(entry.clone()));
        }
    }

    fn start(&mut self, job: Job) {
        let slot = Arc::new(Mutex::new(Some(job)));
        let spawned = self.jobs.clone().map(|finished| {
            let slot = Arc::clone(&slot);
            let before = self.before_job();
            thread::Builder::new()
                .name("saving".to_owned())
                .spawn(move || {
                    let job = slot.lock().take();
                    if let Some(job) = job {
                        let done = job.run_contained(before);
                        let _ = finished.send(Command::Finished(Box::new(done)));
                    }
                })
        });

        match spawned {
            Some(Ok(_)) => {
                self.busy = Some(Busy::default());
                return;
            }
            Some(Err(error)) => log::warn!("could not save in the background: {error}"),
            None => {}
        }

        let job = slot.lock().take();
        if let Some(job) = job {
            let done = job.run_contained(self.before_job());

            self.complete(done, Vec::new());
        }
    }

    #[cfg(test)]
    fn before_job(&self) -> impl FnOnce() + Send + 'static {
        let gate = self.gate.clone();
        move || gate.pass()
    }

    #[cfg(not(test))]
    fn before_job(&self) -> impl FnOnce() + Send + 'static {
        || {}
    }

    fn finish(&mut self, finished: Finished) -> Flow {
        let Busy { since, waiting } = self.busy.take().unwrap_or_default();

        self.complete(finished, since);
        for queued in waiting {
            match queued {
                Queued::Journaled(entry) => self.note_recorded(&entry),
                Queued::Command(command) => {
                    if let Flow::Stop = self.handle(command) {
                        return Flow::Stop;
                    }
                }
            }
        }
        Flow::Continue
    }

    fn complete(&mut self, finished: Finished, since: Vec<JournalEntry>) {
        match finished {
            Finished::Saved {
                request,
                saved: Ok(saved),
                prepared,
            } => {
                let SaveRequest {
                    ticket,
                    document,
                    path,
                    ..
                } = *request;

                self.file = Some(path.clone());
                self.on_disk = Some(saved.digest.clone());
                self.loaded_with_problems = false;
                self.folded = 0;
                self.restart_from(document, since);
                self.install_prepared(prepared);
                self.report(Report::Saved {
                    ticket,
                    path,
                    backup: saved.backup,
                    dropped_for_size: saved.dropped_for_size,
                    digest: saved.digest,
                });
            }
            Finished::Saved {
                request,
                saved: Err(SaveError::ChangedOnDisk),
                ..
            } => self.report(Report::ChangedOnDisk {
                ticket: request.ticket,
                path: request.path,
            }),
            Finished::Saved {
                request,
                saved: Err(error),
                ..
            } => self.report(Report::SaveFailed {
                ticket: request.ticket,
                path: request.path,
                error,
            }),
            Finished::Kept {
                request: KeepRequest {
                    path, index, kept, ..
                },
                result,
            } => self.report(match result {
                Ok(()) => Report::VersionKept { path, index, kept },
                Err(error) => Report::KeepFailed { path, error },
            }),
            Finished::Rebased {
                entries,
                document,
                prepared,
            } => {
                self.folded = self.folded.saturating_add(entries);
                self.restart_from(document.clone(), since);
                self.install_prepared(prepared);
                self.report(Report::Rebased {
                    entries,
                    base: document,
                });
            }
            Finished::Stopped(Stopped::Save { ticket, path }) => {
                self.report(Report::SaveFailed {
                    ticket,
                    path,
                    error: SaveError::StoppedUnexpectedly,
                });
            }
            Finished::Stopped(Stopped::Keep { path }) => self.report(Report::KeepFailed {
                path,
                error: KeepVersionError::StoppedUnexpectedly,
            }),
            Finished::Stopped(Stopped::Rebase) => self.appended = 0,
        }
    }

    fn restart_from(&mut self, base: Document, since: Vec<JournalEntry>) {
        let mut mirror = Mirror::new(base.clone());

        self.entries = since.into_iter().map(|entry| mirror.log(entry)).collect();
        self.mirror = mirror;
        self.base = base;
    }

    #[cfg(test)]
    fn copy_journal(&self, to: &Path) -> io::Result<()> {
        let journal = self
            .journal
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let mut file = &journal.file;
        let mut bytes = Vec::new();

        io::Seek::seek(&mut file, io::SeekFrom::Start(0))?;

        let read = io::Read::read_to_end(&mut file, &mut bytes);

        io::Seek::seek(&mut file, io::SeekFrom::End(0))?;
        read?;
        fs::write(to, bytes)
    }

    fn append(&mut self, entry: JournalEntry) {
        let logged = self.mirror.log(entry);
        let written = match &mut self.journal {
            Some(journal) if self.protected => encode_entry_sharing(&logged, &mut self.pictures)
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
            && self.busy.is_none()
            && !self.entries.is_empty()
            && self.mirror.document().is_some()
            && self.appended > self.rebase_after.max(self.written)
    }

    fn rebase(&mut self) {
        let Some(document) = self.mirror.document().cloned() else {
            return;
        };
        let entries = self.entries.len();
        let head = OwnedHead {
            file: self.file.clone(),
            on_disk: self.on_disk.clone(),
            loaded_with_problems: self.loaded_with_problems,
            folded: self.folded.saturating_add(entries),
        };
        let plan = self.plan(self.journal_candidates());

        self.start(Job::Rebase(Box::new(RebaseJob {
            entries,
            document,
            head,
            plan,
        })));
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
        self.next_retry = Instant::now() + self.backoff.next_delay();
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

    fn save(&mut self, mut request: Box<SaveRequest>) {
        self.sync();

        let open_model = self.is_open_model(&request.path);

        request.keep_original &= open_model;
        if !open_model && self.opened_elsewhere(&request.path) {
            self.report(Report::SaveFailed {
                ticket: request.ticket,
                path: request.path,
                error: SaveError::OpenInAnotherWindow,
            });
            return;
        }

        let plan = self.plan(paths::journals_for(
            &request.path,
            self.recovery_dir.as_deref(),
        ));
        let unless_changed_from = self
            .on_disk
            .clone()
            .filter(|_| !request.replace_outside_changes);

        self.start(Job::Save(SaveJob {
            request,
            history_from: self.file.clone(),
            unless_changed_from,
            plan,
        }));
    }

    fn keep_version(&mut self, request: KeepRequest) {
        self.sync();
        if !self.is_open_model(&request.path) {
            self.report(Report::KeepFailed {
                path: request.path,
                error: KeepVersionError::NotTheOpenModel,
            });
            return;
        }

        let unless_changed_from = self.on_disk.clone();

        self.start(Job::Keep {
            request,
            unless_changed_from,
        });
    }

    fn plan(&self, candidates: Vec<PathBuf>) -> JournalPlan {
        JournalPlan {
            candidates,
            own: self
                .journal
                .as_ref()
                .filter(|journal| holds(&journal.file, &journal.path))
                .map(|journal| journal.path.clone()),
            recovery_dir: self.recovery_dir.clone(),
        }
    }

    fn opened_elsewhere(&self, file: &Path) -> bool {
        paths::journals_for(file, self.recovery_dir.as_deref())
            .iter()
            .filter(|candidate| !self.owns(candidate))
            .any(|candidate| locked_elsewhere(candidate))
    }

    fn is_open_model(&self, path: &Path) -> bool {
        self.file
            .as_deref()
            .is_some_and(|file| same_file_blocking(file, path))
    }

    fn owns(&self, path: &Path) -> bool {
        self.journal.as_ref().is_some_and(|journal| {
            same_file_blocking(&journal.path, path) && holds(&journal.file, &journal.path)
        })
    }

    fn own_journal_at(&self, path: &Path) -> Option<&File> {
        self.journal
            .as_ref()
            .filter(|journal| {
                same_file_blocking(&journal.path, path) && holds(&journal.file, &journal.path)
            })
            .map(|journal| &journal.file)
    }

    fn journal_path(&self) -> Option<&Path> {
        self.journal.as_ref().map(|journal| journal.path.as_path())
    }

    fn install_prepared(&mut self, prepared: Option<PreparedJournal>) {
        let Some(PreparedJournal {
            target,
            staged,
            length,
            mut pictures,
        }) = prepared
        else {
            self.rewrite();
            return;
        };
        let later = self
            .entries
            .iter()
            .map(|entry| encode_entry_sharing(entry, &mut pictures))
            .collect::<Result<Vec<_>, _>>();

        let later = match later {
            Ok(chunks) => chunks.concat(),
            Err(error) => {
                log::error!("could not encode the changes made while saving: {error}");
                staged.discard();
                self.rewrite();
                return;
            }
        };
        let installed = staged.install(&target, self.own_journal_at(&target), &later);
        match installed {
            Ok(file) => {
                self.pictures = pictures;
                self.adopt(target, file, length.saturating_add(later.len()));
            }
            Err(error) => {
                log::warn!("could not put {} in place: {error}", target.display());
                self.rewrite();
            }
        }
    }

    fn rewrite(&mut self) {
        let candidates = self.journal_candidates();

        if candidates.is_empty() {
            self.fail(JournalFailure::NoFolder);
            return;
        }

        let usable: Vec<PathBuf> = candidates
            .iter()
            .filter(|candidate| {
                let usable =
                    self.own_journal_at(candidate).is_some() || !locked_elsewhere(candidate);

                if !usable {
                    log::warn!("{} belongs to another caditor window", candidate.display());
                }
                usable
            })
            .cloned()
            .collect();

        if usable.is_empty() {
            self.drop_stale_journal(&candidates);
            self.fail(JournalFailure::InUseElsewhere);
            return;
        }

        let head = JournalHead {
            file: self.file.as_deref(),
            on_disk: self.on_disk.as_ref(),
            loaded_with_problems: self.loaded_with_problems,
            folded: self.folded,
        };

        #[cfg(test)]
        self.gate.note_encode();

        let encoded = encode_journal_noting(&head, &self.base, &self.entries, &mut self.pictures);
        let contents = match encoded {
            Ok(contents) => contents,
            Err(error) => {
                log::error!("could not encode the recovery journal: {error}");
                self.fail(JournalFailure::Unconvertible);
                return;
            }
        };
        let permissions = permissions_of(self.file.as_deref());
        let mut failure = None;

        for candidate in usable {
            let own = self.own_journal_at(&candidate);

            let written = write_locked(
                &candidate,
                &contents,
                permissions.as_ref(),
                self.recovery_dir.as_deref(),
                own,
            );
            match written {
                Ok(file) => {
                    self.adopt(candidate, file, contents.len());
                    return;
                }
                Err(error) => {
                    log::warn!("could not create {}: {error}", candidate.display());
                    failure = Some(error);
                }
            }
        }
        self.drop_stale_journal(&candidates);
        self.fail(failure.map_or(JournalFailure::InUseElsewhere, |error| {
            JournalFailure::Writing(WriteFailure::of(&error))
        }));
    }

    fn drop_stale_journal(&mut self, candidates: &[PathBuf]) {
        if let Some(stale) = self.journal.take_if(|previous| {
            !candidates
                .iter()
                .any(|candidate| same_file_blocking(candidate, &previous.path))
        }) {
            self.remove_own(&stale);
        }
    }

    fn adopt(&mut self, path: PathBuf, file: File, length: usize) {
        self.mark_adjacent(&path);

        let previous = self.journal.replace(OpenJournal { path, file });

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
        self.backoff.reset();
        self.written = u64::try_from(length).unwrap_or(u64::MAX);
        self.appended = 0;
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

fn permissions_of(model: Option<&Path>) -> Option<Permissions> {
    model
        .and_then(|model| fs::metadata(model).ok())
        .and_then(|metadata| os::inherited_permissions(&metadata))
}

struct Staged {
    temporary: PathBuf,
    file: File,
}

impl Staged {
    fn install(self, path: &Path, own: Option<&File>, later: &[u8]) -> io::Result<File> {
        let Self {
            temporary,
            mut file,
        } = self;
        match append_and_install(&mut file, &temporary, path, own, later) {
            Ok(()) => Ok(file),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(error)
            }
        }
    }

    fn discard(self) {
        if let Err(error) = fs::remove_file(&self.temporary) {
            log::warn!("could not remove {}: {error}", self.temporary.display());
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
    stage(path, contents, permissions, recovery_dir)?.install(path, own, &[])
}

fn stage(
    path: &Path,
    contents: &[u8],
    permissions: Option<&Permissions>,
    recovery_dir: Option<&Path>,
) -> io::Result<Staged> {
    if let Some(dir) = recovery_dir
        && path.starts_with(dir)
    {
        fs::create_dir_all(dir)?;
    }

    let temporary = temporary_sibling(path)?;
    match write_temporary(&temporary, contents, permissions) {
        Ok(file) => Ok(Staged { temporary, file }),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_temporary(
    temporary: &Path,
    contents: &[u8],
    permissions: Option<&Permissions>,
) -> io::Result<File> {
    let mut file = os::hidden(os::private(
        OpenOptions::new().read(true).write(true).create_new(true),
    ))
    .open(temporary)?;

    if let Some(permissions) = permissions {
        file.set_permissions(permissions.clone())?;
    }
    file.try_lock().map_err(io::Error::from)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(file)
}

fn append_and_install(
    file: &mut File,
    temporary: &Path,
    path: &Path,
    own: Option<&File>,
    later: &[u8],
) -> io::Result<()> {
    if !later.is_empty() {
        file.write_all(later)?;
        file.sync_data()?;
    }
    install(temporary, path, own)?;
    if let Err(error) = sync_parent(path) {
        log::warn!(
            "the folder of {} could not be synced after the journal was renamed into place: \
             {error}",
            path.display()
        );
    }
    remove_orphaned_temporaries(path);
    Ok(())
}
