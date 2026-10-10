use caditor_document::{Document, PlaneReference, capitalized, describe_plane};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    editing::EditingCommand,
    feature_fields::{self, Choice},
    model::{Action, Model},
    selection::PrincipalPlane,
    sketch_placement::{self, DatumTarget},
    widgets,
};

pub const TITLE: &str = "New sketch";
pub const PLANE: &str = "Plane";
pub const UNCHOSEN: &str = "Choose a plane";
pub const CANCEL: &str = "Cancel";
const DESCRIPTION: &str = "Choose the plane to sketch on here, or click a plane or a flat face \
                           in the view";
const CANCEL_HOVER: &str = "Stop choosing a plane (Esc)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offered {
    Principal(PrincipalPlane),
    Datum(DatumTarget),
}

impl Offered {
    fn reference(self) -> PlaneReference {
        match self {
            Self::Principal(plane) => PlaneReference::Principal(plane),
            Self::Datum(DatumTarget::Plane(datum)) => PlaneReference::Datum(datum),
            Self::Datum(DatumTarget::Frame { frame, plane }) => {
                PlaneReference::Frame { frame, plane }
            }
        }
    }

    fn command(self, model: &Model) -> Result<Action, String> {
        let command = match self {
            Self::Principal(plane) => EditingCommand::NewSketch(Some(plane)),
            Self::Datum(datum) => {
                sketch_placement::new_sketch_on_datum(model, datum)?;
                EditingCommand::NewSketchOnDatum(datum)
            }
        };
        Ok(Action::Editing(command))
    }
}

fn offered(document: &Document) -> Vec<Offered> {
    let datums = document.active_features().flat_map(|feature| {
        let id = feature.id();
        let frame = feature
            .kind
            .datum()
            .is_some_and(caditor_document::Datum::is_frame);
        let planes: Vec<Offered> = if datum_tools::is_plane(document, id) {
            vec![Offered::Datum(DatumTarget::Plane(id))]
        } else if frame {
            PrincipalPlane::ALL
                .into_iter()
                .map(|plane| Offered::Datum(DatumTarget::Frame { frame: id, plane }))
                .collect()
        } else {
            Vec::new()
        };
        planes
    });
    PrincipalPlane::ALL
        .into_iter()
        .map(Offered::Principal)
        .chain(datums)
        .collect()
}

pub fn show(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let document = model.document();
    widgets::card(ui, |ui| {
        ui.label(widgets::section_title(TITLE));
        widgets::properties(ui, "new-sketch-properties", |ui| {
            feature_fields::description_row(ui, DESCRIPTION);
            widgets::caption(ui, PLANE);
            let chosen = feature_fields::combo(ui, Id::new("new-sketch-plane"), UNCHOSEN, || {
                offered(document)
                    .into_iter()
                    .map(|offer| Choice {
                        label: capitalized(&describe_plane(document, &offer.reference())),
                        selected: false,
                        change: offer.command(model),
                    })
                    .collect()
            });
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
