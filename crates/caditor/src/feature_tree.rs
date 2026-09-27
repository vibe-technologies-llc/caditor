use caditor_document::{
    Document, Edit, Feature, FeatureError, FeatureKind, FeatureState, FeatureStatus, FixTarget,
    Transaction,
};
use caditor_sketch::Sketch;
use egui::{Align, Button, CollapsingHeader, Id, RichText, Ui};

use crate::{
    field::{self, Expected},
    model::{Action, Model},
    panels::{Focus, PanelState, Renaming},
};

const NAME_FIELD_WIDTH: f32 = 180.0;
const DIMENSION_FIELD_WIDTH: f32 = 140.0;

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    ui.heading("Features");
    let document = model.document();
    if document.features().len() == 0 {
        ui.weak("The model has no features yet.");
    }
    let count = document.features().len();
    for (index, feature) in document.features().enumerate() {
        let position = Position { index, count };
        ui.push_id(("feature", feature.id()), |ui| {
            feature_row(ui, model, state, actions, feature, position);
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct Position {
    index: usize,
    count: usize,
}

fn feature_row(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    position: Position,
) {
    let document = model.document();
    let id = feature.id();
    let status = model.evaluation().feature(id);

    if let Some(renaming) = state.renaming.filter(|renaming| renaming.feature == id) {
        rename_row(ui, document, state, actions, feature, renaming);
        return;
    }

    let header = CollapsingHeader::new(header_text(ui, feature, status))
        .id_salt(("feature-header", id))
        .open(state.focus_inside(id).then_some(true))
        .show(ui, |ui| match &feature.kind {
            FeatureKind::Sketch(sketch) => sketch_body(ui, model, state, actions, feature, sketch),
        });
    let mut header = header.header_response;
    if state.take_focus(Focus::Feature(id)) {
        header.scroll_to_me(Some(Align::Center));
        header = header.highlight();
    }
    if header.double_clicked() {
        start_renaming(state, feature);
    }
    header.context_menu(|ui| context_menu(ui, document, state, actions, feature, position));

    match status.map(|status| &status.state) {
        Some(FeatureState::Failed(error)) => failure(ui, document, state, error),
        Some(FeatureState::Outdated) => {
            ui.indent("outdated", |ui| {
                ui.weak("Not recomputed, because the recompute was cancelled.");
                if ui.button("Recompute").clicked() {
                    actions.push(Action::Recompute);
                }
            });
        }
        Some(FeatureState::UpToDate) | None => {}
    }
}

fn header_text(ui: &Ui, feature: &Feature, status: Option<&FeatureStatus>) -> RichText {
    match status.map(|status| &status.state) {
        Some(FeatureState::Failed(_)) => {
            RichText::new(format!("⚠ {}", feature.name)).color(ui.visuals().error_fg_color)
        }
        Some(FeatureState::Outdated) => {
            RichText::new(format!("⏸ {}", feature.name)).color(ui.visuals().warn_fg_color)
        }
        Some(FeatureState::UpToDate) => RichText::new(&feature.name),
        None => RichText::new(format!("… {}", feature.name)).weak(),
    }
}

fn start_renaming(state: &mut PanelState, feature: &Feature) {
    state.renaming = Some(Renaming {
        feature: feature.id(),
        focus_pending: true,
    });
}

fn rename_row(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    renaming: Renaming,
) {
    let id = feature.id();
    let field = field::commit_field(
        ui,
        Id::new(("rename-feature", id)),
        &feature.name,
        NAME_FIELD_WIDTH,
        renaming.focus_pending,
        |text| {
            field::checked(
                document,
                Transaction::single(
                    format!("Rename {}", feature.name),
                    Edit::RenameFeature {
                        id,
                        name: text.to_owned(),
                    },
                ),
            )
        },
    );
    if let Some(error) = &field.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if let Some(transaction) = field.committed {
        actions.push(Action::Apply(transaction));
    }
    state.renaming = if field.response.lost_focus() && field.error.is_none() {
        None
    } else {
        Some(Renaming {
            feature: id,
            focus_pending: false,
        })
    };
}

fn context_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    position: Position,
) {
    let id = feature.id();
    let name = &feature.name;
    if ui.button("Rename").clicked() {
        start_renaming(state, feature);
        ui.close();
    }
    let moves = [
        ("Move up", position.index.checked_sub(1)),
        (
            "Move down",
            Some(position.index + 1).filter(|below| *below < position.count),
        ),
    ];
    for (label, target) in moves {
        let transaction = target.map(|index| {
            Transaction::single(format!("{label} {name}"), Edit::MoveFeature { id, index })
        });
        let check = transaction.as_ref().map(|transaction| {
            document
                .check(transaction)
                .map_err(|error| error.to_string())
        });
        let enabled = matches!(check, Some(Ok(())));
        let response = ui.add_enabled(enabled, Button::new(label));
        let response = match check {
            Some(Err(reason)) => response.on_disabled_hover_text(reason),
            Some(Ok(())) | None => response,
        };
        if response.clicked()
            && let Some(transaction) = transaction
        {
            actions.push(Action::Apply(transaction));
            ui.close();
        }
    }
    ui.separator();
    let delete = Transaction::single(format!("Delete {name}"), Edit::RemoveFeature { id });
    let check = document.check(&delete);
    let response = ui.add_enabled(check.is_ok(), Button::new("Delete"));
    let response = match check {
        Err(reason) => response.on_disabled_hover_text(reason.to_string()),
        Ok(()) => response,
    };
    if response.clicked() {
        actions.push(Action::Apply(delete));
        ui.close();
    }
}

fn failure(ui: &mut Ui, document: &Document, state: &mut PanelState, error: &FeatureError) {
    ui.indent("failure", |ui| {
        ui.colored_label(ui.visuals().error_fg_color, &error.reason);
        ui.label(&error.remedy);
        let Some(target) = error.fix else {
            return;
        };
        let label = match target {
            FixTarget::Parameter(id) => {
                format!(
                    "Go to {}",
                    document.parameter_name(id).unwrap_or("the parameter")
                )
            }
            FixTarget::Dimension { .. } => "Edit the dimension".to_owned(),
            FixTarget::Feature(id) => format!(
                "Go to {}",
                document
                    .feature(id)
                    .map_or("the feature", |feature| feature.name.as_str())
            ),
        };
        if ui.button(label).clicked() {
            state.request_focus(target.into());
        }
    });
}

fn sketch_body(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    sketch: &Sketch,
) {
    let document = model.document();
    ui.weak(format!(
        "{}, {}",
        count(sketch.entities().len(), "entity", "entities"),
        count(sketch.constraints().len(), "constraint", "constraints")
    ));
    for (constraint, definition) in sketch.constraints() {
        let description = sketch.describe_constraint(constraint);
        let Some(expression) = definition.dimension() else {
            ui.label(description);
            continue;
        };
        ui.label(&description);
        let focus = Focus::Dimension {
            feature: feature.id(),
            constraint,
        };
        let expected = Expected {
            dimension: definition.dimension_kind(),
            non_negative: true,
        };
        let mut error = None;
        ui.horizontal(|ui| {
            let field = field::commit_field(
                ui,
                focus.field_id(),
                &document.expression_text(expression),
                DIMENSION_FIELD_WIDTH,
                state.wants_focus(focus),
                |text| {
                    let value =
                        field::parse_expression(document, model.parameters(), text, expected)?;
                    field::checked(
                        document,
                        Transaction::single(
                            format!("Edit dimension in {}", feature.name),
                            Edit::SetDimension {
                                feature: feature.id(),
                                constraint,
                                value,
                            },
                        ),
                    )
                },
            );
            state.focus_reached(focus, field.response.has_focus());
            if let Some(transaction) = field.committed {
                actions.push(Action::Apply(transaction));
            }
            if field.error.is_none()
                && let Some(preview) = field::value_preview(model.parameters(), expression)
            {
                ui.weak(preview);
            }
            error = field.error;
        });
        if let Some(error) = error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }
}

pub fn count(amount: usize, singular: &str, plural: &str) -> String {
    let noun = if amount == 1 { singular } else { plural };
    format!("{amount} {noun}")
}
