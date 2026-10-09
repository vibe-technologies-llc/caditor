use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, MAX_PARAMETER_NOTE_CHARS, Parameter, ParameterOwner, ParameterUser, Transaction,
};
use caditor_expression::{Expression, ParameterId, check_name};
use egui::{Grid, Id, Label, Rect, TextEdit, Ui, Vec2, vec2};

use crate::{
    appearance::{self, CONTROL_HEIGHT, SPACE_M, SPACE_S, SPACE_XS},
    commands::{Command, CommandFrame},
    field, icons,
    model::{Action, Model, Notice},
    panels::{Focus, PanelState},
    tree_row,
    widgets::{self, DialogWidth},
};

pub const ADD_LABEL: &str = "Add parameter";
pub const EMPTY_PARAMETERS: &str = "Parameters are named values that any dimension can use, such \
                                    as width / 2. Type depth = 20 mm in a dimension or a \
                                    feature's field to name that value too.";
pub const MODEL_PARAMETERS: &str = "Model parameters";
const MODEL_PARAMETERS_EXPLANATION: &str = "Dimensions and feature values given a name by typing \
                                            name = value in their field. Other expressions use \
                                            them by that name.";
const OWNER_GONE: &str = "Its dimension or feature was deleted";
const COLUMNS: usize = 4;
const SPACING: Vec2 = vec2(SPACE_M, SPACE_S);
const VALUE_WIDTH: f32 = 64.0;
const FIELD_MARGIN: f32 = SPACE_M;
const SLACK: f32 = SPACE_XS;
const NAME_SHARE: f32 = 0.5;
const MIN_NAME_WIDTH: f32 = 48.0;
const MIN_EXPRESSION_WIDTH: f32 = 64.0;
const MAX_NAMED_USERS: usize = 4;
const MAX_LISTED_USERS: usize = 12;
pub const USED_BY: &str = "Used by";
const NEW_PARAMETER_NAME: &str = "parameter";
const NEW_PARAMETER_MILLIMETRES: f64 = 10.0;
const NO_PARAMETER_CHOSEN: &str =
    "Click or tab into a parameter's name or expression in the Parameters section first";
pub const MOVE_UP_LABEL: &str = "Move up";
pub const MOVE_DOWN_LABEL: &str = "Move down";
pub const SAVE_NOTE_LABEL: &str = "Save note";
const CANCEL_NOTE_LABEL: &str = "Cancel";
const NOTE_FIELD: &str = "parameter-note-field";
const NOTE_ROWS: usize = 4;
const FILTER_FROM_PARAMETERS: usize = 6;
const FILTER_FIELD: &str = "parameter-filter";
pub const FILTER_HINT: &str = "Filter parameters by name, expression or owner";
const NO_MATCH: &str = "No parameter matches the filter.";
pub const CLEAR_FILTER_LABEL: &str = "Clear the filter";
const NO_PARAMETERS: &str = "The model has no parameters";
const ALL_USED: &str = "Every parameter is in use: a dimension, a feature or another parameter \
                        refers to each one";
const DELETE_UNUSED_HOVER: &str = "Delete every parameter nothing refers to, in one change that \
                                   Undo takes back";
const MAX_NAMED_DELETED: usize = 4;
const NOTE_EXPLANATION: &str = "What the parameter is for, where its value comes from, or what \
                                to keep in mind when changing it. It shows when hovering the \
                                note icon beside its value. Leave it empty to remove the note.";

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    let document = model.document();
    if document.parameters().is_empty() {
        widgets::empty_state(ui, icons::PARAMETERS, EMPTY_PARAMETERS, |ui| {
            let button = widgets::small_button(ui, icons::ADD, ADD_LABEL);
            if ui.add(button).clicked() {
                add(model, state, actions);
            }
        });
        return;
    }
    let query = filter_field(ui, state, document.parameters().len());
    let widths = FieldWidths::fitting(ui.available_width(), CONTROL_HEIGHT);
    let (named, own): (Vec<&Parameter>, Vec<&Parameter>) = document
        .parameters()
        .iter()
        .filter(|parameter| kept_by_filter(ui, document, state, parameter, &query))
        .partition(|parameter| parameter.is_model_parameter());
    if own.is_empty() && named.is_empty() {
        widgets::empty_state(ui, icons::SEARCH, NO_MATCH, |ui| {
            let button = widgets::small_button(ui, icons::CLOSE, CLEAR_FILTER_LABEL);
            if ui.add(button).clicked() {
                state.parameter_filter.clear();
            }
        });
    }
    if !own.is_empty() {
        table(ui, "parameters", model, state, actions, &own, widths);
    }
    if !named.is_empty() {
        if !own.is_empty() {
            ui.add_space(SPACE_M);
        }
        ui.label(widgets::strong(MODEL_PARAMETERS))
            .on_hover_text(MODEL_PARAMETERS_EXPLANATION);
        table(
            ui,
            "model-parameters",
            model,
            state,
            actions,
            &named,
            widths,
        );
    }
    delete_unused_button(ui, model, state, actions);
}

