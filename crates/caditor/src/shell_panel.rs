use caditor_document::{
    Document, Feature, FeatureId, Resolution, Shell, ShellSide, SolidResult, Transaction,
};
use caditor_expression::Dimension;
use caditor_kernel::FaceId;
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_fields::{self, ListChoosing, Quantity, Rule},
    feature_tree::count,
    field, icons,
    model::{Action, Model},
    reference_picking::{Picking, Slot},
    reference_rows::{ReferenceRows, RowCache},
    selection::Selection,
    shell_tools, widgets,
};

pub const DESCRIPTION: &str = "Hollows the body, opening the chosen faces";
pub const WALLS: &str = "Walls";
pub const OWN_THICKNESS: &str = "Own thickness";
pub const ADD_OWN_THICKNESS: &str = "Choose faces";
const ADD_OWN_THICKNESS_HOVER: &str =
    "Click faces in the view to give each its own thickness, or the shell's again";
const NO_OWN_THICKNESS: &str = "None: every wall takes the thickness";
const SHELLS_THICKNESS: &str = "Give this face the shell's thickness";

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
    let quantity = Quantity {
        feature,
        id: Id::new(("shell-thickness", feature)),
        expression: &shell.thickness,
        dimension: Dimension::LENGTH,
        rule: Rule::AboveZero,
    };
    let drafting =
        feature_fields::expression_row_drafting(ui, model, "Thickness", quantity, |thickness| {
            change(
                model,
                feature,
                Shell {
                    thickness,
                    ..shell.clone()
                },
            )
        });
    actions.extend(drafting.into_actions(feature));
}

const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const GONE: &str = "A face that is no longer there";

fn face_row(document: &Document, input: &SolidResult, resolution: &Resolution<FaceId>) -> String {
    match resolution {
        Resolution::One(face) => bodies::describe_face_id(document, input, *face),
        Resolution::Pieces(pieces) => match pieces.first().and_then(|face| input.solid.face(*face))
        {
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

fn face_rows(document: &Document, input: Option<&SolidResult>, shell: &Shell) -> ReferenceRows {
    let summary = count(shell.open.len(), "face", "faces");
    let rows = match input {
        Some(input) => shell
            .resolutions(&input.solid)
            .iter()
            .chain(shell.wall_resolutions(&input.solid).iter())
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); shell.open.len() + shell.faces.len()],
    };
    ReferenceRows { summary, rows }
}

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub shell: &'a Shell,
    pub opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        shell,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Open faces");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), shell)
    });
    ui.vertical(|ui| {
        if shell.open.is_empty() {
            ui.label(widgets::muted("None: the body is hollow and closed", ui));
        } else {
            ui.label(&listed.summary);
        }
        for (index, text) in listed.rows.iter().take(shell.open.len()).enumerate() {
            let text = widgets::muted(text, ui);
            let row = widgets::removable_row_hovered(ui, text, "Close this face");
            if row.hovered && opened {
                previewed = bodies::input(evaluation, id)
                    .map(|input| {
                        bodies::face_pickables(
                            id,
                            &input.solid,
                            shell.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = shell.clone();
                changed.open.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    change(model, id, changed),
                ));
            }
        }
        let chosen = feature_fields::list_in_view(
            ui,
            opened,
            "Click faces in the view to open them or close them again.",
            "Show the hollowed body so you can click faces to open them or close them again",
        );
        match chosen {
            Some(ListChoosing::Start) => {
                if let Some(transaction) = shell_tools::with_selected_faces(model, id, selection) {
                    actions.push(Action::Apply(transaction));
                }
                actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
            }
            Some(ListChoosing::Stop) => actions.push(Action::Editing(EditingCommand::CloseSolid)),
            None => {}
        }
    });
    if !previewed.is_empty() {
        cache.preview(previewed);
    }
    ui.end_row();
}

