use std::path::{self, Path, PathBuf};

use caditor_document::{Document, Edit, EditError, FeatureId, FeatureKind, Prepared, Transaction};
use caditor_file::{
    Drawing, ImportError, ImportedBody, MAX_MODEL_RECORDS, MeshFormat, ModelImport,
    STEP_IMPORT_EXTENSIONS, SketchTarget, bodies_transaction, drawing_transaction, read_mesh_file,
    read_step_file,
};

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    feature_tree::count,
    import_options::Arrangement,
    model::{Action, Model, Notice, SessionBase, display_name},
};

pub const IMPORT_HINT: &str = "Add a DXF drawing to the sketch you are editing or to a new sketch, \
                               or the bodies of a STEP model or an STL, OBJ or 3MF mesh to the \
                               model";
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
    arrangement: Arrangement,
    notes: Vec<String>,
    sketch: FeatureId,
    curves: usize,
    session: u64,
    prepared: Result<Prepared, EditError>,
}

#[derive(Debug)]
pub enum Placement {
    Done(Option<ImportReport>),
    Stale(Drawing, Arrangement),
}

pub fn plan_drawing(
    base: SessionBase,
    path: &Path,
    into: Option<FeatureId>,
    drawing: Drawing,
    arrangement: Arrangement,
) -> Box<DrawingPlan> {
    let SessionBase { base, session } = base;
    let document = base.document();
    let target = match into.filter(|feature| editing::edited_sketch(document, *feature).is_some()) {
        Some(feature) => SketchTarget::Existing(feature),
        None => SketchTarget::New {
            name: new_sketch_name(document, path),
            plane: arrangement.plane.plane(),
        },
    };
    let label = format!("Import {}", display_name(Some(path)));
    let arranged = drawing.arranged(&arrangement.options);
    let import = drawing_transaction(document, &arranged, target, label);
    Box::new(DrawingPlan {
        drawing,
        arrangement,
        notes: arranged.notes,
        sketch: import.sketch,
        curves: import.curves,
        session,
        prepared: base.prepare(import.transaction),
    })
}

pub fn place_drawing(
    model: &mut Model,
    editing: &mut SketchEditing,
    path: &Path,
    result: Result<Box<DrawingPlan>, ImportError>,
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
        arrangement,
        notes,
        sketch,
        curves,
        session,
        prepared,
    } = *plan;
    if curves == 0 {
        return Placement::Done(nothing_imported(model, &file, TOO_SHORT, notes));
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
        return Placement::Stale(drawing, arrangement);
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
    Placement::Done((!notes.is_empty()).then(|| ImportReport {
        heading: format!("Imported “{file}” into {sketch}"),
        notes,
    }))
}

pub fn is_model(path: &Path) -> bool {
    let by_extension = path.extension().is_some_and(|extension| {
        STEP_IMPORT_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    });
    by_extension || MeshFormat::of(path).is_some() || starts_like_step(path)
}

pub fn read_model(path: &Path) -> Result<ModelImport, ImportError> {
    let mut imported = match MeshFormat::of(path) {
        Some(_) => read_mesh_file(path),
        None => read_step_file(path),
    }?;
    let kept = path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    for body in &mut imported.bodies {
        body.import = body.import.clone().from_file(kept.clone());
    }
    Ok(imported)
}

