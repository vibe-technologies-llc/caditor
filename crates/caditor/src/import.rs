use std::path::Path;

use caditor_document::{Document, EditError, FeatureId, Prepared};
use caditor_file::{
    Drawing, ImportError, ModelImport, STEP_IMPORT_EXTENSIONS, SketchTarget, bodies_transaction,
    drawing_transaction,
};
use caditor_geometry::Plane;

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    feature_tree::count,
    model::{Action, Model, Notice, SessionBase, display_name},
};

pub const IMPORT_HINT: &str = "Add a DXF drawing to the sketch you are editing or to a new sketch \
                               on the XY plane, or the bodies of a STEP model to the model";
const STEP_SIGNATURE: &[u8] = b"ISO-10303-21";
const SNIFFED_BYTES: usize = 256;
const MAX_NAME_CHARACTERS: usize = 60;
const TOO_SHORT: &str = "every curve in it is too short to draw";
const NO_BODIES: &str = "it holds no solid bodies";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub heading: String,
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub struct DrawingPlan {
    drawing: Drawing,
    sketch: FeatureId,
    curves: usize,
    session: u64,
    prepared: Result<Prepared, EditError>,
}

#[derive(Debug)]
pub enum Placement {
    Done(Option<ImportReport>),
    Stale(Drawing),
}

pub fn plan_drawing(
    base: SessionBase,
    path: &Path,
    into: Option<FeatureId>,
    drawing: Drawing,
) -> DrawingPlan {
    let SessionBase { base, session } = base;
    let document = base.document();
    let target = match into.filter(|feature| editing::edited_sketch(document, *feature).is_some()) {
        Some(feature) => SketchTarget::Existing(feature),
        None => SketchTarget::New {
            name: new_sketch_name(document, path),
            plane: Plane::XY,
        },
    };
    let label = format!("Import {}", display_name(Some(path)));
    let import = drawing_transaction(document, &drawing, target, label);
    DrawingPlan {
        drawing,
        sketch: import.sketch,
        curves: import.curves,
        session,
        prepared: base.prepare(import.transaction),
    }
}

pub fn place_drawing(
    model: &mut Model,
    editing: &mut SketchEditing,
    path: &Path,
    result: Result<DrawingPlan, ImportError>,
) -> Placement {
    let file = display_name(Some(path));
    let plan = match result {
        Ok(plan) => plan,
        Err(error @ ImportError::Empty { .. }) => {
            let reason = error.to_string();
            let ImportError::Empty { left_out } = error else {
                return Placement::Done(None);
            };
            return Placement::Done(nothing_imported(model, &file, &reason, left_out));
        }
        Err(error) => {
            model.set_notice(Notice::failure(format!(
                "Could not import “{file}”: {error}."
            )));
            return Placement::Done(None);
        }
    };
    let DrawingPlan {
        drawing,
        sketch,
        curves,
        session,
        prepared,
    } = plan;
    if curves == 0 {
        return Placement::Done(nothing_imported(model, &file, TOO_SHORT, drawing.notes));
    }
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            model.set_notice(Notice::failure(format!(
                "Could not import “{file}”: {error}."
            )));
            return Placement::Done(None);
        }
    };
    if model.commit(session, prepared).is_err() {
        return Placement::Stale(drawing);
    }
    editing.perform(EditingCommand::Enter(sketch), model);
    let sketch = model
        .document()
        .feature(sketch)
        .map_or_else(|| "the sketch".to_owned(), |feature| feature.name.clone());
    model.set_notice(Notice::info(format!(
        "Imported {} from “{file}” into {sketch}.",
        count(curves, "curve", "curves")
    )));
    Placement::Done((!drawing.notes.is_empty()).then(|| ImportReport {
        heading: format!("Imported “{file}” into {sketch}"),
        notes: drawing.notes,
    }))
}

pub fn is_model(path: &Path) -> bool {
    let by_extension = path.extension().is_some_and(|extension| {
        STEP_IMPORT_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    });
    by_extension || starts_like_step(path)
}

fn starts_like_step(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut start = [0u8; SNIFFED_BYTES];
    let read = std::io::Read::read(&mut file, &mut start).unwrap_or(0);
    let start = start.get(..read).unwrap_or_default();
    let trimmed = start
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .and_then(|first| start.get(first..))
        .unwrap_or_default();
    trimmed.starts_with(STEP_SIGNATURE)
}

pub fn place_bodies(
    model: &mut Model,
    path: &Path,
    result: Result<ModelImport, ImportError>,
) -> Option<ImportReport> {
    let file = display_name(Some(path));
    let imported = match result {
        Ok(imported) => imported,
        Err(error) => {
            model.set_notice(Notice::failure(format!(
                "Could not import “{file}”: {error}."
            )));
            return None;
        }
    };
    if imported.bodies.is_empty() {
        return nothing_imported(model, &file, NO_BODIES, imported.notes);
    }
    let transaction =
        bodies_transaction(model.document(), &imported.bodies, format!("Import {file}"));
    let revision = model.revision();
    model.perform(Action::Apply(transaction));
    if model.revision() == revision {
        return None;
    }
    model.set_notice(Notice::info(format!(
        "Imported {} from “{file}”.",
        count(imported.bodies.len(), "body", "bodies")
    )));
    (!imported.notes.is_empty()).then(|| ImportReport {
        heading: format!("Imported “{file}”"),
        notes: imported.notes,
    })
}

fn nothing_imported(
    model: &mut Model,
    file: &str,
    reason: &str,
    notes: Vec<String>,
) -> Option<ImportReport> {
    model.set_notice(Notice::failure(format!(
        "Nothing in “{file}” could be imported: {reason}."
    )));
    (!notes.is_empty()).then(|| ImportReport {
        heading: format!("Nothing was imported from “{file}”"),
        notes,
    })
}

fn new_sketch_name(document: &Document, path: &Path) -> String {
    let stem: String = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_owned())
        .unwrap_or_default()
        .chars()
        .take(MAX_NAME_CHARACTERS)
        .collect();
    if stem.is_empty() {
        return editing::next_sketch_name(document);
    }
    if document.features().all(|feature| feature.name != stem) {
        return stem;
    }
    editing::next_feature_name(document, &stem)
}
