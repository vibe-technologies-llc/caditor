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

pub const DESCRIPTION: &str = "Places the imported body: turned about the axes through the origin, then shifted, and \
     resized about its file's origin by the scale, such as 25.4 for a part drawn in inches";
pub const FRAME_DESCRIPTION: &str = "Places the imported body in the coordinate system: its file's origin and axes are the \
     system's, then it is turned about those axes and shifted along them";
pub const PLACED_IN: &str = "Placed in";
pub const SCALE: &str = "Scale";

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
        (expression, dimension, rule): (&Expression, Dimension, Rule),
        rebuild: impl Fn(&mut BodyPlacement, Expression),
    ) {
        let id = self.feature.id();
        let quantity = Quantity {
            feature: id,
            id: Id::new(("import-field", salt, id)),
            expression,
            dimension,
            rule,
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
        let description = match import.placement.frame {
            Some(_) => FRAME_DESCRIPTION,
            None => DESCRIPTION,
        };
        feature_fields::description_row(ui, description);
        let row = feature_fields::FrameRow {
            feature: feature.id(),
            salt: "import-frame",
            caption: PLACED_IN,
            current: import.placement.frame,
        };
        let chosen = feature_fields::frame_row(ui, model.document(), &row, |frame| {
            let mut changed = import.clone();
            changed.placement.frame = frame;
            change(model, feature.id(), changed)
        });
        panel.actions.extend(chosen);
        for axis in MoveAxis::ALL {
            panel.row(
                ui,
                &format!("Turn about {}", axis.name()),
                ("turn", axis.index()),
                (axis.of(&import.placement.turn), Dimension::ANGLE, Rule::Any),
                |placement, value| *axis.of_mut(&mut placement.turn) = value,
            );
        }
        for axis in MoveAxis::ALL {
            panel.row(
                ui,
                &format!("Move along {}", axis.name()),
                ("offset", axis.index()),
                (
                    axis.of(&import.placement.offset),
                    Dimension::LENGTH,
                    Rule::Any,
                ),
                |placement, value| *axis.of_mut(&mut placement.offset) = value,
            );
        }
        panel.row(
            ui,
            SCALE,
            ("scale", 0),
            (&import.placement.scale, Dimension::NONE, Rule::AboveZero),
            |placement, value| placement.scale = value,
        );
    });
}
