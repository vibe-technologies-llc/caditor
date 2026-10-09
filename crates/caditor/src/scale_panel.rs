use caditor_document::{Feature, FeatureId, MoveAxis, Scale, Transaction};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Quantity, Rule},
    field,
    model::{Action, Model},
    scale_tools, widgets,
};

pub const DESCRIPTION: &str = "Resizes the body by the factor, keeping the centre in place";

fn change(model: &Model, feature: FeatureId, scale: Scale) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = scale_tools::edit(document, feature, scale)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

struct Panel<'a> {
    model: &'a Model,
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
        let committed = feature_fields::expression_row(ui, model, caption, quantity, |value| {
            change(model, id, rebuild(value))
        });
        self.actions.extend(committed.map(Action::Apply));
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    scale: &Scale,
) {
    let mut panel = Panel {
        model,
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
        feature_fields::feature_row(ui, model.document(), "Body", scale.body);
    });
}
