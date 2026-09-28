use caditor_document::{Document, Edit, Parameter, Transaction};
use egui::{Grid, Ui};

use crate::{
    field::{self, Expected},
    icons,
    model::{Action, Model},
    panels::{Focus, PanelState},
    widgets,
};

pub const ADD_LABEL: &str = "Add parameter";
const COLUMNS: usize = 3;
const SPACING: [f32; 2] = [6.0, 4.0];
const NAME_FIELD_WIDTH: f32 = 84.0;
const EXPRESSION_FIELD_WIDTH: f32 = 116.0;
const NEW_PARAMETER_NAME: &str = "parameter";
const NEW_PARAMETER_MILLIMETRES: f64 = 10.0;

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
                let error = row(ui, model, state, actions, parameter);
                ui.end_row();
                if let Some(error) = error {
                    ui.label("");
                    let color = ui.visuals().error_fg_color;
                    ui.horizontal_wrapped(|ui| {
                        widgets::icon_label(ui, icons::FAILED, color);
                        ui.colored_label(color, error);
                    });
                    ui.end_row();
                }
            }
        });
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
) -> Option<String> {
    let document = model.document();
    let id = parameter.id();

    let name_focus = Focus::ParameterName(id);
    let name = field::commit_field(
        ui,
        name_focus.field_id(),
        &parameter.name,
        NAME_FIELD_WIDTH,
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
        EXPRESSION_FIELD_WIDTH,
        state.wants_focus(value_focus),
        |text| {
            let expression = field::parse_expression(
                document,
                model.parameters(),
                text,
                Expected::ANYTHING,
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
    for transaction in [name.committed, expression.committed].into_iter().flatten() {
        actions.push(Action::Apply(transaction));
    }

    ui.horizontal(|ui| {
        match model.parameters().get(id) {
            Some(Ok(value)) => {
                ui.label(widgets::muted(model.length_unit().show(*value), ui));
            }
            Some(Err(error)) => {
                let color = ui.visuals().error_fg_color;
                widgets::icon_label(ui, icons::FAILED, color).on_hover_text(format!(
                    "{} cannot be evaluated: {error}. Edit its expression.",
                    parameter.name
                ));
                ui.colored_label(color, "Error");
            }
            None => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            delete_button(ui, document, actions, parameter);
        });
    });
    name.error.or(expression.error)
}

fn delete_button(
    ui: &mut Ui,
    document: &Document,
    actions: &mut Vec<Action>,
    parameter: &Parameter,
) {
    let delete = Transaction::single(
        format!("Delete {}", parameter.name),
        Edit::RemoveParameter { id: parameter.id() },
    );
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
