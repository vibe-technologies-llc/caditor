use caditor_document::{
    Document, Edit, ExtrudeExtent, Feature, FeatureError, FeatureKind, FeatureState, FeatureStatus,
    FixTarget, RevolveExtent, SolidFeature, Transaction,
};
use caditor_sketch::{ConstraintId, Redundancy, Sketch};
use egui::{
    Align, Button, Id, Label, Response, RichText, Sense, Ui, collapsing_header::CollapsingState,
};

use crate::{
    editing::{EditingCommand, SketchEditing},
    field::{self, DimensionTarget},
    model::{Action, Model},
    panels::{Focus, PanelState, Renaming},
    sketch_status::{self, SketchSummary},
    sketch_tools,
};

const NAME_FIELD_WIDTH: f32 = 180.0;
const DIMENSION_FIELD_WIDTH: f32 = 140.0;
const EDIT_SKETCH_LABEL: &str = "Edit sketch";
const FINISH_SKETCH_LABEL: &str = "Finish sketch";
const EDIT_ICON: &str = "🖊";
const DELETE_ICON: &str = "🗙";

pub fn show(
    ui: &mut Ui,
    model: &Model,
    editing: &SketchEditing,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    if editing.feature().is_none() {
        state.opened_for_editing = None;
    }
    ui.heading("Features");
    let document = model.document();
    if document.features().len() == 0 {
        ui.weak("The model has no features yet.");
    }
    let count = document.features().len();
    for (index, feature) in document.features().enumerate() {
        let row = Row {
            feature,
            position: Position { index, count },
            edited: editing.feature() == Some(feature.id()),
        };
        ui.push_id(("feature", feature.id()), |ui| {
            feature_row(ui, model, state, actions, &row);
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct Position {
    index: usize,
    count: usize,
}

struct Row<'a> {
    feature: &'a Feature,
    position: Position,
    edited: bool,
}

fn feature_row(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let document = model.document();
    let feature = row.feature;
    let id = feature.id();
    let status = model.evaluation().feature(id);

    if let Some(renaming) = state.renaming.filter(|renaming| renaming.feature == id) {
        rename_row(ui, document, state, actions, feature, renaming);
        return;
    }

    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("feature-header", id)),
        false,
    );
    let editing_started = row.edited && state.opened_for_editing != Some(id);
    if editing_started {
        state.opened_for_editing = Some(id);
    }
    if state.focus_inside(id) || editing_started {
        collapsing.set_open(true);
    }
    let text = header_text(ui, feature, status);
    let mut toggle = false;
    let mut header = collapsing.show_header(ui, |ui| {
        let label = ui.add(Label::new(text).selectable(false).sense(Sense::click()));
        toggle = label.clicked();
        edit_button(ui, row, actions);
        label
    });
    if toggle {
        header.toggle();
    }
    let (_, header, _) = header.body(|ui| match &feature.kind {
        FeatureKind::Sketch(sketch) => sketch_body(ui, model, state, actions, feature, sketch),
        FeatureKind::Solid(solid) => solid_body(ui, model.document(), solid),
    });
    let mut header = header.inner;
    if state.take_focus(Focus::Feature(id)) {
        header.scroll_to_me(Some(Align::Center));
        header = header.highlight();
    }
    if header.double_clicked() {
        start_renaming(state, feature);
    }
    header.context_menu(|ui| context_menu(ui, document, state, actions, row));

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
            RichText::new(format!("⚑ {}", feature.name)).color(ui.visuals().error_fg_color)
        }
        Some(FeatureState::Outdated) => {
            RichText::new(format!("⏸ {}", feature.name)).color(ui.visuals().warn_fg_color)
        }
        Some(FeatureState::UpToDate) => RichText::new(&feature.name),
        None => RichText::new(format!("… {}", feature.name)).weak(),
    }
}

fn edit_button(ui: &mut Ui, row: &Row<'_>, actions: &mut Vec<Action>) {
    let (hover, command) = edit_command(row);
    let response = ui
        .add(Button::selectable(row.edited, EDIT_ICON).small())
        .on_hover_text(hover);
    if response.clicked() {
        actions.push(Action::Editing(command));
    }
}

fn edit_command(row: &Row<'_>) -> (&'static str, EditingCommand) {
    if row.edited {
        (FINISH_SKETCH_LABEL, EditingCommand::Finish)
    } else {
        (EDIT_SKETCH_LABEL, EditingCommand::Enter(row.feature.id()))
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
    row: &Row<'_>,
) {
    let feature = row.feature;
    let position = row.position;
    let id = feature.id();
    let name = &feature.name;
    let (label, command) = edit_command(row);
    if ui.button(label).clicked() {
        actions.push(Action::Editing(command));
        ui.close();
    }
    ui.separator();
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
            FixTarget::Constraint {
                feature,
                constraint,
            } => match document
                .feature(feature)
                .and_then(|owner| owner.kind.sketch())
            {
                Some(sketch) => format!("Go to {}", sketch.describe_constraint(constraint)),
                None => "Go to the constraint".to_owned(),
            },
        };
        if ui.button(label).clicked() {
            state.request_focus(target.into());
        }
    });
}

