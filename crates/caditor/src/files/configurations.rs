use std::path::{Path, PathBuf};

use caditor_document::{
    CellChange, ColumnOutcome, ConfigurationImport, ImportColumn, ImportRowPlan,
    ImportedConfigurations, RowOutcome,
};
use caditor_file::{
    ConfigurationFileError, configurations_csv, read_configurations, write_configurations,
};
use egui::{Grid, ScrollArea, Ui};

use super::{Event, FileCommand, Files, Purpose, WriteKind, parameters::capitalised};
use crate::{
    appearance::SPACE_S,
    feature_tree::count,
    model::{Action, Model, Notice, display_name},
    widgets::{self, DialogWidth, Tone},
};

pub const NO_CONFIGURATIONS: &str = "The model has no configurations to export";
pub const EXPORT_HINT: &str = "Write the model's configurations to a CSV file: one row each, one \
                               column for every parameter, suppression and body colour they set";
pub const IMPORT_HINT: &str = "Read configurations from a CSV file with a Configuration column and \
                               columns headed Parameter:, Suppressed: or Colour: and a name, \
                               adding new rows and updating those the model has, after a preview";
pub const EXPORT_LABEL: &str = "Export to CSV…";
pub const IMPORT_LABEL: &str = "Import from CSV…";
const LIST_HEIGHT: widgets::ListHeight = widgets::ListHeight::Tall;
const REPLACE_LABEL: &str = "Replace configurations the model already has";
const REPLACE_HINT: &str = "Off, a configuration the model already has keeps its own values and \
                            the file's are left out";