fn filter_field(ui: &mut Ui, state: &mut PanelState, count: usize) -> String {
    let id = Id::new(FILTER_FIELD);
    let focused = ui.memory(|memory| memory.has_focus(id));
    if count < FILTER_FROM_PARAMETERS && state.parameter_filter.is_empty() && !focused {
        return String::new();
    }
    ui.horizontal(|ui| {
        let muted = appearance::tokens(ui).text_muted;
        widgets::icon_label(ui, icons::SEARCH, muted);
        widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut state.parameter_filter)
                    .id(id)
                    .hint_text(FILTER_HINT)
                    .desired_width(f32::INFINITY),
            )
        });
    });
    ui.add_space(SPACE_XS);
    state.parameter_filter.trim().to_lowercase()
}

fn kept_by_filter(
    ui: &Ui,
    document: &Document,
    state: &PanelState,
    parameter: &Parameter,
    query: &str,
) -> bool {
    let id = parameter.id();
    let fields = [Focus::ParameterName(id), Focus::ParameterValue(id)];
    let matches = |text: &str| text.to_lowercase().contains(query);
    query.is_empty()
        || matches(&parameter.name)
        || matches(&document.expression_text(&parameter.expression))
        || parameter
            .owner
            .as_ref()
            .and_then(|owner| document.owner_text(owner))
            .is_some_and(|owner| matches(&owner))
        || fields.into_iter().any(|focus| {
            state.wants_focus(focus) || ui.memory(|memory| memory.has_focus(focus.field_id()))
        })
}

struct UnusedDeletion {
    transaction: Transaction,
    names: Vec<String>,
}

impl UnusedDeletion {
    fn of(document: &Document, used: &BTreeSet<ParameterId>) -> Result<Self, &'static str> {
        if document.parameters().is_empty() {
            return Err(NO_PARAMETERS);
        }
        let unused: Vec<&Parameter> = document
            .parameters()
            .iter()
            .filter(|parameter| !used.contains(&parameter.id()))
            .collect();
        if unused.is_empty() {
            return Err(ALL_USED);
        }
        let names: Vec<String> = unused
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect();
        let edits = unused
            .iter()
            .map(|parameter| Edit::RemoveParameter { id: parameter.id() })
            .collect();
        Ok(Self {
            transaction: Transaction::new(delete_unused_label(names.len()), edits),
            names,
        })
    }

    fn perform(self, actions: &mut Vec<Action>) {
        let count = self.names.len();
        let named: Vec<&str> = self
            .names
            .iter()
            .take(MAX_NAMED_DELETED)
            .map(String::as_str)
            .collect();
        let more = count.saturating_sub(MAX_NAMED_DELETED);
        let listed = match (named.split_last(), more) {
            (Some((only, [])), 0) => (*only).to_owned(),
            (Some((last, first)), 0) => format!("{} and {last}", first.join(", ")),
            (_, more) => format!("{} and {more} more", named.join(", ")),
        };
        let what = if count == 1 {
            "the unused parameter"
        } else {
            "the unused parameters"
        };
        actions.push(Action::Apply(self.transaction));
        actions.push(Action::Inform(Notice::info(format!(
            "Deleted {what} {listed}. Undo brings them back."
        ))));
    }
}

pub fn delete_unused_label(count: usize) -> String {
    if count == 1 {
        "Delete 1 unused parameter".to_owned()
    } else {
        format!("Delete {count} unused parameters")
    }
}

