use caditor_document::{
    Coil, CoilLength, CoilSizing, CoilTurning, Feature, FeatureId, Transaction, capitalized,
    coil_placement_name, coil_shape_name, coil_size_name, describe_axis,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::{CoilPlacement, CoilShape};
use egui::{Id, Ui};

use crate::{
    coil_tools,
    feature_fields::{self, AxisRow, OperationRows, Picker, Quantity, Rule, Segment},
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Winds a round or square section around an axis, from the axis's \
                               start along it, as a spring or coil";
pub const AXIS: &str = "Axis";
pub const SIZED_BY: &str = "Sized by";
pub const HEIGHT: &str = "Height";
pub const PITCH: &str = "Pitch";
pub const TURNS: &str = "Turns";
pub const DIAMETER: &str = "Diameter";
pub const SECTION: &str = "Section";
pub const POSITION: &str = "Position";
pub const TAPER: &str = "Taper";
pub const TURNING: &str = "Turning";
const PICK_HOVER: &str = "Wind around the selected axis, straight edge, round face or sketch line";

fn sizing_hover(sizing: CoilSizing) -> &'static str {
    match sizing {
        CoilSizing::TurnsAndPitch => "Type how many turns and how far apart they are",
        CoilSizing::HeightAndTurns => "Type how high the coil rises and how many turns it makes",
        CoilSizing::HeightAndPitch => {
            "Type how high the coil rises and how far apart its turns are"
        }
    }
}

fn shape_hover(shape: CoilShape) -> &'static str {
    match shape {
        CoilShape::Round => "A round wire of a diameter",
        CoilShape::Square => "A square section of a width, its sides along and across the axis",
    }
}

fn placement_hover(placement: CoilPlacement) -> &'static str {
    match placement {
        CoilPlacement::Inside => "The section lies inside the diameter, its outside on it",
        CoilPlacement::Centred => "The section's middle runs on the diameter",
        CoilPlacement::Outside => "The section lies outside the diameter, its inside on it",
    }
}

fn turning_hover(turning: CoilTurning) -> &'static str {
    match turning {
        CoilTurning::CounterClockwise => {
            "Turns counter-clockwise as it rises, seen from its end looking back along the axis \
             (right-handed)"
        }
        CoilTurning::Clockwise => {
            "Turns clockwise as it rises, seen from its end looking back along the axis \
             (left-handed)"
        }
    }
}

pub fn length_fields(length: &CoilLength) -> [(&'static str, &'static str, &Expression); 2] {
    match length {
        CoilLength::TurnsAndPitch { turns, pitch } => {
            [(TURNS, "turns", turns), (PITCH, "pitch", pitch)]
        }
        CoilLength::HeightAndTurns { height, turns } => {
            [(HEIGHT, "height", height), (TURNS, "turns", turns)]
        }
        CoilLength::HeightAndPitch { height, pitch } => {
            [(HEIGHT, "height", height), (PITCH, "pitch", pitch)]
        }
    }
}

