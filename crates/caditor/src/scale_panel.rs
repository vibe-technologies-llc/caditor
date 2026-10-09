use caditor_document::{Feature, FeatureId, MoveAxis, Scale, Transaction};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Picker, Quantity, Rule},
    field, icons,
    model::{Action, Model},
    reference_picking::Slot,
    scale_tools,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Resizes the body by the factor, keeping the centre in place";
pub const CENTRE_IN: &str = "Centre in";
pub const CENTRE_AT: &str = "Centre at";
pub const BODY_CENTRE: &str = "Body centre";
const CENTRE_HOVER: &str = "Scale about the selected corner, round edge, sphere, sketch point or \
                            datum point";
const BODY_CENTRE_HOVER: &str =
    "Scale about the middle of the body's box as it stands before this scale";
pub const FRAME_DESCRIPTION: &str =
    "The centre is measured from the origin of the coordinate system, along its X, Y and Z axes";

fn change(model: &Model, feature: FeatureId, scale: Scale) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = scale_tools::edit(document, feature, scale)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: (&str, usize),
        (expression, dimension, rule): (&Expression, Dimension, Rule),
        rebuild: impl Fn(Expression) -> Scale,
    ) {
        let id = self.feature.id();
        let quantity = Quantity {
            feature: id,
            id: Id::new(("scale-field", salt, id)),
            expression,
            dimension,
            rule,
        };
        let model = self.model;
        let drafting =
            feature_fields::expression_row_drafting(ui, model, caption, quantity, |value| {
                change(model, id, rebuild(value))
            });
        self.actions.extend(drafting.into_actions(id));
    }

    fn centre_at_row(&mut self, ui: &mut Ui, scale: &Scale) {
        let (model, selection, id) = (self.model, self.selection, self.feature.id());
        let picker = Picker {
            feature: id,
            slot: Slot::ScaleCentre,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                selection,
                (id, Slot::ScaleCentre),
                || scale_tools::centre_change(model, selection, id, scale),
            ),
            hover: CENTRE_HOVER,
        };
        widgets::caption(ui, CENTRE_AT);
        ui.horizontal_wrapped(|ui| {
            feature_fields::reference_picker(ui, model, picker, self.actions);
            let button = widgets::small_button(ui, icons::BODY_CENTRE, BODY_CENTRE);
            if ui.add(button).on_hover_text(BODY_CENTRE_HOVER).clicked() {
                self.actions.push(feature_fields::applied(
                    &self.feature.name,
                    scale_tools::body_centre_change(model, id, scale),
                ));
            }
        });
        ui.end_row();
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    scale: &Scale,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        actions,
    };
    widgets::properties(ui, ("scale-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.row(
            ui,
            "Factor",
            ("factor", 0),
            (&scale.factor, Dimension::NONE, Rule::AboveZero),
            |factor| Scale {
                factor,
                ..scale.clone()
            },
        );
        let row = feature_fields::FrameRow {
            feature: feature.id(),
            salt: "scale-frame",
            caption: CENTRE_IN,
            current: scale.frame,
        };
        let chosen = feature_fields::frame_row(ui, model.document(), &row, |frame| {
            change(
                model,
                feature.id(),
                Scale {
                    frame,
                    ..scale.clone()
                },
            )
        });
        panel.actions.extend(chosen);
        if scale.frame.is_some() {
            feature_fields::description_row(ui, FRAME_DESCRIPTION);
        }
        for axis in MoveAxis::ALL {
            panel.row(
                ui,
                &format!("Centre {}", axis.name()),
                ("center", axis.index()),
                (axis.of(&scale.center), Dimension::LENGTH, Rule::Any),
                |value| {
                    let mut changed = scale.clone();
                    *axis.of_mut(&mut changed.center) = value;
                    changed
                },
            );
        }
        panel.centre_at_row(ui, scale);
        feature_fields::feature_row(ui, model.document(), "Body", scale.body);
    });
}