pub fn kept_source(document: &Document, feature: FeatureId) -> Result<PathBuf, String> {
    let owner = document
        .feature(feature)
        .ok_or_else(|| "The imported body no longer exists.".to_owned())?;
    let FeatureKind::Import(import) = &owner.kind else {
        return Err(format!("{} is not an imported body.", owner.name));
    };
    let path = import.path.clone().ok_or_else(|| {
        format!(
            "{} was imported before caditor kept where files came from. Use Replace from file \
             to choose “{}”.",
            owner.name, import.source
        )
    })?;
    if !path.is_file() {
        return Err(format!(
            "{} cannot be reloaded: “{}” is no longer there. Use Replace from file to choose \
             where it is now.",
            owner.name,
            path.display()
        ));
    }
    Ok(path)
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
    if outgrows(model.document(), &transaction, MAX_MODEL_RECORDS) {
        model.set_notice(Notice::failure(format!(
            "“{file}” was not imported: with it the model would hold more than the {} GiB a model \
             file can, so it could be neither saved nor protected against a crash. Import fewer \
             parts at a time, or split the assembly between models.",
            MAX_MODEL_RECORDS >> 30
        )));
        return None;
    }
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

pub fn replace_body(
    model: &mut Model,
    feature: FeatureId,
    path: &Path,
    result: Result<ModelImport, ImportError>,
) -> Option<ImportReport> {
    let file = display_name(Some(path));
    let Some(existing) = model.document().feature(feature) else {
        model.set_notice(Notice::info(format!(
            "The imported body was deleted before “{file}” was read, so nothing was replaced."
        )));
        return None;
    };
    let name = existing.name.clone();
    let held_before = existing.kind.stored_text_len();
    let imported = match result {
        Ok(imported) => imported,
        Err(error) => {
            model.set_notice(Notice::failure(format!(
                "Could not replace {name} from “{file}”: {error}."
            )));
            return None;
        }
    };
    let body = match replacement(&imported.bodies, &name) {
        Ok(body) => body,
        Err(reason) => {
            model.set_notice(Notice::failure(format!(
                "{name} was not replaced from “{file}”: {reason}."
            )));
            return None;
        }
    };
    let transaction = Transaction::single(
        format!("Replace {name} from {file}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Import(body.import.clone()),
        },
    );
    let held: usize = model
        .document()
        .features()
        .map(|feature| feature.kind.stored_text_len())
        .sum();
    let after = held
        .saturating_sub(held_before)
        .saturating_add(FeatureKind::Import(body.import.clone()).stored_text_len());
    if after > MAX_MODEL_RECORDS {
        model.set_notice(Notice::failure(format!(
            "{name} was not replaced from “{file}”: with it the model would hold more than the \
             {} GiB a model file can.",
            MAX_MODEL_RECORDS >> 30
        )));
        return None;
    }
    let revision = model.revision();
    model.perform(Action::Apply(transaction));
    if model.revision() == revision {
        return None;
    }
    model.set_notice(Notice::info(format!(
        "Replaced {name} with the body in “{file}”. Features using its faces and edges find them \
         again, and any that cannot say so in the tree."
    )));
    (!imported.notes.is_empty()).then(|| ImportReport {
        heading: format!("Replaced {name} from “{file}”"),
        notes: imported.notes,
    })
}

fn replacement<'a>(bodies: &'a [ImportedBody], name: &str) -> Result<&'a ImportedBody, String> {
    let named_after = |body: &ImportedBody| {
        let base = body.name.trim();
        name == base
            || name
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix(' '))
                .is_some_and(|number| number.parse::<u32>().is_ok())
    };
    match bodies {
        [] => Err(NO_BODIES.to_owned()),
        [only] => Ok(only),
        several => several
            .iter()
            .find(|body| body.name.trim() == name)
            .or_else(|| several.iter().find(|body| named_after(body)))
            .ok_or_else(|| {
                format!(
                    "it holds {} and none is named {name}; rename the body in the file or the \
                     feature to match",
                    count(several.len(), "body", "bodies")
                )
            }),
    }
}

fn outgrows(document: &Document, transaction: &Transaction, limit: usize) -> bool {
    let held: usize = document
        .features()
        .map(|feature| feature.kind.stored_text_len())
        .sum();
    let added: usize = transaction
        .edits()
        .iter()
        .map(|edit| match edit {
            Edit::InsertFeature { feature, .. } => feature.kind.stored_text_len(),
            _ => 0,
        })
        .sum();
    held.saturating_add(added) > limit
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

#[cfg(test)]
mod tests {
    use caditor_document::{FeatureKind, Import};
    use caditor_kernel::Solid;

    use super::*;

    fn importing(document: &Document, name: &str, bytes: usize) -> Transaction {
        let mut transaction = document.transaction("Import");
        transaction.add_feature(
            name,
            FeatureKind::Import(Import::new(
                "part.step",
                Solid::default(),
                "x".repeat(bytes),
            )),
        );
        transaction.finish()
    }

    #[test]
    fn an_import_that_would_outgrow_what_a_model_file_holds_is_caught_before_it_lands() {
        let mut document = Document::default();
        document.apply(importing(&document, "First", 600)).unwrap();

        let fitting = importing(&document, "Second", 300);
        let outgrowing = importing(&document, "Second", 500);

        assert!(!outgrows(&document, &fitting, 1000));
        assert!(outgrows(&document, &outgrowing, 1000));
    }
}
