use std::path::Path;

use caditor_document::{Document, FeatureId};
use caditor_file::{Drawing, ImportError, SketchTarget, drawing_transaction};
use caditor_geometry::Plane;
use egui::{KeyboardShortcut, Modifiers};

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    feature_tree::count,
    model::{Action, Model, Notice, display_name},
};

pub const IMPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, egui::Key::I);
pub const IMPORT_HINT: &str =
    "Add a DXF drawing to the sketch you are editing, or to a new sketch on the XY plane";
const MAX_NAME_CHARACTERS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub heading: String,
    pub notes: Vec<String>,
}

pub fn place_drawing(
    model: &mut Model,
    editing: &mut SketchEditing,
    path: &Path,
    into: Option<FeatureId>,
    result: Result<Drawing, ImportError>,
) -> Option<ImportReport> {
    let file = display_name(Some(path));
    let drawing = match result {
        Ok(drawing) => drawing,
        Err(error) => {
            model.set_notice(Notice::error(format!(
                "Could not import “{file}”: {error}."
            )));
            return None;
        }
    };
    let document = model.document();
    let target = match into.filter(|feature| editing::edited_sketch(document, *feature).is_some()) {
        Some(feature) => SketchTarget::Existing(feature),
        None => SketchTarget::New {
            name: new_sketch_name(document, path),
            plane: Plane::XY,
        },
    };
    let import = drawing_transaction(document, &drawing, target, format!("Import {file}"));
    let revision = model.revision();
    model.perform(Action::Apply(import.transaction));
    if model.revision() == revision {
        return None;
    }
    editing.perform(EditingCommand::Enter(import.sketch), model);
    let sketch = model
        .document()
        .feature(import.sketch)
        .map_or_else(|| "the sketch".to_owned(), |feature| feature.name.clone());
    model.set_notice(Notice::info(format!(
        "Imported {} from “{file}” into {sketch}.",
        count(import.curves, "curve", "curves")
    )));
    (!drawing.notes.is_empty()).then(|| ImportReport {
        heading: format!("Imported “{file}” into {sketch}"),
        notes: drawing.notes,
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
