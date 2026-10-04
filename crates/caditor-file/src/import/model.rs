use std::{path::Path, time::SystemTime};

use caditor_document::{Document, FeatureKind, Import, Transaction};
use caditor_kernel::Solid;
use caditor_step::{ReadError, StepBody, read_step, write_step};

use crate::{import::ImportError, read::read_file, reason};

const LATIN_1_NOTE: &str = "The file is not UTF-8 text, so its names were read as Latin-1; \
                            letters outside it may look wrong.";

pub const STEP_IMPORT_EXTENSIONS: [&str; 3] = ["step", "stp", "p21"];

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedBody {
    pub name: String,
    pub import: Import,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelImport {
    pub bodies: Vec<ImportedBody>,
    pub notes: Vec<String>,
}

pub fn read_step_file(path: &Path) -> Result<ModelImport, ImportError> {
    let bytes = read_file(path).map_err(|error| ImportError::Reading(reason::reading(&error)))?;
    let source = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match String::from_utf8(bytes) {
        Ok(text) => parse_step(&text, &source),
        Err(error) => {
            let text: String = error
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect();
            let mut import = parse_step(&text, &source)?;
            import.notes.push(LATIN_1_NOTE.to_owned());
            Ok(import)
        }
    }
}

pub fn parse_step(text: &str, source: &str) -> Result<ModelImport, ImportError> {
    let model = read_step(text).map_err(|error| match error {
        ReadError::NotStep => ImportError::NotStep,
        other => ImportError::Model(other.to_string()),
    })?;
    let mut imported = ModelImport {
        bodies: Vec::with_capacity(model.solids.len()),
        notes: model.notes,
    };
    let mut lost = Vec::new();
    for solid in model.solids {
        match canonical(&solid.name, &solid.solid) {
            Some(lumps) => imported
                .bodies
                .extend(lumps.into_iter().map(|(stored, step)| ImportedBody {
                    import: Import::new(source, stored, step),
                    name: solid.name.clone(),
                })),
            None => lost.push(solid.name),
        }
    }
    for name in lost {
        imported.notes.push(format!(
            "“{name}” was read but could not be stored in the model, so it was left out."
        ));
    }
    if imported.bodies.is_empty() {
        return Err(ImportError::Model(
            "none of its bodies could be stored in the model".to_owned(),
        ));
    }
    Ok(imported)
}

fn canonical(name: &str, solid: &Solid) -> Option<Vec<(Solid, String)>> {
    let (step, mut again) = written_and_read(name, solid)?;
    match again.len() {
        0 => return None,
        1 => return Some(vec![(again.swap_remove(0), step)]),
        _ => {}
    }
    again
        .iter()
        .map(|lump| {
            let (step, mut alone) = written_and_read(name, lump)?;
            (alone.len() == 1).then(|| (alone.swap_remove(0), step))
        })
        .collect()
}

fn written_and_read(name: &str, solid: &Solid) -> Option<(String, Vec<Solid>)> {
    let step = write_step(&[StepBody { name, solid }], name, SystemTime::UNIX_EPOCH).ok()?;
    let again = read_step(&step).ok()?;
    Some((
        step,
        again.solids.into_iter().map(|read| read.solid).collect(),
    ))
}

pub fn bodies_transaction(
    document: &Document,
    bodies: &[ImportedBody],
    label: impl Into<String>,
) -> Transaction {
    let mut builder = document.transaction(label);
    let mut taken: Vec<String> = document
        .features()
        .map(|feature| feature.name.clone())
        .collect();
    for body in bodies {
        let name = unique_name(&body.name, &taken);
        taken.push(name.clone());
        builder.add_feature(name, FeatureKind::Import(body.import.clone()));
    }
    builder.finish()
}

fn unique_name(wanted: &str, taken: &[String]) -> String {
    let wanted = wanted.trim();
    let base = if wanted.is_empty() { "Import" } else { wanted };
    if !taken.iter().any(|name| name == base) {
        return base.to_owned();
    }
    (2..)
        .map(|number| format!("{base} {number}"))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| base.to_owned())
}
