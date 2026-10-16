use std::borrow::Cow;

use caditor_document::{
    BodyOperation, Document, Edit, FeatureId, FeatureKind, PlaneReference, Primitive,
    PrimitiveAnchor, PrimitiveKind, PrimitiveShape, PrincipalPlane, Transaction, displayed_frame,
};
use caditor_geometry::{Plane, Point2, Ray};

use crate::{
    commands::{Command, Reason},
    datum_tools::{self, ChosenPlane},
    editing::{self, EditingCommand},
    field::{self, CheckError},
    hole_tools,
    model::{Action, Model, Notice},
    reference_picking::{Picking, Slot},
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    units::LengthUnit,
};

pub const DEFAULT_BOX: [f64; 3] = [20.0, 20.0, 10.0];
pub const DEFAULT_CYLINDER: [f64; 2] = [10.0, 20.0];
pub const DEFAULT_SPHERE: f64 = 20.0;
pub const DEFAULT_TORUS: [f64; 2] = [30.0, 6.0];
pub const DEFAULT_CONE: [f64; 3] = [20.0, 0.0, 20.0];
pub const DEFAULT_WEDGE: [f64; 4] = [20.0, 20.0, 10.0, 0.0];
pub const DEFAULT_PRISM_SIDES: u32 = 6;
pub const DEFAULT_PRISM: [f64; 2] = [20.0, 10.0];
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Xy);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PrimitiveError {
    #[error("The feature no longer exists")]
    Gone { feature: FeatureId },
    #[error("Click a plane or flat face to place it on")]
    NotAPlace,
    #[error("Select a plane or flat face made before this feature")]
    NoPlane,
    #[error("The chosen plane has no result yet; fix it or recompute")]
    PlaneUnavailable,
    #[error("The click does not meet that plane; look at it from another side")]
    Missed,
    #[error("{}", datum_tools::OneChoiceError::DatumNotAPlane)]
    DatumNotAPlane,
    #[error("{}", datum_tools::OneChoiceError::DatumMadeLater)]
    DatumMadeLater,
    #[error("{}", datum_tools::SEVERAL_PLANES)]
    SeveralPlanes,
    #[error(transparent)]
    ChosenPlane(datum_tools::OneChoiceError),
    #[error(transparent)]
    FaceAttachment(crate::sketch_placement::AttachmentError),
    #[error(transparent)]
    Checked(#[from] CheckError),
}

impl Reason for PrimitiveError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrimitiveSource {
    pub chosen: Option<ChosenPlane>,
}

pub fn command(kind: PrimitiveKind) -> Command {
    match kind {
        PrimitiveKind::Box => Command::NewBox,
        PrimitiveKind::Cylinder => Command::NewCylinder,
        PrimitiveKind::Sphere => Command::NewSphere,
        PrimitiveKind::Torus => Command::NewTorus,
        PrimitiveKind::Cone => Command::NewCone,
        PrimitiveKind::Wedge => Command::NewWedge,
        PrimitiveKind::Prism => Command::NewPrism,
    }
}

pub fn source(model: &Model, selection: &Selection) -> Result<PrimitiveSource, PrimitiveError> {
    let chosen = datum_tools::chosen_plane(model, selection, model.document().bar_index())
        .map_err(PrimitiveError::ChosenPlane)?;
    Ok(PrimitiveSource { chosen })
}

pub fn default_shape(kind: PrimitiveKind, unit: LengthUnit) -> PrimitiveShape {
    let length = |millimetres: f64| unit.default_length(millimetres);
    match kind {
        PrimitiveKind::Box => {
            let [long, wide, high] = DEFAULT_BOX;
            PrimitiveShape::Box {
                length: length(long),
                width: length(wide),
                height: length(high),
            }
        }
        PrimitiveKind::Cylinder => {
            let [diameter, height] = DEFAULT_CYLINDER;
            PrimitiveShape::Cylinder {
                diameter: length(diameter),
                height: length(height),
            }
        }
        PrimitiveKind::Sphere => PrimitiveShape::Sphere {
            diameter: length(DEFAULT_SPHERE),
        },
        PrimitiveKind::Torus => {
            let [diameter, tube] = DEFAULT_TORUS;
            PrimitiveShape::Torus {
                diameter: length(diameter),
                tube: length(tube),
            }
        }
        PrimitiveKind::Cone => {
            let [bottom, top, height] = DEFAULT_CONE;
            PrimitiveShape::Cone {
                bottom: length(bottom),
                top: length(top),
                height: length(height),
            }
        }
        PrimitiveKind::Wedge => {
            let [long, wide, high, top] = DEFAULT_WEDGE;
            PrimitiveShape::Wedge {
                length: length(long),
                width: length(wide),
                height: length(high),
                top: length(top),
            }
        }
        PrimitiveKind::Prism => {
            let [diameter, height] = DEFAULT_PRISM;
            PrimitiveShape::Prism {
                sides: caditor_expression::Expression::Number(f64::from(DEFAULT_PRISM_SIDES)),
                diameter: length(diameter),
                height: length(height),
            }
        }
    }
}

pub fn default_anchor(kind: PrimitiveKind) -> PrimitiveAnchor {
    match kind {
        PrimitiveKind::Box
        | PrimitiveKind::Cylinder
        | PrimitiveKind::Cone
        | PrimitiveKind::Wedge
        | PrimitiveKind::Prism => PrimitiveAnchor::BaseCentre,
        PrimitiveKind::Sphere | PrimitiveKind::Torus => PrimitiveAnchor::Centre,
    }
}

struct Place {
    plane: PlaneReference,
    frame: Plane,
    face: Option<FaceChoice>,
}

fn place_of(model: &Model, pickable: Pickable, index: usize) -> Result<Place, PrimitiveError> {
    match pickable {
        Pickable::Plane(plane) => Ok(Place {
            plane: PlaneReference::Principal(plane),
            frame: plane.plane(),
            face: None,
        }),
        Pickable::Datum(datum) => {
            let plane = datum_tools::plane_reference(model, pickable, index)
                .ok_or(PrimitiveError::DatumNotAPlane)?;
            let frame = datum_tools::result(model.evaluation(), datum)
                .and_then(|result| result.plane())
                .ok_or(PrimitiveError::PlaneUnavailable)?;
            Ok(Place {
                plane,
                frame,
                face: None,
            })
        }
        Pickable::FramePlane { feature, plane } => {
            let reference = datum_tools::plane_reference(model, pickable, index)
                .ok_or(PrimitiveError::DatumMadeLater)?;
            let frame = displayed_frame(model.evaluation(), feature)
                .and_then(|placed| plane.in_frame(&placed))
                .ok_or(PrimitiveError::PlaneUnavailable)?;
            Ok(Place {
                plane: reference,
                frame,
                face: None,
            })
        }
        pickable => {
            let choice = FaceChoice::of(pickable).ok_or(PrimitiveError::NotAPlace)?;
            let (attachment, frame) = sketch_placement::attachment_at(model, choice, index)
                .map_err(PrimitiveError::FaceAttachment)?;
            Ok(Place {
                plane: PlaneReference::Face(attachment),
                frame,
                face: Some(choice),
            })
        }
    }
}

fn chosen_place(
    model: &Model,
    chosen: &ChosenPlane,
    index: usize,
) -> Result<Place, PrimitiveError> {
    match (&chosen.plane, chosen.face) {
        (PlaneReference::Principal(plane), _) => place_of(model, Pickable::Plane(*plane), index),
        (PlaneReference::Datum(datum), _) => place_of(model, Pickable::Datum(*datum), index),
        (PlaneReference::Frame { frame, plane }, _) => place_of(
            model,
            Pickable::FramePlane {
                feature: *frame,
                plane: *plane,
            },
            index,
        ),
        (PlaneReference::Face(_), Some(face)) => place_of(model, face, index),
        (PlaneReference::Face(_), None) => Err(PrimitiveError::NoPlane),
    }
}

fn spot(model: &Model, place: &Place, ray: Option<Ray>) -> Result<Point2, PrimitiveError> {
    if let Some(ray) = ray {
        let distance = ray
            .intersect_plane(&place.frame)
            .ok_or(PrimitiveError::Missed)?;
        return Ok(place.frame.to_local(ray.at(distance)));
    }
    Ok(place
        .face
        .and_then(|face| hole_tools::face_middle(model, face, &place.frame))
        .unwrap_or(Point2::ZERO))
}

pub fn chosen_spot(
    model: &Model,
    chosen: &ChosenPlane,
    index: usize,
) -> Result<(PlaneReference, Point2), PrimitiveError> {
    let place = chosen_place(model, chosen, index)?;
    let spot = spot(model, &place, None)?;
    Ok((place.plane, spot))
}

pub fn clicked_spot(
    model: &Model,
    pickable: Pickable,
    index: usize,
    ray: Option<Ray>,
) -> Result<(PlaneReference, Point2), PrimitiveError> {
    let place = place_of(model, pickable, index)?;
    let spot = spot(model, &place, ray)?;
    Ok((place.plane, spot))
}

fn at(unit: LengthUnit, point: Point2) -> [caditor_expression::Expression; 2] {
    [unit.measured(point.x), unit.measured(point.y)]
}

pub fn create(
    model: &Model,
    kind: PrimitiveKind,
    source: &PrimitiveSource,
) -> Result<(Transaction, FeatureId, bool), PrimitiveError> {
    let document = model.document();
    let unit = model.length_unit();
    let place = source
        .chosen
        .as_ref()
        .map(|chosen| chosen_place(model, chosen, document.bar_index()))
        .transpose()?;
    let (plane, spot, operation) = match &place {
        Some(place) => (
            place.plane.clone(),
            spot(model, place, None)?,
            place
                .face
                .map_or(BodyOperation::NewBody, |face| BodyOperation::Add(face.body)),
        ),
        None => (DEFAULT_PLANE, Point2::ZERO, BodyOperation::NewBody),
    };
    let name = editing::next_feature_name(document, kind.name());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Primitive(Primitive {
            shape: default_shape(kind, unit),
            plane,
            at: at(unit, spot),
            anchor: default_anchor(kind),
            reversed: false,
            operation,
        }),
    );
    Ok((transaction.finish(), feature, place.is_none()))
}

