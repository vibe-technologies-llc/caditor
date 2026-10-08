use caditor_document::{
    BodyPlacement, Document, Edit, Feature, FeatureId, FeatureKind, Import, MoveAxis, Transaction,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Quantity, Rule},
    field,
    model::{Action, Model},
    widgets,
};

pub const DESCRIPTION: &str =
    "Places the imported body: turned about the axes through the origin, then shifted";

pub fn placed(document: &Document, feature: FeatureId, import: Import) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Place {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Import(import),
        },
    ))
}

fn change(model: &Model, feature: FeatureId, import: Import) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = placed(document, feature, import)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

struct Panel<'a> {
    model: &'a Model,
    feature: &'a Feature,
    import: &'a Import,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: (&str, usize),
        (expression, dimension): (&Expression, Dimension),
        rebuild: impl Fn(&mut BodyPlacement, Expression),
    ) {
        let id = self.feature.id();
        let quantity = Quantity {
            id: Id::new(("import-field", salt, id)),
            expression,
            dimension,
            rule: Rule::Any,
        };
        let model = self.model;
        let import = self.import;
        let committed = feature_fields::expression_row(ui, model, caption, quantity, |value| {
            let mut changed = import.clone();
            rebuild(&mut changed.placement, value);
            change(model, id, changed)
        });
        self.actions.extend(committed.map(Action::Apply));
    }
}

pub fn placement(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    import: &Import,
) {
    let mut panel = Panel {
        model,
        feature,
        import,
        actions,
    };
    widgets::properties(ui, ("import-placement", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        for axis in MoveAxis::ALL {
            panel.row(
                ui,
                &format!("Turn about {}", axis.name()),
                ("turn", axis.index()),
                (axis.of(&import.placement.turn), Dimension::ANGLE),
                |placement, value| *axis.of_mut(&mut placement.turn) = value,
            );
        }
        for axis in MoveAxis::ALL {
            panel.row(
                ui,
                &format!("Move along {}", axis.name()),
                ("offset", axis.index()),
                (axis.of(&import.placement.offset), Dimension::LENGTH),
                |placement, value| *axis.of_mut(&mut placement.offset) = value,
            );
        }
    });
}
