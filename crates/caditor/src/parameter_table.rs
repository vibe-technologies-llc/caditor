use std::collections::BTreeSet;

use caditor_document::{Document, Edit, MAX_PARAMETER_NOTE_CHARS, Parameter, Transaction};
use caditor_expression::ParameterId;
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
pub const EMPTY_PARAMETERS: &str =
    "Parameters are named values that any dimension can use, such as width / 2.";
const COLUMNS: usize = 4;
const SPACING: Vec2 = vec2(SPACE_M, SPACE_S);
const VALUE_WIDTH: f32 = 64.0;
const FIELD_MARGIN: f32 = SPACE_M;
const SLACK: f32 = SPACE_XS;
const NAME_SHARE: f32 = 0.5;
const MIN_NAME_WIDTH: f32 = 48.0;
const MIN_EXPRESSION_WIDTH: f32 = 64.0;
const MAX_NAMED_USERS: usize = 4;
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
    let widths = FieldWidths::fitting(ui.available_width(), CONTROL_HEIGHT);
    Grid::new("parameters")
        .num_columns(COLUMNS)
        .striped(true)
        .spacing(SPACING)
        .show(ui, |ui| {
            for caption in ["Name", "Expression", "Value"] {
                widgets::column_caption(ui, caption);
            }
            ui.end_row();
            for parameter in document.parameters() {
                let error = row(ui, model, state, actions, parameter, widths);
                ui.end_row();
                if let Some(error) = error {
                    widgets::error_row(ui, &error);
                }
            }
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
    let document = model.document();
    let name = unused_name(document);
    let mut transaction = document.transaction(format!("Add {name}"));
    let id = transaction.add_parameter(
        name,
        model
            .length_unit()
            .default_length(NEW_PARAMETER_MILLIMETRES),
    );
    actions.push(Action::Apply(transaction.finish()));
    state.request_focus(Focus::ParameterName(id));
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
        field.context_menu(|ui| row_menu(ui, document, state, actions, parameter, used));
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
    ui.separator();
    let deletion = Deletion::of(document, parameter, used);
    let response = widgets::menu_item(ui, icons::DELETE, &deletion.label(), None)
        .on_hover_text(deletion.hint(document));
    if response.clicked() {
        deletion.perform(document, actions);
        ui.close();
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
        let index = document
            .parameters()
            .iter()
            .position(|other| other.id() == id)
            .ok_or_else(|| NO_PARAMETER_CHOSEN.to_owned())?;
        let target = match self {
            Self::Up => index
                .checked_sub(1)
                .ok_or_else(|| format!("{} is already the first parameter", parameter.name))?,
            Self::Down => Some(index + 1)
                .filter(|below| *below < document.parameters().len())
                .ok_or_else(|| format!("{} is already the last parameter", parameter.name))?,
        };
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

fn unused_name(document: &Document) -> String {
    (1..)
        .map(|number| format!("{NEW_PARAMETER_NAME}{number}"))
        .find(|name| document.parameter_named(name).is_none())
        .unwrap_or_else(|| NEW_PARAMETER_NAME.to_owned())
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
