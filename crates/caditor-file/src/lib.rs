mod binary;
mod export;
mod format;
#[cfg(feature = "fuzzing")]
pub mod fuzzing;
mod import;
mod journal;
mod load;
mod lock;
mod logs;
mod paths;
mod read;
mod reason;
mod recent;
mod recovery;
mod save;
mod settings;
mod step_cache;
mod storage;
mod untrusted;

pub use crate::{
    binary::{FileDigest, History, SavedState, Version},
    export::{
        ExportBody, ExportError, ExportFormat, Exported, ImageExportError, MeshResolution,
        PNG_EXTENSION, RgbaImage, STEP_EXTENSION, STEP_EXTENSIONS, SketchExported, SketchFormat,
        export_bodies, export_png, export_sketch,
    },
    format::FORMAT_VERSION,
    import::{
        DXF_EXTENSION, Drawing, DrawingCurve, DrawingImport, ImportError, ImportedBody,
        MAX_DRAWING_CURVES, ModelImport, STEP_IMPORT_EXTENSIONS, SketchTarget, bodies_transaction,
        drawing_transaction, parse_dxf, parse_step, read_dxf, read_step_file,
    },
    journal::JournalEntry,
    load::{LoadError, Loaded, MAX_RECORDS, decode, history, load, load_version},
    logs::{LOGS_KEPT, MAX_LOG_SIZE, SessionLog, ended_unexpectedly, mark_reported, prune_logs},
    paths::{recovery_dir, state_dir},
    reason::{ReadFailure, WriteFailure},
    recent::{RECENT_LIMIT, RecentChange, RecentFiles},
    recovery::{
        FileJournal, Inspection, Recovered, describe_set_aside, discard, inspect, journal_for, scan,
    },
    save::{SaveError, SaveOptions, Saved, encode, save, save_with, write_atomically},
    settings::{Settings, SettingsError, config_dir},
    storage::{
        Closing, Flusher, Report, SaveRequest, Start, Storage, StorageConfig, StorageStopped,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
