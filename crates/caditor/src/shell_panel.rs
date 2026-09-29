use caditor_document::{Document, Feature, FeatureId, Resolution, Shell, Transaction};
use caditor_expression::Dimension;
use caditor_kernel::{FaceId, Solid};
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_tree::count,
    field::{self, Expected},
    model::{Action, Model, Notice},
    reference_rows::{ReferenceRows, RowCache},
    shell_tools,
    widgets::{self, FIELD_WIDTH},
};

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
    widgets::caption(ui, "Thickness");
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
            ui.label(widgets::muted(preview, ui));
        }
        error = field.error;
    });
    ui.end_row();
    if let Some(error) = error {
        widgets::error_row(ui, &error);
    }
}

const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const GONE: &str = "A face that is no longer there";

fn face_row(document: &Document, solid: &Solid, resolution: &Resolution<FaceId>) -> String {
    match resolution {
        Resolution::One(face) => bodies::describe_face_id(document, solid, *face),
        Resolution::Pieces(pieces) => match pieces.first().and_then(|face| solid.face(*face)) {
            Some(first) => format!(
                "{}, split into {} pieces",
                bodies::describe_origin(document, first.origin()),
                pieces.len()
            ),
            None => GONE.to_owned(),
        },
        Resolution::Tied(candidates) => format!(
            "A face that now matches {} separate faces; close it and choose it again",
            candidates.len()
        ),
        Resolution::Missing => GONE.to_owned(),
    }
}

fn face_rows(document: &Document, solid: Option<&Solid>, shell: &Shell) -> ReferenceRows {
    let summary = count(shell.open.len(), "face", "faces");
    let rows = match solid {
        Some(solid) => shell
            .resolutions(solid)
            .iter()
            .map(|resolution| face_row(document, solid, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); shell.open.len()],
    };
    ReferenceRows { summary, rows }
}

fn faces_row(
    ui: &mut Ui,
    model: &Model,
    cache: &mut RowCache,
    feature: FeatureId,
    shell: &Shell,
    opened: bool,
    actions: &mut Vec<Action>,
) {
    widgets::caption(ui, "Open faces");
    let evaluation = model.evaluation();
    let listed = cache.rows(
        feature,
        evaluation.body_before(feature),
        model.revision(),
        || {
            face_rows(
                model.document(),
                bodies::input_solid(evaluation, feature),
                shell,
            )
        },
    );
    ui.vertical(|ui| {
        if shell.open.is_empty() {
            ui.label(widgets::muted("None: the body is hollow and closed", ui));
        } else {
            ui.label(&listed.summary);
        }
        for (index, text) in listed.rows.iter().enumerate() {
            let text = widgets::muted(text, ui);
            if widgets::removable_row(ui, text, "Close this face") {
                let mut changed = shell.clone();
                changed.open.remove(index);
                match change(model, feature, changed) {
                    Ok(transaction) => actions.push(Action::Apply(transaction)),
                    Err(reason) => actions.push(Action::Inform(Notice::error(format!(
                        "The shell was not changed: {reason}"
                    )))),
                }
            }
        }
        if widgets::choose_in_view(
            ui,
            opened,
            "Click flat faces in the view to open them or close them again.",
            "Show the body as it was before this feature so you can click faces",
        ) {
            actions.push(Action::Editing(EditingCommand::OpenSolid(feature)));
        }
    });
    ui.end_row();
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    cache: &mut RowCache,
    actions: &mut Vec<Action>,
    feature: &Feature,
    shell: &Shell,
    opened: bool,
) {
    let id = feature.id();
    widgets::properties(ui, ("shell-properties", id), |ui| {
        thickness_row(ui, model, id, shell, actions);
        faces_row(ui, model, cache, id, shell, opened, actions);
        widgets::caption(ui, "Body");
        ui.label(
            model
                .document()
                .feature(shell.body)
                .map_or("a missing body", |body| body.name.as_str()),
        );
        ui.end_row();
    });
}
