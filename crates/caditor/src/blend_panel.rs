use caditor_document::{
    Blend, BlendKind, ChamferForm, Document, Feature, FeatureId, Resolution, SolidResult,
    Transaction,
};
use caditor_expression::Dimension;
use caditor_kernel::EdgeId;
use egui::{Id, Ui};

use crate::{
    blend_tools::{self, FORMS, FormChoice, KINDS},
    bodies,
    editing::EditingCommand,
    feature_fields::{self, Quantity, Rule, Segment},
    feature_tree::count,
    field,
    model::{Action, Model},
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    widgets,
};

fn change(model: &Model, feature: FeatureId, blend: Blend) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = blend_tools::edit(document, feature, blend)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn kind_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    let segments = KINDS
        .into_iter()
        .map(|kind| Segment {
            label: kind.title(),
            hover: describe_kind(kind),
            change: (kind != blend.kind).then(|| {
                change(
                    model,
                    feature.id(),
                    Blend {
                        kind,
                        ..blend.clone()
                    },
                )
            }),
        })
        .collect();
    actions.extend(feature_fields::segmented_row(
        ui,
        "Shape",
        &feature.name,
        segments,
    ));
}

fn form_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    let current = FormChoice::of(&blend.form);
    let segments = FORMS
        .into_iter()
        .map(|choice| Segment {
            label: choice.short(),
            hover: choice.hover(),
            change: (choice != current)
                .then(|| change(model, feature.id(), choice.applied_to(blend))),
        })
        .collect();
    actions.extend(feature_fields::segmented_row(
        ui,
        "Distances",
        &feature.name,
        segments,
    ));
}

fn size_caption(blend: &Blend) -> &'static str {
    match (blend.kind, blend.chamfer_form()) {
        (BlendKind::Fillet, _) => "Radius",
        (BlendKind::Chamfer, ChamferForm::TwoDistances { .. }) => "First distance",
        (BlendKind::Chamfer, ChamferForm::Equal | ChamferForm::DistanceAngle { .. }) => "Distance",
    }
}

fn form_value_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    let (caption, expression, dimension, rule) = match blend.chamfer_form() {
        ChamferForm::Equal => return,
        ChamferForm::TwoDistances { second } => (
            "Second distance",
            second,
            Dimension::LENGTH,
            Rule::AboveZero,
        ),
        ChamferForm::DistanceAngle { angle } => {
            ("Angle", angle, Dimension::ANGLE, Rule::ChamferAngle)
        }
    };
    let quantity = Quantity {
        feature,
        id: Id::new(("blend-form", feature)),
        expression,
        dimension,
        rule,
    };
    let drafting = feature_fields::expression_row_drafting(ui, model, caption, quantity, |value| {
        let mut changed = blend.clone();
        if let Some(slot) = changed.form.expression_mut() {
            *slot = value;
        }
        change(model, feature, changed)
    });
    actions.extend(drafting.into_actions(feature));
}

fn flip_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    if blend.chamfer_form().is_equal() {
        return;
    }
    if let Some(flipped) = feature_fields::reverse_row(ui, FLIP, blend.flipped) {
        actions.push(feature_fields::applied(
            &feature.name,
            change(
                model,
                feature.id(),
                Blend {
                    flipped,
                    ..blend.clone()
                },
            ),
        ));
    }
}

const FLIP: &str = "Measure from the other face";

fn size_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    blend: &Blend,
    actions: &mut Vec<Action>,
) {
    let quantity = Quantity {
        feature,
        id: Id::new(("blend-size", feature)),
        expression: &blend.size,
        dimension: Dimension::LENGTH,
        rule: Rule::AboveZero,
    };
    let caption = size_caption(blend);
    let drafting = feature_fields::expression_row_drafting(ui, model, caption, quantity, |size| {
        change(
            model,
            feature,
            Blend {
                size,
                ..blend.clone()
            },
        )
    });
    actions.extend(drafting.into_actions(feature));
    feature_fields::draft_failure_row(ui, model, feature);
}

