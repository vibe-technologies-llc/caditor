use caditor_document::{
    Document, Emboss, EmbossForm, Feature, FeatureId, RegionChoice, Resolution, SolidResult,
};
use caditor_expression::Dimension;
use caditor_kernel::FaceId;
use egui::{Id, Ui};

use crate::{
    bodies,
    commands::Command,
    editing::EditingCommand,
    emboss_tools,
    feature_fields::{self, Choice, ListChoosing, Picker, Quantity, Rule, Segment},
    feature_tree::count,
    icons,
    model::{Action, Model},
    offset_face_panel,
    reference_picking::Slot,
    reference_rows::{ReferenceRows, RowCache},
    scene,
    selection::{self, Selection},
    solid_tools, widgets,
};

pub const DEPTH: &str = "Depth";
pub const DESCRIPTION: &str = "Raises the closed outlines or text of a sketch from the chosen faces, or \
                               sinks them in, flat or wrapped round a cylinder or cone";
const RAISE_HOVER: &str = "Add the outlines to the faces";
const RECESS_HOVER: &str = "Cut the outlines into the faces";
const PICK_HOVER: &str = "Emboss the outlines of the selected sketch instead";
const MISSING_SKETCH: &str = "A sketch that is no longer there, or none chosen";
const ALL_REGIONS_HOVER: &str = "Emboss every closed outline of the sketch";
const CHOSEN_REGIONS_HOVER: &str = "Emboss only the regions you choose in the view";
const SELECT_ALL_REGIONS: &str = "Select all";
const SELECT_ALL_REGIONS_HOVER: &str =
    "Take every closed outline of the sketch, and any drawn in it later";
const CLEAR_REGIONS: &str = "Clear";
const CLEAR_REGIONS_HOVER: &str = "Choose no region, then click in the view the regions to emboss";
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";

fn face_rows(document: &Document, input: Option<&SolidResult>, emboss: &Emboss) -> ReferenceRows {
    let summary = count(emboss.faces.len(), "face", "faces");
    let rows = match input {
        Some(input) => emboss
            .resolutions(&input.solid)
            .iter()
            .map(|resolution: &Resolution<FaceId>| {
                offset_face_panel::face_row(document, input, resolution)
            })
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); emboss.faces.len()],
    };
    ReferenceRows { summary, rows }
}

