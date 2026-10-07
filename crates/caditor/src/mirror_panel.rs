use caditor_document::{
    Feature, FeatureId, Mirror, PlaneReference, PrincipalPlane, capitalized, describe_plane,
};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, Picker},
    mirror_tools,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Reflects the body across the plane, joined to the original or in \
                               its place";
pub const KEEP_ORIGINAL: &str = "Keep the original";
const PICK_HOVER: &str = "Mirror across the selected plane or flat face instead";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    mirror: &'a Mirror,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, mirror: Mirror) -> Result<Action, String> {
        mirror_tools::change(self.model, self.id(), mirror).map(Action::Apply)
    }

    fn plane_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let current = capitalized(&describe_plane(document, &self.mirror.plane));
        widgets::caption(ui, "Mirror across");
        let chosen =
            feature_fields::combo(ui, Id::new(("mirror-plane", self.id())), current, || {
                PrincipalPlane::ALL
                    .into_iter()
                    .map(|plane| {
                        let reference = PlaneReference::Principal(plane);
                        Choice {
                            label: capitalized(&describe_plane(document, &reference)),
                            selected: self.mirror.plane == reference,
                            change: self.change(Mirror {
                                plane: reference,
                                ..self.mirror.clone()
                            }),
                        }
                    })
                    .collect()
            });
        ui.end_row();
        self.actions.extend(chosen);
        ui.label("");
        let picker = Picker {
            feature: self.id(),
            slot: Slot::MirrorPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::MirrorPlane),
                || mirror_tools::plane_change(self.model, self.selection, self.id(), self.mirror),
            ),
            hover: PICK_HOVER,
        };
        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn keep_row(&mut self, ui: &mut Ui) {
        if let Some(keep_original) =
            feature_fields::reverse_row(ui, KEEP_ORIGINAL, self.mirror.keep_original)
        {
            let change = mirror_tools::change(
                self.model,
                self.id(),
                Mirror {
                    keep_original,
                    ..self.mirror.clone()
                },
            );
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    mirror: &Mirror,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        mirror,
        actions,
    };
    widgets::properties(ui, ("mirror-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.plane_rows(ui);
        panel.keep_row(ui);
        feature_fields::feature_row(ui, model.document(), "Body", mirror.body);
    });
}
