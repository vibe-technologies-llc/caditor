use caditor_document::{AxisReference, Feature, MeasuredItem, Measurement, PrincipalAxis};
use egui::{Id, Label, Ui};

use crate::{
    feature_fields::{self, Choice, Picker},
    measurement_tools::{self, MeasuredPart, Quantity},
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "A reading of the model taken where it stands in the tree and kept \
                               up to date on every recompute; features below it can use its value \
                               by name";
pub const QUANTITY: &str = "Reads";
const NO_READING: &str = "No reading now";
const NOT_NAMED: &str = "Not named";
const ITEM_HOVER: &str = "Measure the selected item instead";
const AXIS_HOVER: &str = "Measure along the selected axis, edge or sketch line instead";

fn axis_label(axis: PrincipalAxis) -> &'static str {
    match axis {
        PrincipalAxis::X => "X axis",
        PrincipalAxis::Y => "Y axis",
        PrincipalAxis::Z => "Z axis",
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    measurement: &'a Measurement,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn quantity_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, QUANTITY);
        let current = Quantity::of(&self.measurement.reading);
        let (model, id, measurement) = (self.model, self.feature.id(), self.measurement);
        let chosen = feature_fields::combo(
            ui,
            Id::new(("measurement-quantity", id)),
            current.label(),
            || {
                Quantity::offered(&measurement.reading)
                    .into_iter()
                    .map(|quantity| Choice {
                        label: quantity.label().to_owned(),
                        selected: quantity == current,
                        change: measurement_tools::quantity_change(
                            model,
                            id,
                            measurement,
                            quantity,
                        )
                        .map(Action::Apply),
                    })
                    .collect()
            },
        );
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn item_row(&mut self, ui: &mut Ui, part: MeasuredPart) {
        let reading = &self.measurement.reading;
        let Some(item) = part.item(reading) else {
            return;
        };
        widgets::caption(ui, part.caption(reading));
        let (model, selection, id, measurement) = (
            self.model,
            self.selection,
            self.feature.id(),
            self.measurement,
        );
        let slot = Slot::MeasuredItem(part);
        let picker = Picker {
            feature: id,
            slot,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                selection,
                (id, slot),
                || measurement_tools::item_change(model, selection, id, measurement, part),
            ),
            hover: match part {
                MeasuredPart::Axis => AXIS_HOVER,
                MeasuredPart::First | MeasuredPart::Second => ITEM_HOVER,
            },
        };
        let mut picked = Vec::new();
        ui.vertical(|ui| {
            if part == MeasuredPart::Axis {
                picked.extend(self.axis_combo(ui, item));
            } else {
                ui.add(Label::new(item.describe(model.document())).wrap());
            }
            feature_fields::reference_picker(ui, model, picker, &mut picked);
        });
        self.actions.extend(picked);
        ui.end_row();
    }

    fn axis_combo(&self, ui: &mut Ui, item: &MeasuredItem) -> Option<Action> {
        let (model, id, measurement) = (self.model, self.feature.id(), self.measurement);
        let current = match item {
            MeasuredItem::Axis(AxisReference::Principal(axis)) => Some(*axis),
            _ => None,
        };
        let shown = current.map_or_else(
            || item.describe(model.document()),
            |axis| axis_label(axis).to_owned(),
        );
        feature_fields::combo(ui, Id::new(("measurement-axis", id)), shown, || {
            PrincipalAxis::ALL
                .into_iter()
                .map(|axis| Choice {
                    label: axis_label(axis).to_owned(),
                    selected: current == Some(axis),
                    change: measurement_tools::reading_with(
                        model,
                        id,
                        measurement,
                        MeasuredPart::Axis,
                        MeasuredItem::Axis(AxisReference::Principal(axis)),
                    )
                    .map(Action::Apply),
                })
                .collect()
        })
    }

    fn reading_rows(&self, ui: &mut Ui) {
        let document = self.model.document();
        widgets::property(ui, measurement_tools::READING, |ui| {
            match measurement_tools::current_reading(self.model, self.feature.id()) {
                Some(result) => {
                    ui.label(measurement_tools::reading_text(self.model.units(), result));
                }
                None => {
                    ui.label(widgets::muted(NO_READING, ui));
                }
            }
        });
        widgets::property(ui, "Named", |ui| {
            match self
                .measurement
                .parameter
                .and_then(|parameter| document.parameter_name(parameter))
            {
                Some(name) => {
                    ui.label(name);
                }
                None => {
                    ui.label(widgets::muted(NOT_NAMED, ui));
                }
            }
        });
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    measurement: &Measurement,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        measurement,
        actions,
    };
    widgets::properties(ui, ("measurement-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.quantity_row(ui);
        for part in MeasuredPart::of(&measurement.reading) {
            panel.item_row(ui, part);
        }
        panel.reading_rows(ui);
    });
}
