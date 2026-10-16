use std::borrow::Cow;

use caditor_document::{
    Combine, CombineOperation, Document, Edit, FeatureId, FeatureKind, Transaction,
};

use crate::{
    bodies, body_selection,
    commands::Reason,
    editing::{self, EditingCommand},
    model::{Action, Model},
    queued,
    selection::Selection,
};

pub const TITLE: &str = "Combine";
pub const DESCRIPTION: &str = "Join, cut or intersect two bodies into one";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CombineError {
    #[error("Select faces or edges of two bodies, or two bodies in the tree")]
    NeedTwo,
    #[error("Select faces or edges of two bodies only, or two bodies in the tree")]
    MoreThanTwo,
    #[error("{}", queued::BODIES_SHAPE)]
    NoShape,
}

impl Reason for CombineError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

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

pub fn selected_bodies(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<BodyPair, CombineError> {
    let document = model.document();
    let standing = |id: &FeatureId| {
        document
            .active_features()
            .any(|feature| feature.id() == *id)
    };
    let ordered: Vec<FeatureId> = if tree.len() >= 2 {
        document
            .active_features()
            .map(|feature| feature.id())
            .filter(|id| tree.contains(id))
            .collect()
    } else {
        body_selection::bodies_in(selection)
            .into_iter()
            .filter(standing)
            .collect()
    };
    match ordered.as_slice() {
        [target, tool] => {
            let evaluation = model.evaluation();
            if bodies::shown(evaluation, *target).is_none()
                || bodies::shown(evaluation, *tool).is_none()
            {
                return Err(CombineError::NoShape);
            }
            Ok(BodyPair {
                target: *target,
                tool: *tool,
            })
        }
        [] | [_] => Err(CombineError::NeedTwo),
        _ => Err(CombineError::MoreThanTwo),
    }
}

pub fn create(document: &Document, pair: BodyPair) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Combine(Combine::new(pair.target, pair.tool, CombineOperation::Join)),
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
