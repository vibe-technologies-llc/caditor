use caditor_document::{Combine, CombineOperation, Feature, FeatureId, Transaction};
use egui::{Id, Ui};

use crate::{
    combine_tools,
    feature_fields::{self, Choice, Segment},
    field,
    model::{Action, Model},
    widgets,
};

pub const DESCRIPTION: &str = "Combines two bodies into one";
const NOT_A_BODY: &str = "A body that is no longer there";

fn change(model: &Model, feature: FeatureId, combine: Combine) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = combine_tools::edit(document, feature, combine)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn operation_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    combine: &Combine,
    actions: &mut Vec<Action>,
) {
    let segments = CombineOperation::ALL
        .into_iter()
        .map(|operation| Segment {
            label: operation.verb(),
            hover: combine_tools::operation_hover(operation),
            change: (operation != combine.operation).then(|| {
                change(
                    model,
                    feature.id(),
                    Combine {
                        operation,
                        ..combine.clone()
                    },
                )
            }),
        })
        .collect();
    actions.extend(feature_fields::segmented_row(
        ui,
        "Operation",
        &feature.name,
        segments,
    ));
}

#[derive(Clone, Copy)]
enum Role {
    Target,
    Tool,
}

impl Role {
    fn caption(self) -> &'static str {
        match self {
            Self::Target => "Target body",
            Self::Tool => "Tool body",
        }
    }

    fn salt(self) -> &'static str {
        match self {
            Self::Target => "combine-target",
            Self::Tool => "combine-tool",
        }
    }

    fn current(self, combine: &Combine) -> FeatureId {
        match self {
            Self::Target => combine.body,
            Self::Tool => combine.tool,
        }
    }

    fn other(self, combine: &Combine) -> FeatureId {
        match self {
            Self::Target => combine.tool,
            Self::Tool => combine.body,
        }
    }

    fn with(self, combine: &Combine, body: FeatureId) -> Combine {
        match self {
            Self::Target => Combine {
                body,
                ..combine.clone()
            },
            Self::Tool => Combine {
                tool: body,
                ..combine.clone()
            },
        }
    }
}

fn body_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    combine: &Combine,
    role: Role,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let id = feature.id();
    widgets::caption(ui, role.caption());
    let current = role.current(combine);
    let selected = feature_fields::combo_text(
        ui,
        feature_fields::feature_name(document, current),
        NOT_A_BODY,
    );
    let chosen = feature_fields::combo(ui, Id::new((role.salt(), id)), selected, || {
        document
            .bodies_before(id)
            .into_iter()
            .filter(|body| *body != role.other(combine))
            .filter_map(|body| {
                Some(Choice {
                    label: feature_fields::feature_name(document, body)?.to_owned(),
                    selected: body == current,
                    change: change(model, id, role.with(combine, body))
                        .map(|transaction| feature_fields::applied(&feature.name, Ok(transaction))),
                })
            })
            .collect()
    });
    actions.extend(chosen);
    ui.end_row();
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    combine: &Combine,
) {
    widgets::properties(ui, ("combine-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        operation_row(ui, model, feature, combine, actions);
        body_row(ui, model, feature, combine, Role::Target, actions);
        body_row(ui, model, feature, combine, Role::Tool, actions);
    });
}