pub fn create_actions(model: &Model, kind: PrimitiveKind, source: &PrimitiveSource) -> Vec<Action> {
    match create(model, kind, source) {
        Ok((transaction, feature, unplaced)) => {
            let mut actions = vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ];
            if unplaced {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    feature,
                    Slot::PrimitivePlace,
                ))));
            }
            actions
        }
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{}: {reason}.",
            kind.name()
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, primitive: Primitive) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Primitive(primitive),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    primitive: Primitive,
) -> Result<Transaction, PrimitiveError> {
    let document = model.document();
    let transaction = edit(document, feature, primitive).ok_or(PrimitiveError::Gone { feature })?;
    field::checked(document, transaction)
}

pub fn with_operation(primitive: &Primitive, operation: BodyOperation) -> Primitive {
    let cutting = |operation: BodyOperation| {
        matches!(
            operation,
            BodyOperation::Remove(_) | BodyOperation::Intersect(_)
        )
    };
    let reversed = match primitive.plane {
        PlaneReference::Face(_) if cutting(operation) != cutting(primitive.operation) => {
            cutting(operation)
        }
        _ => primitive.reversed,
    };
    Primitive {
        operation,
        reversed,
        ..primitive.clone()
    }
}

pub fn with_shape(primitive: &Primitive, kind: PrimitiveKind, unit: LengthUnit) -> Primitive {
    Primitive {
        shape: default_shape(kind, unit),
        ..primitive.clone()
    }
}