fn delete_unused_button(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let Ok(deletion) = UnusedDeletion::of(document, state.parameter_uses.of(model)) else {
        return;
    };
    ui.add_space(SPACE_S);
    let label = delete_unused_label(deletion.names.len());
    let button = widgets::small_button(ui, icons::command(Command::DeleteUnusedParameters), &label);
    if ui.add(button).on_hover_text(DELETE_UNUSED_HOVER).clicked() {
        deletion.perform(actions);
    }
}

fn table(
    ui: &mut Ui,
    id: &str,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    parameters: &[&Parameter],
    widths: FieldWidths,
) {
    Grid::new(id)
        .num_columns(COLUMNS)
        .striped(true)
        .spacing(SPACING)
        .show(ui, |ui| {
            for caption in ["Name", "Expression", "Value"] {
                widgets::column_caption(ui, caption);
            }
            ui.end_row();
            for parameter in parameters {
                let error = row(ui, model, state, actions, parameter, widths);
                ui.end_row();
                if let Some(owner) = &parameter.owner {
                    owner_row(ui, model.document(), state, parameter, owner, widths);
                }
                if let Some(error) = error {
                    widgets::error_row(ui, &error);
                }
            }
        });
}

fn owner_row(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    parameter: &Parameter,
    owner: &ParameterOwner,
    widths: FieldWidths,
) {
    ui.scope(|ui| {
        ui.set_max_width(widths.name);
        match document.owner_text(owner) {
            Some(text) => {
                let hover = format!(
                    "{} is the value of {text}. Edit it here, or in its field as {} = value. \
                     Click to go to it.",
                    parameter.name, parameter.name
                );
                if widgets::link(ui, &text, None)
                    .on_hover_text(hover)
                    .clicked()
                {
                    go_to_owner(state, owner);
                }
            }
            None => {
                let hover = format!(
                    "{}. {} is kept as a value of its own.",
                    OWNER_GONE, parameter.name
                );
                ui.add(Label::new(widgets::muted(OWNER_GONE, ui)).truncate())
                    .on_hover_text(hover);
            }
        }
    });
    ui.end_row();
}

