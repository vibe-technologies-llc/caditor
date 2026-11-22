use std::borrow::Cow;

use caditor_document::{
    AxisReference, AxisTurn, Document, Edit, FeatureId, FeatureKind, Move, PrincipalAxis,
    Transaction, TurnCentre,
};

use crate::{
    bodies,
    body_selection::{self, Standing},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BodyChoiceError {
    #[error("{instruction}")]
    Nothing { instruction: &'static str },
    #[error("{}", body_selection::NO_BODY_SHOWN)]
    NoBodyShown,
    #[error("{SEVERAL_BODIES}")]
    Several,
    #[error("{}", Pending::BodyShape)]
    NoShape,
}

impl Reason for BodyChoiceError {
    fn reason(&self) -> Cow<'static, str> {
        match self {
            Self::Nothing { instruction } => Cow::Borrowed(*instruction),
            Self::NoBodyShown => Cow::Borrowed(body_selection::NO_BODY_SHOWN),
            Self::Several => Cow::Borrowed(SEVERAL_BODIES),
            Self::NoShape => Cow::Owned(self.to_string()),
        }
    }

    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::NoShape).then_some(Pending::BodyShape)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveSource {
    pub body: FeatureId,
    pub also: Vec<FeatureId>,
}

impl MoveSource {
    pub fn single(body: FeatureId) -> Self {
        Self {
            body,
            also: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.also.len() + 1
    }
}

pub fn bodies_hover(source: &MoveSource) -> String {
    let count = source.count();
    format!("Shift and turn {count} bodies, by distances and angles that can be parameters")
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MoveError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Select an axis, straight edge, round face or sketch line made before this move")]
    NoAxis,
    #[error("The body already turns about the selected axis")]
    AlreadyAbout,
    #[error("Select a face, edge or corner of a body to move")]
    NoBodySelected,
    #[error("The move already moves every selected body")]
    AlreadyBodies,
    #[error("{name} is made after the move, so it cannot be moved by it")]
    LaterBody { name: String },
    #[error("That is a copy this move makes, so it cannot be moved by it")]
    OwnCopy,
    #[error("The move needs a body; add another body before leaving this one out")]
    LastBody,
    #[error(transparent)]
    AxisSelection(datum_tools::OneChoiceError),
    #[error(transparent)]
    Choice(#[from] ChoiceRefusal),
    #[error(transparent)]
    Check(#[from] CheckError),
}

pub fn selected_bodies(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<MoveSource, BodyChoiceError> {
    match chosen_bodies_beside(model, selection, tree, None, NO_BODY) {
        Ok(bodies) => {
            let mut bodies = bodies.into_iter();

            let body = bodies.next().ok_or(BodyChoiceError::Nothing {
                instruction: NO_BODY,
            })?;
            Ok(MoveSource {
                body,
                also: bodies.collect(),
            })
        }
        Err(BodyChoiceError::Nothing { instruction }) => {
            match body_selection::standing_bodies(model) {
                Standing::Only(only) => chosen_body(model, &Selection::default(), &[only], NO_BODY)
                    .map(MoveSource::single),
                Standing::Several => Err(BodyChoiceError::Nothing { instruction }),
                Standing::None => Err(BodyChoiceError::NoBodyShown),
            }
        }
        Err(error) => Err(error),
    }
}

pub fn chosen_or_only_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    reference: Option<Pickable>,
    none: &'static str,
) -> Result<FeatureId, BodyChoiceError> {
    match chosen_body_beside(model, selection, tree, reference, none) {
        Err(BodyChoiceError::Nothing { instruction }) => {
            match body_selection::standing_bodies(model) {
                Standing::Only(only) => chosen_body(model, &Selection::default(), &[only], none),
                Standing::Several => Err(BodyChoiceError::Nothing { instruction }),
                Standing::None => Err(BodyChoiceError::NoBodyShown),
            }
        }
        chosen => chosen,
    }
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
    match chosen_bodies_beside(model, selection, tree, reference, none)?.as_slice() {
        [body] => Ok(*body),
        _ => Err(BodyChoiceError::Several),
    }
}

pub fn chosen_bodies_beside(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    reference: Option<Pickable>,
    none: &'static str,
) -> Result<Vec<FeatureId>, BodyChoiceError> {
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

    if chosen.is_empty() {
        return Err(BodyChoiceError::Nothing { instruction: none });
    }
    chosen
        .into_iter()
        .map(|body| {
            bodies::shown(model.evaluation(), body)
                .map(|_| body)
                .ok_or(BodyChoiceError::NoShape)
        })
        .collect()
}

pub fn create(
    document: &Document,
    source: &MoveSource,
    unit: LengthUnit,
    copy: bool,
) -> (Transaction, FeatureId) {
    let title = if copy { COPY_NAME } else { TITLE };
    let name = editing::next_feature_name(document, title);
    let mut transaction = document.transaction(format!("Create {name}"));

    let feature = transaction.add_feature(
        name,
        FeatureKind::Move(Move {
            body: source.body,
            more_bodies: source.also.clone(),
            offset: std::array::from_fn(|_| unit.default_length(0.0)),
            turn: std::array::from_fn(|_| solid_tools::degrees(0.0)),
            copy,
            about: TurnCentre::Body,
            frame: None,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &MoveSource, copy: bool) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source, model.length_unit(), copy);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn with_bodies(movement: &Move, bodies: Vec<FeatureId>) -> Option<Move> {
    let mut bodies = bodies.into_iter();

    let body = bodies.next()?;
    Some(Move {
        body,
        more_bodies: bodies.collect(),
        ..movement.clone()
    })
}

fn checked_body(
    model: &Model,
    feature: FeatureId,
    body: FeatureId,
) -> Result<FeatureId, MoveError> {
    let document = model.document();

    if body.maker() == feature {
        return Err(MoveError::OwnCopy);
    }
    if document
        .bodies_before_in(feature, model.evaluation())
        .contains(&body)
    {
        Ok(body)
    } else {
        Err(MoveError::LaterBody {
            name: document.body_label(body).unwrap_or_default(),
        })
    }
}

pub fn added_bodies(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    movement: &Move,
) -> Result<Transaction, MoveError> {
    let selected = body_selection::bodies_in(selection);

    if selected.is_empty() {
        return Err(MoveError::NoBodySelected);
    }

    let mut bodies: Vec<FeatureId> = movement.bodies().collect();

    for body in selected {
        if !bodies.contains(&body) {
            bodies.push(checked_body(model, feature, body)?);
        }
    }
    if bodies.len() == movement.bodies().count() {
        return Err(MoveError::AlreadyBodies);
    }

    let changed = with_bodies(movement, bodies).ok_or(MoveError::LastBody)?;
    checked_change(model, feature, changed)
}

pub fn toggled_body(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    movement: &Move,
) -> Result<Transaction, MoveError> {
    let clicked = match body_selection::bodies_in(selection).as_slice() {
        [body] => *body,
        _ => return Err(MoveError::NoBodySelected),
    };

    if movement.bodies().any(|body| body == clicked) {
        return without_body(model, feature, movement, clicked);
    }

    let mut bodies: Vec<FeatureId> = movement.bodies().collect();

    bodies.push(checked_body(model, feature, clicked)?);

    let changed = with_bodies(movement, bodies).ok_or(MoveError::LastBody)?;
    checked_change(model, feature, changed)
}

pub fn without_body(
    model: &Model,
    feature: FeatureId,
    movement: &Move,
    body: FeatureId,
) -> Result<Transaction, MoveError> {
    let kept: Vec<FeatureId> = movement.bodies().filter(|kept| *kept != body).collect();
    let changed = with_bodies(movement, kept).ok_or(MoveError::LastBody)?;
    checked_change(model, feature, changed)
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