fn with_length_value(length: &CoilLength, index: usize, value: Expression) -> CoilLength {
    let mut changed = length.clone();
    if let Some(slot) = changed.expressions_mut().into_iter().nth(index) {
        *slot = value;
    }
    changed
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    coil: &'a Coil,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, coil: Coil) -> Result<Transaction, String> {
        coil_tools::change(self.model, self.id(), coil)
    }

    fn apply(&mut self, coil: Coil) {
        let change = self.change(coil);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn axis_row(&mut self, ui: &mut Ui) {
        let (model, id, coil) = (self.model, self.id(), self.coil);
        let row = AxisRow {
            caption: AXIS,
            shown: Some(capitalized(&describe_axis(model.document(), &coil.axis))),
            picker: Picker {
                feature: id,
                slot: Slot::CoilAxis,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    self.selection,
                    (id, Slot::CoilAxis),
                    || coil_tools::axis_change(model, self.selection, id, coil),
                ),
                hover: PICK_HOVER,
            },
        };
        let listed = || {
            feature_fields::model_axis_choices(model, id, Some(&coil.axis), &|axis| {
                self.change(coil_tools::with_axis(coil, axis))
            })
        };
        let mut actions = Vec::new();
        feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
        self.actions.extend(actions);
        if let Some(reversed) =
            feature_fields::reverse_row(ui, feature_fields::REVERSE_DIRECTION, coil.reversed)
        {
            self.apply(Coil {
                reversed,
                ..coil.clone()
            });
        }
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        (key, expression, dimension, rule): (&'static str, &Expression, Dimension, Rule),
        changed: impl Fn(Expression) -> Coil,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("coil-field", self.id(), key)),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                coil_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn sizing_rows(&mut self, ui: &mut Ui) {
        let current = self.coil.length.sizing();
        let segments = CoilSizing::ALL
            .into_iter()
            .map(|sizing| Segment {
                label: sizing.name(),
                hover: sizing_hover(sizing),
                change: (sizing != current)
                    .then(|| self.change(coil_tools::sized_by(self.model, self.coil, sizing))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SIZED_BY, &self.feature.name, segments);
        self.actions.extend(chosen);
        for (index, (caption, key, expression)) in
            length_fields(&self.coil.length).into_iter().enumerate()
        {
            let (dimension, rule) = if key == "turns" {
                (Dimension::NONE, Rule::CoilTurns)
            } else {
                (Dimension::LENGTH, Rule::AboveZero)
            };
            let coil = self.coil.clone();
            self.value_row(
                ui,
                caption,
                (key, expression, dimension, rule),
                move |value| Coil {
                    length: with_length_value(&coil.length, index, value),
                    ..coil.clone()
                },
            );
        }
    }

    fn diameter_row(&mut self, ui: &mut Ui) {
        let coil = self.coil.clone();
        let field = (
            "diameter",
            &self.coil.diameter,
            Dimension::LENGTH,
            Rule::AboveZero,
        );
        self.value_row(ui, DIAMETER, field, move |diameter| Coil {
            diameter,
            ..coil.clone()
        });
    }

    fn section_rows(&mut self, ui: &mut Ui) {
        let current = self.coil.shape;
        let segments = CoilShape::ALL
            .into_iter()
            .map(|shape| Segment {
                label: coil_shape_name(shape),
                hover: shape_hover(shape),
                change: (shape != current)
                    .then(|| self.change(coil_tools::with_shape(self.coil, shape))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SECTION, &self.feature.name, segments);
        self.actions.extend(chosen);
        let caption = capitalized(coil_size_name(self.coil.shape));
        let coil = self.coil.clone();
        let field = ("size", &self.coil.size, Dimension::LENGTH, Rule::AboveZero);
        self.value_row(ui, &caption, field, move |size| Coil {
            size,
            ..coil.clone()
        });
        let current = self.coil.placement;
        let segments = CoilPlacement::ALL
            .into_iter()
            .map(|placement| Segment {
                label: coil_placement_name(placement),
                hover: placement_hover(placement),
                change: (placement != current)
                    .then(|| self.change(coil_tools::with_placement(self.coil, placement))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, POSITION, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn taper_row(&mut self, ui: &mut Ui) {
        let coil = self.coil.clone();
        let field = ("taper", &self.coil.taper, Dimension::ANGLE, Rule::CoilTaper);
        self.value_row(ui, TAPER, field, move |taper| Coil {
            taper,
            ..coil.clone()
        });
    }

    fn turning_row(&mut self, ui: &mut Ui) {
        let current = self.coil.turning;
        let segments = CoilTurning::ALL
            .into_iter()
            .map(|turning| Segment {
                label: turning.name(),
                hover: turning_hover(turning),
                change: (turning != current)
                    .then(|| self.change(coil_tools::with_turning(self.coil, turning))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, TURNING, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let rows = OperationRows {
            feature: self.id(),
            operation: self.coil.operation,
            ids: ("coil-operation", "coil-body"),
        };
        let chosen = feature_fields::body_operation_rows(ui, self.model, &rows, |operation| {
            self.change(coil_tools::with_operation(self.coil, operation))
        });
        self.actions.extend(chosen);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    coil: &Coil,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        coil,
        actions,
    };
    widgets::properties(ui, ("coil-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.axis_row(ui);
        panel.sizing_rows(ui);
        panel.diameter_row(ui);
        panel.section_rows(ui);
        panel.taper_row(ui);
        panel.turning_row(ui);
        panel.operation_rows(ui);
    });
}