pub struct FacesRow<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub feature: &'a Feature,
    pub emboss: &'a Emboss,
    pub opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        emboss,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Faces to emboss");
    let evaluation = model.evaluation();
    let mut previewed = Vec::new();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), emboss)
    });
    ui.vertical(|ui| {
        if emboss.faces.is_empty() {
            ui.label(widgets::muted("None: nothing is embossed", ui));
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
                            emboss.resolutions(&input.solid).get(index),
                        )
                    })
                    .unwrap_or_default();
            }
            if row.removed {
                let mut changed = emboss.clone();
                changed.faces.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    emboss_tools::change(model, id, changed),
                ));
            }
        }
        let chosen = feature_fields::list_in_view(
            ui,
            opened,
            "Click faces in the view to emboss them or leave them out again.",
            "Show the body as it is with this feature so you can click faces",
        );
        match chosen {
            Some(ListChoosing::Start) => {
                if let Some(transaction) = emboss_tools::with_selected_faces(model, id, selection) {
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

fn sketch_row(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        selection,
        feature,
        emboss,
        ..
    } = *row;
    let id = feature.id();
    let document = model.document();
    let named = |sketch: FeatureId| feature_fields::feature_name(document, sketch);
    widgets::caption(ui, "Outlines");
    let chosen = ui
        .horizontal(|ui| {
            let text =
                feature_fields::combo_text(ui, emboss.sketch.and_then(named), MISSING_SKETCH);
            feature_fields::combo(ui, Id::unique(("emboss-sketch", id)), text, || {
                emboss_tools::sketches(model, id)
                    .into_iter()
                    .map(|sketch| Choice {
                        label: named(sketch).unwrap_or("A sketch").to_owned(),
                        selected: emboss.sketch == Some(sketch),
                        change: emboss_tools::change(
                            model,
                            id,
                            Emboss {
                                sketch: Some(sketch),
                                regions: RegionChoice::All,
                                ..emboss.clone()
                            },
                        )
                        .map(Action::Apply),
                    })
                    .collect()
            })
        })
        .inner;
    ui.end_row();
    actions.extend(chosen);
    ui.label("");
    let picker = Picker {
        feature: id,
        slot: Slot::EmbossSketch,
        selected: feature_fields::offered_change(
            ui.ctx(),
            model,
            selection,
            (id, Slot::EmbossSketch),
            || emboss_tools::sketch_change(model, selection, id, emboss),
        ),
        hover: PICK_HOVER,
    };
    ui.vertical(|ui| {
        feature_fields::reference_picker(ui, model, picker, actions);
    });
    ui.end_row();
}

fn regions_row(ui: &mut Ui, row: &FacesRow<'_>, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        emboss,
        opened,
        ..
    } = *row;
    if emboss.sketch.is_none() {
        return;
    }
    let id = feature.id();
    widgets::caption(ui, "Regions");
    let all = matches!(emboss.regions, RegionChoice::All);
    let chosen_count = selection::swept_regions(model.document(), model.evaluation(), id)
        .map_or(0, |(_, regions)| {
            scene::chosen_regions(&emboss.regions, regions).len()
        });
    let mut change = None;
    ui.vertical(|ui| {
        let choices = [
            ("All closed", ALL_REGIONS_HOVER),
            ("Chosen", CHOSEN_REGIONS_HOVER),
        ];
        match widgets::segmented(ui, &choices, usize::from(!all)) {
            Some(0) => {
                change = Some(emboss_tools::change(
                    model,
                    id,
                    emboss_tools::with_regions(emboss, RegionChoice::All),
                ));
            }
            Some(_) => {
                change = Some(emboss_tools::choose_every_region(model, id));
            }
            None => {}
        }
        ui.horizontal(|ui| {
            if !all {
                let chosen = format!("{} chosen", count(chosen_count, "region", "regions"));
                ui.label(widgets::muted(chosen, ui));
                let every = widgets::small_button(
                    ui,
                    icons::command(Command::SelectAllRegions),
                    SELECT_ALL_REGIONS,
                );
                if ui
                    .add(every)
                    .on_hover_text(SELECT_ALL_REGIONS_HOVER)
                    .clicked()
                {
                    actions.push(feature_fields::applied(
                        &feature.name,
                        solid_tools::select_all_regions(model, feature),
                    ));
                }
            }
            if chosen_count > 0 {
                let clear = widgets::small_button(
                    ui,
                    icons::command(Command::ClearChosenRegions),
                    CLEAR_REGIONS,
                );
                if ui.add(clear).on_hover_text(CLEAR_REGIONS_HOVER).clicked() {
                    actions.push(feature_fields::applied(
                        &feature.name,
                        solid_tools::clear_regions(model, feature),
                    ));
                }
            }
        });
        if !all {
            let chosen = feature_fields::list_in_view(
                ui,
                opened,
                "Click regions in the view to include or leave them out.",
                "Show the regions of the sketch so you can click them",
            );
            match chosen {
                Some(ListChoosing::Start) => {
                    actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
                }
                Some(ListChoosing::Stop) => {
                    actions.push(Action::Editing(EditingCommand::CloseSolid));
                }
                None => {}
            }
        }
    });
    ui.end_row();
    if let Some(change) = change {
        actions.push(feature_fields::applied(&feature.name, change));
    }
}

fn depth_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    emboss: &Emboss,
    actions: &mut Vec<Action>,
) {
    let id = feature.id();
    let quantity = Quantity {
        feature: id,
        id: Id::unique(("emboss-depth", id)),
        expression: &emboss.depth,
        dimension: Dimension::LENGTH,
        rule: Rule::AboveZero,
    };
    let drafting = feature_fields::expression_row_drafting(ui, model, DEPTH, quantity, |depth| {
        emboss_tools::change(
            model,
            id,
            Emboss {
                depth,
                ..emboss.clone()
            },
        )
    });
    actions.extend(drafting.into_actions(id));
}

fn form_row(
    ui: &mut Ui,
    model: &Model,
    feature: &Feature,
    emboss: &Emboss,
    actions: &mut Vec<Action>,
) {
    let segment = |label, hover, form: EmbossForm| Segment {
        label,
        hover,
        change: (emboss.form != form).then(|| {
            emboss_tools::change(
                model,
                feature.id(),
                Emboss {
                    form,
                    ..emboss.clone()
                },
            )
        }),
    };
    let segments: Vec<Segment<'_, emboss_tools::EmbossError>> = vec![
        segment("Raise", RAISE_HOVER, EmbossForm::Raised),
        segment("Recess", RECESS_HOVER, EmbossForm::Recessed),
    ];
    actions.extend(feature_fields::segmented_row(
        ui,
        "Form",
        &feature.name,
        segments,
    ));
}

pub fn show(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        emboss,
        ..
    } = *row;
    let id = feature.id();
    widgets::properties(ui, ("emboss-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, row, cache, actions);
        sketch_row(ui, row, actions);
        regions_row(ui, row, actions);
        form_row(ui, model, feature, emboss, actions);
        depth_row(ui, model, feature, emboss, actions);
        feature_fields::feature_row(ui, model.document(), "Body", emboss.body);
    });
}