fn placed(
    model: &Model,
    feature: FeatureId,
    primitive: &Primitive,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Result<Transaction, PrimitiveError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(PrimitiveError::Gone { feature })?;
    let place = place_of(model, pickable, index)?;
    let spot = spot(model, &place, ray)?;
    change(
        model,
        feature,
        Primitive {
            plane: place.plane,
            at: at(model.length_unit(), spot),
            ..primitive.clone()
        },
    )
}

pub fn placed_on(
    model: &Model,
    feature: FeatureId,
    primitive: &Primitive,
    plane: &PlaneReference,
) -> Result<Transaction, PrimitiveError> {
    let pickable = datum_tools::plane_pickable(plane).ok_or(PrimitiveError::NoPlane)?;
    placed(model, feature, primitive, pickable, None)
}

pub fn place_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    primitive: &Primitive,
) -> Result<Transaction, PrimitiveError> {
    let mut chosen = selection.iter().filter(|pickable| {
        matches!(
            pickable,
            Pickable::Plane(_)
                | Pickable::Datum(_)
                | Pickable::FramePlane { .. }
                | Pickable::Face { .. }
        )
    });
    match (chosen.next(), chosen.next()) {
        (Some(pickable), None) => placed(model, feature, primitive, pickable, None),
        (Some(_), Some(_)) => Err(PrimitiveError::SeveralPlanes),
        (None, _) => Err(PrimitiveError::NoPlane),
    }
}

pub fn is_part_of(feature: FeatureId, pickable: Pickable) -> bool {
    matches!(
        pickable,
        Pickable::Face { body, .. } | Pickable::Edge { body, .. } | Pickable::Vertex { body, .. }
            if body == feature
    )
}

pub fn place_click(
    model: &Model,
    feature: FeatureId,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Vec<Action> {
    let Some(primitive) = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.primitive())
    else {
        return vec![Action::Editing(EditingCommand::StopPicking)];
    };
    match placed(model, feature, primitive, pickable, ray) {
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(reason.to_string()))],
    }
}
