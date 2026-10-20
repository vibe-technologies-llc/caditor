use std::convert::Infallible;

use caditor_document::{Combine, CombineOperation, Feature, FeatureId, Transaction};
use egui::{Id, Ui};

use crate::{
    combine_tools,
    editing::{Combining, EditingCommand},
    feature_fields::{self, Choice, Segment},
    field::{self, CheckError},
    icons,
    model::{Action, Model},
    widgets,
};

pub const DESCRIPTION: &str = "Combines bodies into one";
pub const KEEP_TOOL: &str = "Keep tool";
pub const ALSO_COMBINES: &str = "Also with";
pub const ADD_TOOL_BODY: &str = "Add another tool body";
const NOT_A_BODY: &str = "A body that is no longer there";
pub const SWAP: &str = "Swap target and tool";
const SWAP_HOVER: &str = "Make the tool body the target and the target the tool, in one change";
const CHOOSING_DESCRIPTION: &str = "Click the body to combine into in the view, then the body to \
                                    combine with it, or choose them here";
pub const UNCHOSEN: &str = "Click a body or choose one";
pub const CANCEL: &str = "Cancel";
const CANCEL_HOVER: &str = "Stop choosing bodies to combine (Esc)";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum CombineChangeError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

fn change(
    model: &Model,
    feature: FeatureId,
    combine: Combine,
) -> Result<Transaction, CombineChangeError> {
    let document = model.document();
    let transaction = combine_tools::edit(document, feature, combine)
        .ok_or(CombineChangeError::Gone { feature })?;
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

    fn taken(self, combine: &Combine) -> Vec<FeatureId> {
        match self {
            Self::Target => combine.tools().collect(),
            Self::Tool => std::iter::once(combine.body)
                .chain(combine.more_tools.iter().copied())
                .collect(),
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
    let taken = role.taken(combine);
    let current_name = document.body_label(current);
    let selected = feature_fields::combo_text(ui, current_name.as_deref(), NOT_A_BODY);
    let chosen = feature_fields::combo(ui, Id::unique((role.salt(), id)), selected, || {
        document
            .bodies_before_in(id, model.evaluation())
            .into_iter()
            .filter(|body| !taken.contains(body))
            .filter_map(|body| {
                Some(Choice {
                    label: document.body_label(body)?,
                    selected: body == current,
                    change: change(model, id, role.with(combine, body)).map(|transaction| {
                        feature_fields::applied(
                            &feature.name,
                            Ok::<_, CombineChangeError>(transaction),
                        )
                    }),
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
        swap_row(ui, model, feature, combine, actions);
        more_tool_rows(ui, model, feature, combine, actions);
        keep_tool_row(ui, model, feature, combine, actions);
    });
}

fn more_tool_rows(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    combine: &Combine,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let id = feature.id();
    for (index, tool) in combine.more_tools.iter().enumerate() {
        if index == 0 {
            widgets::caption(ui, ALSO_COMBINES);
        } else {
            ui.label("");
        }
        let name = feature_fields::feature_name(document, *tool);
        let hover = format!("Stop using {}", name.unwrap_or("the missing body"));
        let mut dropped = false;
        ui.horizontal(|ui| {
            match name {
                Some(name) => {
                    ui.label(name);
                }
                None => feature_fields::missing(ui, NOT_A_BODY),
            }
            dropped = widgets::icon_button(ui, icons::REMOVE, &hover).clicked();
        });
        ui.end_row();
        if dropped {
            let mut kept = combine.clone();
            kept.more_tools.retain(|other| other != tool);
            actions.push(feature_fields::applied(
                &feature.name,
                change(model, id, kept),
            ));
        }
    }
    let candidates: Vec<FeatureId> = document
        .bodies_before_in(id, model.evaluation())
        .into_iter()
        .filter(|body| combine.body != *body && !combine.tools().any(|tool| tool == *body))
        .collect();
    if candidates.is_empty() {
        return;
    }
    if combine.more_tools.is_empty() {
        widgets::caption(ui, ALSO_COMBINES);
    } else {
        ui.label("");
    }
    let chosen = feature_fields::combo(
        ui,
        Id::unique(("combine-more-tool", id)),
        ADD_TOOL_BODY,
        || {
            candidates
                .iter()
                .filter_map(|body| {
                    let mut extended = combine.clone();
                    extended.more_tools.push(*body);
                    Some(Choice {
                        label: document.body_label(*body)?,
                        selected: false,
                        change: change(model, id, extended).map(|transaction| {
                            feature_fields::applied(
                                &feature.name,
                                Ok::<_, CombineChangeError>(transaction),
                            )
                        }),
                    })
                })
                .collect()
        },
    );
    actions.extend(chosen);
    ui.end_row();
}

pub fn swapped(combine: &Combine) -> Combine {
    Combine {
        body: combine.tool,
        tool: combine.body,
        ..combine.clone()
    }
}

fn swap_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    combine: &Combine,
    actions: &mut Vec<Action>,
) {
    ui.label("");
    let button = widgets::small_button(ui, icons::SWAP, SWAP);
    if ui.add(button).on_hover_text(SWAP_HOVER).clicked() {
        actions.push(feature_fields::applied(
            &feature.name,
            change(model, feature.id(), swapped(combine)),
        ));
    }
    ui.end_row();
}

fn keep_tool_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    combine: &Combine,
    actions: &mut Vec<Action>,
) {
    if let Some(keep_tool) = feature_fields::reverse_row(ui, KEEP_TOOL, combine.keep_tool) {
        let kept = Combine {
            keep_tool,
            ..combine.clone()
        };
        actions.push(feature_fields::applied(
            &feature.name,
            change(model, feature.id(), kept),
        ));
    }
}

pub fn show_choosing(ui: &mut Ui, model: &Model, combining: Combining, actions: &mut Vec<Action>) {
    let document = model.document();
    let bodies = combine_tools::choosable(model);
    let choices = |except: Option<FeatureId>, command: fn(FeatureId) -> EditingCommand| {
        bodies
            .iter()
            .filter(|body| Some(**body) != except)
            .filter_map(|body| {
                Some(Choice {
                    label: document.body_label(*body)?,
                    selected: false,
                    change: Ok::<_, Infallible>(Action::Editing(command(*body))),
                })
            })
            .collect()
    };
    widgets::card(ui, |ui| {
        ui.label(widgets::section_title(combine_tools::TITLE));
        widgets::properties(ui, "combine-choosing", |ui| {
            feature_fields::description_row(ui, CHOOSING_DESCRIPTION);
            widgets::caption(ui, Role::Target.caption());
            let target = combining
                .target
                .and_then(|target| document.body_label(target))
                .unwrap_or_else(|| UNCHOSEN.to_owned());
            let chosen =
                feature_fields::combo(ui, Id::unique("combine-choosing-target"), target, || {
                    choices(combining.target, |body| {
                        EditingCommand::ChooseCombined(Some(body))
                    })
                });
            actions.extend(chosen);
            ui.end_row();
            if let Some(target) = combining.target {
                widgets::caption(ui, Role::Tool.caption());
                let chosen = feature_fields::combo(
                    ui,
                    Id::unique("combine-choosing-tool"),
                    UNCHOSEN,
                    || choices(Some(target), EditingCommand::CombineWith),
                );
                actions.extend(chosen);
                ui.end_row();
            }
        });
        if ui
            .add(widgets::button(CANCEL))
            .on_hover_text(CANCEL_HOVER)
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::CancelCombine));
        }
    });
}
