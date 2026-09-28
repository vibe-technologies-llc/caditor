use std::{path::Path, time::SystemTime};

use caditor_document::{Document, FeatureKind, Import, Transaction};
use caditor_step::{ReadError, StepBody, read_step, write_step};

use crate::{import::ImportError, reason};

pub const STEP_IMPORT_EXTENSIONS: [&str; 2] = ["step", "stp"];

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
    let bytes =
        std::fs::read(path).map_err(|error| ImportError::Reading(reason::reading(&error)))?;
    let source = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    parse_step(&String::from_utf8_lossy(&bytes), &source)
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
            Some((stored, step)) => imported.bodies.push(ImportedBody {
                import: Import::new(source, stored, step),
                name: solid.name,
            }),
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

fn canonical(name: &str, solid: &caditor_kernel::Solid) -> Option<(caditor_kernel::Solid, String)> {
    let step = write_step(&[StepBody { name, solid }], name, SystemTime::UNIX_EPOCH).ok()?;
    let mut again = read_step(&step).ok()?;
    let stored = (again.solids.len() == 1).then(|| again.solids.swap_remove(0).solid)?;
    Some((stored, step))
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