fn go_to_owner(state: &mut PanelState, owner: &ParameterOwner) {
    let feature = owner.feature();
    state.choose_only(feature);
    state.reveal(feature);
    state.request_focus(match *owner {
        ParameterOwner::Feature { feature, .. } => Focus::Feature(feature),
        ParameterOwner::Dimension { sketch, constraint } => Focus::Dimension {
            feature: sketch,
            constraint,
        },
    });
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FieldWidths {
    name: f32,
    expression: f32,
    value: f32,
}

impl FieldWidths {
    fn fitting(available: f32, delete: f32) -> Self {
        let gaps = (COLUMNS - 1) as f32 * SPACING.x;
        let fields = available - VALUE_WIDTH - delete - gaps - 2.0 * FIELD_MARGIN - SLACK;
        Self {
            name: (fields * NAME_SHARE).max(MIN_NAME_WIDTH),
            expression: (fields * (1.0 - NAME_SHARE)).max(MIN_EXPRESSION_WIDTH),
            value: VALUE_WIDTH,
        }
    }
}

pub fn add(model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    let expression = model
        .length_unit()
        .default_length(NEW_PARAMETER_MILLIMETRES);
    add_with(model, state, actions, NEW_PARAMETER_NAME, expression);
}

pub fn add_with(
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    stem: &str,
    expression: Expression,
) -> String {
    let document = model.document();
    let name = unused_name(document, stem);
    let mut transaction = document.transaction(format!("Add {name}"));
    let id = transaction.add_parameter(name.clone(), expression);
    actions.push(Action::Apply(transaction.finish()));
    state.request_focus(Focus::ParameterName(id));
    name
}

fn row(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    parameter: &Parameter,
    widths: FieldWidths,
) -> Option<String> {
    let document = model.document();
    let id = parameter.id();

    let name_focus = Focus::ParameterName(id);
    let name = field::commit_field(
        ui,
        name_focus.field_id(),
        &parameter.name,
        widths.name,
        state.wants_focus(name_focus),
        |text| {
            field::checked(
                document,
                Transaction::single(
                    format!("Rename {}", parameter.name),
                    Edit::RenameParameter {
                        id,
                        name: text.to_owned(),
                    },
                ),
            )
        },
    );
    let value_focus = Focus::ParameterValue(id);
    let expression = field::commit_field(
        ui,
        value_focus.field_id(),
        &document.expression_text(&parameter.expression),
        widths.expression,
        state.wants_focus(value_focus),
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
            field::checked(
                document,
                Transaction::single(
                    format!("Edit {}", parameter.name),
                    Edit::SetParameterExpression { id, expression },
                ),
            )
        },
    );
    state.focus_reached(name_focus, name.response.has_focus());
    state.focus_reached(value_focus, expression.response.has_focus());
    if name.response.has_focus() || expression.response.has_focus() {
        state.parameter = Some(id);
    }
    for transaction in [name.committed, expression.committed].into_iter().flatten() {
        actions.push(Action::Apply(transaction));
    }

    let used = state.parameter_uses.of(model).contains(&id);
    for field in [&name.response, &expression.response] {
        field.context_menu(|ui| {
            widgets::fitted_menu(ui, |ui| {
                row_menu(ui, document, state, actions, parameter, used);
            });
        });
    }
    ui.scope(|ui| {
        ui.set_width(widths.value);
        ui.horizontal(|ui| match model.parameters().get(id) {
            Some(Ok(value)) => {
                if !used {
                    let color = appearance::tokens(ui).text_muted;
                    widgets::described_icon(ui, icons::UNUSED, color, &unused(&parameter.name));
                }
                note_icon(ui, parameter);
                let shown = model.units().show(*value);
                ui.add(Label::new(widgets::muted(&shown, ui)).truncate())
                    .on_hover_ui(|ui| {
                        ui.label(&shown);
                        ui.label(used_by(&document.parameter_users(id)));
                    });
            }
            Some(Err(error)) => {
                note_icon(ui, parameter);
                let color = ui.visuals().error_fg_color;
                widgets::described_icon(
                    ui,
                    icons::FAILED,
                    color,
                    &format!(
                        "{} cannot be evaluated: {error}. Edit its expression.",
                        parameter.name
                    ),
                );
            }
            None => {}
        });
    });
    let band = Rect::from_x_y_ranges(
        ui.clip_rect().x_range(),
        name.response.rect.expand(SPACING.y / 2.0).y_range(),
    );
    let hovered = ui.rect_contains_pointer(band);
    delete_button(ui, document, actions, parameter, hovered, used);
    name.error.or(expression.error)
}

fn note_icon(ui: &mut Ui, parameter: &Parameter) {
    if parameter.note.is_empty() {
        return;
    }
    let color = appearance::tokens(ui).text_muted;
    widgets::described_icon(
        ui,
        icons::NOTE,
        color,
        &format!("Note on {}: {}", parameter.name, parameter.note),
    );
}

fn row_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    parameter: &Parameter,
    used: bool,
) {
    for direction in Direction::BOTH {
        let moving = direction.transaction(document, parameter);
        if menu_entry(ui, direction.glyph(), direction.label(), &moving)
            && let Ok(transaction) = moving
        {
            actions.push(Action::Apply(transaction));
        }
    }
    let note_label = if parameter.note.is_empty() {
        "Add a note…"
    } else {
        "Edit the note…"
    };
    if widgets::menu_item(ui, icons::EDIT_NOTE, note_label, None).clicked() {
        state.noting = Some(NoteDraft::of(parameter));
        ui.close();
    }
    if used {
        users_menu(ui, document, state, parameter);
    }
    ui.separator();
    let deletion = Deletion::of(document, parameter, used);
    let response = widgets::menu_item(ui, icons::DELETE, &deletion.label(), None)
        .on_hover_text(deletion.hint(document));
    if response.clicked() {
        deletion.perform(document, actions);
        ui.close();
    }
}

