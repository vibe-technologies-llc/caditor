use std::borrow::Cow;

use caditor_document::{
    AxisReference, AxisTurn, Document, Edit, FeatureId, FeatureKind, Move, PrincipalAxis,
    Transaction, TurnCentre,
};

use crate::{
    bodies, body_selection,
    commands::Reason,
    datum_tools,
    editing::{self, EditingCommand},
    feature_fields::ChoiceRefusal,
    field::{self, CheckError},
    model::{Action, Model},
    queued::Pending,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BodyChoiceError {
    #[error("{instruction}")]
    Nothing { instruction: &'static str },
    #[error("{SEVERAL_BODIES}")]
    Several,
    #[error("{}", Pending::BodyShape)]
    NoShape,
}

impl Reason for BodyChoiceError {
    fn reason(&self) -> Cow<'static, str> {
        match self {
            Self::Nothing { instruction } => Cow::Borrowed(*instruction),
            Self::Several => Cow::Borrowed(SEVERAL_BODIES),
            Self::NoShape => Cow::Owned(self.to_string()),
        }
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape).then_some(Pending::BodyShape)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MoveError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select an axis, straight edge, round face or sketch line made before this move")]
    NoAxis,
    #[error("The body already turns about the selected axis")]
    AlreadyAbout,
    #[error(transparent)]
    AxisSelection(datum_tools::OneChoiceError),
    #[error(transparent)]
    Choice(#[from] ChoiceRefusal),
    #[error(transparent)]
    Check(#[from] CheckError),
}

pub fn selected_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<FeatureId, BodyChoiceError> {
    chosen_body(model, selection, tree, NO_BODY)
}

pub fn chosen_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    none: &'static str,
) -> Result<FeatureId, BodyChoiceError> {
    chosen_body_beside(model, selection, tree, None, none)
}

pub fn chosen_body_beside(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    reference: Option<Pickable>,
    none: &'static str,
) -> Result<FeatureId, BodyChoiceError> {
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
        [] => Err(BodyChoiceError::Nothing { instruction: none }),
        [body] => bodies::shown(model.evaluation(), *body)
            .map(|_| *body)
            .ok_or(BodyChoiceError::NoShape),
        _ => Err(BodyChoiceError::Several),
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
            about: TurnCentre::Body,
            frame: None,
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

pub fn checked_change(
    model: &Model,
    feature: FeatureId,
    movement: Move,
) -> Result<Transaction, MoveError> {
    let document = model.document();
    let transaction = edit(document, feature, movement).ok_or(MoveError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn about_axis(movement: &Move, axis: AxisReference) -> Move {
    let angle = movement
        .about
        .axis_turn()
        .map_or_else(|| solid_tools::degrees(0.0), |turn| turn.angle.clone());
    Move {
        about: TurnCentre::Axis(Box::new(AxisTurn { axis, angle })),
        ..movement.clone()
    }
}

pub fn first_axis(model: &Model, selection: &Selection, feature: FeatureId) -> AxisReference {
    let index = model
        .document()
        .feature_index(feature)
        .unwrap_or(usize::MAX);
    datum_tools::only_axis(model, selection, index)
        .ok()
        .flatten()
        .unwrap_or(AxisReference::Principal(PrincipalAxis::Z))
}

pub fn axis_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    movement: &Move,
) -> Result<Transaction, MoveError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(MoveError::Gone { feature })?;
    let axis = datum_tools::only_axis(model, selection, index)
        .map_err(MoveError::AxisSelection)?
        .ok_or(MoveError::NoAxis)?;
    if movement.about.axis() == Some(&axis) {
        return Err(MoveError::AlreadyAbout);
    }
    checked_change(model, feature, about_axis(movement, axis))
}
