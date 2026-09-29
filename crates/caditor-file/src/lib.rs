mod binary;
mod export;
mod format;
#[cfg(feature = "fuzzing")]
pub mod fuzzing;
mod import;
mod journal;
mod load;
mod lock;
mod paths;
mod read;
mod reason;
mod recent;
mod recovery;
mod save;
mod settings;
mod storage;

pub use crate::{
    binary::{History, SavedState, Version},
    export::{
        ExportBody, ExportError, ExportFormat, Exported, MeshResolution, STEP_EXTENSION,
        STEP_EXTENSIONS, export_bodies,
    },
    format::FORMAT_VERSION,
    import::{
        DXF_EXTENSION, Drawing, DrawingCurve, DrawingImport, ImportError, ImportedBody,
        MAX_DRAWING_CURVES, ModelImport, STEP_IMPORT_EXTENSIONS, SketchTarget, bodies_transaction,
        drawing_transaction, parse_dxf, parse_step, read_dxf, read_step_file,
    },
    journal::JournalEntry,
    load::{LoadError, Loaded, decode, history, load, load_version},
    paths::{recovery_dir, state_dir},
    recent::{RECENT_LIMIT, RecentFiles},
    recovery::{
        FileJournal, Inspection, Recovered, describe_set_aside, discard, inspect, journal_for, scan,
    },
    save::{SaveError, SaveOptions, encode, save, save_with, write_atomically},
    settings::{Settings, config_dir},
    storage::{
        Closing, Flusher, Report, SaveRequest, Start, Storage, StorageConfig, StorageStopped,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