fn users_menu(ui: &mut Ui, document: &Document, state: &mut PanelState, parameter: &Parameter) {
    let users = document.parameter_user_ids(parameter.id());
    ui.separator();
    ui.label(widgets::muted(USED_BY, ui));
    for user in users.iter().take(MAX_LISTED_USERS) {
        let (glyph, name, focus) = match *user {
            ParameterUser::Parameter(id) => (
                icons::PARAMETERS,
                document.parameter_name(id).unwrap_or_default().to_owned(),
                Focus::ParameterValue(id),
            ),
            ParameterUser::Feature(id) => match document.feature(id) {
                Some(feature) => (
                    icons::feature(&feature.kind),
                    feature.name.clone(),
                    Focus::Feature(id),
                ),
                None => continue,
            },
        };
        let response = widgets::menu_item(ui, glyph, &name, None)
            .on_hover_text(format!("Go to {name}, which uses {}", parameter.name));
        if response.clicked() {
            if let Focus::Feature(id) = focus {
                state.choose_only(id);
                state.reveal(id);
            }
            state.request_focus(focus);
            ui.close();
        }
    }
    let more = users.len().saturating_sub(MAX_LISTED_USERS);
    if more > 0 {
        ui.label(widgets::muted(format!("and {more} more"), ui));
    }
}

fn menu_entry<T>(ui: &mut Ui, glyph: &str, label: &str, outcome: &Result<T, String>) -> bool {
    let response = ui
        .add_enabled_ui(outcome.is_ok(), |ui| {
            widgets::menu_item(ui, glyph, label, None)
        })
        .inner;
    let response = match outcome {
        Err(reason) => response.on_disabled_hover_text(reason),
        Ok(_) => response,
    };
    let clicked = response.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

impl Direction {
    const BOTH: [Self; 2] = [Self::Up, Self::Down];

    fn label(self) -> &'static str {
        match self {
            Self::Up => MOVE_UP_LABEL,
            Self::Down => MOVE_DOWN_LABEL,
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Up => icons::MOVE_UP,
            Self::Down => icons::MOVE_DOWN,
        }
    }

    fn command(self) -> Command {
        match self {
            Self::Up => Command::MoveParameterUp,
            Self::Down => Command::MoveParameterDown,
        }
    }

    fn transaction(
        self,
        document: &Document,
        parameter: &Parameter,
    ) -> Result<Transaction, String> {
        let id = parameter.id();
        let group: Vec<usize> = document
            .parameters()
            .iter()
            .enumerate()
            .filter(|(_, other)| other.is_model_parameter() == parameter.is_model_parameter())
            .map(|(index, _)| index)
            .collect();
        let place = group
            .iter()
            .position(|index| document.parameters().get(*index).map(Parameter::id) == Some(id))
            .ok_or_else(|| NO_PARAMETER_CHOSEN.to_owned())?;
        let kind = if parameter.is_model_parameter() {
            "model parameter"
        } else {
            "parameter"
        };
        let target = match self {
            Self::Up => place.checked_sub(1).and_then(|above| group.get(above)),
            Self::Down => group.get(place + 1),
        };
        let target = *target.ok_or_else(|| match self {
            Self::Up => format!("{} is already the first {kind}", parameter.name),
            Self::Down => format!("{} is already the last {kind}", parameter.name),
        })?;
        let way = match self {
            Self::Up => "up",
            Self::Down => "down",
        };
        Ok(Transaction::single(
            format!("Move {} {way}", parameter.name),
            Edit::MoveParameter { id, index: target },
        ))
    }
}

enum Deletion {
    Unused {
        name: String,
        transaction: Transaction,
    },
    Inlined {
        id: ParameterId,
        name: String,
        expression: String,
        transaction: Result<Transaction, String>,
    },
}

impl Deletion {
    fn of(document: &Document, parameter: &Parameter, used: bool) -> Self {
        let name = parameter.name.clone();
        if !used {
            return Self::Unused {
                transaction: delete_transaction(parameter),
                name,
            };
        }
        Self::Inlined {
            id: parameter.id(),
            expression: document.expression_text(&parameter.expression),
            transaction: document
                .inline_parameter(parameter.id())
                .map_err(|error| error.to_string()),
            name,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Unused { name, .. } => format!("Delete {name}"),
            Self::Inlined { .. } => "Delete, keeping its value in its uses".to_owned(),
        }
    }

