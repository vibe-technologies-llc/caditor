use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Ray, RigidTransform, Vector3};
use caditor_kernel::{
    Curve, EdgeId, EdgeReference, FaceId, FaceReference, ReferenceError, Solid, Surface,
};

use crate::{
    attachment::{AttachmentError, FaceAttachment},
    describe::describe_origin,
    document::{Document, Feature, FeatureId},
    origins,
    recompute::{Evaluation, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    tolerance,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrincipalPlane {
    Xy,
    Xz,
    Yz,
}

impl PrincipalPlane {
    pub const ALL: [Self; 3] = [Self::Xy, Self::Xz, Self::Yz];

    pub fn plane(self) -> Plane {
        match self {
            Self::Xy => Plane::XY,
            Self::Xz => Plane::XZ,
            Self::Yz => Plane::YZ,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Xy => "XY plane",
            Self::Xz => "XZ plane",
            Self::Yz => "YZ plane",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrincipalAxis {
    X,
    Y,
    Z,
}

impl PrincipalAxis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    pub fn direction(self) -> Vector3 {
        match self {
            Self::X => Vector3::X,
            Self::Y => Vector3::Y,
            Self::Z => Vector3::Z,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::X => "X axis",
            Self::Y => "Y axis",
            Self::Z => "Z axis",
        }
    }

    pub fn ray(self) -> Option<Ray> {
        Ray::new(Point3::ZERO, self.direction())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrincipalGeometry {
    Origin,
    Axis(PrincipalAxis),
    Plane(PrincipalPlane),
}

impl PrincipalGeometry {
    pub const ALL: [Self; 7] = [
        Self::Plane(PrincipalPlane::Xy),
        Self::Plane(PrincipalPlane::Xz),
        Self::Plane(PrincipalPlane::Yz),
        Self::Axis(PrincipalAxis::X),
        Self::Axis(PrincipalAxis::Y),
        Self::Axis(PrincipalAxis::Z),
        Self::Origin,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Origin => "Origin",
            Self::Axis(axis) => axis.name(),
            Self::Plane(plane) => plane.name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaneReference {
    Principal(PrincipalPlane),
    Datum(FeatureId),
    Face(FaceAttachment),
}

impl PlaneReference {
    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(feature) => Some(*feature),
            Self::Principal(_) | Self::Face(_) => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Face(attachment) => Some(attachment.body),
            Self::Principal(_) | Self::Datum(_) => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Face(attachment) => attachment.origin_features(),
            Self::Principal(_) | Self::Datum(_) => BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AxisReference {
    Principal(PrincipalAxis),
    Datum(FeatureId),
    Edge {
        body: FeatureId,
        edge: Box<EdgeReference>,
    },
    Face {
        body: FeatureId,
        face: FaceReference,
    },
}

impl AxisReference {
    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(feature) => Some(*feature),
            Self::Principal(_) | Self::Edge { .. } | Self::Face { .. } => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edge { body, .. } | Self::Face { body, .. } => Some(*body),
            Self::Principal(_) | Self::Datum(_) => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Edge { edge, .. } => origins::of_edge(edge),
            Self::Face { face, .. } => origins::of_face(face).into_iter().collect(),
            Self::Principal(_) | Self::Datum(_) => BTreeSet::new(),
        }
    }

    pub fn capture_edge(body: FeatureId, solid: &Solid, edge: EdgeId) -> Option<Self> {
        edge_ray(solid, edge)?;
        Some(Self::Edge {
            body,
            edge: Box::new(EdgeReference::capture(solid, edge)?),
        })
    }

    pub fn capture_face(body: FeatureId, solid: &Solid, face: FaceId) -> Option<Self> {
        face_axis(solid, face)?;
        Some(Self::Face {
            body,
            face: FaceReference::capture(solid, face)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaneRotation {
    pub axis: AxisReference,
    pub angle: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatumPlane {
    pub base: PlaneReference,
    pub rotation: Option<PlaneRotation>,
    pub offset: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatumAxis {
    Along(AxisReference),
    Intersection(PlaneReference, PlaneReference),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    Plane(DatumPlane),
    Axis(DatumAxis),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DatumResult {
    Plane(Plane),
    Axis(Ray),
}

impl DatumResult {
    pub fn plane(&self) -> Option<Plane> {
        match self {
            Self::Plane(plane) => Some(*plane),
            Self::Axis(_) => None,
        }
    }

    pub fn axis(&self) -> Option<Ray> {
        match self {
            Self::Axis(axis) => Some(*axis),
            Self::Plane(_) => None,
        }
    }
}

impl Datum {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Plane(_) => "Plane",
            Self::Axis(_) => "Axis",
        }
    }

    pub fn is_plane(&self) -> bool {
        matches!(self, Self::Plane(_))
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        self.is_plane() == other.is_plane()
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Plane(plane) => std::iter::once(&plane.offset)
                .chain(plane.rotation.as_ref().map(|rotation| &rotation.angle))
                .collect(),
            Self::Axis(_) => Vec::new(),
        }
    }

    fn planes(&self) -> Vec<&PlaneReference> {
        match self {
            Self::Plane(plane) => vec![&plane.base],
            Self::Axis(DatumAxis::Intersection(first, second)) => vec![first, second],
            Self::Axis(DatumAxis::Along(_)) => Vec::new(),
        }
    }

    fn axes(&self) -> Vec<&AxisReference> {
        match self {
            Self::Plane(plane) => plane
                .rotation
                .as_ref()
                .map(|rotation| &rotation.axis)
                .into_iter()
                .collect(),
            Self::Axis(DatumAxis::Along(axis)) => vec![axis],
            Self::Axis(DatumAxis::Intersection(..)) => Vec::new(),
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn plane_datums(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .filter_map(PlaneReference::datum)
            .collect()
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::datum)
            .collect()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .filter_map(PlaneReference::body)
            .chain(self.axes().into_iter().filter_map(AxisReference::body))
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = self.plane_datums();
        used.extend(self.axis_datums());
        used.extend(self.bodies());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .flat_map(PlaneReference::origin_features)
            .chain(
                self.axes()
                    .into_iter()
                    .flat_map(AxisReference::origin_features),
            )
            .collect()
    }
}

fn feature_name(document: &Document, feature: FeatureId) -> String {
    document.feature(feature).map_or_else(
        || "a deleted feature".to_owned(),
        |feature| feature.name.clone(),
    )
}

pub fn describe_plane(document: &Document, reference: &PlaneReference) -> String {
    match reference {
        PlaneReference::Principal(plane) => format!("the {}", plane.name()),
        PlaneReference::Datum(feature) => feature_name(document, *feature),
        PlaneReference::Face(attachment) => describe_origin(document, attachment.face.origin()),
    }
}

pub fn describe_axis(document: &Document, reference: &AxisReference) -> String {
    match reference {
        AxisReference::Principal(axis) => format!("the {}", axis.name()),
        AxisReference::Datum(feature) => feature_name(document, *feature),
        AxisReference::Edge { body, .. } => {
            format!("an edge of {}", feature_name(document, *body))
        }
        AxisReference::Face { face, .. } => {
            format!("the axis of {}", describe_origin(document, face.origin()))
        }
    }
}

pub(crate) fn edge_ray(solid: &Solid, edge: EdgeId) -> Option<Ray> {
    let definition = solid.edge(edge)?;
    let Curve::Line(line) = definition.curve() else {
        return None;
    };
    Ray::new(line.point(definition.interval().start()), line.direction())
}

pub(crate) fn face_axis(solid: &Solid, face: FaceId) -> Option<Ray> {
    let (origin, direction) = match solid.face(face)?.surface() {
        Surface::Cylinder(surface) => (surface.frame().origin(), surface.frame().normal()),
        Surface::Cone(surface) => (surface.frame().origin(), surface.frame().normal()),
        Surface::Torus(surface) => (surface.frame().origin(), surface.frame().normal()),
        Surface::Revolution(surface) => (surface.axis_origin(), surface.axis_direction()),
        _ => return None,
    };
    Ray::new(origin, direction)
}

pub fn displayed_axis(
    evaluation: &Evaluation,
    user: FeatureId,
    reference: &AxisReference,
) -> Option<Ray> {
    let body_seen = |body: FeatureId| {
        evaluation
            .body_seen_by(user, body)
            .or_else(|| evaluation.body(body))
    };
    match reference {
        AxisReference::Principal(axis) => axis.ray(),
        AxisReference::Datum(feature) => evaluation
            .feature(*feature)?
            .result
            .as_deref()?
            .datum()?
            .axis(),
        AxisReference::Edge { body, edge } => {
            let solid = body_seen(*body)?;
            edge_ray(solid, edge.resolve(solid).ok()?)
        }
        AxisReference::Face { body, face } => {
            let solid = body_seen(*body)?;
            face_axis(solid, face.resolve(solid).ok()?)
        }
    }
}

fn one_line(rays: impl IntoIterator<Item = Option<Ray>>) -> Option<Ray> {
    let rays: Vec<Ray> = rays.into_iter().collect::<Option<_>>()?;
    let (first, rest) = rays.split_first()?;
    rest.iter()
        .all(|other| tolerance::same_line(*first, *other))
        .then_some(*first)
}

pub(crate) struct Resolver<'a> {
    pub feature: &'a Feature,
    pub inputs: &'a Inputs<'a>,
}

impl Resolver<'_> {
    fn error(&self, reason: String, remedy: String, fix: FeatureId) -> Failure {
        Failure::Error(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(fix)),
            constraints: Vec::new(),
        })
    }

    fn own_error(&self, reason: String, remedy: &str) -> Failure {
        self.error(reason, remedy.to_owned(), self.feature.id())
    }

    fn datum(&self, feature: FeatureId) -> Result<DatumResult, Failure> {
        let name = feature_name(self.inputs.document, feature);
        match self.inputs.features.get(&feature).map(AsRef::as_ref) {
            Some(FeatureResult::Datum(result)) => Ok(*result),
            Some(_) => Err(self.own_error(
                format!("{name} is not a plane or an axis."),
                "Choose a plane or an axis instead.",
            )),
            None => Err(self.error(
                format!("It uses {name}, which has an error."),
                format!("Fix {name} first."),
                feature,
            )),
        }
    }

    fn body(&self, body: FeatureId) -> Result<&Solid, Failure> {
        self.inputs.body(body).ok_or_else(|| {
            let name = feature_name(self.inputs.document, body);
            self.error(
                format!("The body made by {name} has no shape."),
                format!("Fix {name} first."),
                body,
            )
        })
    }

    pub fn plane(&self, reference: &PlaneReference) -> Result<Plane, Failure> {
        match reference {
            PlaneReference::Principal(plane) => Ok(plane.plane()),
            PlaneReference::Datum(feature) => self.datum(*feature)?.plane().ok_or_else(|| {
                self.own_error(
                    format!(
                        "{} is an axis, not a plane.",
                        feature_name(self.inputs.document, *feature)
                    ),
                    "Choose a plane instead.",
                )
            }),
            PlaneReference::Face(attachment) => {
                let solid = self.body(attachment.body)?;
                let body = feature_name(self.inputs.document, attachment.body);
                attachment.resolve(solid).map_err(|error| {
                    let reason = match error {
                        AttachmentError::Missing => {
                            format!("The face it is based on is no longer part of {body}.")
                        }
                        AttachmentError::Ambiguous => format!(
                            "The face of {body} it is based on was split into parts that no \
                             longer lie in one plane."
                        ),
                        AttachmentError::NotFlat => {
                            format!("The face of {body} it is based on is no longer flat.")
                        }
                    };
                    self.own_error(reason, "Choose another plane or flat face for it.")
                })
            }
        }
    }

    pub fn axis(&self, reference: &AxisReference) -> Result<Ray, Failure> {
        let document = self.inputs.document;
        match reference {
            AxisReference::Principal(axis) => axis.ray().ok_or_else(|| {
                self.own_error(
                    "The axis has no direction.".to_owned(),
                    "Choose another axis.",
                )
            }),
            AxisReference::Datum(feature) => self.datum(*feature)?.axis().ok_or_else(|| {
                self.own_error(
                    format!(
                        "{} is a plane, not an axis.",
                        feature_name(document, *feature)
                    ),
                    "Choose an axis instead.",
                )
            }),
            AxisReference::Edge { body, edge } => {
                let solid = self.body(*body)?;
                let name = feature_name(document, *body);
                let pieces = match edge.resolve(solid) {
                    Ok(found) => vec![found],
                    Err(ReferenceError::Ambiguous(pieces)) => pieces,
                    Err(ReferenceError::Missing) => {
                        return Err(self.own_error(
                            format!("The edge it uses is no longer part of {name}."),
                            "Choose another edge or axis for it.",
                        ));
                    }
                };
                one_line(pieces.iter().map(|piece| edge_ray(solid, *piece))).ok_or_else(|| {
                    self.own_error(
                        format!("The edge of {name} it uses is no longer straight."),
                        "Choose a straight edge or another axis for it.",
                    )
                })
            }
            AxisReference::Face { body, face } => {
                let solid = self.body(*body)?;
                let described = describe_origin(document, face.origin());
                let pieces = match face.resolve(solid) {
                    Ok(found) => vec![found],
                    Err(ReferenceError::Ambiguous(pieces)) => pieces,
                    Err(ReferenceError::Missing) => {
                        return Err(self.own_error(
                            format!(
                                "{described} is no longer part of {}.",
                                feature_name(document, *body)
                            ),
                            "Choose another face or axis for it.",
                        ));
                    }
                };
                one_line(pieces.iter().map(|piece| face_axis(solid, *piece))).ok_or_else(|| {
                    self.own_error(
                        format!("{described} is no longer round about an axis."),
                        "Choose a cylindrical or conical face, or another axis.",
                    )
                })
            }
        }
    }

    pub fn value(
        &self,
        expression: &Expression,
        what: &str,
        dimension: Dimension,
    ) -> Result<f64, Failure> {
        let example = match dimension {
            Dimension::ANGLE => "an angle, such as 30 deg",
            Dimension::NONE => "a plain number, such as 4",
            _ => "a length, such as 10 mm",
        };
        expression
            .evaluate_as(dimension, &|id| self.inputs.parameters.value(id))
            .map_err(|error| {
                let (remedy, fix) = match &error {
                    EvalError::ParameterFailed { id, name } => (
                        format!("Fix {name} under Parameters, or edit the {what}."),
                        FixTarget::Parameter(*id),
                    ),
                    EvalError::WrongKind { .. } => (
                        format!("Edit the {what} so it gives {example}."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                    _ => (
                        format!("Edit the {what} or the parameters it uses."),
                        FixTarget::Feature(self.feature.id()),
                    ),
                };
                Failure::Error(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                })
            })
    }
}

fn intersection(first: Plane, second: Plane) -> Option<Ray> {
    if tolerance::parallel(first.normal(), second.normal()) {
        return None;
    }
    let direction = first.normal().cross(second.normal());
    let first_distance = first.normal().dot(first.origin());
    let second_distance = second.normal().dot(second.origin());
    let point = (second.normal().cross(direction) * first_distance
        + direction.cross(first.normal()) * second_distance)
        / direction.length_squared();
    Ray::new(point, direction)
}

pub(crate) fn evaluate(
    feature: &Feature,
    datum: &Datum,
    inputs: &Inputs<'_>,
) -> Result<FeatureResult, Failure> {
    let resolver = Resolver { feature, inputs };
    let document = inputs.document;
    let result = match datum {
        Datum::Plane(definition) => {
            let mut plane = resolver.plane(&definition.base)?;
            if let Some(rotation) = &definition.rotation {
                let axis = resolver.axis(&rotation.axis)?;
                if !tolerance::perpendicular(axis.direction(), plane.normal()) {
                    return Err(resolver.own_error(
                        format!(
                            "{} does not run along {}, so turning the plane about it cannot \
                             work.",
                            capitalized(&describe_axis(document, &rotation.axis)),
                            describe_plane(document, &definition.base)
                        ),
                        "Choose an axis that lies in the plane or runs parallel to it, such as \
                         an edge of the face.",
                    ));
                }
                let angle = resolver.value(&rotation.angle, "angle", Dimension::ANGLE)?;
                let transform = RigidTransform::rotation_about(
                    axis.origin(),
                    axis.direction(),
                    angle.to_radians(),
                )
                .ok_or_else(|| {
                    resolver.own_error(
                        "The rotation axis has no direction.".to_owned(),
                        "Choose another axis.",
                    )
                })?;
                let through_axis = Plane::from_frame(
                    plane.origin() + plane.normal() * plane.signed_distance(axis.origin()),
                    plane.normal(),
                    plane.x_axis(),
                )
                .unwrap_or(plane);
                plane = through_axis.transformed(&transform);
            }
            let offset = resolver.value(&definition.offset, "offset", Dimension::LENGTH)?;
            let moved = Plane::from_frame(
                plane.origin() + plane.normal() * offset,
                plane.normal(),
                plane.x_axis(),
            )
            .ok_or_else(|| {
                resolver.own_error(
                    "The plane could not be placed.".to_owned(),
                    "Change the offset or the angle.",
                )
            })?;
            DatumResult::Plane(moved)
        }
        Datum::Axis(DatumAxis::Along(reference)) => DatumResult::Axis(resolver.axis(reference)?),
        Datum::Axis(DatumAxis::Intersection(first, second)) => {
            let planes = (resolver.plane(first)?, resolver.plane(second)?);
            let line = intersection(planes.0, planes.1).ok_or_else(|| {
                resolver.own_error(
                    format!(
                        "{} and {} are parallel, so they do not meet in a line.",
                        capitalized(&describe_plane(document, first)),
                        describe_plane(document, second)
                    ),
                    "Choose two planes or flat faces that cross.",
                )
            })?;
            DatumResult::Axis(line)
        }
    };
    Ok(FeatureResult::Datum(result))
}

pub fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}
