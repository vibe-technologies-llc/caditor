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
const UNNAMED_LABEL: &str = "Configuration";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationFile {
    pub id: ConfigurationId,
    pub name: String,
    pub path: PathBuf,
}

pub struct ConfigurationJob {
    pub document: Document,
    pub files: Vec<ConfigurationFile>,
    pub left_out: BTreeSet<FeatureId>,
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

fn label(name: &str, position: usize) -> String {
    let safe = safe_name(name);
    if safe.is_empty() {
        format!("{UNNAMED_LABEL} {position}")
    } else {
        safe
    }
}

fn file_for(path: &Path, label: &str, format: ExportFormat) -> PathBuf {
    let mut file = path.file_stem().map(OsString::from).unwrap_or_default();
    if !file.is_empty() {
        file.push(" ");
    }
    file.push(label);
    with_format_extension(path.with_file_name(file), format)
}

fn folded(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

pub fn files_for(
    path: &Path,
    configurations: Vec<(ConfigurationId, String)>,
    format: ExportFormat,
) -> Vec<ConfigurationFile> {
    let labels: Vec<String> = configurations
        .iter()
        .zip(1..)
        .map(|((_, name), position)| label(name, position))
        .collect();
    let natural: BTreeSet<String> = labels
        .iter()
        .map(|label| folded(&file_for(path, label, format)))
        .collect();

    let mut taken = BTreeSet::new();
    configurations
        .into_iter()
        .zip(labels)
        .map(|((id, name), label)| {
            let mut file = file_for(path, &label, format);
            let mut copy = 1;
            while taken.contains(&folded(&file)) || (copy > 1 && natural.contains(&folded(&file))) {
                copy += 1;
                file = file_for(path, &format!("{label} ({copy})"), format);
            }
            taken.insert(folded(&file));
            ConfigurationFile {
                id,
                name,
                path: file,
            }
        })
        .collect()
}

impl ConfigurationJob {
    pub fn run(
        self,
        options: &MeshOptions<'_>,
        properties: &ModelProperties,
        cancel: &CancelToken,
    ) -> Result<Vec<ConfigurationExported>, ExportError> {
        let mut engine = Recompute::default();
        let mut outcomes = Vec::with_capacity(self.files.len());
        for ConfigurationFile { id, name, path } in self.files {
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let mut document = self.document.clone();
            let switched = document
                .activating(id)
                .and_then(|transaction| document.apply(transaction));
            if let Err(error) = switched {
                outcomes.push(ConfigurationExported {
                    name,
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
                name,
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
        return Notice::success(summary);
    }
    Notice::failure(format!("{summary} But {}.", problems.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(chosen: &str, names: &[&str], format: ExportFormat) -> Vec<PathBuf> {
        let configurations = names
            .iter()
            .zip(1..)
            .map(|(name, raw)| (ConfigurationId::from_raw(raw), (*name).to_owned()))
            .collect();

        files_for(Path::new(chosen), configurations, format)
            .into_iter()
            .map(|file| file.path)
            .collect()
    }

    #[test]
    fn each_configuration_is_written_beside_the_chosen_file_under_its_own_name() {
        let written = files(
            "/parts/bracket.stl",
            &["M4", "A/B: big?", " . "],
            ExportFormat::Stl,
        );

        assert_eq!(
            written,
            [
                PathBuf::from("/parts/bracket M4.stl"),
                PathBuf::from("/parts/bracket A-B- big-.stl"),
                PathBuf::from("/parts/bracket Configuration 3.stl"),
            ]
        );
    }

    #[test]
    fn configurations_whose_names_make_the_same_file_get_files_of_their_own() {
        let written = files(
            "/parts/bracket.step",
            &[
                "M4/20",
                "M4-20",
                "m4-20",
                "A.",
                "A",
                "M4-20 (2)",
                "",
                "Configuration 7",
            ],
            ExportFormat::Step,
        );

        assert_eq!(
            written,
            [
                PathBuf::from("/parts/bracket M4-20.step"),
                PathBuf::from("/parts/bracket M4-20 (3).step"),
                PathBuf::from("/parts/bracket m4-20 (4).step"),
                PathBuf::from("/parts/bracket A.step"),
                PathBuf::from("/parts/bracket A (2).step"),
                PathBuf::from("/parts/bracket M4-20 (2).step"),
                PathBuf::from("/parts/bracket Configuration 7.step"),
                PathBuf::from("/parts/bracket Configuration 7 (2).step"),
            ]
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
