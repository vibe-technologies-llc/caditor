use caditor_document::{Feature, FeatureId, Split};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, Picker},
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    split_tools, widgets,
};

pub const DESCRIPTION: &str = "Cuts the body along a plane, a sketch curve swept through it, \
                               the surface of a face carried on past its edges or another body: \
                               the side the plane, curve or face faces, or what lies outside the \
                               other body, stays in the body and the rest becomes a body of its \
                               own";
pub const KEEP_OTHER_SIDE: &str = "Keep the other side";
const PICK_HOVER: &str = "Split along the selected plane, face, sketch curve or other body instead";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    split: &'a Split,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, split: Split) -> Result<Action, String> {
        split_tools::change(self.model, self.id(), split).map(Action::Apply)
    }

    fn along_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let current = split_tools::describe(document, &self.split.along);
        widgets::caption(ui, "Split along");
        let chosen =
            feature_fields::combo(ui, Id::new(("split-plane", self.id())), current, || {
                split_tools::choices(self.model, self.id(), self.split)
                    .into_iter()
                    .map(|along| Choice {
                        label: split_tools::describe(document, &along),
                        selected: self.split.along == along,
                        change: self.change(Split {
                            along,
                            ..self.split.clone()
                        }),
                    })
                    .collect()
            });
        ui.end_row();
        self.actions.extend(chosen);
        ui.label("");
        let picker = Picker {
            feature: self.id(),
            slot: Slot::SplitPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::SplitPlane),
                || split_tools::along_change(self.model, self.selection, self.id(), self.split),
            ),
            hover: PICK_HOVER,
        };
        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn side_row(&mut self, ui: &mut Ui) {
        if let Some(flipped) = feature_fields::reverse_row(ui, KEEP_OTHER_SIDE, self.split.flipped)
        {
            let change = split_tools::change(
                self.model,
                self.id(),
                Split {
                    flipped,
                    ..self.split.clone()
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
    split: &Split,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        split,
        actions,
    };
    widgets::properties(ui, ("split-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.along_rows(ui);
        panel.side_row(ui);
        feature_fields::feature_row(ui, model.document(), "Body", split.body);
        feature_fields::feature_row(ui, model.document(), "Split-off body", feature.id());
    });
}
