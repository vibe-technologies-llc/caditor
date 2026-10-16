use std::{
    collections::BTreeSet,
    fmt,
    path::{self, Path, PathBuf},
};

use caditor_document::{
    CancelToken, Document, Edit, EditError, FeatureId, FeatureKind, MAX_RECORDS, Prepared,
    Transaction,
};
use caditor_file::{
    Drawing, ImportError, ImportedBody, MAX_MODEL_RECORDS, MeshFormat, MeshUnit, ModelImport,
    ReadingProgress, STEP_IMPORT_EXTENSIONS, SketchTarget, bodies_transaction, drawing_transaction,
    read_mesh_file, read_mesh_file_in, read_step_file_reporting,
};

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    feature_tree::count,
    import_options::Arrangement,
    model::{Action, Model, Notice, SessionBase, display_name},
};

pub const IMPORT_HINT: &str = "Add a DXF or SVG drawing to the sketch you are editing or to a new \
                               sketch, or the bodies of a STEP model or an STL, OBJ or 3MF mesh \
                               to the model";
const STEP_SIGNATURE: &[u8] = b"ISO-10303-21";
const SNIFFED_BYTES: usize = 256;
const MAX_NAME_CHARACTERS: usize = 60;
pub const NO_BODIES: &str = "it holds no solid bodies";
pub const TOO_SHORT: &str = "every curve in it is too short to draw";

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
    model.set_notice(Notice::success(format!(
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

pub fn read_model(
    path: &Path,
    cancel: &CancelToken,
    progress: &ReadingProgress,
) -> Result<ModelImport, ImportError> {
    let imported = match MeshFormat::of(path) {
        Some(_) => read_mesh_file(path, cancel),
        None => read_step_file_reporting(path, cancel, progress),
    }?;
    Ok(kept_from(path, imported))
}

pub fn read_mesh_in(
    path: &Path,
    unit: MeshUnit,
    cancel: &CancelToken,
) -> Result<ModelImport, ImportError> {
    read_mesh_file_in(path, Some(unit), cancel).map(|imported| kept_from(path, imported))
}

fn kept_from(path: &Path, mut imported: ModelImport) -> ModelImport {
    let kept = path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    for body in &mut imported.bodies {
        body.import = body.import.clone().from_file(kept.clone());
    }
    imported
}

pub struct KeptSource {
    pub path: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeptSourceError {
    #[error("The imported body no longer exists.")]
    Deleted,
    #[error("{name} is not an imported body.")]
    NotImported { name: String },
    #[error(
        "{name} was imported before caditor kept where files came from. Use Replace from file \
         to choose “{origin}”."
    )]
    PathNotKept { name: String, origin: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodiesRefusal {
    #[error("{NO_BODIES}")]
    NoBodies,
    #[error(
        "it holds {} and none is named {name}; rename the body in the file or the feature to \
         match",
        count(*.bodies, "body", "bodies")
    )]
    NoneNamed { bodies: usize, name: String },
}

pub fn kept_source(document: &Document, feature: FeatureId) -> Result<KeptSource, KeptSourceError> {
    let owner = document.feature(feature).ok_or(KeptSourceError::Deleted)?;
    let FeatureKind::Import(import) = &owner.kind else {
        return Err(KeptSourceError::NotImported {
            name: owner.name.clone(),
        });
    };
    let path = import
        .path
        .clone()
        .ok_or_else(|| KeptSourceError::PathNotKept {
            name: owner.name.clone(),
            origin: import.source.clone(),
        })?;
    Ok(KeptSource {
        path,
        name: owner.name.clone(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacementSource {
    Picked,
    Kept { name: String },
}

impl ReplacementSource {
    pub fn check(&self, path: &Path) -> Result<(), SourceRefusal> {
        if let Self::Kept { name } = self
            && !path.is_file()
        {
            return Err(SourceRefusal::Gone {
                name: name.clone(),
                path: path.to_path_buf(),
            });
        }
        if is_model(path) {
            Ok(())
        } else {
            Err(SourceRefusal::NotAModel {
                file: display_name(Some(path)),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceRefusal {
    #[error(
        "{name} cannot be reloaded: “{}” is no longer there. Use Replace from file to choose \
         where it is now.",
        path.display()
    )]
    Gone { name: String, path: PathBuf },
    #[error("“{file}” is not a STEP, STL, OBJ or 3MF file, so it cannot replace an imported body.")]
    NotAModel { file: String },
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
        return nothing_imported(model, &file, BodiesRefusal::NoBodies, imported.notes);
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
    if outnumbers(model.document(), &transaction, MAX_RECORDS) {
        model.set_notice(Notice::failure(format!(
            "“{file}” was not imported: with its {} the model would hold more than the {} \
             features and parameters caditor opens, so some would be lost when it is opened \
             again. Import fewer parts at a time, or split the assembly between models.",
            count(imported.bodies.len(), "body", "bodies"),
            MAX_RECORDS
        )));
        return None;
    }
    let revision = model.revision();
    model.perform(Action::Apply(transaction));
    if model.revision() == revision {
        return None;
    }
    model.set_notice(Notice::success(format!(
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
        model.set_notice(Notice::warning(format!(
            "The imported body was deleted before “{file}” was read, so nothing was replaced."
        )));
        return None;
    };
    let name = existing.name.clone();
    let held_before = existing.kind.stored_text_len();
    let placement = existing
        .kind
        .import()
        .map(|import| import.placement.clone())
        .unwrap_or_default();
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
        Err(refusal) => {
            model.set_notice(Notice::failure(format!(
                "{name} was not replaced from “{file}”: {refusal}."
            )));
            return None;
        }
    };
    let kept_in_place = FeatureKind::Import(body.import.clone().placed(placement));
    let transaction = Transaction::single(
        format!("Replace {name} from {file}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: kept_in_place,
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
    model.set_notice(Notice::success(format!(
        "Replaced {name} with the body in “{file}”. Features using its faces and edges find them \
         again, and any that cannot say so in the tree."
    )));
    (!imported.notes.is_empty()).then(|| ImportReport {
        heading: format!("Replaced {name} from “{file}”"),
        notes: imported.notes,
    })
}

fn replacement<'a>(
    bodies: &'a [ImportedBody],
    name: &str,
) -> Result<&'a ImportedBody, BodiesRefusal> {
    let named_after = |body: &ImportedBody| {
        let base = body.name.trim();
        name == base
            || name
                .strip_prefix(base)
                .and_then(|rest| rest.strip_prefix(' '))
                .is_some_and(|number| number.parse::<u32>().is_ok())
    };
    match bodies {
        [] => Err(BodiesRefusal::NoBodies),
        [only] => Ok(only),
        several => several
            .iter()
            .find(|body| body.name.trim() == name)
            .or_else(|| several.iter().find(|body| named_after(body)))
            .ok_or_else(|| BodiesRefusal::NoneNamed {
                bodies: several.len(),
                name: name.to_owned(),
            }),
    }
}

pub fn outgrows(document: &Document, transaction: &Transaction, limit: usize) -> bool {
    let added = transaction.edits().iter().filter_map(|edit| match edit {
        Edit::InsertFeature { feature, .. } => Some(&feature.kind),
        _ => None,
    });
    stored_len(
        document
            .features()
            .map(|feature| &feature.kind)
            .chain(added),
    ) > limit
}

pub fn outnumbers(document: &Document, transaction: &Transaction, limit: usize) -> bool {
    let added = transaction
        .edits()
        .iter()
        .filter(|edit| matches!(edit, Edit::InsertFeature { .. }))
        .count();
    document.parameters().len() + document.features().len() + added > limit
}

fn stored_len<'a>(kinds: impl Iterator<Item = &'a FeatureKind>) -> usize {
    let mut texts = BTreeSet::new();
    kinds
        .map(|kind| match kind.import() {
            Some(import) if !texts.insert(import.step.address()) => {
                import.source.len() + import.path_len()
            }
            _ => kind.stored_text_len(),
        })
        .fold(0, usize::saturating_add)
}

fn nothing_imported(
    model: &mut Model,
    file: &str,
    reason: impl fmt::Display,
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

pub fn new_sketch_name(document: &Document, path: &Path) -> String {
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

    #[test]
    fn copies_of_one_part_count_its_text_once() {
        let document = Document::default();
        let part = Import::new("part.step", Solid::default(), "x".repeat(600));
        let mut transaction = document.transaction("Import");
        for name in ["Part", "Part 2", "Part 3"] {
            transaction.add_feature(name, FeatureKind::Import(part.clone()));
        }
        let copies = transaction.finish();

        assert!(!outgrows(&document, &copies, 1000));
        assert!(outgrows(
            &document,
            &importing(&document, "Big", 1001),
            1000
        ));
    }

    #[test]
    fn an_import_that_would_hold_more_features_than_a_model_opens_is_caught_before_it_lands() {
        let mut document = Document::default();
        document.apply(importing(&document, "First", 1)).unwrap();

        let mut two = document.transaction("Import");
        for name in ["Screw", "Screw 2"] {
            two.add_feature(
                name,
                FeatureKind::Import(Import::new("part.step", Solid::default(), "x".to_owned())),
            );
        }
        let two = two.finish();

        assert!(!outnumbers(&document, &two, 3));
        assert!(outnumbers(&document, &two, 2));
    }

    fn document_with_import_from(path: &Path) -> (Document, FeatureId) {
        let mut document = Document::default();
        let mut transaction = document.transaction("Import");
        let id = transaction.add_feature(
            "Bracket",
            FeatureKind::Import(
                Import::new("part.step", Solid::default(), String::new())
                    .from_file(path.to_path_buf()),
            ),
        );
        document.apply(transaction.finish()).unwrap();
        (document, id)
    }

    #[test]
    fn the_kept_source_is_read_from_the_document_without_touching_the_file() {
        let path = PathBuf::from("/nonexistent-share/part.step");
        let (document, id) = document_with_import_from(&path);

        let kept = kept_source(&document, id).unwrap();

        assert_eq!(kept.path, path);
        assert_eq!(kept.name, "Bracket");
    }

    #[test]
    fn a_kept_source_that_has_gone_is_refused_in_words_by_the_check() {
        let path = PathBuf::from("/nonexistent-share/part.step");
        let source = ReplacementSource::Kept {
            name: "Bracket".to_owned(),
        };

        let refusal = source.check(&path).unwrap_err();

        assert_eq!(
            refusal.to_string(),
            "Bracket cannot be reloaded: “/nonexistent-share/part.step” is no longer there. Use \
             Replace from file to choose where it is now."
        );
    }

    #[test]
    fn a_picked_file_that_is_no_model_is_refused_in_words_by_the_check() {
        let refusal = ReplacementSource::Picked
            .check(Path::new("/nonexistent-share/plan.dxf"))
            .unwrap_err();

        assert_eq!(
            refusal.to_string(),
            "“plan.dxf” is not a STEP, STL, OBJ or 3MF file, so it cannot replace an imported \
             body."
        );
    }
}
