use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId};
use caditor_geometry::{Plane, Point3, Ray, RigidTransform, Vector3};
use caditor_kernel::{
    Curve, EdgeId, EdgeReference, FaceId, FaceReference, LINEAR_RESOLUTION, ReferenceError, Solid,
    Surface, VertexId, VertexName, vertex_names,
};
use caditor_sketch::EntityId;

use crate::{
    attachment::{AttachmentError, FaceAttachment},
    datum_construction,
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

    pub fn in_frame(self, frame: &Plane) -> Option<Plane> {
        let placement = RigidTransform::from_frame(frame)?;
        Some(self.plane().transformed(&placement))
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

    pub fn in_frame(self, frame: &Plane) -> Option<Ray> {
        let placement = RigidTransform::from_frame(frame)?;
        Ray::new(frame.origin(), placement.apply_vector(self.direction()))
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
    Frame {
        frame: FeatureId,
        plane: PrincipalPlane,
    },
}

impl PlaneReference {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Face(attachment) => attachment.face.heap_size(),
            Self::Principal(_) | Self::Datum(_) | Self::Frame { .. } => 0,
        }
    }

    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(feature) => Some(*feature),
            Self::Principal(_) | Self::Face(_) | Self::Frame { .. } => None,
        }
    }

    pub fn frame(&self) -> Option<FeatureId> {
        match self {
            Self::Frame { frame, .. } => Some(*frame),
            Self::Principal(_) | Self::Datum(_) | Self::Face(_) => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Face(attachment) => Some(attachment.body),
            Self::Principal(_) | Self::Datum(_) | Self::Frame { .. } => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Face(attachment) => attachment.origin_features(),
            Self::Principal(_) | Self::Datum(_) | Self::Frame { .. } => BTreeSet::new(),
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
    Sketch {
        sketch: FeatureId,
        entity: EntityId,
    },
    Frame {
        frame: FeatureId,
        axis: PrincipalAxis,
    },
}

impl AxisReference {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Edge { .. } => size_of::<EdgeReference>(),
            Self::Face { face, .. } => face.heap_size(),
            Self::Principal(_) | Self::Datum(_) | Self::Sketch { .. } | Self::Frame { .. } => 0,
        }
    }

    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(feature) => Some(*feature),
            Self::Principal(_)
            | Self::Edge { .. }
            | Self::Face { .. }
            | Self::Sketch { .. }
            | Self::Frame { .. } => None,
        }
    }

    pub fn frame(&self) -> Option<FeatureId> {
        match self {
            Self::Frame { frame, .. } => Some(*frame),
            Self::Principal(_)
            | Self::Datum(_)
            | Self::Edge { .. }
            | Self::Face { .. }
            | Self::Sketch { .. } => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Edge { body, .. } | Self::Face { body, .. } => Some(*body),
            Self::Principal(_) | Self::Datum(_) | Self::Sketch { .. } | Self::Frame { .. } => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch { sketch, .. } => Some(*sketch),
            Self::Principal(_)
            | Self::Datum(_)
            | Self::Edge { .. }
            | Self::Face { .. }
            | Self::Frame { .. } => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Edge { edge, .. } => origins::of_edge(edge),
            Self::Face { face, .. } => origins::of_face(face).into_iter().collect(),
            Self::Principal(_) | Self::Datum(_) | Self::Sketch { .. } | Self::Frame { .. } => {
                BTreeSet::new()
            }
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
pub enum PointReference {
    Origin,
    Datum(FeatureId),
    Vertex {
        body: FeatureId,
        vertex: VertexName,
    },
    Centre {
        body: FeatureId,
        edge: Box<EdgeReference>,
    },
    SurfaceCentre {
        body: FeatureId,
        face: FaceReference,
    },
    Sketch {
        sketch: FeatureId,
        entity: EntityId,
    },
}

impl PointReference {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Centre { .. } => size_of::<EdgeReference>(),
            Self::SurfaceCentre { face, .. } => face.heap_size(),
            Self::Origin | Self::Datum(_) | Self::Vertex { .. } | Self::Sketch { .. } => 0,
        }
    }

    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(feature) => Some(*feature),
            Self::Origin
            | Self::Vertex { .. }
            | Self::Centre { .. }
            | Self::SurfaceCentre { .. }
            | Self::Sketch { .. } => None,
        }
    }

    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Vertex { body, .. }
            | Self::Centre { body, .. }
            | Self::SurfaceCentre { body, .. } => Some(*body),
            Self::Origin | Self::Datum(_) | Self::Sketch { .. } => None,
        }
    }

    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            Self::Sketch { sketch, .. } => Some(*sketch),
            Self::Origin
            | Self::Datum(_)
            | Self::Vertex { .. }
            | Self::Centre { .. }
            | Self::SurfaceCentre { .. } => None,
        }
    }

    pub fn origin_features(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Centre { edge, .. } => origins::of_edge(edge),
            Self::SurfaceCentre { face, .. } => origins::of_face(face).into_iter().collect(),
            Self::Origin | Self::Datum(_) | Self::Vertex { .. } | Self::Sketch { .. } => {
                BTreeSet::new()
            }
        }
    }

    pub fn capture_surface_centre(body: FeatureId, solid: &Solid, face: FaceId) -> Option<Self> {
        surface_centre(solid, face)?;
        Some(Self::SurfaceCentre {
            body,
            face: FaceReference::capture(solid, face)?,
        })
    }

    pub fn capture_centre(body: FeatureId, solid: &Solid, edge: EdgeId) -> Option<Self> {
        edge_centre(solid, edge)?;
        Some(Self::Centre {
            body,
            edge: Box::new(EdgeReference::capture(solid, edge)?),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatumPoint {
    pub base: PointReference,
    pub offset: [Expression; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurveStation {
    pub body: FeatureId,
    pub edge: Box<EdgeReference>,
    pub distance: Expression,
}

impl CurveStation {
    pub fn capture(
        body: FeatureId,
        solid: &Solid,
        edge: EdgeId,
        distance: Expression,
    ) -> Option<Self> {
        Some(Self {
            body,
            edge: Box::new(EdgeReference::capture(solid, edge)?),
            distance,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceTangent {
    pub body: FeatureId,
    pub face: FaceReference,
    pub toward: PointReference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlaneThrough {
    Points([PointReference; 3]),
    Midway(PlaneReference, PlaneReference),
    AxisAndPoint(AxisReference, PointReference),
    NormalTo(AxisReference, PointReference),
    Tangent(Box<FaceTangent>),
    SquareToCurve(Box<CurveStation>),
    Lines(AxisReference, AxisReference),
    TangentAt(Box<FaceTangent>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PointBy {
    LinesCross(AxisReference, AxisReference),
    AxisAndPlane(AxisReference, PlaneReference),
    ThreePlanes([PlaneReference; 3]),
    Along(Box<CurveStation>),
    EdgeMiddle {
        body: FeatureId,
        edge: Box<EdgeReference>,
    },
    FaceCentre {
        body: FeatureId,
        face: FaceReference,
    },
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
    Points(PointReference, PointReference),
    NormalTo(PlaneReference, PointReference),
    SquareToFace(Box<FaceTangent>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatumFrame {
    pub origin: PointReference,
    pub x_axis: AxisReference,
    pub plane: PlaneReference,
    pub reverse_x: bool,
    pub reverse_z: bool,
}

impl DatumFrame {
    pub fn world() -> Self {
        Self {
            origin: PointReference::Origin,
            x_axis: AxisReference::Principal(PrincipalAxis::X),
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            reverse_x: false,
            reverse_z: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Datum {
    Plane(DatumPlane),
    PlaneThrough(PlaneThrough),
    Axis(DatumAxis),
    Point(DatumPoint),
    PointBy(PointBy),
    Frame(Box<DatumFrame>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatumKind {
    Plane,
    Axis,
    Point,
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DatumResult {
    Plane(Plane),
    Axis(Ray),
    Point(Point3),
    Frame(Plane),
}

impl DatumResult {
    pub fn plane(&self) -> Option<Plane> {
        match self {
            Self::Plane(plane) => Some(*plane),
            Self::Axis(_) | Self::Point(_) | Self::Frame(_) => None,
        }
    }

    pub fn axis(&self) -> Option<Ray> {
        match self {
            Self::Axis(axis) => Some(*axis),
            Self::Plane(_) | Self::Point(_) | Self::Frame(_) => None,
        }
    }

    pub fn point(&self) -> Option<Point3> {
        match self {
            Self::Point(point) => Some(*point),
            Self::Plane(_) | Self::Axis(_) | Self::Frame(_) => None,
        }
    }

    pub fn frame(&self) -> Option<Plane> {
        match self {
            Self::Frame(frame) => Some(*frame),
            Self::Plane(_) | Self::Axis(_) | Self::Point(_) => None,
        }
    }
}

impl Datum {
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Plane(plane) => {
                plane.base.heap_size()
                    + plane.offset.heap_size()
                    + plane.rotation.as_ref().map_or(0, |rotation| {
                        rotation.axis.heap_size() + rotation.angle.heap_size()
                    })
            }
            Self::PlaneThrough(_)
            | Self::Axis(_)
            | Self::Point(_)
            | Self::PointBy(_)
            | Self::Frame(_) => self
                .planes()
                .into_iter()
                .map(PlaneReference::heap_size)
                .chain(self.axes().into_iter().map(AxisReference::heap_size))
                .chain(self.points().into_iter().map(PointReference::heap_size))
                .chain(self.expressions().into_iter().map(Expression::heap_size))
                .chain(self.held_references_heap_size())
                .sum(),
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind() {
            DatumKind::Plane => "Plane",
            DatumKind::Axis => "Axis",
            DatumKind::Point => "Point",
            DatumKind::Frame => "Coordinate system",
        }
    }

    pub fn kind(&self) -> DatumKind {
        match self {
            Self::Plane(_) | Self::PlaneThrough(_) => DatumKind::Plane,
            Self::Axis(_) => DatumKind::Axis,
            Self::Point(_) | Self::PointBy(_) => DatumKind::Point,
            Self::Frame(_) => DatumKind::Frame,
        }
    }

    fn held_references_heap_size(&self) -> Option<usize> {
        match self {
            Self::PlaneThrough(
                PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent),
            )
            | Self::Axis(DatumAxis::SquareToFace(tangent)) => Some(tangent.face.heap_size()),
            Self::PointBy(PointBy::FaceCentre { face, .. }) => Some(face.heap_size()),
            Self::PlaneThrough(PlaneThrough::SquareToCurve(_))
            | Self::PointBy(PointBy::Along(_) | PointBy::EdgeMiddle { .. }) => {
                Some(size_of::<EdgeReference>())
            }
            Self::Frame(_) => Some(size_of::<DatumFrame>()),
            _ => None,
        }
    }

    fn held_edge(&self) -> Option<(FeatureId, &EdgeReference)> {
        match self {
            Self::PointBy(PointBy::EdgeMiddle { body, edge }) => Some((*body, edge)),
            _ => None,
        }
    }

    fn held_face(&self) -> Option<(FeatureId, &FaceReference)> {
        match self {
            Self::PointBy(PointBy::FaceCentre { body, face }) => Some((*body, face)),
            _ => None,
        }
    }

    fn station(&self) -> Option<&CurveStation> {
        match self {
            Self::PlaneThrough(PlaneThrough::SquareToCurve(station))
            | Self::PointBy(PointBy::Along(station)) => Some(station),
            _ => None,
        }
    }

    pub fn is_plane(&self) -> bool {
        self.kind() == DatumKind::Plane
    }

    pub fn is_axis(&self) -> bool {
        self.kind() == DatumKind::Axis
    }

    pub fn is_point(&self) -> bool {
        self.kind() == DatumKind::Point
    }

    pub fn is_frame(&self) -> bool {
        self.kind() == DatumKind::Frame
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        self.kind() == other.kind()
    }

    fn expressions(&self) -> Vec<&Expression> {
        match self {
            Self::Plane(plane) => std::iter::once(&plane.offset)
                .chain(plane.rotation.as_ref().map(|rotation| &rotation.angle))
                .collect(),
            Self::Point(point) => point.offset.iter().collect(),
            Self::PlaneThrough(_) | Self::Axis(_) | Self::PointBy(_) | Self::Frame(_) => self
                .station()
                .map(|station| &station.distance)
                .into_iter()
                .collect(),
        }
    }

    pub(crate) fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match self {
            Self::Plane(plane) => std::iter::once(&mut plane.offset)
                .chain(plane.rotation.as_mut().map(|rotation| &mut rotation.angle))
                .collect(),
            Self::Point(point) => point.offset.iter_mut().collect(),
            Self::PlaneThrough(PlaneThrough::SquareToCurve(station))
            | Self::PointBy(PointBy::Along(station)) => vec![&mut station.distance],
            Self::PlaneThrough(_) | Self::Axis(_) | Self::PointBy(_) | Self::Frame(_) => Vec::new(),
        }
    }

    fn planes(&self) -> Vec<&PlaneReference> {
        match self {
            Self::Plane(plane) => vec![&plane.base],
            Self::Frame(frame) => vec![&frame.plane],
            Self::PlaneThrough(PlaneThrough::Midway(first, second))
            | Self::Axis(DatumAxis::Intersection(first, second)) => vec![first, second],
            Self::Axis(DatumAxis::NormalTo(plane, _))
            | Self::PointBy(PointBy::AxisAndPlane(_, plane)) => vec![plane],
            Self::PointBy(PointBy::ThreePlanes(planes)) => planes.iter().collect(),
            Self::PlaneThrough(
                PlaneThrough::Points(_)
                | PlaneThrough::AxisAndPoint(..)
                | PlaneThrough::NormalTo(..)
                | PlaneThrough::Tangent(_)
                | PlaneThrough::SquareToCurve(_)
                | PlaneThrough::Lines(..)
                | PlaneThrough::TangentAt(_),
            )
            | Self::Axis(
                DatumAxis::Along(_) | DatumAxis::Points(..) | DatumAxis::SquareToFace(_),
            )
            | Self::PointBy(
                PointBy::LinesCross(..)
                | PointBy::Along(_)
                | PointBy::EdgeMiddle { .. }
                | PointBy::FaceCentre { .. },
            )
            | Self::Point(_) => Vec::new(),
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
            Self::Frame(frame) => vec![&frame.x_axis],
            Self::Axis(DatumAxis::Along(axis))
            | Self::PlaneThrough(
                PlaneThrough::AxisAndPoint(axis, _) | PlaneThrough::NormalTo(axis, _),
            )
            | Self::PointBy(PointBy::AxisAndPlane(axis, _)) => {
                vec![axis]
            }
            Self::PlaneThrough(PlaneThrough::Lines(first, second))
            | Self::PointBy(PointBy::LinesCross(first, second)) => vec![first, second],
            Self::Axis(
                DatumAxis::Intersection(..)
                | DatumAxis::Points(..)
                | DatumAxis::NormalTo(..)
                | DatumAxis::SquareToFace(_),
            )
            | Self::PlaneThrough(
                PlaneThrough::Points(_)
                | PlaneThrough::Midway(..)
                | PlaneThrough::Tangent(_)
                | PlaneThrough::SquareToCurve(_)
                | PlaneThrough::TangentAt(_),
            )
            | Self::PointBy(
                PointBy::ThreePlanes(_)
                | PointBy::Along(_)
                | PointBy::EdgeMiddle { .. }
                | PointBy::FaceCentre { .. },
            )
            | Self::Point(_) => Vec::new(),
        }
    }

    pub fn points(&self) -> Vec<&PointReference> {
        match self {
            Self::Point(point) => vec![&point.base],
            Self::Frame(frame) => vec![&frame.origin],
            Self::PlaneThrough(PlaneThrough::Points(points)) => points.iter().collect(),
            Self::PlaneThrough(
                PlaneThrough::AxisAndPoint(_, point) | PlaneThrough::NormalTo(_, point),
            )
            | Self::Axis(DatumAxis::NormalTo(_, point)) => vec![point],
            Self::Axis(DatumAxis::Points(first, second)) => vec![first, second],
            Self::PlaneThrough(
                PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent),
            )
            | Self::Axis(DatumAxis::SquareToFace(tangent)) => vec![&tangent.toward],
            Self::Plane(_)
            | Self::PlaneThrough(
                PlaneThrough::Midway(..) | PlaneThrough::SquareToCurve(_) | PlaneThrough::Lines(..),
            )
            | Self::Axis(DatumAxis::Along(_) | DatumAxis::Intersection(..))
            | Self::PointBy(_) => Vec::new(),
        }
    }

    pub fn axis_sketches(&self) -> BTreeSet<FeatureId> {
        self.axes()
            .into_iter()
            .filter_map(AxisReference::sketch)
            .collect()
    }

    pub fn point_datums(&self) -> BTreeSet<FeatureId> {
        self.points()
            .into_iter()
            .filter_map(PointReference::datum)
            .collect()
    }

    pub fn point_sketches(&self) -> BTreeSet<FeatureId> {
        self.points()
            .into_iter()
            .filter_map(PointReference::sketch)
            .collect()
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

    pub fn frames(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .filter_map(PlaneReference::frame)
            .chain(self.axes().into_iter().filter_map(AxisReference::frame))
            .collect()
    }

    pub fn bodies(&self) -> BTreeSet<FeatureId> {
        self.planes()
            .into_iter()
            .filter_map(PlaneReference::body)
            .chain(self.axes().into_iter().filter_map(AxisReference::body))
            .chain(self.points().into_iter().filter_map(PointReference::body))
            .chain(self.station().map(|station| station.body))
            .chain(self.tangent().map(|tangent| tangent.body))
            .chain(self.held_edge().map(|(body, _)| body))
            .chain(self.held_face().map(|(body, _)| body))
            .collect()
    }

    fn tangent(&self) -> Option<&FaceTangent> {
        match self {
            Self::PlaneThrough(
                PlaneThrough::Tangent(tangent) | PlaneThrough::TangentAt(tangent),
            )
            | Self::Axis(DatumAxis::SquareToFace(tangent)) => Some(tangent),
            _ => None,
        }
    }

    pub fn features(&self) -> BTreeSet<FeatureId> {
        let mut used = self.plane_datums();
        used.extend(self.axis_datums());
        used.extend(self.axis_sketches());
        used.extend(self.point_datums());
        used.extend(self.point_sketches());
        used.extend(self.bodies());
        used.extend(self.frames());
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
            .chain(
                self.points()
                    .into_iter()
                    .flat_map(PointReference::origin_features),
            )
            .chain(
                self.station()
                    .into_iter()
                    .flat_map(|station| origins::of_edge(&station.edge)),
            )
            .chain(
                self.tangent()
                    .into_iter()
                    .flat_map(|tangent| origins::of_face(&tangent.face)),
            )
            .chain(
                self.held_edge()
                    .into_iter()
                    .flat_map(|(_, edge)| origins::of_edge(edge)),
            )
            .chain(
                self.held_face()
                    .into_iter()
                    .flat_map(|(_, face)| origins::of_face(face)),
            )
            .collect()
    }
}

pub(crate) fn feature_name(document: &Document, feature: FeatureId) -> String {
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
        PlaneReference::Frame { frame, plane } => {
            format!("the {} of {}", plane.name(), feature_name(document, *frame))
        }
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
        AxisReference::Sketch { sketch, entity } => {
            let label = document
                .feature(*sketch)
                .and_then(|feature| feature.kind.sketch())
                .map_or_else(
                    || "a line".to_owned(),
                    |definition| definition.entity_label(*entity),
                );
            format!("{label} of {}", feature_name(document, *sketch))
        }
        AxisReference::Frame { frame, axis } => {
            format!("the {} of {}", axis.name(), feature_name(document, *frame))
        }
    }
}

fn sketch_line(sketch: &caditor_sketch::Sketch, entity: EntityId) -> Option<Ray> {
    let (start, end) = sketch.line_endpoints(entity)?;
    let plane = sketch.plane();
    let (from, to) = (plane.to_world(start), plane.to_world(end));
    Ray::new(from, to - from)
}

pub fn describe_point(document: &Document, reference: &PointReference) -> String {
    match reference {
        PointReference::Origin => "the origin".to_owned(),
        PointReference::Datum(feature) => feature_name(document, *feature),
        PointReference::Vertex { body, .. } => {
            format!("a corner of {}", feature_name(document, *body))
        }
        PointReference::Centre { body, .. } => {
            format!(
                "the centre of a round edge of {}",
                feature_name(document, *body)
            )
        }
        PointReference::SurfaceCentre { face, .. } => {
            format!("the centre of {}", describe_origin(document, face.origin()))
        }
        PointReference::Sketch { sketch, entity } => {
            let label = document
                .feature(*sketch)
                .and_then(|feature| feature.kind.sketch())
                .map_or_else(
                    || "a point".to_owned(),
                    |definition| definition.entity_label(*entity),
                );
            format!("{label} of {}", feature_name(document, *sketch))
        }
    }
}

pub fn describe_curve(document: &Document, station: &CurveStation) -> String {
    format!("an edge of {}", feature_name(document, station.body))
}

pub fn describe_points(document: &Document, points: &[&PointReference]) -> String {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for point in points {
        let described = describe_point(document, point);
        match groups.iter_mut().find(|(text, _)| *text == described) {
            Some((_, count)) => *count += 1,
            None => groups.push((described, 1)),
        }
    }
    let phrases: Vec<String> = groups
        .into_iter()
        .map(
            |(text, count)| match (count, text.strip_prefix("a corner of ")) {
                (1, _) => text,
                (count, Some(body)) => format!("{} corners of {body}", count_word(count)),
                (count, None) => format!("{text} ({} times)", count_word(count)),
            },
        )
        .collect();
    crate::document::list_names(&phrases)
}

fn count_word(count: usize) -> String {
    match count {
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        other => other.to_string(),
    }
}

pub(crate) fn edge_centre(solid: &Solid, edge: EdgeId) -> Option<Point3> {
    match solid.edge(edge)?.curve() {
        Curve::Circle(circle) => Some(circle.center()),
        Curve::Ellipse(ellipse) => Some(ellipse.center()),
        _ => None,
    }
}

pub(crate) fn surface_centre(solid: &Solid, face: FaceId) -> Option<Point3> {
    match solid.face(face)?.surface() {
        Surface::Sphere(surface) => Some(surface.center()),
        Surface::Torus(surface) => Some(surface.frame().origin()),
        _ => None,
    }
}

fn vertex_named(solid: &Solid, vertex: VertexName) -> Result<VertexId, ReferenceError<VertexId>> {
    let found: Vec<VertexId> = vertex_names(solid)
        .iter()
        .filter(|(_, name)| **name == vertex)
        .map(|(id, _)| *id)
        .collect();
    match found.as_slice() {
        [one] => Ok(*one),
        [] => Err(ReferenceError::Missing),
        _ => Err(ReferenceError::Ambiguous(Vec::new())),
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
        AxisReference::Sketch { sketch, entity } => {
            let result = evaluation.feature(*sketch)?.result.as_deref()?.sketch()?;
            sketch_line(&result.geometry, *entity)
        }
        AxisReference::Frame { frame, axis } => {
            axis.in_frame(&displayed_frame(evaluation, *frame)?)
        }
    }
}

pub fn displayed_frame(evaluation: &Evaluation, frame: FeatureId) -> Option<Plane> {
    evaluation
        .feature(frame)?
        .result
        .as_deref()?
        .datum()?
        .frame()
}

pub fn displayed_plane(evaluation: &Evaluation, reference: &PlaneReference) -> Option<Plane> {
    match reference {
        PlaneReference::Principal(plane) => Some(plane.plane()),
        PlaneReference::Datum(feature) => evaluation
            .feature(*feature)?
            .result
            .as_deref()?
            .datum()?
            .plane(),
        PlaneReference::Face(attachment) => {
            let solid = evaluation.body(attachment.body)?;
            attachment.resolve(solid).ok()
        }
        PlaneReference::Frame { frame, plane } => {
            plane.in_frame(&displayed_frame(evaluation, *frame)?)
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
    pub(crate) fn error(&self, reason: String, remedy: String, fix: FeatureId) -> Failure {
        Failure::Error(Box::new(FeatureError {
            reason,
            remedy,
            fix: Some(FixTarget::Feature(fix)),
            constraints: Vec::new(),
            place: None,
        }))
    }

    pub(crate) fn own_error(&self, reason: String, remedy: &str) -> Failure {
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

    pub(crate) fn frame(&self, feature: FeatureId) -> Result<Plane, Failure> {
        self.datum(feature)?.frame().ok_or_else(|| {
            self.own_error(
                format!(
                    "{} is not a coordinate system.",
                    feature_name(self.inputs.document, feature)
                ),
                "Choose a coordinate system instead.",
            )
        })
    }

    pub(crate) fn body(&self, body: FeatureId) -> Result<&Solid, Failure> {
        self.inputs
            .body(body)
            .ok_or_else(|| self.inputs.missing_body(body))
    }

    pub fn plane(&self, reference: &PlaneReference) -> Result<Plane, Failure> {
        match reference {
            PlaneReference::Principal(plane) => Ok(plane.plane()),
            PlaneReference::Datum(feature) => self.datum(*feature)?.plane().ok_or_else(|| {
                self.own_error(
                    format!(
                        "{} is not a plane.",
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
            PlaneReference::Frame { frame, plane } => {
                plane.in_frame(&self.frame(*frame)?).ok_or_else(|| {
                    self.own_error(
                        format!(
                            "The {} of {} could not be placed.",
                            plane.name(),
                            feature_name(self.inputs.document, *frame)
                        ),
                        "Choose another plane.",
                    )
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
                    format!("{} is not an axis.", feature_name(document, *feature)),
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
            AxisReference::Sketch { sketch, entity } => {
                let name = feature_name(document, *sketch);
                let result = self
                    .inputs
                    .features
                    .get(sketch)
                    .and_then(|result| result.sketch())
                    .ok_or_else(|| {
                        self.error(
                            format!("It uses a line of {name}, which has an error."),
                            format!("Fix {name} first."),
                            *sketch,
                        )
                    })?;
                sketch_line(&result.geometry, *entity).ok_or_else(|| {
                    self.own_error(
                        format!(
                            "The line of {name} it uses no longer exists or is no longer a line."
                        ),
                        "Choose another line or axis for it.",
                    )
                })
            }
            AxisReference::Frame { frame, axis } => {
                axis.in_frame(&self.frame(*frame)?).ok_or_else(|| {
                    self.own_error(
                        format!(
                            "The {} of {} could not be placed.",
                            axis.name(),
                            feature_name(document, *frame)
                        ),
                        "Choose another axis.",
                    )
                })
            }
        }
    }

    pub fn point(&self, reference: &PointReference) -> Result<Point3, Failure> {
        let document = self.inputs.document;
        match reference {
            PointReference::Origin => Ok(Point3::ZERO),
            PointReference::Datum(feature) => self.datum(*feature)?.point().ok_or_else(|| {
                self.own_error(
                    format!("{} is not a point.", feature_name(document, *feature)),
                    "Choose a point instead.",
                )
            }),
            PointReference::Vertex { body, vertex } => {
                let solid = self.body(*body)?;
                let name = feature_name(document, *body);
                match vertex_named(solid, *vertex) {
                    Ok(found) => solid
                        .vertex(found)
                        .map(|found| found.point())
                        .ok_or_else(|| {
                            self.own_error(
                                format!("The corner it uses is no longer part of {name}."),
                                "Choose another point for it.",
                            )
                        }),
                    Err(ReferenceError::Missing) => Err(self.own_error(
                        format!("The corner it uses is no longer part of {name}."),
                        "Choose another point for it.",
                    )),
                    Err(ReferenceError::Ambiguous(_)) => Err(self.own_error(
                        format!(
                            "The corner of {name} it uses is now several corners, so it is \
                             unclear which one to follow."
                        ),
                        "Choose the point again.",
                    )),
                }
            }
            PointReference::Centre { body, edge } => {
                let solid = self.body(*body)?;
                let name = feature_name(document, *body);
                let pieces = match edge.resolve(solid) {
                    Ok(found) => vec![found],
                    Err(ReferenceError::Ambiguous(pieces)) => pieces,
                    Err(ReferenceError::Missing) => {
                        return Err(self.own_error(
                            format!("The round edge it uses is no longer part of {name}."),
                            "Choose another point for it.",
                        ));
                    }
                };
                let centres: Option<Vec<Point3>> = pieces
                    .iter()
                    .map(|piece| edge_centre(solid, *piece))
                    .collect();
                match centres.as_deref() {
                    Some([first, rest @ ..])
                        if rest
                            .iter()
                            .all(|other| other.distance(*first) <= LINEAR_RESOLUTION) =>
                    {
                        Ok(*first)
                    }
                    _ => Err(self.own_error(
                        format!("The edge of {name} it uses is no longer round."),
                        "Choose a circular edge or another point for it.",
                    )),
                }
            }
            PointReference::SurfaceCentre { body, face } => {
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
                            "Choose another face or point for it.",
                        ));
                    }
                };
                let centres: Option<Vec<Point3>> = pieces
                    .iter()
                    .map(|piece| surface_centre(solid, *piece))
                    .collect();
                match centres.as_deref() {
                    Some([first, rest @ ..])
                        if rest
                            .iter()
                            .all(|other| other.distance(*first) <= LINEAR_RESOLUTION) =>
                    {
                        Ok(*first)
                    }
                    _ => Err(self.own_error(
                        format!("{described} is no longer a sphere or a torus."),
                        "Choose a spherical or toroidal face, or another point for it.",
                    )),
                }
            }
            PointReference::Sketch { sketch, entity } => {
                let name = feature_name(document, *sketch);
                let result = self
                    .inputs
                    .features
                    .get(sketch)
                    .and_then(|result| result.sketch())
                    .ok_or_else(|| {
                        self.error(
                            format!("It uses a point of {name}, which has an error."),
                            format!("Fix {name} first."),
                            *sketch,
                        )
                    })?;
                let point = result.geometry.point(*entity).ok_or_else(|| {
                    self.own_error(
                        format!("The point of {name} it uses no longer exists."),
                        "Choose another point for it.",
                    )
                })?;
                Ok(result.geometry.plane().to_world(point))
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
                Failure::Error(Box::new(FeatureError {
                    reason: format!("The {what} cannot be evaluated: {error}."),
                    remedy,
                    fix: Some(fix),
                    constraints: Vec::new(),
                    place: None,
                }))
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

fn plane_through(resolver: &Resolver<'_>, through: &PlaneThrough) -> Result<Plane, Failure> {
    let document = resolver.inputs.document;
    match through {
        PlaneThrough::Points(references) => {
            let [first, second, third] = [
                resolver.point(&references[0])?,
                resolver.point(&references[1])?,
                resolver.point(&references[2])?,
            ];
            let along = second - first;
            let normal = along.cross(third - first);
            let spread = along.length().max((third - first).length());
            if normal.length() <= LINEAR_RESOLUTION * spread.max(1.0) {
                return Err(resolver.own_error(
                    "Its three points lie on one line, or two of them coincide, so no single \
                     plane passes through them."
                        .to_owned(),
                    "Choose three points that do not lie on one line.",
                ));
            }
            Plane::with_x_axis(first, normal, along).ok_or_else(|| {
                resolver.own_error(
                    "The plane could not be placed through its points.".to_owned(),
                    "Choose three points farther apart.",
                )
            })
        }
        PlaneThrough::Midway(first, second) => {
            let (one, other) = (resolver.plane(first)?, resolver.plane(second)?);
            let facing = if one.normal().dot(other.normal()) < 0.0 {
                -other.normal()
            } else {
                other.normal()
            };
            if tolerance::parallel(one.normal(), facing) {
                let gap = one.signed_distance(other.origin());
                return Plane::from_frame(
                    one.origin() + one.normal() * (gap / 2.0),
                    one.normal(),
                    one.x_axis(),
                )
                .ok_or_else(|| {
                    resolver.own_error(
                        "The plane could not be placed between the two planes.".to_owned(),
                        "Choose two other planes.",
                    )
                });
            }
            let line = intersection(one, other).ok_or_else(|| {
                resolver.own_error(
                    format!(
                        "{} and {} could not be halved.",
                        capitalized(&describe_plane(document, first)),
                        describe_plane(document, second)
                    ),
                    "Choose two other planes.",
                )
            })?;
            let normal = one.normal() + facing;
            Plane::with_x_axis(line.origin(), normal, line.direction()).ok_or_else(|| {
                resolver.own_error(
                    "The plane could not be placed between the two planes.".to_owned(),
                    "Choose two other planes.",
                )
            })
        }
        PlaneThrough::AxisAndPoint(axis, point) => {
            let line = resolver.axis(axis)?;
            let at = resolver.point(point)?;
            let normal = line.direction().cross(at - line.origin());
            if normal.length() <= LINEAR_RESOLUTION {
                return Err(resolver.own_error(
                    format!(
                        "{} lies on {}, so no single plane is fixed by them.",
                        capitalized(&describe_point(document, point)),
                        describe_axis(document, axis)
                    ),
                    "Choose a point off the axis.",
                ));
            }
            Plane::with_x_axis(line.origin(), normal, line.direction()).ok_or_else(|| {
                resolver.own_error(
                    "The plane could not be placed through the axis and the point.".to_owned(),
                    "Choose another axis or point.",
                )
            })
        }
        PlaneThrough::NormalTo(axis, point) => {
            let line = resolver.axis(axis)?;
            let at = resolver.point(point)?;
            Plane::new(at, line.direction()).ok_or_else(|| {
                resolver.own_error(
                    "The plane could not be placed square to the axis.".to_owned(),
                    "Choose another axis.",
                )
            })
        }
        PlaneThrough::Tangent(tangent) => datum_construction::tangent_plane(resolver, tangent),
        PlaneThrough::TangentAt(tangent) => datum_construction::tangent_plane_at(resolver, tangent),
        PlaneThrough::SquareToCurve(station) => {
            datum_construction::square_to_curve(resolver, station)
        }
        PlaneThrough::Lines(first, second) => {
            datum_construction::plane_through_lines(resolver, first, second)
        }
    }
}

fn axis_through(resolver: &Resolver<'_>, axis: &DatumAxis) -> Result<Ray, Failure> {
    let document = resolver.inputs.document;
    match axis {
        DatumAxis::Along(reference) => resolver.axis(reference),
        DatumAxis::Intersection(first, second) => {
            let planes = (resolver.plane(first)?, resolver.plane(second)?);
            intersection(planes.0, planes.1).ok_or_else(|| {
                resolver.own_error(
                    format!(
                        "{} and {} are parallel, so they do not meet in a line.",
                        capitalized(&describe_plane(document, first)),
                        describe_plane(document, second)
                    ),
                    "Choose two planes or flat faces that cross.",
                )
            })
        }
        DatumAxis::Points(first, second) => {
            let (from, to) = (resolver.point(first)?, resolver.point(second)?);
            if from.distance(to) <= LINEAR_RESOLUTION {
                return Err(resolver.own_error(
                    format!(
                        "{} and {} are at the same place, so no single axis passes through \
                         them.",
                        capitalized(&describe_point(document, first)),
                        describe_point(document, second)
                    ),
                    "Choose two points apart.",
                ));
            }
            Ray::new(from, to - from).ok_or_else(|| {
                resolver.own_error(
                    "The axis could not be placed through its points.".to_owned(),
                    "Choose two other points.",
                )
            })
        }
        DatumAxis::SquareToFace(tangent) => datum_construction::square_to_face(resolver, tangent),
        DatumAxis::NormalTo(plane, point) => {
            let square = resolver.plane(plane)?;
            Ray::new(resolver.point(point)?, square.normal()).ok_or_else(|| {
                resolver.own_error(
                    "The axis could not be placed square to the plane.".to_owned(),
                    "Choose another plane.",
                )
            })
        }
    }
}

fn datum_point(resolver: &Resolver<'_>, point: &DatumPoint) -> Result<Point3, Failure> {
    let base = resolver.point(&point.base)?;
    let mut offset = Vector3::ZERO;
    for (index, (expression, what)) in point
        .offset
        .iter()
        .zip(["offset along X", "offset along Y", "offset along Z"])
        .enumerate()
    {
        let value = resolver.value(expression, what, Dimension::LENGTH)?;
        if let Some(slot) = offset.as_mut().get_mut(index) {
            *slot = value;
        }
    }
    let at = base + offset;
    if !at.is_finite() {
        return Err(resolver.own_error(
            "The point could not be placed.".to_owned(),
            "Change its offsets.",
        ));
    }
    Ok(at)
}

pub(crate) fn evaluate(
    feature: &Feature,
    datum: &Datum,
    inputs: &Inputs<'_>,
) -> Result<FeatureResult, Failure> {
    let resolver = Resolver { feature, inputs };
    let document = inputs.document;
    let result = match datum {
        Datum::PlaneThrough(through) => DatumResult::Plane(plane_through(&resolver, through)?),
        Datum::Point(point) => DatumResult::Point(datum_point(&resolver, point)?),
        Datum::PointBy(by) => DatumResult::Point(datum_construction::point_by(&resolver, by)?),
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
        Datum::Axis(axis) => DatumResult::Axis(axis_through(&resolver, axis)?),
        Datum::Frame(frame) => {
            DatumResult::Frame(datum_construction::coordinate_system(&resolver, frame)?)
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
