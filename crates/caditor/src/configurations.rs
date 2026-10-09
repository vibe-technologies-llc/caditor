use caditor_document::{
    Configuration, ConfigurationId, ConfiguredValue, Document, MAX_CONFIGURATION_NAME_CHARS, Rgb,
    Setting, Transaction,
};
use egui::{ComboBox, Grid, Id, Key, ScrollArea, TextEdit, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    body_appearance::{DEFAULT_COLOUR_NAME, SWATCHES},
    commands::Command,
    dialog_parts::BodyRoom,
    field, icons,
    model::Model,
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Configurations";
pub const CLOSE_LABEL: &str = "Close";
pub const ADD_LABEL: &str = "Add configuration";
pub const CONFIGURE_LABEL: &str = "Configure a value";
pub const NAME_CAPTION: &str = "Name";
const EXPLANATION: &str = "Keep several versions of one part in this model, such as a bracket in \
                           M4, M6 and M8. Each configuration sets the values chosen here; the \
                           active one is the model as it stands, and editing a value changes it. \
                           Switch to another to see it, and export every one from Export.";
const EMPTY: &str = "This model has no configurations yet. Add one to keep its current values, \
                     then add another and give it other values.";
const NO_VALUES: &str = "No value is configured yet, so every configuration is the same model. \
                         Choose a parameter, a feature to suppress or a body's colour below.";
const DIALOG_ID: &str = "configurations";
const HEIGHT_SHARE: f32 = 0.8;
const CELL_WIDTH: f32 = 96.0;
const RENAME_HOVER: &str = "Rename it";
const DUPLICATE_HOVER: &str = "Add a copy of it";
const DELETE_HOVER: &str = "Delete it";
const DELETED_PARAMETER: &str = "a deleted parameter";
const DELETED_FEATURE: &str = "a deleted feature";

#[derive(Debug, Clone, PartialEq)]
struct Renaming {
    id: ConfigurationId,
    to: String,
    focus_pending: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigurationsDraft {
    renaming: Option<Renaming>,
    problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Apply(Transaction),
}

pub fn rename_field_id() -> Id {
    Id::new((DIALOG_ID, "rename"))
}

pub fn cell_id(row: ConfigurationId, value: ConfiguredValue) -> Id {
    Id::new((DIALOG_ID, "cell", row, value))
}

pub fn value_name(document: &Document, value: ConfiguredValue) -> String {
    match value {
        ConfiguredValue::Parameter(id) => document
            .parameter_name(id)
            .unwrap_or(DELETED_PARAMETER)
            .to_owned(),
        ConfiguredValue::Suppressed(id) => format!(
            "{} suppressed",
            document
                .feature(id)
                .map_or(DELETED_FEATURE, |feature| feature.name.as_str())
        ),
        ConfiguredValue::Colour(id) => format!(
            "{} colour",
            document.body_name(id).unwrap_or(DELETED_FEATURE)
        ),
    }
}

fn shown(document: &Document, value: ConfiguredValue) -> bool {
    document.live_setting(value).is_some()
}

pub fn offered_values(document: &Document) -> Vec<ConfiguredValue> {
    let configurations = document.configurations();
    let parameters = document
        .parameters()
        .iter()
        .map(|parameter| ConfiguredValue::Parameter(parameter.id()));
    let suppressions = document
        .features()
        .map(|feature| ConfiguredValue::Suppressed(feature.id()));
    let colours = document
        .features()
        .filter(|feature| feature.makes_body())
        .map(|feature| ConfiguredValue::Colour(feature.id()));
    parameters
        .chain(suppressions)
        .chain(colours)
        .filter(|value| !configurations.configures(*value))
        .collect()
}

pub fn colour_name(colour: Option<Rgb>) -> String {
    match colour {
        None => DEFAULT_COLOUR_NAME.to_owned(),
        Some(colour) => SWATCHES
            .iter()
            .find(|swatch| swatch.colour == colour)
            .map_or_else(|| colour.hex(), |swatch| swatch.name.to_owned()),
    }
}

fn checked(document: &Document, built: Result<Transaction, String>) -> Result<Transaction, String> {
    let transaction = built?;
    document
        .check(&transaction)
        .map_err(|error| field::sentence(&error.to_string()))?;
    Ok(transaction)
}

fn applying(
    draft: &mut ConfigurationsDraft,
    built: Result<Transaction, String>,
) -> Option<Outcome> {
    match built {
        Ok(transaction) => {
            draft.problem = None;
            Some(Outcome::Apply(transaction))
        }
        Err(problem) => {
            draft.problem = Some(problem);
            None
        }
    }
}

pub fn switching(document: &Document, id: ConfigurationId) -> Result<Transaction, String> {
    checked(
        document,
        document.activating(id).map_err(|error| error.to_string()),
    )
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    draft: &mut ConfigurationsDraft,
) -> Option<Outcome> {
    let response = widgets::dialog(ctx, DIALOG_ID, TITLE, DialogWidth::Wide, |ui| {
        ui.label(widgets::muted(EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let mut room = BodyRoom::measure(ui, DIALOG_ID, HEIGHT_SHARE);
        let outcome = ScrollArea::vertical()
            .max_height(room.height)
            .show(ui, |ui| body(ui, model, draft))
            .inner;
        room.body_ended(ui);
        let closed = widgets::footer(ui, |ui| {
            ui.add(widgets::primary_button(ui, CLOSE_LABEL)).clicked()
        });
        room.dialog_ended(ui);
        match (outcome, closed) {
            (Some(outcome), _) => Some(outcome),
            (None, true) => Some(Outcome::Close),
            (None, false) => None,
        }
    });
    let closed = response.should_close().then_some(Outcome::Close);
    response.inner.or(closed)
}

fn body(ui: &mut Ui, model: &Model, draft: &mut ConfigurationsDraft) -> Option<Outcome> {
    let document = model.document();
    let configurations = document.configurations();
    let mut outcome = None;
    if configurations.rows.is_empty() {
        ui.label(widgets::muted(EMPTY, ui));
    } else {
        let values: Vec<ConfiguredValue> = configurations
            .values
            .iter()
            .copied()
            .filter(|value| shown(document, *value))
            .collect();
        if values.is_empty() {
            ui.label(widgets::muted(NO_VALUES, ui));
            ui.add_space(SPACE_S);
        }
        ScrollArea::horizontal()
            .id_salt((DIALOG_ID, "table"))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if let Some(chosen) = table(ui, model, draft, &values) {
                    outcome = Some(chosen);
                }
            });
    }
    ui.add_space(SPACE_M);
    if let Some(added) = adding(ui, document, draft) {
        outcome = Some(added);
    }
    if let Some(problem) = &draft.problem {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Error, |ui| ui.label(problem));
    }
    outcome
}

fn table(
    ui: &mut Ui,
    model: &Model,
    draft: &mut ConfigurationsDraft,
    values: &[ConfiguredValue],
) -> Option<Outcome> {
    let document = model.document();
    let mut outcome = None;
    Grid::new((DIALOG_ID, "grid"))
        .striped(true)
        .spacing([SPACE_M, SPACE_S])
        .show(ui, |ui| {
            ui.label("");
            widgets::column_caption(ui, NAME_CAPTION);
            for value in values {
                let name = value_name(document, *value);
                ui.horizontal(|ui| {
                    widgets::column_caption(ui, &name);
                    let hover = format!("Stop configuring {name}");
                    if widgets::icon_button(ui, icons::REMOVE, &hover).clicked() {
                        outcome =
                            applying(draft, checked(document, Ok(document.unconfiguring(*value))));
                    }
                });
            }
            ui.label("");
            ui.end_row();
            for row in &document.configurations().rows {
                if let Some(chosen) = table_row(ui, model, draft, values, row) {
                    outcome = Some(chosen);
                }
                ui.end_row();
            }
        });
    outcome
}

fn table_row(
    ui: &mut Ui,
    model: &Model,
    draft: &mut ConfigurationsDraft,
    values: &[ConfiguredValue],
    row: &Configuration,
) -> Option<Outcome> {
    let document = model.document();
    let active = document.configurations().is_active(row.id);
    let mut outcome = None;
    let switch_hover = format!("Switch to {}", row.name);
    let switch = widgets::named(ui.radio(active, ""), &switch_hover).on_hover_text(if active {
        "This is the model as it stands"
    } else {
        switch_hover.as_str()
    });
    if switch.clicked() && !active {
        outcome = applying(draft, switching(document, row.id));
    }
    if draft
        .renaming
        .as_ref()
        .is_some_and(|renaming| renaming.id == row.id)
    {
        if let Some(renamed) = renaming_cell(ui, document, draft) {
            outcome = Some(renamed);
        }
    } else if active {
        ui.label(widgets::strong(&row.name));
    } else {
        ui.label(&row.name);
    }
    for value in values {
        if let Some(changed) = cell(ui, model, draft, row, *value) {
            outcome = Some(changed);
        }
    }
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, icons::RENAME, RENAME_HOVER).clicked() {
            draft.renaming = Some(Renaming {
                id: row.id,
                to: row.name.clone(),
                focus_pending: true,
            });
        }
        if widgets::icon_button(ui, icons::COPY, DUPLICATE_HOVER).clicked() {
            let built = document
                .duplicating_configuration(row.id)
                .map(|(transaction, _)| transaction)
                .map_err(|error| error.to_string());
            outcome = applying(draft, checked(document, built));
        }
        if widgets::icon_button(ui, icons::DELETE, DELETE_HOVER).clicked() {
            let built = document
                .deleting_configuration(row.id)
                .map_err(|error| error.to_string());
            outcome = applying(draft, checked(document, built));
        }
    });
    outcome
}