fn side_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    shell: &Shell,
    actions: &mut Vec<Action>,
) {
    let choices: Vec<(&str, &str)> = ShellSide::ALL
        .iter()
        .map(|side| (side.label(), side.explanation()))
        .collect();
    let chosen = ShellSide::ALL
        .iter()
        .position(|side| *side == shell.side)
        .unwrap_or(0);
    let picked = widgets::property(ui, WALLS, |ui| widgets::segmented(ui, &choices, chosen));
    let Some(side) = picked.and_then(|index| ShellSide::ALL.get(index).copied()) else {
        return;
    };
    if side == shell.side {
        return;
    }
    let document = model.document();
    let change = shell_tools::side_change(document, feature.id(), side)
        .ok_or_else(|| "The feature no longer exists".to_owned())
        .and_then(|transaction| field::checked(document, transaction));
    actions.push(feature_fields::applied(&feature.name, change));
}

fn own_thickness_list(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        shell,
        ..
    } = *row;
    let id = feature.id();
    widgets::caption(ui, OWN_THICKNESS);
    ui.vertical(|ui| {
        if shell.faces.is_empty() {
            ui.label(widgets::muted(NO_OWN_THICKNESS, ui));
        }
        let slot = (id, Slot::ShellOwnThickness);
        let stop = feature_fields::STOP_CHOOSING;
        if !feature_fields::choosing_prompt(ui, model, slot, stop, actions) {
            let add = widgets::small_button(ui, icons::ADD, ADD_OWN_THICKNESS);
            if ui.add(add).on_hover_text(ADD_OWN_THICKNESS_HOVER).clicked() {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    id,
                    Slot::ShellOwnThickness,
                ))));
            }
        }
    });
    ui.end_row();
}

fn own_thickness_rows(
    ui: &mut Ui,
    row: &FacesRow<'_>,
    described: &[String],
    cache: &mut RowCache,
    actions: &mut Vec<Action>,
) {
    let FacesRow {
        model,
        feature,
        shell,
        opened,
        ..
    } = *row;
    let id = feature.id();
    let evaluation = model.evaluation();
    for (index, entry) in shell.faces.iter().enumerate() {
        let caption = described.get(index).map_or(GONE, String::as_str);
        let owned_as = format!("Thickness of {caption}");
        let quantity = Quantity {
            feature: id,
            id: Id::new(("shell-own-thickness", id, index)),
            expression: &entry.thickness,
            dimension: Dimension::LENGTH,
            rule: Rule::AboveZero,
        };
        let value = feature_fields::expression_row_removable(
            ui,
            model,
            (caption, &owned_as),
            quantity,
            |thickness| {
                let mut changed = shell.clone();
                if let Some(entry) = changed.faces.get_mut(index) {
                    entry.thickness = thickness;
                }
                change(model, id, changed)
            },
            SHELLS_THICKNESS,
        );
        actions.extend(value.drafting.into_actions(id));
        if value.hovered && opened {
            let previewed = bodies::input(evaluation, id)
                .map(|input| {
                    bodies::face_pickables(
                        id,
                        &input.solid,
                        shell.wall_resolutions(&input.solid).get(index),
                    )
                })
                .unwrap_or_default();
            cache.preview(previewed);
        }
        if value.removed {
            let mut changed = shell.clone();
            if index < changed.faces.len() {
                changed.faces.remove(index);
            }
            actions.push(feature_fields::applied(
                &feature.name,
                change(model, id, changed),
            ));
        }
    }
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        shell,
        ..
    } = *row;
    let id = feature.id();
    let evaluation = model.evaluation();
    let described: Vec<String> = cache
        .rows(id, evaluation.body_before(id), model.revision(), || {
            face_rows(model.document(), bodies::input(evaluation, id), shell)
        })
        .rows
        .iter()
        .skip(shell.open.len())
        .cloned()
        .collect();
    widgets::properties(ui, ("shell-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, row, cache, actions);
        thickness_row(ui, model, id, shell, actions);
        own_thickness_list(ui, row, actions);
        own_thickness_rows(ui, row, &described, cache, actions);
        side_row(ui, model, feature, shell, actions);
        feature_fields::feature_row(ui, model.document(), "Body", shell.body);
    });
}
