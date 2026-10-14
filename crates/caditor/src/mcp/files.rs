use std::{
    fs,
    path::{Path, PathBuf},
};

use caditor_document::MAX_RECORDS;
use caditor_file::{
    DrawingOptions, ImportError, MAX_MODEL_RECORDS, MeshResolution, ReadingProgress, SketchTarget,
    TextOutlines, bodies_transaction, drawing_transaction, read_drawing,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    files::is_drawing_file,
    fonts,
    headless::{self, Settings, Target},
    import,
    mcp::{
        places::principal_plane,
        reading::{RenderError, picture_size},
        references::{self, Named},
        session::{Cancelled, SaveFailure, Session, Stored, Work, has_model_extension},
        tools::{Answer, ToolError},
        writing::{PlaneName, apply, with_feature},
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Importing {
    pub file: String,
    pub sketch: Option<Named>,
    pub plane: Option<PlaneName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavingAs {
    pub file: String,
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
    #[error("“{file}” is a link; give a plain file")]
    Link { file: String },
    #[error("“{file}” does not exist")]
    Missing { file: String },
    #[error(
        "“{file}” is not a file caditor imports: a STEP model (.step, .stp), an STL, OBJ or 3MF \
         mesh, or a DXF or SVG drawing"
    )]
    NotImportable { file: String },
    #[error("could not import “{file}”: {source}")]
    Import {
        file: String,
        #[source]
        source: ImportError,
    },
    #[error("nothing in “{file}” could be imported: {reason}")]
    NothingImported { file: String, reason: &'static str },
    #[error(
        "“{file}” was not imported: with it the model would hold more than a model file can, so \
         it could be neither saved nor protected against a crash; import fewer parts at a time"
    )]
    OutgrowsTheFile { file: String },
    #[error(
        "“{file}” was not imported: with it the model would hold more than the {MAX_RECORDS} \
         features and parameters caditor opens; import fewer parts at a time"
    )]
    TooManyRecords { file: String },
    #[error("sketch and plane place a DXF or SVG drawing; a model's bodies come in where they are")]
    PlacedModel,
    #[error("a drawing goes into an existing sketch or onto a plane in a new one, not both")]
    SketchAndPlane,
    #[error("“{file}” must end in .caditor to save the model to it")]
    NotAModelFile { file: String },
    #[error(
        "“{file}” already exists; save_as writes only a new file, so choose a name not taken, or \
         use save for the model's own file"
    )]
    Exists { file: String },
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

fn answer_saved(session: &Session, stored: Stored) -> Map<String, Value> {
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
    answer
}

pub fn save(session: &mut Session) -> Result<Map<String, Value>, ToolError> {
    let stored = session.save()?;
    Ok(answer_saved(session, stored))
}

pub fn save_as(session: &mut Session, asked: &SavingAs) -> Result<Map<String, Value>, ToolError> {
    let target = allowed(session.model(), session.folders(), &asked.file)?;
    let shown = display_name(Some(&target));
    if !has_model_extension(&target) {
        return Err(FileError::NotAModelFile { file: shown }.into());
    }
    if target != session.model() && target.exists() {
        return Err(FileError::Exists { file: shown }.into());
    }
    let stored = session.save_to(target)?;
    Ok(answer_saved(session, stored))
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

fn notes_into(details: &mut Map<String, Value>, notes: Vec<String>) {
    if !notes.is_empty() {
        details.insert("notes".to_owned(), json!(notes));
    }
}

fn import_failure(file: &str, source: ImportError) -> ToolError {
    match source {
        ImportError::Cancelled => Cancelled.into(),
        source => FileError::Import {
            file: file.to_owned(),
            source,
        }
        .into(),
    }
}

fn import_model(
    session: &mut Session,
    work: &Work<'_>,
    path: &Path,
    shown: String,
) -> Result<Answer, ToolError> {
    work.progress(&format!("Reading {shown}"));
    let imported = import::read_model(path, &work.cancel_token(), &ReadingProgress::default())
        .map_err(|source| import_failure(&shown, source))?;
    if work.is_cancelled() {
        return Err(Cancelled.into());
    }
    if imported.bodies.is_empty() {
        return Err(FileError::NothingImported {
            file: shown,
            reason: import::NO_BODIES,
        }
        .into());
    }
    let document = session.document();
    let transaction = bodies_transaction(document, &imported.bodies, format!("Import {shown}"));
    if import::outgrows(document, &transaction, MAX_MODEL_RECORDS) {
        return Err(FileError::OutgrowsTheFile { file: shown }.into());
    }
    if import::outnumbers(document, &transaction, MAX_RECORDS) {
        return Err(FileError::TooManyRecords { file: shown }.into());
    }
    let mut details = Map::new();
    details.insert("bodies".to_owned(), json!(imported.bodies.len()));
    notes_into(&mut details, imported.notes);
    apply(session, work, transaction, details)
}

fn import_drawing(
    session: &mut Session,
    work: &Work<'_>,
    (path, shown): (&Path, String),
    asked: &Importing,
) -> Result<Answer, ToolError> {
    let target = match (&asked.sketch, asked.plane) {
        (Some(_), Some(_)) => return Err(FileError::SketchAndPlane.into()),
        (Some(named), None) => {
            let (feature, _) = references::sketch(session.document(), named)?;
            SketchTarget::Existing(feature.id())
        }
        (None, plane) => SketchTarget::New {
            name: import::new_sketch_name(session.document(), path),
            plane: principal_plane(plane.unwrap_or(PlaneName::Xy)).plane(),
        },
    };
    work.progress(&format!("Reading {shown}"));
    let text_outlines = TextOutlines::InFont {
        upright: fonts::INTER,
        italic: Some(fonts::INTER_ITALIC),
    };
    let drawing = read_drawing(path, text_outlines, &work.cancel_token())
        .map_err(|source| import_failure(&shown, source))?;
    let arranged = drawing.arranged(&DrawingOptions::default());
    let imported = drawing_transaction(
        session.document(),
        &arranged,
        target,
        format!("Import {shown}"),
    );
    if imported.curves == 0 {
        return Err(FileError::NothingImported {
            file: shown,
            reason: import::TOO_SHORT,
        }
        .into());
    }
    let mut details = Map::new();
    details.insert("curves".to_owned(), json!(imported.curves));
    notes_into(&mut details, arranged.notes);
    let answer = apply(session, work, imported.transaction, details)?;
    Ok(with_feature(session, answer, imported.sketch))
}

pub fn import_file(
    session: &mut Session,
    work: &Work<'_>,
    asked: &Importing,
) -> Result<Answer, ToolError> {
    let path = allowed(session.model(), session.folders(), &asked.file)?;
    let shown = display_name(Some(&path));
    if !path.is_file() {
        return Err(FileError::Missing { file: shown }.into());
    }
    if is_drawing_file(&path) {
        return import_drawing(session, work, (&path, shown), asked);
    }
    if has_model_extension(&path) || !import::is_model(&path) {
        return Err(FileError::NotImportable { file: shown }.into());
    }
    if asked.sketch.is_some() || asked.plane.is_some() {
        return Err(FileError::PlacedModel.into());
    }
    import_model(session, work, &path, shown)
}