fn setting_change(
    document: &Document,
    row: &Configuration,
    value: ConfiguredValue,
    setting: Setting,
) -> Result<Transaction, String> {
    checked(
        document,
        document
            .setting_configuration(row.id, value, setting)
            .map_err(|error| error.to_string()),
    )
}

fn cell(
    ui: &mut Ui,
    model: &Model,
    draft: &mut ConfigurationsDraft,
    row: &Configuration,
    value: ConfiguredValue,
) -> Option<Outcome> {
    let document = model.document();
    let setting = document.configuration_setting(row.id, value);
    let label = format!("{} in {}", value_name(document, value), row.name);
    match (value, setting) {
        (ConfiguredValue::Parameter(id), setting) => {
            let stored = setting
                .as_ref()
                .and_then(Setting::expression)
                .map(|expression| document.expression_text(expression))
                .unwrap_or_default();
            let response = field::commit_field(
                ui,
                cell_id(row.id, value),
                &stored,
                CELL_WIDTH,
                false,
                |text| {
                    let current = match model.parameters().get(id) {
                        Some(Ok(value)) => Some(value.dimension),
                        _ => None,
                    };
                    let expression = field::parameter_expression(
                        document,
                        model.parameters(),
                        text,
                        current,
                        model.units(),
                    )?;
                    setting_change(document, row, value, Setting::Expression(expression))
                },
            );
            let response_label = widgets::named(response.response, &label);
            if let Some(error) = &response.error {
                response_label.on_hover_text(error);
            }
            response.committed.map(Outcome::Apply)
        }
        (ConfiguredValue::Suppressed(_), setting) => {
            let mut suppressed = matches!(setting, Some(Setting::Suppressed(true)));
            let changed = widgets::named(ui.checkbox(&mut suppressed, ""), &label)
                .on_hover_text("Suppressed in this configuration")
                .changed();
            if changed {
                return applying(
                    draft,
                    setting_change(document, row, value, Setting::Suppressed(suppressed)),
                );
            }
            None
        }
        (ConfiguredValue::Colour(_), setting) => {
            let current = match setting {
                Some(Setting::Colour(colour)) => colour,
                _ => None,
            };
            let mut chosen = None;
            ComboBox::from_id_salt(cell_id(row.id, value))
                .width(CELL_WIDTH)
                .selected_text(colour_name(current))
                .show_ui(ui, |ui| {
                    if widgets::menu_option(ui, current.is_none(), DEFAULT_COLOUR_NAME).clicked() {
                        chosen = Some(None);
                    }
                    for swatch in &SWATCHES {
                        let picked = current == Some(swatch.colour);
                        if widgets::menu_option(ui, picked, swatch.name).clicked() {
                            chosen = Some(Some(swatch.colour));
                        }
                    }
                });
            let colour = chosen.filter(|colour| *colour != current)?;
            applying(
                draft,
                setting_change(document, row, value, Setting::Colour(colour)),
            )
        }
    }
}