    fn hint(&self, document: &Document) -> String {
        match self {
            Self::Unused { name, .. } => format!("Delete {name}; nothing refers to it"),
            Self::Inlined {
                id,
                name,
                expression,
                transaction,
            } => match transaction {
                Ok(_) => format!(
                    "Delete {name} and write {expression} in its place wherever it is used, so \
                     the model stays as it is. {}",
                    used_by(&document.parameter_users(*id))
                ),
                Err(reason) => reason.clone(),
            },
        }
    }

    fn transaction(&self) -> Result<&Transaction, &str> {
        match self {
            Self::Unused { transaction, .. }
            | Self::Inlined {
                transaction: Ok(transaction),
                ..
            } => Ok(transaction),
            Self::Inlined {
                transaction: Err(reason),
                ..
            } => Err(reason),
        }
    }

    fn perform(&self, document: &Document, actions: &mut Vec<Action>) {
        match self.transaction() {
            Ok(transaction) => actions.push(Action::Apply(transaction.clone())),
            Err(reason) => {
                actions.push(Action::Inform(Notice::info(reason)));
                return;
            }
        }
        if let Self::Inlined {
            id,
            name,
            expression,
            ..
        } = self
        {
            let count = document.parameter_users(*id).len();
            let uses = if count == 1 { "use" } else { "uses" };
            actions.push(Action::Inform(Notice::info(format!(
                "Deleted {name} and wrote {expression} into its {count} {uses}. Undo brings it back."
            ))));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    parameter: ParameterId,
    text: String,
    focus_pending: bool,
}

impl NoteDraft {
    fn of(parameter: &Parameter) -> Self {
        Self {
            parameter: parameter.id(),
            text: parameter.note.clone(),
            focus_pending: true,
        }
    }
}

pub fn note_dialog(
    ctx: &egui::Context,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let Some(draft) = &mut state.noting else {
        return;
    };
    let Some(parameter) = document.parameter(draft.parameter) else {
        state.noting = None;
        return;
    };
    let title = format!("Note on {}", parameter.name);
    let response = widgets::dialog(ctx, "parameter-note", &title, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(NOTE_EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut draft.text)
                    .id(Id::new(NOTE_FIELD))
                    .char_limit(MAX_PARAMETER_NOTE_CHARS)
                    .desired_rows(NOTE_ROWS)
                    .desired_width(f32::INFINITY),
            )
        });
        widgets::named(field.clone(), &title);
        if std::mem::take(&mut draft.focus_pending) {
            field.request_focus();
        }
        widgets::footer(ui, |ui| {
            if ui
                .add(widgets::primary_button(ui, SAVE_NOTE_LABEL))
                .clicked()
            {
                return Some(true);
            }
            ui.add(widgets::button(CANCEL_NOTE_LABEL))
                .clicked()
                .then_some(false)
        })
    });
    let closed = response.should_close().then_some(false);
    let Some(save) = response.inner.or(closed) else {
        return;
    };
    let note = draft.text.trim().to_owned();
    let changed = note != parameter.note;
    let label = if note.is_empty() {
        format!("Remove the note on {}", parameter.name)
    } else {
        format!("Note on {}", parameter.name)
    };
    let edit = Edit::SetParameterNote {
        id: parameter.id(),
        note,
    };
    state.noting = None;
    if save && changed {
        actions.push(Action::Apply(Transaction::single(label, edit)));
    }
}

fn unused(name: &str) -> String {
    format!("{name} is unused: nothing refers to it yet.")
}

pub fn used_by(users: &[String]) -> String {
    match users {
        [] => "Nothing uses it yet.".to_owned(),
        [only] => format!("Used by {only}."),
        users => {
            let named: Vec<&str> = users
                .iter()
                .take(MAX_NAMED_USERS)
                .map(String::as_str)
                .collect();
            let more = users.len().saturating_sub(MAX_NAMED_USERS);
            if more == 0 {
                match named.split_last() {
                    Some((last, first)) => format!("Used by {} and {last}.", first.join(", ")),
                    None => String::new(),
                }
            } else {
                format!("Used by {} and {more} more.", named.join(", "))
            }
        }
    }
}

