use std::{
    fs,
    path::{Path, PathBuf},
};

use caditor_file::MeshResolution;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    headless::{self, Settings, Target},
    mcp::{
        reading::{RenderError, picture_size},
        session::{Cancelled, SaveFailure, Session, Work},
        tools::ToolError,
    },
    model::display_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionName {
    Coarse,
    Standard,
    Fine,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exporting {
    pub file: String,
    pub resolution: Option<ResolutionName>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("“{file}” names no file")]
    NotAFile { file: String },
    #[error("the folder of “{file}” does not exist")]
    NoFolder { file: String },
    #[error(
        "“{file}” lies outside the folders this server may write ({folders}); the user can allow \
         another with --folder when starting it"
    )]
    OutsideFolders { file: String, folders: String },
    #[error("“{file}” is a link; export to a plain file")]
    Link { file: String },
    #[error(
        "“{file}” must end in .step, .stp, .stl, .3mf, .obj, .glb or .png, the formats caditor \
         exports"
    )]
    UnknownFormat { file: String },
    #[error("width and height are for a .png export only")]
    SizeForMesh,
    #[error("the model has no bodies to export{failing}")]
    NoBodies { failing: String },
    #[error(transparent)]
    Size(RenderError),
    #[error(transparent)]
    Save(#[from] SaveFailure),
    #[error("{0:#}")]
    Export(anyhow::Error),
}

impl From<SaveFailure> for ToolError {
    fn from(failure: SaveFailure) -> Self {
        Self::Files(FileError::Save(failure))
    }
}

pub fn allowed(model: &Path, folders: &[PathBuf], file: &str) -> Result<PathBuf, FileError> {
    let asked = Path::new(file.trim());
    let path = match model.parent() {
        Some(folder) if asked.is_relative() => folder.join(asked),
        _ => asked.to_path_buf(),
    };
    let not_a_file = || FileError::NotAFile {
        file: file.to_owned(),
    };
    let name = path.file_name().ok_or_else(not_a_file)?;
    let folder = path.parent().ok_or_else(not_a_file)?;
    let folder = dunce::canonicalize(folder).map_err(|_| FileError::NoFolder {
        file: file.to_owned(),
    })?;
    if !folders.iter().any(|allowed| folder.starts_with(allowed)) {
        let listed: Vec<String> = folders
            .iter()
            .map(|folder| format!("“{}”", folder.display()))
            .collect();
        return Err(FileError::OutsideFolders {
            file: file.to_owned(),
            folders: listed.join(", "),
        });
    }
    let target = folder.join(name);
    let is_link =
        fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink());
    if is_link {
        return Err(FileError::Link {
            file: file.to_owned(),
        });
    }
    Ok(target)
}

pub fn save(session: &mut Session) -> Result<Map<String, Value>, ToolError> {
    let stored = session.save()?;
    let mut answer = Map::new();
    answer.insert(
        "saved".to_owned(),
        json!(session.model().display().to_string()),
    );
    if let Some(backup) = stored.backup {
        answer.insert(
            "original_kept_as".to_owned(),
            json!(backup.display().to_string()),
        );
    }
    if stored.dropped_for_size > 0 {
        answer.insert(
            "versions_dropped_for_size".to_owned(),
            json!(stored.dropped_for_size),
        );
    }
    Ok(answer)
}

pub fn export(
    session: &mut Session,
    work: &Work<'_>,
    asked: &Exporting,
) -> Result<Map<String, Value>, ToolError> {
    let output = allowed(session.model(), session.folders(), &asked.file)?;
    let shown = display_name(Some(&output));
    let target = Target::of(&output).ok_or(FileError::UnknownFormat {
        file: shown.clone(),
    })?;
    let size = match target {
        Target::Image => Some(picture_size(asked.width, asked.height).map_err(FileError::Size)?),
        Target::Mesh(_) if asked.width.is_some() || asked.height.is_some() => {
            return Err(FileError::SizeForMesh.into());
        }
        Target::Mesh(_) => None,
    };
    let resolution = match asked.resolution {
        Some(ResolutionName::Coarse) => MeshResolution::Coarse,
        Some(ResolutionName::Standard) | None => MeshResolution::Standard,
        Some(ResolutionName::Fine) => MeshResolution::Fine,
    };
    session.evaluate(work)?;
    let document = session.document();
    let evaluation = session.evaluation();
    let bodies = headless::bodies(document, evaluation);
    if bodies.is_empty() {
        let failing: Vec<String> = crate::mcp::reading::problems(document, evaluation)
            .into_iter()
            .map(|(_, name, words)| format!("{name} is {}", words.state))
            .collect();
        let failing = if failing.is_empty() {
            String::new()
        } else {
            format!(": {}", failing.join("; "))
        };
        return Err(FileError::NoBodies { failing }.into());
    }
    work.progress(&format!("Exporting to {shown}"));
    let delivered = headless::deliver(
        document,
        evaluation,
        &bodies,
        &Settings {
            target,
            resolution,
            size,
        },
        &output,
        &work.cancel_token(),
    )
    .map_err(FileError::Export)?;
    if work.is_cancelled() {
        return Err(Cancelled.into());
    }
    let mut answer = Map::new();
    answer.insert("exported".to_owned(), json!(output.display().to_string()));
    answer.insert("bodies".to_owned(), json!(delivered.bodies));
    if !delivered.warnings.is_empty() {
        answer.insert("warnings".to_owned(), json!(delivered.warnings));
    }
    Ok(answer)
}
