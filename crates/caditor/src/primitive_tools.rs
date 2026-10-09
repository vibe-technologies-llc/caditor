use caditor_document::{
    BodyOperation, Document, Edit, FeatureId, FeatureKind, PlaneReference, Primitive,
    PrimitiveAnchor, PrimitiveKind, PrimitiveShape, PrincipalPlane, Transaction, displayed_frame,
};
use caditor_geometry::{Plane, Point2, Ray};

use crate::{
    commands::Command,
    datum_tools::{self, ChosenPlane},
    editing::{self, EditingCommand},
    field, hole_tools,
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
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Xy);
const GONE: &str = "The feature no longer exists";
const NOT_A_PLACE: &str = "Click a plane or flat face to place it on";
const NO_PLANE: &str = "Select a plane or flat face made before this feature";
const PLANE_UNAVAILABLE: &str = "The chosen plane has no result yet; fix it or recompute";
const MISSED: &str = "The click does not meet that plane; look at it from another side";

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
    }
}

pub fn source(model: &Model, selection: &Selection) -> Result<PrimitiveSource, &'static str> {
    let chosen = datum_tools::chosen_plane(model, selection, model.document().bar_index())?;
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
    }
}

pub fn default_anchor(kind: PrimitiveKind) -> PrimitiveAnchor {
    match kind {
        PrimitiveKind::Box | PrimitiveKind::Cylinder => PrimitiveAnchor::BaseCentre,
        PrimitiveKind::Sphere | PrimitiveKind::Torus => PrimitiveAnchor::Centre,
    }
}

struct Place {
    plane: PlaneReference,
    frame: Plane,
    face: Option<FaceChoice>,
}

fn place_of(model: &Model, pickable: Pickable, index: usize) -> Result<Place, &'static str> {
    match pickable {
        Pickable::Plane(plane) => Ok(Place {
            plane: PlaneReference::Principal(plane),
            frame: plane.plane(),
            face: None,
        }),
        Pickable::Datum(datum) => {
            let plane = datum_tools::plane_reference(model, pickable, index)
                .ok_or(datum_tools::DATUM_NOT_A_PLANE)?;
            let frame = datum_tools::result(model.evaluation(), datum)
                .and_then(|result| result.plane())
                .ok_or(PLANE_UNAVAILABLE)?;
            Ok(Place {
                plane,
                frame,
                face: None,
            })
        }
        Pickable::FramePlane { feature, plane } => {
            let reference = datum_tools::plane_reference(model, pickable, index)
                .ok_or(datum_tools::DATUM_MADE_LATER)?;
            let frame = displayed_frame(model.evaluation(), feature)
                .and_then(|placed| plane.in_frame(&placed))
                .ok_or(PLANE_UNAVAILABLE)?;
            Ok(Place {
                plane: reference,
                frame,
                face: None,
            })
        }
        pickable => {
            let choice = FaceChoice::of(pickable).ok_or(NOT_A_PLACE)?;
            let (attachment, frame) = sketch_placement::attachment_at(model, choice, index)?;
            Ok(Place {
                plane: PlaneReference::Face(attachment),
                frame,
                face: Some(choice),
            })
        }
    }
}

fn chosen_place(model: &Model, chosen: &ChosenPlane, index: usize) -> Result<Place, &'static str> {
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
        (PlaneReference::Face(_), None) => Err(NO_PLANE),
    }
}

fn spot(model: &Model, place: &Place, ray: Option<Ray>) -> Result<Point2, &'static str> {
    if let Some(ray) = ray {
        let distance = ray.intersect_plane(&place.frame).ok_or(MISSED)?;
        return Ok(place.frame.to_local(ray.at(distance)));
    }
    Ok(place
        .face
        .and_then(|face| hole_tools::face_middle(model, face, &place.frame))
        .unwrap_or(Point2::ZERO))
}

fn at(unit: LengthUnit, point: Point2) -> [caditor_expression::Expression; 2] {
    [unit.measured(point.x), unit.measured(point.y)]
}

pub fn create(
    model: &Model,
    kind: PrimitiveKind,
    source: &PrimitiveSource,
) -> Result<(Transaction, FeatureId, bool), &'static str> {
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
        Err(reason) => vec![Action::Inform(Notice::info(format!(
            "{}: {reason}.",
            kind.name()
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, primitive: Primitive) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Primitive(primitive),
        },
    ))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    primitive: Primitive,
) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, primitive).ok_or_else(|| GONE.to_owned())?;
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
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
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

pub fn place_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    primitive: &Primitive,
) -> Result<Transaction, String> {
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
        (Some(_), Some(_)) => Err(datum_tools::SEVERAL_PLANES.to_owned()),
        (None, _) => Err(NO_PLANE.to_owned()),
    }
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
        Err(reason) => vec![Action::Inform(Notice::info(reason))],
    }
}
