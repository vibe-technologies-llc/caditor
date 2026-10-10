use std::collections::BTreeSet;

use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Ray, RigidTransform};
use caditor_kernel::{FaceId, FaceReference, LINEAR_RESOLUTION, ReferenceError, Solid, Surface};

use crate::{
    attachment::{AttachmentError, FaceAttachment},
    datum::{AxisReference, PlaneReference, PointReference, Resolver, feature_name},
    describe::describe_origin,
    document::{Feature, FeatureId},
    mate_placement::{self, FlushAndConcentric, Round, Unplaced},
    movement::Context,
    origins,
    recompute::{CancelToken, Failure, FeatureResult, Inputs},
    solid::SolidResult,
};

pub const MAX_MATE_ANGLE_DEGREES: f64 = 180.0;

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
pub struct FaceAxisMate {
    pub faces: FaceMate,
    pub axes: AxisMate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FacePair {
    pub face: FaceReference,
    pub target: PlaneReference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AngleSides {
    Faces(FacePair),
    Axes(AxisMate),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AngleMate {
    pub sides: AngleSides,
    pub angle: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PointTarget {
    Point(PointReference),
    Plane(PlaneReference),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PointMate {
    pub point: PointReference,
    pub target: PointTarget,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatePair {
    Faces(Box<FaceMate>),
    Axes(Box<AxisMate>),
    FaceAxis(Box<FaceAxisMate>),
    Angle(Box<AngleMate>),
    Tangent(Box<FacePair>),
    Point(Box<PointMate>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mate {
    pub body: FeatureId,
    pub pair: MatePair,
    pub flipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundFaceError {
    Missing,
    NotRound,
}

pub fn round_face(solid: &Solid, face: &FaceReference) -> Result<Round, RoundFaceError> {
    let pieces = match face.resolve(solid) {
        Ok(found) => vec![found],
        Err(ReferenceError::Ambiguous(pieces)) => pieces,
        Err(ReferenceError::Missing) => return Err(RoundFaceError::Missing),
    };
    let rounds: Option<Vec<Round>> = pieces.iter().map(|piece| round_of(solid, *piece)).collect();
    match rounds.as_deref() {
        Some([first, rest @ ..])
            if rest
                .iter()
                .all(|other| first.same_as(other, LINEAR_RESOLUTION)) =>
        {
            Ok(*first)
        }
        _ => Err(RoundFaceError::NotRound),
    }
}

fn round_of(solid: &Solid, face: FaceId) -> Option<Round> {
    match solid.face(face)?.surface() {
        Surface::Cylinder(cylinder) => Some(Round::Cylinder {
            axis: Ray::new(cylinder.frame().origin(), cylinder.frame().normal())?,
            radius: cylinder.radius(),
        }),
        Surface::Sphere(sphere) => Some(Round::Sphere {
            centre: sphere.center(),
            radius: sphere.radius(),
        }),
        _ => None,
    }
}

impl Mate {
    pub fn faces(&self) -> Option<&FaceMate> {
        match &self.pair {
            MatePair::Faces(faces) => Some(faces),
            MatePair::FaceAxis(both) => Some(&both.faces),
            MatePair::Axes(_) | MatePair::Angle(_) | MatePair::Tangent(_) | MatePair::Point(_) => {
                None
            }
        }
    }

    pub fn axes(&self) -> Option<&AxisMate> {
        match &self.pair {
            MatePair::Axes(axes) => Some(axes),
            MatePair::FaceAxis(both) => Some(&both.axes),
            MatePair::Angle(angle) => match &angle.sides {
                AngleSides::Axes(axes) => Some(axes),
                AngleSides::Faces(_) => None,
            },
            MatePair::Faces(_) | MatePair::Tangent(_) | MatePair::Point(_) => None,
        }
    }

    pub fn distance(&self) -> Option<&Expression> {
        self.faces().map(|faces| &faces.distance)
    }

    pub fn angle(&self) -> Option<&Expression> {
        match &self.pair {
            MatePair::Angle(angle) => Some(&angle.angle),
            MatePair::Faces(_)
            | MatePair::Axes(_)
            | MatePair::FaceAxis(_)
            | MatePair::Tangent(_)
            | MatePair::Point(_) => None,
        }
    }

    pub fn flips(&self) -> bool {
        match &self.pair {
            MatePair::Faces(_)
            | MatePair::Axes(_)
            | MatePair::FaceAxis(_)
            | MatePair::Tangent(_) => true,
            MatePair::Angle(_) | MatePair::Point(_) => false,
        }
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        self.distance().into_iter().chain(self.angle()).collect()
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match &mut self.pair {
            MatePair::Faces(faces) => vec![&mut faces.distance],
            MatePair::FaceAxis(both) => vec![&mut both.faces.distance],
            MatePair::Angle(angle) => vec![&mut angle.angle],
            MatePair::Axes(_) | MatePair::Tangent(_) | MatePair::Point(_) => Vec::new(),
        }
    }

    pub fn lengths_mut(&mut self) -> Vec<&mut Expression> {
        match &mut self.pair {
            MatePair::Faces(faces) => vec![&mut faces.distance],
            MatePair::FaceAxis(both) => vec![&mut both.faces.distance],
            MatePair::Axes(_) | MatePair::Angle(_) | MatePair::Tangent(_) | MatePair::Point(_) => {
                Vec::new()
            }
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

    pub fn moving_faces(&self) -> Vec<&FaceReference> {
        match &self.pair {
            MatePair::Faces(faces) => vec![&faces.face],
            MatePair::FaceAxis(both) => vec![&both.faces.face],
            MatePair::Angle(angle) => match &angle.sides {
                AngleSides::Faces(faces) => vec![&faces.face],
                AngleSides::Axes(_) => Vec::new(),
            },
            MatePair::Tangent(tangent) => vec![&tangent.face],
            MatePair::Axes(_) | MatePair::Point(_) => Vec::new(),
        }
    }

    pub fn planes(&self) -> Vec<&PlaneReference> {
        match &self.pair {
            MatePair::Faces(faces) => vec![&faces.target],
            MatePair::FaceAxis(both) => vec![&both.faces.target],
            MatePair::Angle(angle) => match &angle.sides {
                AngleSides::Faces(faces) => vec![&faces.target],
                AngleSides::Axes(_) => Vec::new(),
            },
            MatePair::Tangent(tangent) => vec![&tangent.target],
            MatePair::Point(point) => match &point.target {
                PointTarget::Plane(plane) => vec![plane],
                PointTarget::Point(_) => Vec::new(),
            },
            MatePair::Axes(_) => Vec::new(),
        }
    }

    pub fn axis_references(&self) -> Vec<&AxisReference> {
        self.axes()
            .map(|axes| vec![&axes.axis, &axes.target])
            .unwrap_or_default()
    }

    pub fn points(&self) -> Vec<&PointReference> {
        match &self.pair {
            MatePair::Point(point) => match &point.target {
                PointTarget::Point(target) => vec![&point.point, target],
                PointTarget::Plane(_) => vec![&point.point],
            },
            MatePair::Faces(_)
            | MatePair::Axes(_)
            | MatePair::FaceAxis(_)
            | MatePair::Angle(_)
            | MatePair::Tangent(_) => Vec::new(),
        }
    }

    pub fn heap_size(&self) -> usize {
        let boxed = match &self.pair {
            MatePair::Faces(_) => size_of::<FaceMate>(),
            MatePair::Axes(_) => size_of::<AxisMate>(),
            MatePair::FaceAxis(_) => size_of::<FaceAxisMate>(),
            MatePair::Angle(_) => size_of::<AngleMate>(),
            MatePair::Tangent(_) => size_of::<FacePair>(),
            MatePair::Point(_) => size_of::<PointMate>(),
        };
        boxed
            + self
                .moving_faces()
                .iter()
                .map(|face| face.heap_size())
                .sum::<usize>()
            + self
                .planes()
                .iter()
                .map(|plane| plane.heap_size())
                .sum::<usize>()
            + self
                .axis_references()
                .iter()
                .map(|axis| axis.heap_size())
                .sum::<usize>()
            + self
                .points()
                .iter()
                .map(|point| point.heap_size())
                .sum::<usize>()
            + self
                .expressions()
                .iter()
                .map(|expression| expression.heap_size())
                .sum::<usize>()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        let mut bodies: BTreeSet<FeatureId> = self
            .axis_references()
            .into_iter()
            .filter_map(AxisReference::body)
            .collect();
        bodies.extend(self.planes().into_iter().filter_map(PlaneReference::body));
        bodies.extend(self.points().into_iter().filter_map(PointReference::body));
        bodies
    }

    pub fn plane_datums(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .filter_map(PlaneReference::datum)
            .collect()
    }

    pub fn axis_datums(&self) -> BTreeSet<FeatureId> {
        self.axis_references()
            .into_iter()
            .filter_map(AxisReference::datum)
            .collect()
    }

    pub fn point_datums(&self) -> BTreeSet<FeatureId> {
        self.points()
            .into_iter()
            .filter_map(PointReference::datum)
            .collect()
    }

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        let mut frames: BTreeSet<FeatureId> = self
            .axis_references()
            .into_iter()
            .filter_map(AxisReference::frame)
            .collect();
        frames.extend(self.planes().into_iter().filter_map(PlaneReference::frame));
        frames.extend(self.points().into_iter().filter_map(PointReference::frame));
        frames
    }

    pub fn sketches(&self) -> BTreeSet<FeatureId> {
        let mut sketches: BTreeSet<FeatureId> = self
            .axis_references()
            .into_iter()
            .filter_map(AxisReference::sketch)
            .collect();
        sketches.extend(self.points().into_iter().filter_map(PointReference::sketch));
        sketches
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = BTreeSet::from([self.body]);
        used.extend(self.bodies());
        used.extend(self.plane_datums());
        used.extend(self.axis_datums());
        used.extend(self.point_datums());
        used.extend(self.sketches());
        used
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        let mut found: BTreeSet<FeatureId> = self
            .moving_faces()
            .into_iter()
            .flat_map(origins::of_face)
            .collect();
        found.extend(
            self.planes()
                .into_iter()
                .flat_map(PlaneReference::origin_features),
        );
        found.extend(
            self.axis_references()
                .into_iter()
                .flat_map(AxisReference::origin_features),
        );
        found.extend(
            self.points()
                .into_iter()
                .flat_map(PointReference::origin_features),
        );
        found
    }
}

struct Placing<'a> {
    context: Context<'a>,
    resolver: Resolver<'a>,
    body: FeatureId,
    solid: &'a Solid,
    body_name: String,
    centre: Point3,
}

impl Placing<'_> {
    fn moving_plane(&self, face: &FaceReference) -> Result<Plane, Failure> {
        let attachment = FaceAttachment {
            body: self.body,
            face: face.clone(),
        };
        let body_name = &self.body_name;
        attachment.resolve(self.solid).map_err(|error| {
            let described = describe_origin(self.context.inputs.document, face.origin());
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
            self.context.error(
                reason,
                format!("Choose another flat face of {body_name} to mate."),
            )
        })
    }

    fn moving_round(&self, face: &FaceReference) -> Result<Round, Failure> {
        let body_name = &self.body_name;
        round_face(self.solid, face).map_err(|error| {
            let described = describe_origin(self.context.inputs.document, face.origin());
            let reason = match error {
                RoundFaceError::Missing => {
                    format!("{described}, which it mates, is no longer part of {body_name}.")
                }
                RoundFaceError::NotRound => {
                    format!("{described}, which it mates, is no longer a whole cylinder or sphere.")
                }
            };
            self.context.error(
                reason,
                format!(
                    "Choose a cylindrical or spherical face of {body_name} to rest on the plane."
                ),
            )
        })
    }

    fn distance(&self, distance: &Expression) -> Result<f64, Failure> {
        self.context
            .value(distance, Dimension::LENGTH, "distance between the faces")
    }

    fn angle(&self, angle: &Expression) -> Result<f64, Failure> {
        let degrees = self.context.value(angle, Dimension::ANGLE, "angle")?;
        if !(0.0..=MAX_MATE_ANGLE_DEGREES).contains(&degrees) {
            return Err(self.context.error(
                format!("The angle of the mate is {degrees} deg, outside 0 to 180 deg."),
                "Edit the angle to lie between 0 and 180 deg.".to_owned(),
            ));
        }
        Ok(degrees.to_radians())
    }

    fn placement(
        &self,
        pair: &MatePair,
        flipped: bool,
    ) -> Result<Result<RigidTransform, Unplaced>, Failure> {
        let (resolver, centre) = (&self.resolver, self.centre);
        Ok(match pair {
            MatePair::Faces(faces) => {
                let moving = self.moving_plane(&faces.face)?;
                let target = resolver.plane(&faces.target)?;
                let distance = self.distance(&faces.distance)?;
                mate_placement::faces(&moving, &target, distance, flipped, centre)
            }
            MatePair::Axes(axes) => {
                let moving = resolver.axis(&axes.axis)?;
                let target = resolver.axis(&axes.target)?;
                mate_placement::axes(moving, target, flipped, centre)
            }
            MatePair::FaceAxis(both) => {
                let face = self.moving_plane(&both.faces.face)?;
                let onto = resolver.plane(&both.faces.target)?;
                let distance = self.distance(&both.faces.distance)?;
                let axis = resolver.axis(&both.axes.axis)?;
                let along = resolver.axis(&both.axes.target)?;
                let pair = FlushAndConcentric {
                    face: &face,
                    onto: &onto,
                    distance,
                    axis,
                    along,
                };
                mate_placement::flush_and_concentric(&pair, flipped, centre)
            }
            MatePair::Angle(angle) => {
                let radians = self.angle(&angle.angle)?;
                match &angle.sides {
                    AngleSides::Faces(faces) => {
                        let moving = self.moving_plane(&faces.face)?;
                        let target = resolver.plane(&faces.target)?;
                        mate_placement::face_angle(&moving, &target, radians, centre)
                    }
                    AngleSides::Axes(axes) => {
                        let moving = resolver.axis(&axes.axis)?;
                        let target = resolver.axis(&axes.target)?;
                        mate_placement::axis_angle(moving, target, radians, centre)
                    }
                }
            }
            MatePair::Tangent(tangent) => {
                let round = self.moving_round(&tangent.face)?;
                let target = resolver.plane(&tangent.target)?;
                mate_placement::tangent(round, &target, flipped, centre)
            }
            MatePair::Point(point) => {
                let moving = resolver.point(&point.point)?;
                match &point.target {
                    PointTarget::Point(target) => {
                        mate_placement::point_onto_point(moving, resolver.point(target)?)
                    }
                    PointTarget::Plane(plane) => {
                        mate_placement::point_onto_plane(moving, &resolver.plane(plane)?)
                    }
                }
            }
        })
    }

    fn unplaced(&self, unplaced: Unplaced) -> Failure {
        let body_name = &self.body_name;
        match unplaced {
            Unplaced::TooFar => self.context.error(
                format!("The mate would take the body of {body_name} too far from the origin."),
                "Choose faces or axes nearer the body, or a smaller distance.".to_owned(),
            ),
            Unplaced::AxisAlongPlane => self.context.error(
                format!(
                    "The axis it mates onto runs along the plane it mates onto, so sliding \
                     {body_name} along the axis never brings the face onto the plane."
                ),
                "Choose a face and plane square to the axes, such as a shoulder and the face \
                 around a hole."
                    .to_owned(),
            ),
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
    let placing = Placing {
        context: Context { feature, inputs },
        resolver: Resolver { feature, inputs },
        body: definition.body,
        solid,
        body_name,
        centre,
    };
    let placement = placing
        .placement(&definition.pair, definition.flipped)?
        .map_err(|unplaced| placing.unplaced(unplaced))?;
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let moved = solid
        .transformed(&placement)
        .map_err(|_| placing.unplaced(Unplaced::TooFar))?;
    Ok(FeatureResult::Solid(SolidResult::new(
        definition.body,
        moved,
    )))
}