const NO_SHAPE_YET: &str = "An edge of a body that has no shape yet";
const GONE: &str = "An edge that is no longer there";

fn edge_row(document: &Document, input: &SolidResult, resolution: &Resolution<EdgeId>) -> String {
    match resolution {
        Resolution::One(edge) => bodies::describe_edge_id(document, input, *edge),
        Resolution::Pieces(pieces) => match pieces.first() {
            Some(first) => format!(
                "{}, split into {} pieces",
                bodies::describe_edge_id(document, input, *first),
                pieces.len()
            ),
            None => GONE.to_owned(),
        },
        Resolution::Tied(candidates) => format!(
            "An edge that now matches {} separate edges; leave it out and choose it again",
            candidates.len()
        ),
        Resolution::Missing => GONE.to_owned(),
    }
}

fn edge_rows(document: &Document, input: Option<&SolidResult>, blend: &Blend) -> ReferenceRows {
    let mut summary = count(blend.edges.len(), "edge", "edges");
    let Some(input) = input else {
        return ReferenceRows {
            summary,
            rows: vec![NO_SHAPE_YET.to_owned(); blend.edges.len()],
        };
    };
    let solid = &input.solid;
    let chosen = blend_tools::chosen_edges(solid, blend);
    let extra = chosen.followed.len().saturating_sub(chosen.explicit.len());
    if extra > 0 {
        summary.push_str(&format!(
            ", and {} that {} smoothly",
            count(extra, "edge", "edges"),
            if extra == 1 { "continues" } else { "continue" }
        ));
    }
    let rows = blend
        .resolutions(solid)
        .iter()
        .map(|resolution| edge_row(document, input, resolution))
        .collect();
    ReferenceRows { summary, rows }
}

pub struct EdgesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub blend: &'a Blend,
    pub opened: bool,
}

fn edges_row(ui: &mut Ui, row: &EdgesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let EdgesRow {
        model,
        selection,
        feature,
        blend,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Edges");
    let evaluation = model.evaluation();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        edge_rows(model.document(), bodies::input(evaluation, id), blend)
    });
    ui.vertical(|ui| {
        ui.label(&listed.summary);
        for (index, text) in listed.rows.iter().enumerate() {
            let text = widgets::muted(text, ui);
            if widgets::removable_row(ui, text, "Leave this edge out") {
                let mut changed = blend.clone();
                changed.edges.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    change(model, id, changed),
                ));
            }
        }
        if widgets::choose_in_view(
            ui,
            feature_fields::choosing_list(ui, opened),
            "Click edges in the view to add them or leave them out.",
            "Show the body as it was before this feature, with its edges and those selected now \
             highlighted, so you can click edges to add or leave out",
        ) {
            if let Some(transaction) = blend_tools::with_selected_edges(model, id, selection) {
                actions.push(Action::Apply(transaction));
            }
            actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
        }
    });
    ui.end_row();
}

pub fn show(ui: &mut Ui, row: &EdgesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let EdgesRow {
        model,
        feature,
        blend,
        ..
    } = *row;
    let id = feature.id();
    widgets::properties(ui, ("blend-properties", id), |ui| {
        kind_row(ui, model, feature, blend, actions);
        if blend.kind == BlendKind::Chamfer {
            form_row(ui, model, feature, blend, actions);
        }
        edges_row(ui, row, cache, actions);
        size_row(ui, model, id, blend, actions);
        form_value_row(ui, model, id, blend, actions);
        flip_row(ui, model, feature, blend, actions);
        feature_fields::feature_row(ui, model.document(), "Body", blend.body);
    });
}

pub fn describe_kind(kind: BlendKind) -> &'static str {
    match kind {
        BlendKind::Fillet => "Round the selected edges",
        BlendKind::Chamfer => "Bevel the selected edges",
    }
}
