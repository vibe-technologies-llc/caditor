use caditor_document::{
    Combine, CombineOperation, Document, Edit, FeatureId, FeatureKind, Transaction,
};

use crate::{
    bodies,
    editing::{self, EditingCommand},
    model::{Action, Model},
    selection::{Pickable, Selection},
};

pub const TITLE: &str = "Combine";
pub const DESCRIPTION: &str = "Join, cut or intersect two bodies into one";
const NEED_TWO: &str = "Select faces or edges of two bodies";
const MORE_THAN_TWO: &str = "Select faces or edges of two bodies only";
const NO_SHAPE: &str = "A selected body has no shape yet; recompute the model, then try again";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyPair {
    pub target: FeatureId,
    pub tool: FeatureId,
}

pub fn operation_hover(operation: CombineOperation) -> &'static str {
    match operation {
        CombineOperation::Join => "Add the tool body to the target",
        CombineOperation::Cut => "Remove the tool body from the target",
        CombineOperation::Intersect => "Keep only where the two bodies overlap",
    }
}

pub fn selected_bodies(model: &Model, selection: &Selection) -> Result<BodyPair, &'static str> {
    let chosen: Vec<FeatureId> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, .. }
            | Pickable::Edge { body, .. }
            | Pickable::Vertex { body, .. } => Some(body),
            _ => None,
        })
        .collect();
    let document = model.document();
    let mut ordered: Vec<FeatureId> = document
        .active_features()
        .map(|feature| feature.id())
        .filter(|id| chosen.contains(id))
        .collect();
    ordered.dedup();
    match ordered.as_slice() {
        [target, tool] => {
            let evaluation = model.evaluation();
            if bodies::shown(evaluation, *target).is_none()
                || bodies::shown(evaluation, *tool).is_none()
            {
                return Err(NO_SHAPE);
            }
            Ok(BodyPair {
                target: *target,
                tool: *tool,
            })
        }
        [] | [_] => Err(NEED_TWO),
        _ => Err(MORE_THAN_TWO),
    }
}

pub fn create(document: &Document, pair: BodyPair) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Combine(Combine {
            body: pair.target,
            tool: pair.tool,
            operation: CombineOperation::Join,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, pair: BodyPair) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), pair);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, combine: Combine) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Combine(combine),
        },
    ))
}
