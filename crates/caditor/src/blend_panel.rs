use caditor_document::{Blend, BlendKind, Feature, FeatureId, Transaction};
use caditor_expression::Dimension;
use egui::{Button, ComboBox, Grid, Id, Ui};

use crate::{
    blend_tools::{self, KINDS},
    bodies,
    editing::EditingCommand,
    feature_tree::count,
    field::{self, Expected},
    model::{Action, Model},
};

const FIELD_WIDTH: f32 = 110.0;

fn change(model: &Model, feature: FeatureId, blend: Blend) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = blend_tools::edit(document, feature, blend)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn apply(actions: &mut Vec<Action>, change: Result<Transaction, String>) {
    match change {
        Ok(transaction) => actions.push(Action::Apply(transaction)),
        Err(reason) => log::warn!("could not change the blend: {reason}"),
    }
}

fn kind_row(ui: &mut Ui, model: &Model, feature: FeatureId, blend: &Blend) -> Option<Action> {
    ui.label("Shape");
    let mut chosen = None;
    ComboBox::from_id_salt(("blend-kind", feature))
        .selected_text(blend.kind.title())
        .show_ui(ui, |ui| {
            for kind in KINDS {
                let selected = kind == blend.kind;
                if ui.add(Button::selectable(selected, kind.title())).clicked() && !selected {
                    chosen = Some(kind);
                }
            }
        });
    ui.end_row();
    let kind = chosen?;
    let changed = Blend {
        kind,
        ..blend.clone()
    };
    change(model, feature, changed).ok().map(Action::Apply)
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
    ui.label(title);
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
            && let Some(preview) = field::value_preview(parameters, &blend.size)
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

fn edges_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    opened: bool,
    actions: &mut Vec<Action>,
) {
    ui.label("Edges");
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
                ui.weak(text);
                let remove = ui.small_button("🗙").on_hover_text("Leave this edge out");
                if remove.clicked() {
                    let mut changed = blend.clone();
                    changed.edges.remove(index);
                    apply(actions, change(model, feature, changed));
                }
            });
        }
        if opened {
            ui.weak("Click edges in the view to add them or leave them out.");
        } else if ui
            .small_button("Choose in the view")
            .on_hover_text("Show the body as it was before this feature so you can click edges")
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
    blend: &Blend,
    opened: bool,
) {
    let id = feature.id();
    Grid::new(("blend-properties", id))
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            if let Some(action) = kind_row(ui, model, id, blend) {
                actions.push(action);
            }
            size_row(ui, model, id, blend, actions);
            edges_row(ui, model, id, blend, opened, actions);
            ui.label("Body");
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
