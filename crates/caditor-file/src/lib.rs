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
mod os;
mod paths;
mod read;
mod reason;
mod recent;
mod recovery;
mod save;
mod selection_sets;
mod settings;
mod step_cache;
mod storage;
mod untrusted;

pub use crate::{
    binary::{FileDigest, History, MAX_MODEL_RECORDS, SavedState, Version},
    export::{
        Annotations, Construction, DrawingExported, DrawingSheet, ExportBody, ExportError,
        ExportFormat, ExportThread, Exported, FaceExported, ImageExportError, Look, MeshOptions,
        MeshResolution, NamedFace, NamedSketch, Nesting, PNG_EXTENSION, PixelRows, PngExportError,
        RgbaImage, STEP_EXTENSION, STEP_EXTENSIONS, SheetLayout, SketchExported, SketchFormat,
        StlEncoding, export_bodies, export_drawing, export_face, export_faces, export_png,
        export_sketch, export_sketches,
    },
    format::FORMAT_VERSION,
    import::{
        DRAWING_IMPORT_EXTENSIONS, DXF_EXTENSION, Drawing, DrawingCurve, DrawingImport,
        DrawingOptions, DrawingUnit, ImportError, ImportedBody, ImportedFace, MAX_DRAWING_CURVES,
        MAX_SCALE, MESH_IMPORT_EXTENSIONS, MIN_SCALE, MeshFormat, ModelImport,
        STEP_IMPORT_EXTENSIONS, SVG_EXTENSIONS, SketchTarget, bodies_transaction,
        drawing_transaction, parse_dxf, parse_mesh, parse_step, parse_svg, read_drawing, read_dxf,
        read_mesh_file, read_step_file,
    },
    journal::JournalEntry,
    load::{LoadError, Loaded, MAX_RECORDS, decode, history, load, load_cancellable, load_version},
    logs::{LOGS_KEPT, MAX_LOG_SIZE, SessionLog, ended_unexpectedly, mark_reported, prune_logs},
    paths::{recovery_dir, state_dir},
    reason::{ReadFailure, WriteFailure},
    recent::{RECENT_LIMIT, RecentChange, RecentFiles},
    recovery::{
        FileJournal, Inspection, Recovered, describe_set_aside, discard, inspect, journal_for, scan,
    },
    save::{
        KeepVersionError, SaveError, SaveOptions, Saved, encode, save, save_with, set_version_kept,
        write_atomically,
    },
    settings::{Settings, SettingsError, config_dir},
    storage::{
        Closing, Flusher, JournalFailure, KeepRequest, Report, SaveRequest, Start, Storage,
        StorageConfig, StorageStopped,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
