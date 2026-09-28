use caditor_document::{Feature, FeatureId, Shell, Transaction};
use caditor_expression::Dimension;
use egui::{Grid, Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_tree::count,
    field::{self, Expected},
    model::{Action, Model},
    shell_tools,
};

const FIELD_WIDTH: f32 = 110.0;

fn change(model: &Model, feature: FeatureId, shell: Shell) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = shell_tools::edit(document, feature, shell)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn thickness_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    shell: &Shell,
    actions: &mut Vec<Action>,
) {
    ui.label("Thickness");
    let document = model.document();
    let parameters = model.parameters();
    let mut error = None;
    ui.horizontal(|ui| {
        let field = field::commit_field(
            ui,
            Id::new(("shell-thickness", feature)),
            &document.expression_text(&shell.thickness),
            FIELD_WIDTH,
            false,
            |text| {
                let parsed = field::parse_expression(
                    document,
                    parameters,
                    text,
                    Expected {
                        dimension: Some(Dimension::LENGTH),
                        non_negative: false,
                    },
                    model.length_unit(),
                )?;
                let value = parameters
                    .evaluate_expression(&parsed)
                    .map_err(|error| field::sentence(&error.to_string()))?
                    .value;
                if value <= 0.0 {
                    return Err("Enter a thickness above zero".to_owned());
                }
                change(
                    model,
                    feature,
                    Shell {
                        thickness: parsed,
                        ..shell.clone()
                    },
                )
            },
        );
        if let Some(transaction) = field.committed {
            actions.push(Action::Apply(transaction));
        }
        if field.error.is_none()
            && let Some(preview) =
                field::value_preview(parameters, &shell.thickness, model.length_unit())
        {
            ui.weak(preview);
        }
        error = field.error;
    });
    ui.end_row();
    if let Some(error) = error {
        ui.label("");
        ui.colored_label(ui.visuals().error_fg_color, error);
        ui.end_row();
    }
}

fn faces_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    shell: &Shell,
    opened: bool,
    actions: &mut Vec<Action>,
) {
    ui.label("Open faces");
    let document = model.document();
    let solid = bodies::input_solid(model.evaluation(), feature);
    ui.vertical(|ui| {
        if shell.open.is_empty() {
            ui.label("None: the body is hollow and closed");
        } else {
            ui.label(count(shell.open.len(), "face", "faces"));
        }
        for (index, reference) in shell.open.iter().enumerate() {
            let text = solid
                .and_then(|solid| {
                    let face = *shell_tools::resolved(solid, reference).first()?;
                    let key = bodies::face_keys(solid)
                        .into_iter()
                        .find_map(|(id, key)| (id == face).then_some(key))?;
                    Some(bodies::describe_face(document, solid, key))
                })
                .unwrap_or_else(|| "A face that is no longer there".to_owned());
            ui.horizontal(|ui| {
                ui.weak(text);
                let close = ui.small_button("🗙").on_hover_text("Close this face");
                if close.clicked() {
                    let mut changed = shell.clone();
                    changed.open.remove(index);
                    match change(model, feature, changed) {
                        Ok(transaction) => actions.push(Action::Apply(transaction)),
                        Err(reason) => log::warn!("could not change the shell: {reason}"),
                    }
                }
            });
        }
        if opened {
            ui.weak("Click flat faces in the view to open them or close them again.");
        } else if ui
            .small_button("Choose in the view")
            .on_hover_text("Show the body as it was before this feature so you can click faces")
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::OpenSolid(feature)));
        }
    });
    ui.end_row();
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    shell: &Shell,
    opened: bool,
) {
    let id = feature.id();
    Grid::new(("shell-properties", id))
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            thickness_row(ui, model, id, shell, actions);
            faces_row(ui, model, id, shell, opened, actions);
            ui.label("Body");
            ui.label(
                model
                    .document()
                    .feature(shell.body)
                    .map_or("a missing body", |body| body.name.as_str()),
            );
            ui.end_row();
        });
}
