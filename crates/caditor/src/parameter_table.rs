use caditor_document::{Document, Edit, Parameter, Transaction};
use egui::{Grid, Label, RichText, Ui};

use crate::{
    commands::{Command, CommandFrame},
    field, icons,
    model::{Action, Model},
    panels::{Focus, PanelState},
    widgets,
};

pub const ADD_LABEL: &str = "Add parameter";
const COLUMNS: usize = 4;
const SPACING: [f32; 2] = [6.0, 4.0];
const VALUE_WIDTH: f32 = 72.0;
const FIELD_MARGIN: f32 = 8.0;
const SLACK: f32 = 2.0;
const NAME_SHARE: f32 = 0.42;
const MIN_NAME_WIDTH: f32 = 48.0;
const MIN_EXPRESSION_WIDTH: f32 = 64.0;
const NEW_PARAMETER_NAME: &str = "parameter";
const NEW_PARAMETER_MILLIMETRES: f64 = 10.0;
const NO_PARAMETER_CHOSEN: &str =
    "Click or tab into a parameter's name or expression in the Parameters section first";

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    let document = model.document();
    if document.parameters().is_empty() {
        ui.label(widgets::muted(
            "Parameters are named values that any dimension can use, such as width / 2.",
            ui,
        ));
        let button = widgets::small_button(ui, icons::ADD, ADD_LABEL);
        if ui.add(button).clicked() {
            add(model, state, actions);
        }
        return;
    }
    let widths = FieldWidths::fitting(ui.available_width(), ui.spacing().interact_size.y);
    Grid::new("parameters")
        .num_columns(COLUMNS)
        .striped(true)
        .spacing(SPACING)
        .show(ui, |ui| {
            for caption in ["Name", "Expression", "Value"] {
                widgets::column_caption(ui, caption);
            }
            ui.label("");
            ui.end_row();
            for parameter in document.parameters() {
                let error = row(ui, model, state, actions, parameter, widths);
                ui.end_row();
                if let Some(error) = error {
                    ui.label("");
                    let color = ui.visuals().error_fg_color;
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(widths.expression);
                        widgets::icon_label(ui, icons::FAILED, color);
                        ui.colored_label(color, error);
                    });
                    ui.end_row();
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
        let gaps = (COLUMNS - 1) as f32 * SPACING[0];
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
                model.length_unit(),
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
                let shown = model.length_unit().show(*value);
                ui.add(Label::new(widgets::muted(&shown, ui)).truncate())
                    .on_hover_text(shown);
            }
            Some(Err(error)) => {
                let color = ui.visuals().error_fg_color;
                widgets::icon_label(ui, icons::FAILED, color).on_hover_text(format!(
                    "{} cannot be evaluated: {error}. Edit its expression.",
                    parameter.name
                ));
                ui.add(Label::new(RichText::new("Error").color(color)).truncate());
            }
            None => {}
        });
    });
    delete_button(ui, document, actions, parameter);
    name.error.or(expression.error)
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
) {
    let delete = delete_transaction(parameter);
    let check = document.can_remove_parameter(parameter.id());
    let hover = format!("Delete {}", parameter.name);
    let response = ui
        .add_enabled_ui(check.is_ok(), |ui| {
            widgets::icon_button(ui, icons::DELETE, &hover)
        })
        .inner;
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
        .map(|number| match number {
            1 => NEW_PARAMETER_NAME.to_owned(),
            _ => format!("{NEW_PARAMETER_NAME}{number}"),
        })
        .find(|name| document.parameter_named(name).is_none())
        .unwrap_or_else(|| NEW_PARAMETER_NAME.to_owned())
}