fn solid_body(ui: &mut Ui, document: &Document, solid: &SolidFeature) {
    let name_of = |id| {
        document
            .feature(id)
            .map_or("a missing feature", |feature| feature.name.as_str())
    };
    let text = |expression| document.expression_text(expression);
    let (shape, extent) = match solid {
        SolidFeature::Extrude(extrude) => (
            "Extrusion",
            match &extrude.extent {
                ExtrudeExtent::OneSide { distance, reversed } => format!(
                    "{}{}",
                    text(distance),
                    if *reversed { ", reversed" } else { "" }
                ),
                ExtrudeExtent::Symmetric { distance } => format!("{} symmetric", text(distance)),
                ExtrudeExtent::TwoSides { forward, backward } => {
                    format!("{} forward, {} back", text(forward), text(backward))
                }
            },
        ),
        SolidFeature::Revolve(revolve) => (
            "Revolution",
            match &revolve.extent {
                RevolveExtent::Full => "full turn".to_owned(),
                RevolveExtent::OneSide { angle, reversed } => format!(
                    "{}{}",
                    text(angle),
                    if *reversed { ", reversed" } else { "" }
                ),
                RevolveExtent::Symmetric { angle } => format!("{} symmetric", text(angle)),
            },
        ),
    };
    ui.label(format!("{shape} of {}, {extent}", name_of(solid.sketch())));
    let operation = solid.operation();
    ui.label(match operation.target() {
        Some(body) => format!("{} {}", operation.verb(), name_of(body)),
        None => operation.verb().to_owned(),
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
    let summary = SketchSummary::of(model.evaluation(), feature.id());
    ui.horizontal_wrapped(|ui| {
        if let Some(focus) = sketch_status::show(ui, &summary) {
            state.request_focus(focus);
        }
    });
    ui.weak(format!(
        "{}, {}",
        count(sketch.entities().len(), "entity", "entities"),
        count(sketch.constraints().len(), "constraint", "constraints")
    ));
    let involved = involved_constraints(model, feature);
    let solution = sketch_status::up_to_date_solution(model.evaluation(), feature.id());
    for (constraint, definition) in sketch.constraints() {
        let description = sketch.describe_constraint(constraint);
        let redundancy = solution.and_then(|solution| solution.redundancy(constraint));
        let text = if involved.contains(&constraint) {
            RichText::new(&description).color(ui.visuals().error_fg_color)
        } else if redundancy.is_some() {
            RichText::new(&description).color(ui.visuals().warn_fg_color)
        } else {
            RichText::new(&description)
        };
        let delete = || {
            Action::Apply(sketch_tools::remove_items(
                model,
                feature.id(),
                format!("Delete {description}"),
                Vec::new(),
                vec![constraint],
            ))
        };
        let row = ui
            .horizontal(|ui| {
                let row = ui.add(Label::new(text).selectable(false).sense(Sense::click()));
                row.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        actions.push(delete());
                        ui.close();
                    }
                });
                let button = ui
                    .small_button(DELETE_ICON)
                    .on_hover_text("Delete this constraint");
                if button.clicked() {
                    actions.push(delete());
                }
                row
            })
            .inner;
        if let Some(redundancy) = redundancy {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                redundancy_text(sketch, redundancy),
            );
        }
        reveal_if_focused(
            state,
            row,
            Focus::Constraint {
                feature: feature.id(),
                constraint,
            },
        );
        let Some(expression) = definition.dimension() else {
            continue;
        };
        let focus = Focus::Dimension {
            feature: feature.id(),
            constraint,
        };
        let target = DimensionTarget {
            feature: feature.id(),
            constraint,
        };
        let mut error = None;
        ui.horizontal(|ui| {
            let field = field::commit_field(
                ui,
                focus.field_id(),
                &document.expression_text(expression),
                DIMENSION_FIELD_WIDTH,
                state.wants_focus(focus),
                |text| field::dimension_transaction(document, model.parameters(), target, text),
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

fn redundancy_text(sketch: &Sketch, redundancy: &Redundancy) -> String {
    let duplicates: Vec<String> = redundancy
        .duplicates
        .iter()
        .map(|duplicate| sketch.describe_constraint(*duplicate))
        .collect();
    match duplicates.as_slice() {
        [] => "Redundant: other constraints already do this. Delete it.".to_owned(),
        [only] => format!("Redundant: {only} already does this. Delete one of them."),
        [rest @ .., last] => format!(
            "Redundant: {} and {last} already do this. Delete one of them.",
            rest.join(", ")
        ),
    }
}

fn involved_constraints(model: &Model, feature: &Feature) -> Vec<ConstraintId> {
    match model
        .evaluation()
        .feature(feature.id())
        .map(|status| &status.state)
    {
        Some(FeatureState::Failed(error)) => error.constraints.clone(),
        Some(FeatureState::UpToDate | FeatureState::Outdated) | None => Vec::new(),
    }
}

fn reveal_if_focused(state: &mut PanelState, row: Response, focus: Focus) {
    if state.take_focus(focus) {
        row.scroll_to_me(Some(Align::Center));
        row.highlight();
    }
}

pub fn count(amount: usize, singular: &str, plural: &str) -> String {
    let noun = if amount == 1 { singular } else { plural };
    format!("{amount} {noun}")
}
