mod base64;
mod binary;
mod clipboard;
mod configuration_table;
mod configurations;
mod csv;
mod environment;
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
mod parameters;
mod paths;
mod picture;
mod read;
mod reason;
mod recent;
mod recovery;
mod reference_images;
mod save;
mod selection_sets;
mod settings;
mod step_cache;
mod storage;
mod tally;
mod themes;
mod untrusted;

pub use crate::{
    binary::{FileDigest, History, MAX_MODEL_RECORDS, Retention, SavedState, Version},
    clipboard::{
        CLIPBOARD_HEADER, CLIPBOARD_VERSION, ClipboardError, ClipboardKind, CopiedFeatures,
        MAX_CLIPBOARD_TEXT, PastedGeometry, clipboard_kind, features_clipboard_text,
        read_features_clipboard, read_sketch_clipboard, sketch_clipboard_text,
    },
    configuration_table::{
        ConfigurationFileError, MAX_CONFIGURATION_COLUMNS, MAX_CONFIGURATION_ROWS,
        MAX_CONFIGURATIONS_FILE, configurations_csv, parse_configurations, read_configurations,
        write_configurations,
    },
    environment::{
        EnvironmentPixels, EnvironmentReadError, MAX_ENVIRONMENT_PIXELS, decode_environment,
        read_environment_file,
    },
    export::{
        Annotations, Construction, DrawingExported, DrawingSheet, ExportBody, ExportError,
        ExportFace, ExportFormat, ExportThread, Exported, FaceExported, ImageExportError, Look,
        MeshOptions, MeshResolution, NamedFace, NamedSketch, Nesting, PNG_EXTENSION, PixelRows,
        PngExportError, RgbaImage, STEP_EXTENSION, STEP_EXTENSIONS, SheetLayout, SketchExported,
        SketchFormat, StlEncoding, export_bodies, export_drawing, export_face, export_faces,
        export_png, export_sketch, export_sketches, png_bytes,
    },
    format::FORMAT_VERSION,
    import::{
        DRAWING_IMPORT_EXTENSIONS, DXF_EXTENSION, Drawing, DrawingCurve, DrawingImport,
        DrawingOptions, DrawingUnit, ImportError, ImportedBody, ImportedFace, MAX_DRAWING_CURVES,
        MAX_SCALE, MESH_IMPORT_EXTENSIONS, MIN_SCALE, MeshFormat, MeshUnit, ModelImport,
        ReadingProgress, ReadingStage, STEP_IMPORT_EXTENSIONS, SVG_EXTENSIONS, SketchTarget,
        TextOutlines, UnitBasis, UnitReading, bodies_transaction, drawing_transaction, parse_dxf,
        parse_mesh, parse_mesh_in, parse_step, parse_svg, read_drawing, read_dxf, read_mesh_file,
        read_mesh_file_in, read_step_file, read_step_file_reporting,
    },
    journal::JournalEntry,
    load::{
        LoadError, Loaded, decode, history, load, load_cancellable, load_version,
        load_version_cancellable,
    },
    logs::{LOGS_KEPT, MAX_LOG_SIZE, SessionLog, ended_unexpectedly, mark_reported, prune_logs},
    parameters::{
        MAX_PARAMETER_ROWS, MAX_PARAMETERS_FILE, PARAMETERS_EXTENSION, ParameterFileError,
        parameters_csv, parse_parameters, read_parameters, write_parameters,
    },
    paths::{recovery_dir, state_dir},
    picture::{
        PictureReadError, ReadPicture, RgbaPixels, decode_picture, read_picture, read_picture_file,
    },
    reason::{ReadFailure, UpdateFailure, WriteFailure},
    recent::{RECENT_LIMIT, RecentChange, RecentFiles},
    recovery::{
        DiscardError, FileJournal, Inspection, Recovered, describe_set_aside, discard, inspect,
        journal_for, scan,
    },
    save::{
        KeepVersionError, SaveError, SaveOptions, Saved, encode, save, save_with, set_version_kept,
        write_atomically,
    },
    settings::{Settings, SettingsError, config_dir},
    storage::{
        Closing, Flusher, Handover, JournalFailure, KeepRequest, Report, SaveRequest, Start,
        Storage, StorageConfig, StorageStopped,
    },
    themes::{
        MAX_THEME_FILE, MAX_THEME_FILES, ReadTheme, THEME_EXTENSION, THEMES_FOLDER, ThemeFile,
        ThemeFileError, parse_theme, read_theme, read_themes, themes_folder,
    },
};

pub const FILE_EXTENSION: &str = "caditor";

#[cfg(test)]
mod tests;
