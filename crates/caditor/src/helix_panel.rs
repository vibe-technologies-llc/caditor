use caditor_document::{
    CoilSizing, CoilTurning, Feature, FeatureId, Helix, Transaction, capitalized, describe_axis,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    coil_panel::{self, AXIS, DIAMETER, SIZED_BY, TAPER, TURNING},
    feature_fields::{self, AxisRow, Picker, Quantity, Rule, Segment},
    helix_tools::{self, HelixError},
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "A helix curve winding around an axis from the axis's start along \
                               it, for a sweep or pipe to follow";
pub const START_ANGLE: &str = "Start angle";
const PICK_HOVER: &str = "Wind around the selected axis, straight edge, round face or sketch line";

fn sizing_hover(sizing: CoilSizing) -> &'static str {
    match sizing {
        CoilSizing::TurnsAndPitch => "Type how many turns and how far apart they are",
        CoilSizing::HeightAndTurns => "Type how high the helix rises and how many turns it makes",
        CoilSizing::HeightAndPitch => {
            "Type how high the helix rises and how far apart its turns are"
        }
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    helix: &'a Helix,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, helix: Helix) -> Result<Transaction, HelixError> {
        helix_tools::change(self.model, self.id(), helix)
    }

    fn apply(&mut self, helix: Helix) {
        let change = self.change(helix);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn axis_row(&mut self, ui: &mut Ui) {
        let (model, id, helix) = (self.model, self.id(), self.helix);
        let row = AxisRow {
            caption: AXIS,
            shown: Some(capitalized(&describe_axis(model.document(), &helix.axis))),
            picker: Picker {
                feature: id,
                slot: Slot::HelixAxis,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    self.selection,
                    (id, Slot::HelixAxis),
                    || helix_tools::axis_change(model, self.selection, id, helix),
                ),
                hover: PICK_HOVER,
            },
        };
        let listed = || {
            feature_fields::model_axis_choices(model, id, Some(&helix.axis), &|axis| {
                self.change(helix_tools::with_axis(helix, axis))
            })
        };
        let mut actions = Vec::new();
        feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
        self.actions.extend(actions);
        if let Some(reversed) =
            feature_fields::reverse_row(ui, feature_fields::REVERSE_DIRECTION, helix.reversed)
        {
            self.apply(Helix {
                reversed,
                ..helix.clone()
            });
        }
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        (key, expression, dimension, rule): (&'static str, &Expression, Dimension, Rule),
        changed: impl Fn(Expression) -> Helix,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("helix-field", self.id(), key)),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                helix_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn sizing_rows(&mut self, ui: &mut Ui) {
        let current = self.helix.length.sizing();
        let segments = CoilSizing::ALL
            .into_iter()
            .map(|sizing| Segment {
                label: sizing.name(),
                hover: sizing_hover(sizing),
                change: (sizing != current)
                    .then(|| self.change(helix_tools::sized_by(self.model, self.helix, sizing))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SIZED_BY, &self.feature.name, segments);
        self.actions.extend(chosen);
        for (index, (caption, key, expression)) in coil_panel::length_fields(&self.helix.length)
            .into_iter()
            .enumerate()
        {
            let (dimension, rule) = if key == "turns" {
                (Dimension::NONE, Rule::CoilTurns)
            } else {
                (Dimension::LENGTH, Rule::AboveZero)
            };
            let helix = self.helix.clone();
            self.value_row(
                ui,
                caption,
                (key, expression, dimension, rule),
                move |value| Helix {
                    length: coil_panel::with_length_value(&helix.length, index, value),
                    ..helix.clone()
                },
            );
        }
    }

    fn diameter_row(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        let field = (
            "diameter",
            &self.helix.diameter,
            Dimension::LENGTH,
            Rule::AboveZero,
        );
        self.value_row(ui, DIAMETER, field, move |diameter| Helix {
            diameter,
            ..helix.clone()
        });
    }

    fn taper_row(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        let field = (
            "taper",
            &self.helix.taper,
            Dimension::ANGLE,
            Rule::CoilTaper,
        );
        self.value_row(ui, TAPER, field, move |taper| Helix {
            taper,
            ..helix.clone()
        });
    }

    fn start_row(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        let field = ("start", &self.helix.start, Dimension::ANGLE, Rule::Any);
        self.value_row(ui, START_ANGLE, field, move |start| Helix {
            start,
            ..helix.clone()
        });
    }

    fn turning_row(&mut self, ui: &mut Ui) {
        let current = self.helix.turning;
        let segments = CoilTurning::ALL
            .into_iter()
            .map(|turning| Segment {
                label: turning.name(),
                hover: coil_panel::turning_hover(turning),
                change: (turning != current)
                    .then(|| self.change(helix_tools::with_turning(self.helix, turning))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, TURNING, &self.feature.name, segments);
        self.actions.extend(chosen);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    helix: &Helix,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        helix,
        actions,
    };
    widgets::properties(ui, ("helix-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.axis_row(ui);
        panel.sizing_rows(ui);
        panel.diameter_row(ui);
        panel.taper_row(ui);
        panel.start_row(ui);
        panel.turning_row(ui);
    });
}
