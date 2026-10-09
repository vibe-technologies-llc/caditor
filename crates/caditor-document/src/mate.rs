use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Ray, RigidTransform, Vector3};
use caditor_kernel::{FaceReference, Solid};

use crate::{
    attachment::{AttachmentError, FaceAttachment},
    datum::{AxisReference, PlaneReference, Resolver, feature_name},
    describe::describe_origin,
    document::{Feature, FeatureId},
    movement::Context,
    origins,
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
    solid::SolidResult,
};

const ALIGNED_ANGLE: f64 = 1e-12;
const DISTINCT_SINE: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct FaceMate {
    pub face: FaceReference,
    pub target: PlaneReference,
    pub distance: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisMate {
    pub axis: AxisReference,
    pub target: AxisReference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatePair {
    Faces(Box<FaceMate>),
    Axes(Box<AxisMate>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mate {
    pub body: FeatureId,
    pub pair: MatePair,
    pub flipped: bool,
}

impl Mate {
    pub fn faces(&self) -> Option<&FaceMate> {
        match &self.pair {
            MatePair::Faces(faces) => Some(faces),
            MatePair::Axes(_) => None,
        }
    }

    pub fn axes(&self) -> Option<&AxisMate> {
        match &self.pair {
            MatePair::Axes(axes) => Some(axes),
            MatePair::Faces(_) => None,
        }
    }

    pub fn distance(&self) -> Option<&Expression> {
        self.faces().map(|faces| &faces.distance)
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match &mut self.pair {
            MatePair::Faces(faces) => vec![&mut faces.distance],
            MatePair::Axes(_) => Vec::new(),
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.distance()
            .map(Expression::parameters)
            .unwrap_or_default()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.distance()
            .is_some_and(|distance| distance.uses(parameter))
    }

    pub fn heap_size(&self) -> usize {
        match &self.pair {
            MatePair::Faces(faces) => {
                size_of::<FaceMate>()
                    + faces.face.heap_size()
                    + faces.target.heap_size()
                    + faces.distance.heap_size()
            }
            MatePair::Axes(axes) => {
                size_of::<AxisMate>() + axes.axis.heap_size() + axes.target.heap_size()
            }
        }
    }

    fn axis_references(&self) -> Vec<&AxisReference> {
        self.axes()
            .map(|axes| vec![&axes.axis, &axes.target])
            .unwrap_or_default()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        let mut bodies: BTreeSet<FeatureId> = self
            .axis_references()
            .into_iter()
            .filter_map(AxisReference::body)
            .collect();
        bodies.extend(self.faces().and_then(|faces| faces.target.body()));
        bodies
    }

    pub fn plane_datums(&self) -> BTreeSet<FeatureId> {
        self.faces()
            .and_then(|faces| faces.target.datum())
            .into_iter()
            .collect()
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.axis_references()
            .into_iter()
            .filter_map(AxisReference::datum)
            .collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        let mut frames: BTreeSet<FeatureId> = self
            .axis_references()
            .into_iter()
            .filter_map(AxisReference::frame)
            .collect();
        frames.extend(self.faces().and_then(|faces| faces.target.frame()));
        frames
    }

    pub fn sketches(&self) -> BTreeSet<FeatureId> {
        self.axis_references()
            .into_iter()
            .filter_map(AxisReference::sketch)
            .collect()
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.bodies());
        used.extend(self.plane_datums());
        used.extend(self.axis_datums());
        used.extend(self.sketches());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match &self.pair {
            MatePair::Faces(faces) => {
                let mut origins = origins::of_face(&faces.face);
                origins.extend(faces.target.origin_features());
                origins
            }
            MatePair::Axes(axes) => {
                let mut origins = axes.axis.origin_features();
                origins.extend(axes.target.origin_features());
                origins
            }
        }
    }
}

pub(crate) fn evaluate(
    feature: &Feature,
    definition: &Mate,
    inputs: &Inputs<'_>,
    cancel: &CancelToken,
) -> Result<FeatureResult, Failure> {
    let context = Context { feature, inputs };
    let resolver = Resolver { feature, inputs };
    let body_name = feature_name(inputs.document, definition.body);
    let Some(solid) = inputs.body(definition.body) else {
        return Err(inputs.missing_body(definition.body));
    };
    let centre = solid
        .bounding_box()
        .map(|bounds| bounds.center())
        .ok_or_else(|| {
            context.error(
                format!("The body of {body_name} has no size to place."),
                format!("Fix the features that make {body_name} first."),
            )
        })?;
    let placement = match &definition.pair {
        MatePair::Faces(faces) => {
            let moving = moving_face(&context, definition.body, solid, &faces.face, &body_name)?;
            let target = resolver.plane(&faces.target)?;
            let distance = context.value(
                &faces.distance,
                Dimension::LENGTH,
                "distance between the faces",
            )?;
            face_placement(&moving, &target, distance, definition.flipped, centre)
        }
        MatePair::Axes(axes) => {
            let moving = resolver.axis(&axes.axis)?;
            let target = resolver.axis(&axes.target)?;
            axis_placement(moving, target, definition.flipped, centre)
        }
    };
    let far = || {
        context.error(
            format!("The mate would take the body of {body_name} too far from the origin."),
            "Choose faces or axes nearer the body, or a smaller distance.".to_owned(),
        )
    };
    let placement = placement.ok_or_else(far)?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let moved = solid.transformed(&placement).map_err(|_| far())?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        moved,
    )))
}

fn moving_face(
    context: &Context<'_>,
    body: FeatureId,
    solid: &Solid,
    face: &FaceReference,
    body_name: &str,
) -> Result<Plane, Failure> {
    let attachment = FaceAttachment {
        body,
        face: face.clone(),
    };
    attachment.resolve(solid).map_err(|error| {
        let described = describe_origin(context.inputs.document, face.origin());
        let reason = match error {
            AttachmentError::Missing => {
                format!("{described}, which it mates, is no longer part of {body_name}.")
            }
            AttachmentError::Ambiguous => format!(
                "{described}, which it mates, was split into parts that no longer lie in one \
                 plane."
            ),
            AttachmentError::NotFlat => {
                format!("{described}, which it mates, is no longer flat.")
            }
        };
        context.error(
            reason,
            format!("Choose another flat face of {body_name} to mate."),
        )
    })
}

fn turning(from: Vector3, to: Vector3, about: Point3, across: Vector3) -> Option<RigidTransform> {
    let from = from.try_normalize()?;
    let to = to.try_normalize()?;
    let cross = from.cross(to);
    let angle = cross.length().atan2(from.dot(to));
    if angle <= ALIGNED_ANGLE {
        return Some(RigidTransform::IDENTITY);
    }
    let axis = if cross.length() > DISTINCT_SINE {
        cross
    } else {
        across
    };
    RigidTransform::rotation_about(about, axis, angle)
}

fn face_placement(
    moving: &Plane,
    target: &Plane,
    distance: f64,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let normal = target.normal().try_normalize()?;
    let facing = if flipped { normal } else { -normal };
    let turned = turning(moving.normal(), facing, centre, moving.x_axis())?;
    let origin = turned.apply_point(moving.origin());
    let goal = target.origin() + normal * distance;
    let shift = RigidTransform::translation(normal * (goal - origin).dot(normal))?;
    Some(turned.then(&shift))
}

fn axis_placement(
    moving: Ray,
    target: Ray,
    flipped: bool,
    centre: Point3,
) -> Option<RigidTransform> {
    let along = moving.direction().try_normalize()?;
    let direction = target.direction().try_normalize()?;
    let direction = if flipped { -direction } else { direction };
    let pivot = moving.origin() + along * (centre - moving.origin()).dot(along);
    let turned = turning(along, direction, pivot, along.any_orthonormal_vector())?;
    let point = turned.apply_point(pivot);
    let foot = target.origin() + direction * (point - target.origin()).dot(direction);
    let shift = RigidTransform::translation(foot - point)?;
    Some(turned.then(&shift))
}