const CHANGES_SHOWN: usize = 3;
const COLUMN_GRID: &str = "configuration-import-columns";
const ROW_GRID: &str = "configuration-import-rows";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationFileCommand {
    Replace(bool),
    Apply,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationImportDraft {
    path: PathBuf,
    session: u64,
    file: ImportedConfigurations,
    replace: bool,
    plan: ConfigurationImport,
}

pub fn file_name(model: &Model) -> String {
    let name = model.display_name();
    let stem = Path::new(&name)
        .file_stem()
        .map_or_else(|| name.clone(), |stem| stem.to_string_lossy().into_owned());
    format!(
        "{stem} configurations.{}",
        caditor_file::PARAMETERS_EXTENSION
    )
}

impl Files {
    pub(super) fn ask_configuration_export(&mut self, model: &mut Model) {
        if model.document().configurations().rows.is_empty() {
            model.set_notice(Notice::warning(format!("{NO_CONFIGURATIONS}.")));
            return;
        }
        self.pick(Purpose::ExportConfigurations, model);
    }

    pub(super) fn export_configurations(&mut self, path: PathBuf, model: &Model) {
        let text = configurations_csv(model.document());
        let exported = model.document().configurations().rows.len();
        let failed = path.clone();
        self.spawn_write(
            WriteKind::Configurations,
            path.clone(),
            move || Event::ConfigurationsExported {
                result: write_configurations(&path, &text),
                path,
                exported,
            },
            move || Event::ConfigurationsExported {
                path: failed,
                exported,
                result: Err(ConfigurationFileError::Internal),
            },
        );
    }

    pub(super) fn configurations_exported(
        &mut self,
        path: &Path,
        exported: usize,
        result: Result<(), ConfigurationFileError>,
        model: &mut Model,
    ) {
        let name = display_name(Some(path));
        model.set_notice(match result {
            Ok(()) => Notice::success(format!(
                "Exported {} to “{name}”.",
                count(exported, "configuration", "configurations")
            )),
            Err(error) => Notice::failure(format!(
                "Could not export the configurations to “{name}”: {error}."
            )),
        });
    }

    pub(super) fn read_configuration_file(&mut self, path: PathBuf, model: &Model) {
        let session = model.session();
        let failed = path.clone();
        self.spawn_own(
            "import",
            move || Event::ConfigurationsRead {
                result: read_configurations(&path),
                path,
                session,
            },
            move || Event::ConfigurationsRead {
                path: failed,
                session,
                result: Err(ConfigurationFileError::Internal),
            },
        );
    }

    pub(super) fn configurations_read(
        &mut self,
        path: PathBuf,
        session: u64,
        result: Result<ImportedConfigurations, ConfigurationFileError>,
        model: &mut Model,
    ) {
        if session != model.session() {
            return;
        }
        let name = display_name(Some(&path));
        let file = match result {
            Ok(file) if file.rows.is_empty() => {
                model.set_notice(Notice::warning(format!(
                    "“{name}” lists no configurations, so nothing was imported."
                )));
                return;
            }
            Ok(file) => file,
            Err(error) => {
                model.set_notice(Notice::failure(format!(
                    "Could not import configurations from “{name}”: {error}."
                )));
                return;
            }
        };
        match plan(model, &path, &file, true) {
            Ok(plan) => {
                self.configuration_import = Some(ConfigurationImportDraft {
                    path,
                    session,
                    file,
                    replace: true,
                    plan,
                });
            }
            Err(reason) => model.set_notice(Notice::failure(reason)),
        }
    }

    pub(super) fn configuration_import_command(
        &mut self,
        command: ConfigurationFileCommand,
        model: &mut Model,
    ) {
        let Some(draft) = self.configuration_import.as_mut() else {
            return;
        };
        match command {
            ConfigurationFileCommand::Cancel => self.configuration_import = None,
            ConfigurationFileCommand::Replace(replace) => {
                draft.replace = replace;
                match plan(model, &draft.path, &draft.file, replace) {
                    Ok(plan) => draft.plan = plan,
                    Err(reason) => {
                        self.configuration_import = None;
                        model.set_notice(Notice::failure(reason));
                    }
                }
            }
            ConfigurationFileCommand::Apply => {
                let Some(draft) = self.configuration_import.take() else {
                    return;
                };
                if draft.session != model.session() {
                    return;
                }
                match plan(model, &draft.path, &draft.file, draft.replace) {
                    Ok(plan) => {
                        let notice = summary(&plan);
                        if let Some(transaction) = plan.transaction {
                            model.perform(Action::Apply(transaction));
                        }
                        model.set_notice(notice);
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
    file: &ImportedConfigurations,
    replace: bool,
) -> Result<ConfigurationImport, String> {
    let name = display_name(Some(path));
    model
        .document()
        .plan_configuration_import(format!("Import configurations from {name}"), file, replace)
        .map_err(|error| format!("Could not import configurations from “{name}”: {error}."))
}

fn summary(plan: &ConfigurationImport) -> Notice {
    let added = plan.count_rows(|outcome| matches!(outcome, RowOutcome::Added { .. }));
    let changed = plan.count_rows(|outcome| matches!(outcome, RowOutcome::Changed { .. }));
    let kept = plan.count_rows(|outcome| matches!(outcome, RowOutcome::Kept { .. }));
    let refused = plan.count_rows(|outcome| matches!(outcome, RowOutcome::Refused(_)));
    let configured = plan.count_columns(|outcome| matches!(outcome, ColumnOutcome::Added));
    let skipped = plan.count_columns(|outcome| matches!(outcome, ColumnOutcome::Refused(_)));
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!(
            "added {}",
            count(added, "configuration", "configurations")
        ));
    }
    if changed > 0 {
        parts.push(format!("changed {changed}"));
    }
    if configured > 0 && plan.transaction.is_some() {
        parts.push(format!(
            "configured {}",
            count(configured, "more value", "more values")
        ));
    }
    if kept > 0 {
        parts.push(format!("kept the model's values for {kept}"));
    }
    if refused > 0 {
        parts.push(format!(
            "left out {} that could not be read",
            count(refused, "row", "rows")
        ));
    }
    if skipped > 0 {
        parts.push(format!(
            "left out {} the model has no match for",
            count(skipped, "column", "columns")
        ));
    }
    if parts.is_empty() {
        return Notice::success(
            "The configurations in the file match the model's, so nothing changed.",
        );
    }
    let mut sentence = format!("Imported configurations: {}.", parts.join(", "));
    if plan.transaction.is_some() {
        sentence.push_str(" Undo takes the import back in one step.");
    }
    if refused + skipped > 0 {
        Notice::warning(sentence)
    } else {
        Notice::success(sentence)
    }
}

fn row_pill(ui: &mut Ui, outcome: &RowOutcome) {
    let (tone, text) = match outcome {
        RowOutcome::Added { .. } => (Tone::Success, "New"),
        RowOutcome::Changed { .. } => (Tone::Warning, "Replaces"),
        RowOutcome::Kept { .. } => (Tone::Info, "Kept"),
        RowOutcome::Unchanged => (Tone::Info, "Same"),
        RowOutcome::Refused(_) => (Tone::Error, "Left out"),
    };
    widgets::status_pill(ui, tone, text);
}

fn column_pill(ui: &mut Ui, outcome: &ColumnOutcome) {
    let (tone, text) = match outcome {
        ColumnOutcome::Added => (Tone::Success, "New value"),
        ColumnOutcome::Known => (Tone::Info, "Configured"),
        ColumnOutcome::Refused(_) => (Tone::Error, "Left out"),
    };
    widgets::status_pill(ui, tone, text);
}

fn changes_text(changes: &[CellChange]) -> String {
    let mut shown: Vec<String> = changes
        .iter()
        .take(CHANGES_SHOWN)
        .map(|change| {
            format!(
                "{}: {} (was {})",
                change.column, change.after, change.before
            )
        })
        .collect();
    if changes.len() > CHANGES_SHOWN {
        shown.push(format!("and {} more", changes.len() - CHANGES_SHOWN));
    }
    shown.join("; ")
}

fn row_detail(row: &ImportRowPlan) -> String {
    match &row.outcome {
        RowOutcome::Added { values: 0 } => "with the model's current values".to_owned(),
        RowOutcome::Added { values } => count(*values, "value set", "values set"),
        RowOutcome::Changed { changes } => changes_text(changes),
        RowOutcome::Kept { changes } => {
            format!(
                "keeps the model's own; the file says {}",
                changes_text(changes)
            )
        }
        RowOutcome::Unchanged => "nothing differs".to_owned(),
        RowOutcome::Refused(reason) => capitalised(reason.to_string()),
    }
}

fn column_detail(column: &ImportColumn) -> String {
    match &column.outcome {
        ColumnOutcome::Added => "becomes a configured value".to_owned(),
        ColumnOutcome::Known => "already configured".to_owned(),
        ColumnOutcome::Refused(reason) => capitalised(reason.to_string()),
    }
}

pub fn dialog(ctx: &egui::Context, draft: &ConfigurationImportDraft) -> Option<FileCommand> {
    let name = display_name(Some(&draft.path));
    let response = widgets::dialog(
        ctx,
        "configuration-import",
        &format!("Import configurations from “{name}”"),
        DialogWidth::Medium,
        |ui| {
            let mut command = None;
            let plan = &draft.plan;
            let refused = plan.count_rows(|outcome| matches!(outcome, RowOutcome::Refused(_)));
            let skipped =
                plan.count_columns(|outcome| matches!(outcome, ColumnOutcome::Refused(_)));
            let conflicts = plan.conflicts();
            ui.label(format!(
                "{} and {} in the file. Nothing changes until you import.",
                count(plan.rows.len(), "row", "rows"),
                count(plan.columns.len(), "column", "columns")
            ));
            if conflicts > 0 {
                widgets::callout(ui, Tone::Warning, |ui| {
                    ui.label(format!(
                        "{} already in the model with other values.",
                        count(conflicts, "configuration is", "configurations are")
                    ));
                });
            }
            if skipped > 0 {
                widgets::callout(ui, Tone::Error, |ui| {
                    ui.label(format!(
                        "{} name something the model does not have and will be left out; the \
                         reason is beside each.",
                        count(skipped, "column", "columns")
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
            let toggle =
                widgets::checkbox(ui, &mut replace, REPLACE_LABEL).on_hover_text(REPLACE_HINT);
            if toggle.changed() {
                command = Some(ConfigurationFileCommand::Replace(replace));
            }
            ui.add_space(SPACE_S);
            let height = widgets::list_height(ui.ctx(), LIST_HEIGHT);
            widgets::card(ui, |ui| {
                ScrollArea::vertical().max_height(height).show(ui, |ui| {
                    Grid::new(COLUMN_GRID)
                        .num_columns(3)
                        .spacing([SPACE_S * 2.0, SPACE_S])
                        .show(ui, |ui| {
                            for column in &plan.columns {
                                ui.label(widgets::strong(column.header.as_str()));
                                column_pill(ui, &column.outcome);
                                ui.add(egui::Label::new(column_detail(column)).wrap());
                                ui.end_row();
                            }
                        });
                    ui.separator();
                    Grid::new(ROW_GRID)
                        .num_columns(3)
                        .spacing([SPACE_S * 2.0, SPACE_S])
                        .show(ui, |ui| {
                            for row in &plan.rows {
                                ui.label(widgets::strong(row.name.as_str()));
                                row_pill(ui, &row.outcome);
                                ui.add(egui::Label::new(row_detail(row)).wrap());
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
                    command = Some(ConfigurationFileCommand::Apply);
                }
                if ui.add(widgets::button("Cancel")).clicked() {
                    command = Some(ConfigurationFileCommand::Cancel);
                }
            });
            command
        },
    );
    let closed = response
        .should_close()
        .then_some(ConfigurationFileCommand::Cancel);
    response
        .inner
        .or(closed)
        .map(FileCommand::ConfigurationImport)
}
