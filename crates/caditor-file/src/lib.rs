mod binary;
mod export;
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
    binary::{History, SavedState, Version},
    export::{ExportBody, ExportError, Exported, MeshFormat, MeshResolution, export_mesh},
    format::FORMAT_VERSION,
    journal::JournalEntry,
    load::{LoadError, Loaded, decode, history, load, load_version},
    paths::{recovery_dir, state_dir},
    recent::{RECENT_LIMIT, RecentFiles},
    recovery::{FileJournal, Inspection, Recovered, discard, inspect, journal_for, scan},
    save::{SaveError, SaveOptions, encode, save, save_with, write_atomically},
    storage::{
        Closing, Flusher, Report, SaveRequest, Start, Storage, StorageConfig, StorageStopped,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
