use caditor_document::{Document, Edit, FeatureId, FeatureKind, Scale, Transaction};
use caditor_expression::Expression;

use crate::{
    editing::{self, EditingCommand},
    model::{Action, Model},
    move_tools,
    selection::Selection,
    units::LengthUnit,
};

pub const TITLE: &str = "Scale body";
pub const DESCRIPTION: &str =
    "Resize a body by a factor about a centre point, such as 25.4 for a part drawn in inches";
const NO_BODY: &str = "Select a face or edge of the body to scale";
const DEFAULT_FACTOR: f64 = 1.0;

pub fn selected_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<FeatureId, &'static str> {
    move_tools::chosen_body(model, selection, tree, NO_BODY)
}

pub fn create(document: &Document, body: FeatureId, unit: LengthUnit) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Scale(Scale {
            body,
            factor: Expression::Number(DEFAULT_FACTOR),
            center: std::array::from_fn(|_| unit.default_length(0.0)),
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, body: FeatureId) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), body, model.length_unit());
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, scale: Scale) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Scale(scale),
        },
    ))
}
