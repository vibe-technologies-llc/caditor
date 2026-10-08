use caditor_document::{Document, Edit, FeatureId, FeatureKind, Move, Transaction};

use crate::{
    bodies, body_selection,
    editing::{self, EditingCommand},
    model::{Action, Model},
    selection::{Pickable, Selection},
    solid_tools,
    units::LengthUnit,
};

pub const TITLE: &str = "Move body";
pub const COPY_NAME: &str = "Copy";
pub const DESCRIPTION: &str =
    "Shift and turn a body, by distances and angles that can be parameters";
const NO_BODY: &str = "Select a face or edge of the body to move";
const SEVERAL_BODIES: &str = "Select faces or edges of one body only, or one body in the tree";
const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";

pub fn selected_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<FeatureId, &'static str> {
    chosen_body(model, selection, tree, NO_BODY)
}

pub fn chosen_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    none: &'static str,
) -> Result<FeatureId, &'static str> {
    chosen_body_beside(model, selection, tree, None, none)
}

pub fn chosen_body_beside(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    reference: Option<Pickable>,
    none: &'static str,
) -> Result<FeatureId, &'static str> {
    let rest: Vec<FeatureId> = body_selection::bodies_in(selection)
        .into_iter()
        .filter(|body| {
            selection
                .iter()
                .any(|pickable| pickable.body() == Some(*body) && Some(pickable) != reference)
        })
        .collect();
    let chosen = if !tree.is_empty() {
        tree.to_vec()
    } else if rest.is_empty() {
        reference.and_then(Pickable::body).into_iter().collect()
    } else {
        rest
    };
    match chosen.as_slice() {
        [] => Err(none),
        [body] => bodies::shown(model.evaluation(), *body)
            .map(|_| *body)
            .ok_or(NO_SHAPE),
        _ => Err(SEVERAL_BODIES),
    }
}

pub fn create(
    document: &Document,
    body: FeatureId,
    unit: LengthUnit,
    copy: bool,
) -> (Transaction, FeatureId) {
    let title = if copy { COPY_NAME } else { TITLE };
    let name = editing::next_feature_name(document, title);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Move(Move {
            body,
            offset: std::array::from_fn(|_| unit.default_length(0.0)),
            turn: std::array::from_fn(|_| solid_tools::degrees(0.0)),
            copy,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, body: FeatureId, copy: bool) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), body, model.length_unit(), copy);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, movement: Move) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Move(movement),
        },
    ))
}
