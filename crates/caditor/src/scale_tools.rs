use std::borrow::Cow;

use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Scale, Transaction, displayed_frame,
};
use caditor_expression::Expression;
use caditor_geometry::Point3;

use crate::{
    commands::Reason,
    datum_tools,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    measure,
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
    sketch_placement,
    units::LengthUnit,
};

pub const TITLE: &str = "Scale body";
pub const DESCRIPTION: &str =
    "Resize a body by a factor about a centre point, such as 25.4 for a part drawn in inches";
const NO_BODY: &str = "Select a face or edge of the body to scale";
const DEFAULT_FACTOR: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScaleError {
    #[error("{0}")]
    Body(String),
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error(
        "Select one corner, round edge, sphere or torus, sketch point, datum point or the origin \
         to scale about"
    )]
    ChooseCentre,
    #[error("The body has no shape before this scale yet; recompute the model, then try again")]
    NoShape { body: FeatureId },
    #[error(
        "The coordinate system the centre is measured in has no result; mend it or set Centre in \
         to World"
    )]
    NoFrame { frame: FeatureId },
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl From<ScaleError> for String {
    fn from(error: ScaleError) -> Self {
        error.to_string()
    }
}

impl Reason for ScaleError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

pub fn selected_body(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<FeatureId, ScaleError> {
    move_tools::chosen_body(model, selection, tree, NO_BODY)
        .map_err(|error| ScaleError::Body(error.into()))
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
            frame: None,
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

fn picked_point(model: &Model, pickable: Pickable, index: usize) -> Option<Point3> {
    let reference = datum_tools::point_reference(model, pickable, index)?;
    match reference.body() {
        Some(body) => {
            let state = sketch_placement::body_state_before(model, body, index).ok()?;
            reference.on_body(state)
        }
        None => measure::point_of(model, pickable),
    }
}

fn centred_at(
    model: &Model,
    feature: FeatureId,
    scale: &Scale,
    point: Point3,
) -> Result<Transaction, ScaleError> {
    let local = match scale.frame {
        Some(frame) => {
            let frame =
                displayed_frame(model.evaluation(), frame).ok_or(ScaleError::NoFrame { frame })?;
            let from = point - frame.origin();
            [frame.x_axis(), frame.y_axis(), frame.normal()].map(|axis| from.dot(axis))
        }
        None => [point.x, point.y, point.z],
    };
    let unit = model.units().length;
    let changed = Scale {
        center: local.map(|value| unit.measured(value)),
        ..scale.clone()
    };
    let document = model.document();
    let transaction = edit(document, feature, changed).ok_or(ScaleError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn centre_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    scale: &Scale,
) -> Result<Transaction, ScaleError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(ScaleError::Gone { feature })?;
    let mut picked = selection.iter();
    let point = match (picked.next(), picked.next()) {
        (Some(only), None) => picked_point(model, only, index),
        _ => None,
    }
    .ok_or(ScaleError::ChooseCentre)?;
    centred_at(model, feature, scale, point)
}

pub fn body_centre_change(
    model: &Model,
    feature: FeatureId,
    scale: &Scale,
) -> Result<Transaction, ScaleError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(ScaleError::Gone { feature })?;
    let centre = sketch_placement::body_state_before(model, scale.body, index)
        .ok()
        .and_then(|solid| solid.bounding_box())
        .ok_or(ScaleError::NoShape { body: scale.body })?
        .center();
    centred_at(model, feature, scale, centre)
}
