use caditor_document::{Document, Edit, Parameter, Transaction};
use egui::{Grid, Id, Label, Rect, Ui, Vec2, vec2};

use crate::{
    appearance::{CONTROL_HEIGHT, SPACE_M, SPACE_S, SPACE_XS},
    commands::{Command, CommandFrame},
    field, icons,
    model::{Action, Model},
    panels::{Focus, PanelState},
    tree_row, widgets,
};

pub const ADD_LABEL: &str = "Add parameter";
pub const EMPTY_PARAMETERS: &str =
    "Parameters are named values that any dimension can use, such as width / 2.";
const COLUMNS: usize = 4;
const SPACING: Vec2 = vec2(SPACE_M, SPACE_S);
const VALUE_WIDTH: f32 = 88.0;
const FIELD_MARGIN: f32 = SPACE_M;
const SLACK: f32 = SPACE_XS;
const NAME_SHARE: f32 = 0.45;
const MIN_NAME_WIDTH: f32 = 48.0;
const MIN_EXPRESSION_WIDTH: f32 = 64.0;
const MAX_NAMED_USERS: usize = 4;
const NEW_PARAMETER_NAME: &str = "parameter";
const NEW_PARAMETER_MILLIMETRES: f64 = 10.0;
const NO_PARAMETER_CHOSEN: &str =
    "Click or tab into a parameter's name or expression in the Parameters section first";

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

    ui.scope(|ui| {
        ui.set_width(widths.value);
        ui.horizontal(|ui| match model.parameters().get(id) {
            Some(Ok(value)) => {
                let shown = model.units().show(*value);
                ui.add(Label::new(widgets::muted(&shown, ui)).truncate())
                    .on_hover_ui(|ui| {
                        ui.label(&shown);
                        ui.label(used_by(&document.parameter_users(id)));
                    });
            }
            Some(Err(error)) => {
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
    delete_button(ui, document, actions, parameter, hovered);
    name.error.or(expression.error)
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
    let delete = chosen.map_or_else(
        || Err(NO_PARAMETER_CHOSEN.to_owned()),
        |parameter| {
            document
                .can_remove_parameter(parameter.id())
                .map(|()| delete_transaction(parameter))
                .map_err(|error| error.to_string())
        },
    );
    let detail = chosen.map(|parameter| parameter.name.clone());
    if commands.invoke_detailed(Command::DeleteParameter, detail, &delete)
        && let Ok(delete) = delete
    {
        state.parameter = None;
        actions.push(Action::Apply(delete));
    }
}

fn delete_transaction(parameter: &Parameter) -> Transaction {
    Transaction::single(
        format!("Delete {}", parameter.name),
        Edit::RemoveParameter { id: parameter.id() },
    )
}

fn delete_button(
    ui: &mut Ui,
    document: &Document,
    actions: &mut Vec<Action>,
    parameter: &Parameter,
    row_hovered: bool,
) {
    let delete = delete_transaction(parameter);
    let check = document.can_remove_parameter(parameter.id());
    let hover = format!("Delete {}", parameter.name);
    let focus_key = Id::new(("parameter-delete-focused", parameter.id()));
    let focused = ui.data(|data| data.get_temp::<bool>(focus_key).unwrap_or(false));
    let response = tree_row::slot(ui, |ui| {
        if !row_hovered && !focused {
            ui.set_opacity(0.0);
        }
        ui.add_enabled_ui(check.is_ok(), |ui| {
            widgets::icon_button(ui, icons::DELETE, &hover)
        })
        .inner
    });
    if response.has_focus() != focused {
        ui.data_mut(|data| data.insert_temp(focus_key, response.has_focus()));
        ui.ctx().request_repaint();
    }
    let response = match check {
        Ok(()) => response,
        Err(reason) => response.on_disabled_hover_text(reason.to_string()),
    };
    if response.clicked() {
        actions.push(Action::Apply(delete));
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