fn renaming_cell(
    ui: &mut Ui,
    document: &Document,
    draft: &mut ConfigurationsDraft,
) -> Option<Outcome> {
    let renaming = draft.renaming.as_mut()?;
    let focus = std::mem::take(&mut renaming.focus_pending);
    let mut done = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut renaming.to)
                    .id(rename_field_id())
                    .char_limit(MAX_CONFIGURATION_NAME_CHARS)
                    .desired_width(CELL_WIDTH),
            )
        });
        let field = widgets::named(field, NAME_CAPTION);
        if focus {
            field.request_focus();
        }
        let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let escape = field.has_focus() && ui.input(|input| input.key_pressed(Key::Escape));
        done = widgets::icon_button(ui, icons::DONE, "Rename the configuration").clicked() || enter;
        cancelled =
            widgets::icon_button(ui, icons::CLOSE, "Keep the name (Esc)").clicked() || escape;
    });
    if cancelled {
        draft.renaming = None;
        return None;
    }
    if !done {
        return None;
    }
    let renaming = draft.renaming.take()?;
    let built = document
        .renaming_configuration(renaming.id, &renaming.to)
        .map_err(|error| error.to_string());
    applying(draft, checked(document, built))
}

fn adding(ui: &mut Ui, document: &Document, draft: &mut ConfigurationsDraft) -> Option<Outcome> {
    let mut outcome = None;
    ui.horizontal_wrapped(|ui| {
        let add = widgets::small_button(ui, icons::command(Command::Configurations), ADD_LABEL);
        if ui.add(add).clicked() {
            let (transaction, _) = document.adding_configuration(None);
            outcome = applying(draft, checked(document, Ok(transaction)));
        }
        if document.configurations().rows.is_empty() {
            return;
        }
        let offered = offered_values(document);
        let mut chosen = None;
        ui.add_enabled_ui(!offered.is_empty(), |ui| {
            ComboBox::from_id_salt((DIALOG_ID, "configure"))
                .selected_text(CONFIGURE_LABEL)
                .show_ui(ui, |ui| {
                    for value in &offered {
                        if widgets::menu_option(ui, false, &value_name(document, *value)).clicked()
                        {
                            chosen = Some(*value);
                        }
                    }
                });
        });
        if let Some(value) = chosen {
            let built = document
                .configuring(value)
                .map_err(|error| error.to_string());
            outcome = applying(draft, checked(document, built));
        }
    });
    outcome
}
