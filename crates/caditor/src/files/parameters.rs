use std::path::{Path, PathBuf};

use caditor_document::{ImportOutcome, ImportRow, ImportedParameter, ParameterImport};
use caditor_file::{
    PARAMETERS_EXTENSION, ParameterFileError, parameters_csv, read_parameters, write_parameters,
};
use egui::{Grid, ScrollArea, Ui};

use super::{Event, FileCommand, Files, Purpose};
use crate::{
    appearance::SPACE_S,
    feature_tree::count,
    model::{Action, Model, Notice, display_name},
    widgets::{self, DialogWidth, Tone},
};

pub const NO_PARAMETERS: &str = "The model has no parameters to export";
pub const EXPORT_HINT: &str = "Write the model's parameters to a CSV file: names, expressions \
                               with their units, values and notes";
pub const IMPORT_HINT: &str = "Read parameters from a CSV file with name and expression columns, \
                               adding new names and updating those the model has, after a \
                               preview";
const LIST_HEIGHT: f32 = 320.0;
const REPLACE_LABEL: &str = "Replace values the model already has";
const REPLACE_HINT: &str = "Off, a parameter the model already has keeps its own expression and \
                            the file's is left out";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParametersCommand {
    Replace(bool),
    Apply,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterImportDraft {
    path: PathBuf,
    session: u64,
    rows: Vec<ImportedParameter>,
    replace: bool,
    plan: ParameterImport,
}

pub fn with_csv_extension(path: PathBuf) -> PathBuf {
    let has = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(PARAMETERS_EXTENSION));
    if has {
        return path;
    }
    let mut named = path.into_os_string();
    named.push(".");
    named.push(PARAMETERS_EXTENSION);
    PathBuf::from(named)
}

pub fn file_name(model: &Model) -> String {
    let name = model.display_name();
    let stem = Path::new(&name)
        .file_stem()
        .map_or_else(|| name.clone(), |stem| stem.to_string_lossy().into_owned());
    format!("{stem} parameters.{PARAMETERS_EXTENSION}")
}

impl Files {
    pub(super) fn ask_parameter_export(&mut self, model: &mut Model) {
        if model.document().parameters().is_empty() {
            model.set_notice(Notice::info(format!("{NO_PARAMETERS}.")));
            return;
        }
        self.pick(Purpose::ExportParameters, model);
    }

    pub(super) fn export_parameters(&mut self, path: PathBuf, model: &Model) {
        let text = parameters_csv(model.document(), model.parameters());
        let exported = model.document().parameters().len();
        let failed = path.clone();
        self.spawn(
            move || Event::ParametersExported {
                result: write_parameters(&path, &text),
                path,
                exported,
            },
            move || Event::ParametersExported {
                path: failed,
                exported,
                result: Err(ParameterFileError::Internal),
            },
        );
    }

    pub(super) fn parameters_exported(
        &mut self,
        path: &Path,
        exported: usize,
        result: Result<(), ParameterFileError>,
        model: &mut Model,
    ) {
        let name = display_name(Some(path));
        model.set_notice(match result {
            Ok(()) => Notice::info(format!(
                "Exported {} to “{name}”.",
                count(exported, "parameter", "parameters")
            )),
            Err(error) => Notice::failure(format!(
                "Could not export the parameters to “{name}”: {error}."
            )),
        });
    }

    pub(super) fn read_parameter_file(&mut self, path: PathBuf, model: &Model) {
        let session = model.session();
        let failed = path.clone();
        self.spawn_own(
            "import",
            move || Event::ParametersRead {
                result: read_parameters(&path),
                path,
                session,
            },
            move || Event::ParametersRead {
                path: failed,
                session,
                result: Err(ParameterFileError::Internal),
            },
        );
    }

    pub(super) fn parameters_read(
        &mut self,
        path: PathBuf,
        session: u64,
        result: Result<Vec<ImportedParameter>, ParameterFileError>,
        model: &mut Model,
    ) {
        if session != model.session() {
            return;
        }
        let name = display_name(Some(&path));
        let rows = match result {
            Ok(rows) if rows.is_empty() => {
                model.set_notice(Notice::info(format!(
                    "“{name}” lists no parameters, so nothing was imported."
                )));
                return;
            }
            Ok(rows) => rows,
            Err(error) => {
                model.set_notice(Notice::failure(format!(
                    "Could not import parameters from “{name}”: {error}."
                )));
                return;
            }
        };
        match plan(model, &path, &rows, true) {
            Ok(plan) => {
                self.parameter_import = Some(ParameterImportDraft {
                    path,
                    session,
                    rows,
                    replace: true,
                    plan,
                });
            }
            Err(reason) => model.set_notice(Notice::failure(reason)),
        }
    }

    pub(super) fn parameter_import_command(
        &mut self,
        command: ParametersCommand,
        model: &mut Model,
    ) {
        let Some(draft) = self.parameter_import.as_mut() else {
            return;
        };
        match command {
            ParametersCommand::Cancel => self.parameter_import = None,
            ParametersCommand::Replace(replace) => {
                draft.replace = replace;
                match plan(model, &draft.path, &draft.rows, replace) {
                    Ok(plan) => draft.plan = plan,
                    Err(reason) => {
                        self.parameter_import = None;
                        model.set_notice(Notice::failure(reason));
                    }
                }
            }
            ParametersCommand::Apply => {
                let Some(draft) = self.parameter_import.take() else {
                    return;
                };
                if draft.session != model.session() {
                    return;
                }
                match plan(model, &draft.path, &draft.rows, draft.replace) {
                    Ok(plan) => {
                        let note = summary(&plan);
                        if let Some(transaction) = plan.transaction {
                            model.perform(Action::Apply(transaction));
                        }
                        model.set_notice(Notice::info(note));
                    }
                    Err(reason) => model.set_notice(Notice::failure(reason)),
                }
            }
        }
    }
}

