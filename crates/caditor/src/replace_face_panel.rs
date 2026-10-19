use caditor_document::{Document, Feature, ReplaceFace, SolidResult};
use egui::{Id, Ui};

use crate::{
    bodies, datum_tools,
    editing::EditingCommand,
    feature_fields::{self, ListChoosing, Picker},
    feature_tree::count,
    model::{Action, Model},
    offset_face_panel::face_row,
    plane_selector,
    reference_picking::Slot,
    reference_rows::{ReferenceRows, RowCache},
    replace_face_tools,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Replaces the chosen flat faces with a plane or another flat face's \
                               plane, the faces around them extending or trimming to meet it";
pub const TARGET: &str = "Replace with";
const PICK_HOVER: &str = "Replace the faces with the selected plane or flat face instead";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";

fn face_rows(
    document: &Document,
    input: Option<&SolidResult>,
    replace: &ReplaceFace,
) -> ReferenceRows {
    let summary = count(replace.faces.len(), "face", "faces");
    let rows = match input {
        Some(input) => replace
            .resolutions(&input.solid)
            .iter()
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); replace.faces.len()],
    };
    ReferenceRows { summary, rows }
}

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub replace: &'a ReplaceFace,
    pub opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        replace,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Faces to replace");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), replace)
    });
    ui.vertical(|ui| {
        if replace.faces.is_empty() {
            ui.label(widgets::muted("None: nothing is replaced", ui));
        } else {
            ui.label(&listed.summary);
        }
        for (index, text) in listed.rows.iter().enumerate() {
            let text = widgets::muted(text, ui);
            let row = widgets::removable_row_hovered(ui, text, "Leave this face out");
            if row.hovered && opened {
                previewed = bodies::input(evaluation, id)
                    .map(|input| {
                        bodies::face_pickables(
                            id,
                            &input.solid,
                            replace.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = replace.clone();
                changed.faces.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    replace_face_tools::change(model, id, changed),
                ));
            }
        }
        let chosen = feature_fields::list_in_view(
            ui,
            opened,
            "Click faces in the view to replace them or leave them out again.",
            "Show the body as it is with this feature so you can click faces",
        );
        match chosen {
            Some(ListChoosing::Start) => {
                if let Some(transaction) =
                    replace_face_tools::with_selected_faces(model, id, selection)
                {
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

fn target_row(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        replace,
        ..
    } = *row;
    let id = feature.id();
    let picker = Picker {
        feature: id,
        slot: Slot::ReplaceTarget,
        selected: feature_fields::offered_change(
            ui.ctx(),
            model,
            selection,
            (id, Slot::ReplaceTarget),
            || replace_face_tools::plane_change(model, selection, id, replace),
        ),
        hover: PICK_HOVER,
    };
    let target = plane_selector::Row {
        caption: TARGET,
        id: Id::unique(("replace-target", id)),
        planes: datum_tools::listed_planes(model.document(), id),
        current: replace.target.as_ref(),
        picker,
        remove: None,
    };
    plane_selector::row(
        ui,
        model,
        target,
        |plane| replace_face_tools::target_change(model, id, replace, plane.clone()),
        actions,
    );
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        replace,
        ..
    } = *row;
    widgets::properties(ui, ("replace-face-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, row, cache, actions);
        target_row(ui, row, actions);
        feature_fields::feature_row(ui, model.document(), "Body", replace.body);
    });
}
