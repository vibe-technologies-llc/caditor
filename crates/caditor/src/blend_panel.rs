use caditor_document::{Blend, BlendKind, Feature, FeatureId, Transaction};
use caditor_expression::Dimension;
use egui::{Button, ComboBox, Id, Ui};

use crate::{
    blend_tools::{self, KINDS},
    bodies,
    editing::EditingCommand,
    feature_tree::count,
    field::{self, Expected},
    icons,
    model::{Action, Model, Notice},
    widgets::{self, FIELD_WIDTH},
};

fn change(model: &Model, feature: FeatureId, blend: Blend) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = blend_tools::edit(document, feature, blend)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn apply(actions: &mut Vec<Action>, change: Result<Transaction, String>) {
    match change {
        Ok(transaction) => actions.push(Action::Apply(transaction)),
        Err(reason) => actions.push(Action::Inform(Notice::error(format!(
            "The blend was not changed: {reason}"
        )))),
    }
}

fn kind_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    widgets::caption(ui, "Shape");
    let mut chosen = None;
    ComboBox::from_id_salt(("blend-kind", feature))
        .selected_text(blend.kind.title())
        .show_ui(ui, |ui| {
            for kind in KINDS {
                let selected = kind == blend.kind;
                let change = (!selected).then(|| {
                    change(
                        model,
                        feature,
                        Blend {
                            kind,
                            ..blend.clone()
                        },
                    )
                });
                let enabled = !matches!(change, Some(Err(_)));
                let response = ui.add_enabled(enabled, Button::selectable(selected, kind.title()));
                let response = match &change {
                    Some(Err(reason)) => response.on_disabled_hover_text(reason),
                    Some(Ok(_)) | None => response,
                };
                if response.clicked() {
                    chosen = change;
                }
            }
        });
    ui.end_row();
    if let Some(change) = chosen {
        apply(actions, change);
    }
}

fn size_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    let what = blend.kind.size_name();
    let mut title = what.to_owned();
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    widgets::caption(ui, &title);
    let document = model.document();
    let parameters = model.parameters();
    let mut error = None;
    ui.horizontal(|ui| {
        let field = field::commit_field(
            ui,
            Id::new(("blend-size", feature)),
            &document.expression_text(&blend.size),
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
                    return Err(format!("Enter a {what} above zero"));
                }
                change(
                    model,
                    feature,
                    Blend {
                        size: parsed,
                        ..blend.clone()
                    },
                )
            },
        );
        if let Some(transaction) = field.committed {
            actions.push(Action::Apply(transaction));
        }
        if field.error.is_none()
            && let Some(preview) =
                field::value_preview(parameters, &blend.size, model.length_unit())
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

fn edges_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    opened: bool,
    actions: &mut Vec<Action>,
) {
    widgets::caption(ui, "Edges");
    let document = model.document();
    let solid = bodies::input_solid(model.evaluation(), feature);
    let chosen = solid.map(|solid| blend_tools::chosen_edges(solid, blend));
    ui.vertical(|ui| {
        let mut summary = count(blend.edges.len(), "edge", "edges");
        if let Some(chosen) = &chosen {
            let extra = chosen.followed.len().saturating_sub(chosen.explicit.len());
            if extra > 0 {
                summary.push_str(&format!(
                    ", and {} that {} smoothly",
                    count(extra, "edge", "edges"),
                    if extra == 1 { "continues" } else { "continue" }
                ));
            }
        }
        ui.label(summary);
        for (index, reference) in blend.edges.iter().enumerate() {
            let text = solid
                .and_then(|solid| {
                    let edge = reference.resolve(solid).ok()?;
                    Some(bodies::describe_edge(
                        document,
                        solid,
                        solid.edge(edge)?.name(),
                    ))
                })
                .unwrap_or_else(|| "An edge that is no longer there".to_owned());
            ui.horizontal(|ui| {
                ui.label(widgets::muted(text, ui));
                let remove = widgets::icon_button(ui, icons::REMOVE, "Leave this edge out");
                if remove.clicked() {
                    let mut changed = blend.clone();
                    changed.edges.remove(index);
                    apply(actions, change(model, feature, changed));
                }
            });
        }
        if widgets::choose_in_view(
            ui,
            opened,
            "Click edges in the view to add them or leave them out.",
            "Show the body as it was before this feature so you can click edges",
        ) {
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
    blend: &Blend,
    opened: bool,
) {
    let id = feature.id();
    widgets::properties(ui, ("blend-properties", id), |ui| {
        kind_row(ui, model, id, blend, actions);
        size_row(ui, model, id, blend, actions);
        edges_row(ui, model, id, blend, opened, actions);
        widgets::caption(ui, "Body");
        ui.label(
            model
                .document()
                .feature(blend.body)
                .map_or("a missing body", |body| body.name.as_str()),
        );
        ui.end_row();
    });
}

pub fn describe_kind(kind: BlendKind) -> &'static str {
    match kind {
        BlendKind::Fillet => "Round the selected edges",
        BlendKind::Chamfer => "Bevel the selected edges",
    }
}
