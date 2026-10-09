use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use caditor_document::{
    CancelToken, ConfigurationId, Document, EditError, FeatureId, ModelEvaluator, ModelProperties,
    Recompute,
};
use caditor_file::{ExportBody, ExportError, ExportFormat, Exported, MeshOptions};

use crate::{
    export::{ExportSource, with_format_extension},
    feature_tree::count,
    model::{Notice, display_name},
};

const UNSAFE_NAME_CHARACTERS: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const SAFE_REPLACEMENT: char = '-';

#[derive(Debug, thiserror::Error)]
pub enum ConfigurationFailure {
    #[error("the model could not be switched to it ({0})")]
    Switch(EditError),
    #[error("{0}")]
    Export(ExportError),
}

#[derive(Debug)]
pub struct ConfigurationExported {
    pub name: String,
    pub path: PathBuf,
    pub failed: usize,
    pub result: Result<Exported, ConfigurationFailure>,
}

pub struct ConfigurationJob {
    pub document: Document,
    pub configurations: Vec<(ConfigurationId, String)>,
    pub left_out: BTreeSet<FeatureId>,
    pub path: PathBuf,
    pub format: ExportFormat,
    pub done: Arc<AtomicUsize>,
}

pub fn exportable(document: &Document) -> Vec<(ConfigurationId, String)> {
    let configurations = document.configurations();
    if configurations.rows.len() < 2 {
        return Vec::new();
    }
    configurations
        .rows
        .iter()
        .map(|row| (row.id, row.name.clone()))
        .collect()
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_control() || UNSAFE_NAME_CHARACTERS.contains(&character) {
                SAFE_REPLACEMENT
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_owned()
}

pub fn file_for(path: &Path, name: &str, format: ExportFormat) -> PathBuf {
    let stem = path.file_stem().map(OsString::from).unwrap_or_default();
    let mut file = stem;
    let safe = safe_name(name);
    if !safe.is_empty() {
        if !file.is_empty() {
            file.push(" ");
        }
        file.push(safe);
    }
    let named = path.with_file_name(file);
    with_format_extension(named, format)
}

impl ConfigurationJob {
    pub fn run(
        self,
        options: &MeshOptions<'_>,
        properties: &ModelProperties,
        cancel: &CancelToken,
    ) -> Result<Vec<ConfigurationExported>, ExportError> {
        let mut engine = Recompute::default();
        let mut outcomes = Vec::with_capacity(self.configurations.len());
        for (id, name) in &self.configurations {
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let path = file_for(&self.path, name, self.format);
            let mut document = self.document.clone();
            let switched = document
                .activating(*id)
                .and_then(|transaction| document.apply(transaction));
            if let Err(error) = switched {
                outcomes.push(ConfigurationExported {
                    name: name.clone(),
                    path,
                    failed: 0,
                    result: Err(ConfigurationFailure::Switch(error)),
                });
                self.done.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            let evaluation = engine.run_without_display(&document, &ModelEvaluator, cancel);
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let sources: Vec<ExportSource> = evaluation
                .bodies()
                .map(|(body, _)| body)
                .filter(|body| !self.left_out.contains(body))
                .filter_map(|body| ExportSource::of(&document, &evaluation, body))
                .collect();
            let bodies: Vec<ExportBody<'_>> =
                sources.iter().filter_map(ExportSource::exported).collect();
            let exported = caditor_file::export_bodies(
                &path,
                self.format,
                options,
                &bodies,
                properties,
                cancel,
            );
            if matches!(exported, Err(ExportError::Cancelled)) {
                return Err(ExportError::Cancelled);
            }
            outcomes.push(ConfigurationExported {
                name: name.clone(),
                path,
                failed: evaluation.failed_count(),
                result: exported.map_err(ConfigurationFailure::Export),
            });
            self.done.fetch_add(1, Ordering::SeqCst);
        }
        Ok(outcomes)
    }
}

fn quoted(path: &Path) -> String {
    format!("“{}”", display_name(Some(path)))
}

fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub fn finished(result: Result<Vec<ConfigurationExported>, ExportError>) -> Notice {
    let outcomes = match result {
        Ok(outcomes) => outcomes,
        Err(ExportError::Cancelled) => {
            return Notice::info(
                "The export was cancelled; the configurations already written were kept.",
            );
        }
        Err(error) => {
            return Notice::failure(format!("Could not export the configurations: {error}."));
        }
    };
    let written: Vec<String> = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_ok())
        .map(|outcome| quoted(&outcome.path))
        .collect();
    let mut problems: Vec<String> = Vec::new();
    for outcome in &outcomes {
        match &outcome.result {
            Ok(exported) => {
                if outcome.failed > 0 {
                    problems.push(format!(
                        "in “{}” {} failed, so its bodies were exported as they were before them",
                        outcome.name,
                        count(outcome.failed, "feature", "features")
                    ));
                }
                if !exported.left_out.is_empty() {
                    let reasons: Vec<String> =
                        exported.left_out.iter().map(ToString::to_string).collect();
                    problems.push(format!(
                        "in “{}” {} left out: {}",
                        outcome.name,
                        count(reasons.len(), "body was", "bodies were"),
                        reasons.join("; ")
                    ));
                }
            }
            Err(error) => problems.push(format!("“{}” was not exported: {error}", outcome.name)),
        }
    }
    let summary = if written.is_empty() {
        "No configuration was exported.".to_owned()
    } else {
        format!(
            "Exported {} to {}.",
            count(written.len(), "configuration", "configurations"),
            listed(&written)
        )
    };
    if problems.is_empty() {
        return Notice::info(summary);
    }
    Notice::failure(format!("{summary} But {}.", problems.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_configuration_is_written_beside_the_chosen_file_under_its_own_name() {
        let chosen = Path::new("/parts/bracket.stl");

        assert_eq!(
            file_for(chosen, "M4", ExportFormat::Stl),
            PathBuf::from("/parts/bracket M4.stl")
        );
        assert_eq!(
            file_for(chosen, "A/B: big?", ExportFormat::Stl),
            PathBuf::from("/parts/bracket A-B- big-.stl")
        );
        assert_eq!(
            file_for(Path::new("/parts/bracket.step"), " . ", ExportFormat::Step),
            PathBuf::from("/parts/bracket.step")
        );
    }

    #[test]
    fn the_notice_names_every_file_and_what_went_wrong() {
        let outcomes = vec![
            ConfigurationExported {
                name: "M4".to_owned(),
                path: PathBuf::from("bracket M4.stl"),
                failed: 0,
                result: Ok(Exported {
                    bodies: 1,
                    triangles: Some(12),
                    left_out: Vec::new(),
                    moved: None,
                }),
            },
            ConfigurationExported {
                name: "M8".to_owned(),
                path: PathBuf::from("bracket M8.stl"),
                failed: 0,
                result: Err(ConfigurationFailure::Switch(
                    EditError::MissingConfiguration,
                )),
            },
        ];

        let notice = finished(Ok(outcomes));

        assert!(notice.outlasts_edits);
        assert_eq!(
            notice.text,
            "Exported 1 configuration to “bracket M4.stl”. But “M8” was not exported: the model \
             could not be switched to it (That configuration no longer exists)."
        );
    }
}
