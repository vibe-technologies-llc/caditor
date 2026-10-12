use caditor_document::PlaneReference;
use egui::{Id, Ui};

use crate::{
    datum_tools,
    editing::EditingCommand,
    feature_fields,
    model::{Action, Model},
    plane_selector,
    sketch_placement::{self, DatumTarget},
    widgets,
};

pub const TITLE: &str = "New sketch";
pub const PLANE: &str = "Plane";
pub const CANCEL: &str = "Cancel";
const DESCRIPTION: &str = "Choose the plane to sketch on here, or click a plane or a flat face \
                           in the view";
const CANCEL_HOVER: &str = "Stop choosing a plane (Esc)";
const FACES_IN_THE_VIEW: &str = "Click a flat face in the view to sketch on it";

fn command(model: &Model, plane: &PlaneReference) -> Result<Action, String> {
    let command = match plane {
        PlaneReference::Principal(plane) => EditingCommand::NewSketch(Some(*plane)),
        PlaneReference::Datum(datum) => on_datum(model, DatumTarget::Plane(*datum))?,
        PlaneReference::Frame { frame, plane } => on_datum(
            model,
            DatumTarget::Frame {
                frame: *frame,
                plane: *plane,
            },
        )?,
        PlaneReference::Face(_) => return Err(FACES_IN_THE_VIEW.to_owned()),
    };
    Ok(Action::Editing(command))
}

fn on_datum(model: &Model, target: DatumTarget) -> Result<EditingCommand, String> {
    sketch_placement::new_sketch_on_datum(model, target)?;
    Ok(EditingCommand::NewSketchOnDatum(target))
}

pub fn show(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let document = model.document();
    widgets::card(ui, |ui| {
        ui.label(widgets::section_title(TITLE));
        widgets::properties(ui, "new-sketch-properties", |ui| {
            feature_fields::description_row(ui, DESCRIPTION);
            widgets::caption(ui, PLANE);
            let chosen = plane_selector::combo(
                ui,
                Id::new("new-sketch-plane"),
                document,
                datum_tools::listed_planes_at_the_bar(document),
                None,
                |plane| command(model, plane),
            );
            ui.end_row();
            actions.extend(chosen);
        });
        if ui
            .add(widgets::button(CANCEL))
            .on_hover_text(CANCEL_HOVER)
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        }
    });
}
