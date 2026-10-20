use std::borrow::Cow;

use caditor_document::{
    Combine, CombineOperation, Document, Edit, FeatureId, FeatureKind, Transaction,
};

use crate::{
    bodies, body_selection,
    commands::Reason,
    editing::{self, EditingCommand},
    model::{Action, Model},
    queued::Pending,
    selection::{self, Selection},
    visibility,
};

pub const TITLE: &str = "Combine";
pub const DESCRIPTION: &str = "Join, cut or intersect two bodies into one";
pub const CHOOSE_TARGET_PROMPT: &str = "Click the body to combine into";
pub const STOP_CHOOSING_HOVER: &str =
    "Click the bodies to combine in the view, or click here to stop choosing (Esc)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CombineError {
    #[error("The model needs two shown bodies to combine")]
    NeedTwo,
    #[error("No other shown body to combine the selected one with")]
    NoOtherBody,
    #[error("Select faces or edges of two bodies only, or two bodies in the tree")]
    MoreThanTwo,
    #[error("Choose two different bodies to combine")]
    SameBody,
    #[error("{}", Pending::BodiesShape)]
    NoShape,
}

impl Reason for CombineError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape).then_some(Pending::BodiesShape)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyPair {
    pub target: FeatureId,
    pub tool: FeatureId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombineStart {
    Pair(BodyPair),
    Choose(Option<FeatureId>),
}

pub fn tool_prompt(model: &Model, target: FeatureId) -> String {
    format!(
        "Click the body to combine into {}",
        selection::body_name(model.document(), target)
    )
}

pub fn choosable(model: &Model) -> Vec<FeatureId> {
    let document = model.document();
    let evaluation = model.evaluation();
    document
        .bodies_standing_in(evaluation)
        .into_iter()
        .filter(|body| {
            visibility::is_shown(document, *body) && bodies::shown(evaluation, *body).is_some()
        })
        .collect()
}

pub fn pair(model: &Model, target: FeatureId, tool: FeatureId) -> Result<BodyPair, CombineError> {
    if target == tool {
        return Err(CombineError::SameBody);
    }
    let evaluation = model.evaluation();
    if bodies::shown(evaluation, target).is_none() || bodies::shown(evaluation, tool).is_none() {
        return Err(CombineError::NoShape);
    }
    Ok(BodyPair { target, tool })
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
) -> Result<CombineStart, CombineError> {
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
        let viewed: Vec<FeatureId> = body_selection::bodies_in(selection)
            .into_iter()
            .filter(standing)
            .collect();
        if viewed.is_empty() {
            tree.iter().copied().filter(standing).collect()
        } else {
            viewed
        }
    };
    match ordered.as_slice() {
        [target, tool] => pair(model, *target, *tool).map(CombineStart::Pair),
        [target] => {
            let others = choosable(model).into_iter().any(|body| body != *target);
            if bodies::shown(model.evaluation(), *target).is_none() {
                Err(CombineError::NoShape)
            } else if others {
                Ok(CombineStart::Choose(Some(*target)))
            } else {
                Err(CombineError::NoOtherBody)
            }
        }
        [] if choosable(model).len() >= 2 => Ok(CombineStart::Choose(None)),
        [] => Err(CombineError::NeedTwo),
        _ => Err(CombineError::MoreThanTwo),
    }
}

pub fn start_actions(model: &Model, start: CombineStart) -> Vec<Action> {
    match start {
        CombineStart::Pair(pair) => create_actions(model, pair),
        CombineStart::Choose(target) => {
            vec![Action::Editing(EditingCommand::ChooseCombined(target))]
        }
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
