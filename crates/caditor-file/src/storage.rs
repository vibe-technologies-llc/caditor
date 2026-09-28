use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError},
    thread,
    time::Duration,
};

use caditor_document::Document;

use crate::{
    journal::{JournalEntry, encode_entry, encode_journal},
    paths, reason,
    save::{self, SaveOptions, sync_parent, temporary_sibling},
};

const PREDECESSOR_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageConfig {
    pub recovery_dir: Option<PathBuf>,
}

pub struct Start {
    pub file: Option<PathBuf>,
    pub base: Document,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Saved {
        ticket: u64,
        path: PathBuf,
        backup: Option<PathBuf>,
    },
    SaveFailed {
        ticket: u64,
        path: PathBuf,
        reason: String,
    },
    JournalFailed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the storage worker stopped")]
pub struct StorageStopped;

enum Command {
    Record(JournalEntry),
    Save(SaveRequest),
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
                    base,
                    entries,
                    replaces,
                    after,
                } = start;
                if let Some(predecessor) = after
                    && !predecessor.wait(PREDECESSOR_TIMEOUT)
                {
                    log::warn!("the previous storage worker did not finish in time");
                }
                let mut worker = Worker {
                    untitled: config.recovery_dir.as_deref().map(paths::untitled_journal),
                    recovery_dir: config.recovery_dir,
                    file,
                    journal: None,
                    reports: sender,
                    wake: Box::new(wake),
                    unsynced: false,
                };
                worker.start(&base, &entries, replaces.as_deref());
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
    file: Option<PathBuf>,
    journal: Option<OpenJournal>,
    reports: Sender<Report>,
    wake: Box<dyn Fn() + Send>,
    unsynced: bool,
}

enum Flow {
    Continue,
    Stop,
}

impl Worker {
    fn start(&mut self, base: &Document, entries: &[JournalEntry], replaces: Option<&Path>) {
        self.journal = self.create_journal(base, entries);
        if let Some(replaced) = replaces
            && self.journal.as_ref().map(|journal| journal.path.as_path()) != Some(replaced)
        {
            remove_journal(replaced);
        }
    }

    fn run(&mut self, queue: &Receiver<Command>) {
        while let Ok(command) = queue.recv() {
            let mut next = Some(command);
            while let Some(command) = next.take() {
                if let Flow::Stop = self.handle(command) {
                    return;
                }
                next = queue.try_recv().ok();
            }
            self.sync();
        }
        self.sync();
    }

    fn handle(&mut self, command: Command) -> Flow {
        match command {
            Command::Record(entry) => self.append(&entry),
            Command::Save(request) => self.save(request),
            Command::Flush(done) => {
                self.sync();
                let _ = done.send(());
            }
            Command::Close { discard, done } => {
                self.sync();
                if let Some(journal) = self.journal.take()
                    && discard
                {
                    remove_journal(&journal.path);
                }
                let _ = done.send(());
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    fn append(&mut self, entry: &JournalEntry) {
        let Some(journal) = &mut self.journal else {
            return;
        };
        let written = encode_entry(entry)
            .map_err(io::Error::other)
            .and_then(|chunk| journal.file.write_all(&chunk));
        match written {
            Ok(()) => self.unsynced = true,
            Err(error) => {
                self.journal = None;
                self.report(Report::JournalFailed {
                    reason: reason::writing(&error),
                });
            }
        }
    }

    fn sync(&mut self) {
        if !self.unsynced {
            return;
        }
        self.unsynced = false;
        if let Some(journal) = &self.journal
            && let Err(error) = journal.file.sync_data()
        {
            self.journal = None;
            self.report(Report::JournalFailed {
                reason: reason::writing(&error),
            });
        }
    }

    fn save(&mut self, request: SaveRequest) {
        self.sync();
        let options = SaveOptions {
            keep_original: request.keep_original,
            history_from: self.file.as_deref(),
            label: request.label.as_deref(),
        };
        let report = match save::save_with(&request.document, &request.path, &options) {
            Ok(backup) => {
                self.file = Some(request.path.clone());
                let previous = self.journal.take();
                self.journal = self.create_journal(&request.document, &[]);
                if let Some(previous) = previous
                    && self.journal.as_ref().map(|journal| &journal.path) != Some(&previous.path)
                {
                    remove_journal(&previous.path);
                }
                Report::Saved {
                    ticket: request.ticket,
                    path: request.path,
                    backup,
                }
            }
            Err(error) => Report::SaveFailed {
                ticket: request.ticket,
                path: request.path,
                reason: error.reason,
            },
        };
        self.report(report);
    }

    fn create_journal(&mut self, base: &Document, entries: &[JournalEntry]) -> Option<OpenJournal> {
        let contents = match encode_journal(self.file.as_deref(), base, entries) {
            Ok(contents) => contents,
            Err(error) => {
                log::error!("could not encode the recovery journal: {error}");
                self.report(Report::JournalFailed {
                    reason: "the model could not be converted for the recovery journal".to_owned(),
                });
                return None;
            }
        };
        let mut failure = None;
        for candidate in self.journal_candidates() {
            match write_locked(&candidate, &contents, self.recovery_dir.as_deref()) {
                Ok(file) => {
                    return Some(OpenJournal {
                        path: candidate,
                        file,
                    });
                }
                Err(error) => {
                    log::warn!("could not create {}: {error}", candidate.display());
                    failure = Some(error);
                }
            }
        }
        if let Some(error) = failure {
            self.report(Report::JournalFailed {
                reason: reason::writing(&error),
            });
        }
        None
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

fn write_locked(path: &Path, contents: &[u8], recovery_dir: Option<&Path>) -> io::Result<File> {
    if let Some(dir) = recovery_dir
        && path.starts_with(dir)
    {
        fs::create_dir_all(dir)?;
    }
    let temporary = temporary_sibling(path)?;
    let written = write_locked_then_rename(&temporary, path, contents);
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn write_locked_then_rename(temporary: &Path, path: &Path, contents: &[u8]) -> io::Result<File> {
    let mut file = File::create_new(temporary)?;
    file.try_lock().map_err(io::Error::from)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    sync_parent(path)?;
    Ok(file)
}

fn remove_journal(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("could not remove {}: {error}", path.display()),
    }
}
