use std::{io, path::PathBuf, process::ExitStatus};

pub const USAGE: &str = "caditor-notices [--target TRIPLE] OUTPUT";

#[derive(Debug, thiserror::Error)]
pub enum NoticesError {
    #[error("{argument} is not an option of caditor-notices; usage: {USAGE}")]
    UnknownOption { argument: String },
    #[error("--target needs a target triple; usage: {USAGE}")]
    TargetMissing,
    #[error("no output file named; usage: {USAGE}")]
    OutputMissing,
    #[error("two output files named, {first} and {second}; usage: {USAGE}")]
    TwoOutputs { first: PathBuf, second: PathBuf },
    #[error("cargo could not be started: {0}")]
    CargoNotStarted(io::Error),
    #[error("cargo metadata failed ({status}): {report}")]
    MetadataFailed { status: ExitStatus, report: String },
    #[error("cargo metadata printed something unreadable: {0}")]
    MetadataUnreadable(serde_json::Error),
    #[error("the package {package} is not in the dependency graph")]
    PackageMissing { package: String },
    #[error("cannot read {path}: {source}")]
    Unreadable { path: PathBuf, source: io::Error },
    #[error("{path} is not UTF-8 text")]
    NotText { path: PathBuf },
    #[error(
        "{package} ships no licence file and its licence {license} is none this tool writes out; \
         add its text to crates/caditor-notices/licenses"
    )]
    NoLicenseText { package: String, license: String },
    #[error("cannot write {path}: {source}")]
    Unwritable { path: PathBuf, source: io::Error },
}