fn plan(
    model: &Model,
    path: &Path,
    rows: &[ImportedParameter],
    replace: bool,
) -> Result<ParameterImport, String> {
    let name = display_name(Some(path));
    model
        .document()
        .plan_parameter_import(format!("Import parameters from {name}"), rows, replace)
        .map_err(|error| format!("Could not import parameters from “{name}”: {error}."))
}

fn summary(plan: &ParameterImport) -> String {
    let added = plan.count(|outcome| matches!(outcome, ImportOutcome::Added));
    let changed = plan.count(|outcome| matches!(outcome, ImportOutcome::Changed { .. }));
    let kept = plan.count(|outcome| matches!(outcome, ImportOutcome::Kept { .. }));
    let refused = plan.count(|outcome| matches!(outcome, ImportOutcome::Refused(_)));
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("added {}", count(added, "parameter", "parameters")));
    }
    if changed > 0 {
        parts.push(format!("changed {changed}"));
    }
    if kept > 0 {
        parts.push(format!("kept the model's value for {kept}"));
    }
    if refused > 0 {
        parts.push(format!("left out {refused} that could not be read"));
    }
    if parts.is_empty() {
        return "The parameters in the file match the model's, so nothing changed.".to_owned();
    }
    let mut sentence = format!("Imported parameters: {}.", parts.join(", "));
    if added + changed > 0 {
        sentence.push_str(" Undo takes the import back in one step.");
    }
    sentence
}

fn outcome_pill(ui: &mut Ui, outcome: &ImportOutcome) {
    let (tone, text) = match outcome {
        ImportOutcome::Added => (Tone::Success, "New"),
        ImportOutcome::Changed { .. } => (Tone::Warning, "Replaces"),
        ImportOutcome::Kept { .. } => (Tone::Info, "Kept"),
        ImportOutcome::Unchanged => (Tone::Info, "Same"),
        ImportOutcome::Refused(_) => (Tone::Error, "Left out"),
    };
    widgets::status_pill(ui, tone, text);
}

fn detail(row: &ImportRow) -> String {
    match &row.outcome {
        ImportOutcome::Added => row.expression.clone(),
        ImportOutcome::Changed { before } => format!("{} (was {before})", row.expression),
        ImportOutcome::Kept { theirs } => format!("keeps the model's own; the file says {theirs}"),
        ImportOutcome::Unchanged => row.expression.clone(),
        ImportOutcome::Refused(reason) => {
            let mut reason = reason.to_string();
            if let Some(first) = reason.get(..1) {
                let upper = first.to_uppercase();
                reason.replace_range(..1, &upper);
            }
            reason
        }
    }
}

pub fn dialog(ctx: &egui::Context, draft: &ParameterImportDraft) -> Option<FileCommand> {
    let name = display_name(Some(&draft.path));
    let response = widgets::dialog(
        ctx,
        "parameter-import",
        &format!("Import parameters from “{name}”"),
        DialogWidth::Medium,
        |ui| {
            let mut command = None;
            let plan = &draft.plan;
            let refused = plan.count(|outcome| matches!(outcome, ImportOutcome::Refused(_)));
            let conflicts = plan.conflicts();
            ui.label(format!(
                "{} in the file. Nothing changes until you import.",
                count(plan.rows.len(), "row", "rows")
            ));
            if conflicts > 0 {
                widgets::callout(ui, Tone::Warning, |ui| {
                    ui.label(format!(
                        "{} already in the model with another value.",
                        count(conflicts, "parameter is", "parameters are")
                    ));
                });
            }
            if refused > 0 {
                widgets::callout(ui, Tone::Error, |ui| {
                    ui.label(format!(
                        "{} cannot be imported and will be left out; the reason is beside each.",
                        count(refused, "row", "rows")
                    ));
                });
            }
            let mut replace = draft.replace;
            let toggle = ui
                .checkbox(&mut replace, REPLACE_LABEL)
                .on_hover_text(REPLACE_HINT);
            if toggle.changed() {
                command = Some(ParametersCommand::Replace(replace));
            }
            ui.add_space(SPACE_S);
            let height = widgets::list_height(ui.ctx(), LIST_HEIGHT);
            widgets::card(ui, |ui| {
                ScrollArea::vertical().max_height(height).show(ui, |ui| {
                    Grid::new("parameter-import-rows")
                        .num_columns(3)
                        .spacing([SPACE_S * 2.0, SPACE_S])
                        .show(ui, |ui| {
                            for row in &plan.rows {
                                ui.label(widgets::strong(row.name.as_str()));
                                outcome_pill(ui, &row.outcome);
                                ui.add(egui::Label::new(detail(row)).wrap());
                                ui.end_row();
                            }
                        });
                });
            });
            let blocker = plan
                .transaction
                .is_none()
                .then_some("Nothing in the file would change the model.");
            widgets::footer(ui, |ui| {
                let import = widgets::primary_button(ui, "Import");
                let response = ui.add_enabled(blocker.is_none(), import);
                let response = match blocker {
                    Some(blocker) => response.on_disabled_hover_text(blocker),
                    None => response,
                };
                if response.clicked() {
                    command = Some(ParametersCommand::Apply);
                }
                if ui.add(widgets::button("Cancel")).clicked() {
                    command = Some(ParametersCommand::Cancel);
                }
            });
            command
        },
    );
    let closed = response.should_close().then_some(ParametersCommand::Cancel);
    response.inner.or(closed).map(FileCommand::Parameters)
}