pub fn commands(
    model: &Model,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    if commands.available(Command::AddParameter) {
        add(model, state, actions);
    }
    let document = model.document();
    let chosen = state.parameter.and_then(|id| document.parameter(id));
    let detail = chosen.map(|parameter| parameter.name.clone());
    for direction in Direction::BOTH {
        let moving = chosen.map_or_else(
            || Err(NO_PARAMETER_CHOSEN.to_owned()),
            |parameter| direction.transaction(document, parameter),
        );
        if commands.invoke_detailed(direction.command(), detail.clone(), &moving)
            && let Ok(moving) = moving
        {
            actions.push(Action::Apply(moving));
        }
    }
    let noting = chosen.ok_or_else(|| NO_PARAMETER_CHOSEN.to_owned());
    if commands.invoke_detailed(Command::ParameterNote, detail.clone(), &noting)
        && let Ok(parameter) = noting
    {
        state.noting = Some(NoteDraft::of(parameter));
    }
    let delete = chosen.ok_or_else(|| NO_PARAMETER_CHOSEN.to_owned());
    if commands.invoke_detailed(Command::DeleteParameter, detail, &delete)
        && let Ok(parameter) = delete
    {
        let used = state.parameter_uses.of(model).contains(&parameter.id());
        state.parameter = None;
        Deletion::of(document, parameter, used).perform(document, actions);
    }
    let unused = UnusedDeletion::of(document, state.parameter_uses.of(model));
    let detail = unused
        .as_ref()
        .ok()
        .map(|deletion| delete_unused_label(deletion.names.len()));
    if commands.invoke_detailed(Command::DeleteUnusedParameters, detail, &unused)
        && let Ok(deletion) = unused
    {
        deletion.perform(actions);
    }
}

fn delete_transaction(parameter: &Parameter) -> Transaction {
    Transaction::single(
        format!("Delete {}", parameter.name),
        Edit::RemoveParameter { id: parameter.id() },
    )
}

#[derive(Debug, Clone, Default)]
pub struct ParameterUses {
    revision: Option<u64>,
    used: BTreeSet<ParameterId>,
}

impl ParameterUses {
    fn of(&mut self, model: &Model) -> &BTreeSet<ParameterId> {
        let revision = model.revision();
        if self.revision != Some(revision) {
            self.used = model.document().used_parameters();
            self.revision = Some(revision);
        }
        &self.used
    }
}

fn delete_button(
    ui: &mut Ui,
    document: &Document,
    actions: &mut Vec<Action>,
    parameter: &Parameter,
    row_hovered: bool,
    used: bool,
) {
    let hover = format!("Delete {}", parameter.name);
    let focus_key = Id::new(("parameter-delete-focused", parameter.id()));
    let focused = ui.data(|data| data.get_temp::<bool>(focus_key).unwrap_or(false));
    let response = tree_row::slot(ui, |ui| {
        if !row_hovered && !focused {
            ui.set_opacity(0.0);
        }
        widgets::icon_button(ui, icons::DELETE, &hover)
    });
    if response.has_focus() != focused {
        ui.data_mut(|data| data.insert_temp(focus_key, response.has_focus()));
        ui.ctx().request_repaint();
    }
    let response = if used {
        response.on_hover_ui(|ui| {
            ui.label(Deletion::of(document, parameter, used).hint(document));
        })
    } else {
        response
    };
    if response.clicked() {
        Deletion::of(document, parameter, used).perform(document, actions);
    }
}

fn unused_name(document: &Document, stem: &str) -> String {
    let stem = if check_name(&format!("{stem}1")).is_ok() {
        stem
    } else {
        NEW_PARAMETER_NAME
    };
    (1..=document.parameters().len() + 1)
        .map(|number| format!("{stem}{number}"))
        .find(|name| document.parameter_named(name).is_none())
        .unwrap_or_else(|| stem.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_parameter_says_what_uses_it_or_that_nothing_does() {
        assert_eq!(used_by(&[]), "Nothing uses it yet.");
        assert_eq!(used_by(&names(&["Base sketch"])), "Used by Base sketch.");
        assert_eq!(
            used_by(&names(&["Base sketch", "Extrude 1"])),
            "Used by Base sketch and Extrude 1."
        );
        assert_eq!(
            used_by(&names(&["a", "b", "c", "d"])),
            "Used by a, b, c and d."
        );
        assert_eq!(
            used_by(&names(&["a", "b", "c", "d", "e", "f"])),
            "Used by a, b, c, d and 2 more."
        );
    }
}
