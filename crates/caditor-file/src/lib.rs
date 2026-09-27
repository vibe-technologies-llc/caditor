mod format;
mod journal;
mod load;
mod paths;
mod reason;
mod recent;
mod recovery;
mod save;
mod storage;

pub use crate::{
    format::FORMAT_VERSION,
    journal::JournalEntry,
    load::{LoadError, Loaded, decode, load},
    paths::{recovery_dir, state_dir},
    recent::{RECENT_LIMIT, RecentFiles},
    recovery::{FileJournal, Inspection, Recovered, discard, inspect, journal_for, scan},
    save::{SaveError, encode, save, write_atomically},
    storage::{
        Closing, Flusher, Report, SaveRequest, Start, Storage, StorageConfig, StorageStopped,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
