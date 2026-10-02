use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use parking_lot::Mutex;

use crate::{paths, read::open_file};

const LOGS: &str = "logs";
const PREFIX: &str = "caditor-";
const LOG_EXTENSION: &str = "log";
const PRIVATE_MODE: u32 = 0o600;
const TAIL: u64 = 512;
const ENDED: &str = "caditor ended this session.";
const REPORTED: &str = "This session ended unexpectedly; caditor said so when it next started.";
const CUT: &str = "The log reached its size limit, so later messages were left out.";
pub const LOGS_KEPT: usize = 10;
pub const MAX_LOG_SIZE: u64 = 16 << 20;

pub struct SessionLog {
    path: PathBuf,
    file: Mutex<LogFile>,
}

struct LogFile {
    file: File,
    written: u64,
    cut: bool,
}

impl SessionLog {
    pub fn create(state_dir: &Path) -> io::Result<Self> {
        let dir = state_dir.join(LOGS);
        fs::create_dir_all(&dir)?;
        let name = format!(
            "{PREFIX}{:020}-{}.{LOG_EXTENSION}",
            paths::now_seconds(),
            std::process::id()
        );
        let path = dir.join(name);
        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .mode(PRIVATE_MODE)
            .open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(LogFile {
                file,
                written: 0,
                cut: false,
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut log = self.file.lock();
        if log.cut {
            return Ok(());
        }
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if log.written.saturating_add(length) > MAX_LOG_SIZE {
            log.cut = true;
            return writeln!(log.file, "{CUT}");
        }
        log.written = log.written.saturating_add(length);
        log.file.write_all(bytes)
    }

    pub fn end(&self) {
        let mut log = self.file.lock();
        if let Err(error) = writeln!(log.file, "{ENDED}").and_then(|()| log.file.sync_data()) {
            log::warn!("could not close the log: {error}");
        }
    }
}

struct LogName {
    path: PathBuf,
    process: u32,
}

fn logs_in(state_dir: &Path) -> Vec<LogName> {
    let Ok(entries) = fs::read_dir(state_dir.join(LOGS)) else {
        return Vec::new();
    };
    let mut logs: Vec<LogName> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let stem = name
                .to_str()?
                .strip_prefix(PREFIX)?
                .strip_suffix(LOG_EXTENSION)?
                .strip_suffix('.')?;
            let (seconds, process) = stem.split_once('-')?;
            seconds.parse::<u64>().ok()?;
            Some(LogName {
                path: entry.path(),
                process: process.parse().ok()?,
            })
        })
        .collect();
    logs.sort_by(|a, b| b.path.cmp(&a.path));
    logs
}

fn running(process: u32) -> bool {
    Path::new("/proc").join(process.to_string()).exists()
}

fn last_line(path: &Path) -> io::Result<String> {
    let mut file = open_file(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL)))?;
    let mut tail = Vec::new();
    file.take(TAIL).read_to_end(&mut tail)?;
    let text = String::from_utf8_lossy(&tail);
    Ok(text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .to_owned())
}

pub fn ended_unexpectedly(state_dir: &Path, own: &Path) -> Vec<PathBuf> {
    logs_in(state_dir)
        .into_iter()
        .filter(|log| log.path != own && !running(log.process))
        .filter(|log| match last_line(&log.path) {
            Ok(line) => !line.ends_with(ENDED) && !line.ends_with(REPORTED),
            Err(error) => {
                log::warn!("could not read {}: {error}", log.path.display());
                false
            }
        })
        .map(|log| log.path)
        .collect()
}

pub fn mark_reported(log: &Path) -> io::Result<()> {
    open_file(log)?;
    let mut file = OpenOptions::new().append(true).open(log)?;
    writeln!(file, "\n{REPORTED}")
}

pub fn prune_logs(state_dir: &Path, own: &Path) {
    for log in logs_in(state_dir).into_iter().skip(LOGS_KEPT) {
        if log.path == own || running(log.process) {
            continue;
        }
        if let Err(error) = fs::remove_file(&log.path) {
            log::warn!("could not remove {}: {error}", log.path.display());
        }
    }
}
