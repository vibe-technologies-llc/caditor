use caditor_document::{Document, Edit, Parameter, Transaction};
use caditor_expression::{Expression, Unit};
use egui::{Button, Grid, RichText, Ui};

use crate::{
    field::{self, Expected},
    model::{Action, Model},
    panels::{Focus, PanelState},
};

const NAME_FIELD_WIDTH: f32 = 80.0;
const EXPRESSION_FIELD_WIDTH: f32 = 110.0;
const NEW_PARAMETER_NAME: &str = "parameter";
const NEW_PARAMETER_MILLIMETRES: f64 = 10.0;

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    ui.heading("Parameters");
    let document = model.document();
    if document.parameters().is_empty() {
        ui.weak("Parameters are named values that any dimension can use, such as width / 2.");
    }

    let mut errors = Vec::new();
    Grid::new("parameters")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            for parameter in document.parameters() {
                if let Some(error) = row(ui, model, state, actions, parameter) {
                    errors.push(format!("{}: {error}", parameter.name));
                }
                ui.end_row();
            }
        });
    for error in errors {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }

    if ui.button("Add parameter").clicked() {
        let name = unused_name(document);
        let mut transaction = document.transaction(format!("Add {name}"));
        let id = transaction.add_parameter(
            name,
            Expression::Measure(NEW_PARAMETER_MILLIMETRES, Unit::Millimetre),
        );
        actions.push(Action::Apply(transaction.finish()));
        state.request_focus(Focus::ParameterName(id));
    }
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
            let expression =
                field::parse_expression(document, model.parameters(), text, Expected::ANYTHING)?;
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

    match model.parameters().get(id) {
        Some(Ok(value)) => {
            ui.weak(value.to_string());
        }
        Some(Err(error)) => {
            ui.label(RichText::new("⚑ error").color(ui.visuals().error_fg_color))
                .on_hover_text(format!(
                    "{} cannot be evaluated: {error}. Edit its expression.",
                    parameter.name
                ));
        }
        None => {
            ui.label("");
        }
    }

    delete_button(ui, document, actions, parameter);
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
    let check = document.check(&delete);
    let response = ui.add_enabled(check.is_ok(), Button::new("🗑").small());
    let response = match check {
        Ok(()) => response.on_hover_text(format!("Delete {}", parameter.name)),
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
