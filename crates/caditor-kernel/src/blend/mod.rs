mod corner;
mod feet;
mod section;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Aabb, Plane, Point2, Point3, RigidTransform, Vector2, Vector3};
use thiserror::Error;

use self::{
    corner::Corner,
    section::{BLEND_CURVE, Blend, FIRST_SIDE, SECOND_SIDE, Section, SectionCurve, SectionSide},
};
use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    build::{AngularExtent, Axis2, LinearExtent, SweepError, extrude, revolve},
    curve::{Circle, Curve, Line},
    interrupt::{self, Interrupted},
    intersect::{boxes_overlap, intersect_curves},
    interval::Interval,
    naming::{EdgeNaming, EdgeReference, FaceName, FaceOrigin, ReferenceError},
    profile::{Profile, ProfileCurve, ProfileError, ProfileShape, Selection},
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceContainment, FaceId, Solid, SolidClassifier, VertexId},
};

const DIRECTION_TOLERANCE: f64 = 1e-7;
const SMOOTH_TOLERANCE: f64 = 1e-6;
const TANGENT_CONTINUITY: f64 = 1e-6;
const TANGENT_FACE_ANGLE: f64 = 0.01;
const TANGENT_FACE_SAMPLES: [f64; 3] = [0.25, 0.5, 0.75];
const PERPENDICULAR_END: f64 = 1e-9;
const SHALLOWEST_END: f64 = 0.1;
const END_MARGIN: f64 = 0.25;
const CUTTER_SCALE: f64 = 4.0;
const CLEARANCE: f64 = 0.25;
const SMALLEST_RADIUS: f64 = 10.0 * LINEAR_RESOLUTION;
const APART_TOOLS: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlendShape {
    Fillet { radius: f64 },
    Chamfer { distance: f64 },
}

impl BlendShape {
    fn size(self) -> f64 {
        match self {
            Self::Fillet { radius } => radius,
            Self::Chamfer { distance } => distance,
        }
    }

    fn origin(self, feature: u64) -> FaceOrigin {
        match self {
            Self::Fillet { .. } => FaceOrigin::Fillet { feature },
            Self::Chamfer { .. } => FaceOrigin::Chamfer { feature },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum BlendError {
    #[error("the size is not a finite number above 0.000001 mm")]
    InvalidSize,
    #[error("no edge is chosen")]
    NoEdges,
    #[error("edge {0:?} is not part of the solid")]
    MissingEdge(EdgeId),
    #[error(
        "edge {0:?} is neither straight along the faces it joins nor a circle around their common axis"
    )]
    Unsupported(EdgeId),
    #[error("the faces meet smoothly at edge {0:?}")]
    Smooth(EdgeId),
    #[error("the size does not fit on the faces next to edge {0:?}")]
    TooLarge(EdgeId),
    #[error("the blend around edge {0:?} would reach past its own start to close off its ends")]
    WrapsAround(EdgeId),
    #[error("edge {edge:?} ends at vertex {vertex:?} where the blend cannot be closed off")]
    UnsupportedEnd {
        edge: EdgeId,
        vertex: Option<VertexId>,
    },
    #[error("edge {0:?} could not be found again after the concave edges were filled")]
    Lost(EdgeId),
    #[error("after the concave edges were filled: {0}")]
    AfterFill(Box<BlendError>),
    #[error("the blend shape could not be built: {error}")]
    Profile {
        error: ProfileError,
        edge: Option<EdgeId>,
    },
    #[error("the blend shape could not be swept: {error}")]
    Sweep {
        error: SweepError,
        edge: Option<EdgeId>,
    },
    #[error("{error}")]
    Boolean {
        error: BooleanError,
        edge: Option<EdgeId>,
    },
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<ProfileError> for BlendError {
    fn from(error: ProfileError) -> Self {
        match error {
            ProfileError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Profile { error, edge: None },
        }
    }
}

impl From<SweepError> for BlendError {
    fn from(error: SweepError) -> Self {
        match error {
            SweepError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Sweep { error, edge: None },
        }
    }
}

impl From<BooleanError> for BlendError {
    fn from(error: BooleanError) -> Self {
        match error {
            BooleanError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Boolean { error, edge: None },
        }
    }
}

impl BlendError {
    fn remapped(self, map: impl Fn(EdgeId) -> Option<EdgeId>) -> Self {
        let Some(original) = self.edge().map(&map) else {
            return self;
        };
        let Some(edge) = original else {
            return Self::AfterFill(Box::new(self));
        };
        match self {
            Self::MissingEdge(_) => Self::MissingEdge(edge),
            Self::Unsupported(_) => Self::Unsupported(edge),
            Self::Smooth(_) => Self::Smooth(edge),
            Self::TooLarge(_) => Self::TooLarge(edge),
            Self::WrapsAround(_) => Self::WrapsAround(edge),
            Self::UnsupportedEnd { .. } => Self::UnsupportedEnd { edge, vertex: None },
            Self::Profile { error, .. } => Self::Profile {
                error,
                edge: Some(edge),
            },
            Self::Sweep { error, .. } => Self::Sweep {
                error,
                edge: Some(edge),
            },
            Self::Boolean { error, .. } => Self::Boolean {
                error,
                edge: Some(edge),
            },
            other => other,
        }
    }

    fn at(self, edge: Option<EdgeId>) -> Self {
        match self {
            Self::Profile { error, edge: None } => Self::Profile { error, edge },
            Self::Sweep { error, edge: None } => Self::Sweep { error, edge },
            Self::Boolean { error, edge: None } => Self::Boolean { error, edge },
            other => other,
        }
    }

    pub fn edge(&self) -> Option<EdgeId> {
        match self {
            Self::MissingEdge(edge)
            | Self::Unsupported(edge)
            | Self::Smooth(edge)
            | Self::TooLarge(edge)
            | Self::WrapsAround(edge)
            | Self::Lost(edge)
            | Self::UnsupportedEnd { edge, .. } => Some(*edge),
            Self::Profile { edge, .. } | Self::Sweep { edge, .. } | Self::Boolean { edge, .. } => {
                *edge
            }
            Self::InvalidSize | Self::AfterFill(_) | Self::NoEdges | Self::Cancelled(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Sweep {
    Along { length: f64 },
    Around { angle: f64, closed: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EdgeGeometry {
    edge: EdgeId,
    faces: [FaceId; 2],
    frame: Plane,
    sweep: Sweep,
    section: Section,
}

impl EdgeGeometry {
    fn place(&self, point: Point2, fraction: f64) -> Option<Point3> {
        let local = self.frame.to_world(point);
        match self.sweep {
            Sweep::Along { length } => Some(local + self.frame.normal() * (fraction * length)),
            Sweep::Around { angle, .. } => RigidTransform::rotation_about(
                self.frame.origin(),
                self.frame.y_axis(),
                fraction * angle,
            )
            .map(|turn| turn.apply_point(local)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    Flush,
    Extended(f64),
    Clipped { extension: f64, plane: Plane },
    Setback(f64),
}

impl End {
    fn extension(self) -> f64 {
        match self {
            Self::Flush => 0.0,
            Self::Extended(extension) | Self::Clipped { extension, .. } => extension,
            Self::Setback(distance) => -distance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Named {
    name: FaceName,
    origin: Option<FaceOrigin>,
}

fn face_normal(solid: &Solid, face: FaceId, point: Point3) -> Option<Vector3> {
    let definition = solid.face(face)?;
    let surface = definition.surface();
    let uv = surface.project(point, None);
    Some(surface.normal(uv.x, uv.y)? * definition.sense().sign())
}

fn edge_faces(solid: &Solid, edge: EdgeId) -> Vec<(FaceId, bool)> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|definition| definition.coedges())
        .filter_map(|coedge| {
            let face = solid.coedge_face(*coedge)?;
            Some((face, solid.coedge(*coedge)?.sense().is_same()))
        })
        .collect()
}

fn parallel_within(a: Vector3, b: Vector3, tolerance: f64) -> bool {
    a.cross(b).length() <= tolerance
}

fn smooth_within(a: Vector3, b: Vector3) -> bool {
    a.dot(b) > 0.0 && parallel_within(a, b, SMOOTH_TOLERANCE)
}

fn knife_within(a: Vector3, b: Vector3) -> bool {
    a.dot(b) < 0.0 && parallel_within(a, b, SMOOTH_TOLERANCE)
}

fn on_axis(point: Point3, origin: Point3, axis: Vector3) -> bool {
    (point - origin).cross(axis).length() <= LINEAR_RESOLUTION
}

fn flat(vector: Vector3, frame: &Plane) -> Vector2 {
    Vector2::new(vector.dot(frame.x_axis()), vector.dot(frame.y_axis()))
}

fn along_curve(surface: &Surface, direction: Vector3, frame: &Plane) -> Option<SectionCurve> {
    match surface {
        Surface::Plane(plane) => (plane.frame().normal().dot(direction).abs()
            <= DIRECTION_TOLERANCE)
            .then_some(SectionCurve::Line),
        Surface::Cylinder(cylinder) => {
            let axis = cylinder.frame();
            parallel_within(axis.normal(), direction, DIRECTION_TOLERANCE).then(|| {
                SectionCurve::Circle {
                    center: frame.to_local(axis.origin()),
                    radius: cylinder.radius(),
                }
            })
        }
        _ => None,
    }
}

fn around_curve(
    surface: &Surface,
    center: Point3,
    axis: Vector3,
    frame: &Plane,
) -> Option<SectionCurve> {
    let coaxial = |origin: Point3, normal: Vector3| {
        parallel_within(normal, axis, DIRECTION_TOLERANCE) && on_axis(origin, center, axis)
    };
    match surface {
        Surface::Plane(plane) => parallel_within(plane.frame().normal(), axis, DIRECTION_TOLERANCE)
            .then_some(SectionCurve::Line),
        Surface::Cylinder(cylinder) => {
            coaxial(cylinder.frame().origin(), cylinder.frame().normal())
                .then_some(SectionCurve::Line)
        }
        Surface::Cone(cone) => {
            coaxial(cone.frame().origin(), cone.frame().normal()).then_some(SectionCurve::Line)
        }
        Surface::Sphere(sphere) => {
            on_axis(sphere.center(), center, axis).then(|| SectionCurve::Circle {
                center: frame.to_local(sphere.center()),
                radius: sphere.radius(),
            })
        }
        Surface::Torus(torus) => {
            coaxial(torus.frame().origin(), torus.frame().normal()).then(|| SectionCurve::Circle {
                center: frame.to_local(torus.frame().origin())
                    + Vector2::new(torus.major_radius(), 0.0),
                radius: torus.minor_radius(),
            })
        }
        Surface::Extrusion(_) | Surface::Revolution(_) | Surface::BSpline(_) => None,
    }
}

fn analyze(solid: &Solid, edge: EdgeId) -> Result<EdgeGeometry, BlendError> {
    let definition = solid.edge(edge).ok_or(BlendError::MissingEdge(edge))?;
    let unsupported = || BlendError::Unsupported(edge);
    let sides = edge_faces(solid, edge);
    let [(first_face, first_same), (second_face, second_same)] = sides.as_slice() else {
        return Err(unsupported());
    };
    if first_face == second_face {
        return Err(unsupported());
    }
    let interval = definition.interval();
    let start = definition.curve().point(interval.start());
    let tangent = definition
        .curve()
        .evaluate(interval.start())
        .first
        .try_normalize()
        .ok_or_else(unsupported)?;
    let normals = [
        face_normal(solid, *first_face, start).ok_or_else(unsupported)?,
        face_normal(solid, *second_face, start).ok_or_else(unsupported)?,
    ];
    if smooth_within(normals[0], normals[1]) {
        return Err(BlendError::Smooth(edge));
    }
    if knife_within(normals[0], normals[1]) {
        return Err(BlendError::TooLarge(edge));
    }
    let inward = [
        normals[0].cross(if *first_same { tangent } else { -tangent }),
        normals[1].cross(if *second_same { tangent } else { -tangent }),
    ];
    let (frame, sweep, corner) = match definition.curve() {
        Curve::Line(_) => {
            let frame = Plane::from_frame(start, tangent, inward[0]).ok_or_else(unsupported)?;
            (
                frame,
                Sweep::Along {
                    length: interval.length(),
                },
                Point2::ZERO,
            )
        }
        Curve::Circle(circle) => {
            let axis = circle.frame().normal();
            let radial = (start - circle.center())
                .try_normalize()
                .ok_or_else(unsupported)?;
            let frame = Plane::from_frame(circle.center(), radial.cross(axis), radial)
                .ok_or_else(unsupported)?;
            (
                frame,
                Sweep::Around {
                    angle: interval.length(),
                    closed: definition.is_closed(),
                },
                Point2::new(circle.radius(), 0.0),
            )
        }
        _ => return Err(unsupported()),
    };
    let curve = |face: FaceId| -> Result<SectionCurve, BlendError> {
        let surface = solid.face(face).ok_or_else(unsupported)?.surface();
        match (definition.curve(), sweep) {
            (Curve::Circle(circle), Sweep::Around { .. }) => {
                around_curve(surface, circle.center(), circle.frame().normal(), &frame)
            }
            _ => along_curve(surface, tangent, &frame),
        }
        .ok_or_else(unsupported)
    };
    let side =
        |inward: Vector3, normal: Vector3, face: FaceId| -> Result<SectionSide, BlendError> {
            let flat_inward = flat(inward, &frame)
                .try_normalize()
                .ok_or_else(unsupported)?;
            let flat_normal = flat(normal, &frame)
                .try_normalize()
                .ok_or_else(unsupported)?;
            Ok(SectionSide {
                curve: curve(face)?,
                inward: flat_inward,
                normal: flat_normal,
            })
        };
    let sides = [
        side(inward[0], normals[0], *first_face)?,
        side(inward[1], normals[1], *second_face)?,
    ];
    let convex = normals[1].dot(inward[0]) < 0.0;
    Ok(EdgeGeometry {
        edge,
        faces: [*first_face, *second_face],
        frame,
        sweep,
        section: Section {
            corner,
            sides,
            convex,
        },
    })
}

struct Topology {
    around: BTreeMap<VertexId, Vec<EdgeId>>,
}

impl Topology {
    fn new(solid: &Solid) -> Self {
        let mut around: BTreeMap<VertexId, Vec<EdgeId>> = BTreeMap::new();
        for (id, edge) in solid.edges() {
            around.entry(edge.start()).or_default().push(id);
            if edge.end() != edge.start() {
                around.entry(edge.end()).or_default().push(id);
            }
        }
        Self { around }
    }

    fn edges_at(&self, vertex: VertexId) -> &[EdgeId] {
        self.around.get(&vertex).map_or(&[], Vec::as_slice)
    }
}

fn leaving(solid: &Solid, edge: EdgeId, vertex: VertexId) -> Option<Vector3> {
    let definition = solid.edge(edge)?;
    if definition.is_closed() {
        return None;
    }
    let interval = definition.interval();
    let (parameter, sign) = if definition.start() == vertex {
        (interval.start(), -1.0)
    } else if definition.end() == vertex {
        (interval.end(), 1.0)
    } else {
        return None;
    };
    Some(
        definition
            .curve()
            .evaluate(parameter)
            .first
            .try_normalize()?
            * sign,
    )
}

fn face_set(solid: &Solid, edge: EdgeId) -> BTreeSet<FaceId> {
    edge_faces(solid, edge)
        .into_iter()
        .map(|(face, _)| face)
        .collect()
}

fn continues(solid: &Solid, edge: EdgeId, other: EdgeId, vertex: VertexId) -> bool {
    if edge == other {
        return false;
    }
    let (Some(out), Some(other_out)) =
        (leaving(solid, edge, vertex), leaving(solid, other, vertex))
    else {
        return false;
    };
    out.dot(other_out) <= -1.0 + TANGENT_CONTINUITY
        && !face_set(solid, edge).is_disjoint(&face_set(solid, other))
}

fn is_sharp(solid: &Solid, edge: EdgeId, at: Point3) -> bool {
    let faces = edge_faces(solid, edge);
    let [(first, _), (second, _)] = faces.as_slice() else {
        return false;
    };
    match (
        face_normal(solid, *first, at),
        face_normal(solid, *second, at),
    ) {
        (Some(a), Some(b)) => !smooth_within(a, b),
        _ => false,
    }
}

fn propagate(
    solid: &Solid,
    topology: &Topology,
    edges: &[EdgeId],
) -> Result<Vec<EdgeId>, BlendError> {
    let mut chosen = BTreeSet::new();
    let mut queue = Vec::new();
    let mut smooth = None;
    for edge in edges {
        let Some(definition) = solid.edge(*edge) else {
            return Err(BlendError::MissingEdge(*edge));
        };
        let start = definition.curve().point(definition.interval().start());
        if !is_sharp(solid, *edge, start) {
            smooth = smooth.or(Some(*edge));
            continue;
        }
        if chosen.insert(*edge) {
            queue.push(*edge);
        }
    }
    if let (true, Some(edge)) = (chosen.is_empty(), smooth) {
        return Err(BlendError::Smooth(edge));
    }
    follow(solid, topology, &mut chosen, queue, is_sharp);
    Ok(chosen.into_iter().collect())
}

fn follow(
    solid: &Solid,
    topology: &Topology,
    chosen: &mut BTreeSet<EdgeId>,
    mut queue: Vec<EdgeId>,
    admits: impl Fn(&Solid, EdgeId, Point3) -> bool,
) {
    while let Some(edge) = queue.pop() {
        let Some(definition) = solid.edge(edge) else {
            continue;
        };
        for vertex in [definition.start(), definition.end()] {
            let Some(point) = solid.vertex(vertex).map(|vertex| vertex.point()) else {
                continue;
            };
            for other in topology.edges_at(vertex) {
                if !chosen.contains(other)
                    && continues(solid, edge, *other, vertex)
                    && admits(solid, *other, point)
                {
                    chosen.insert(*other);
                    queue.push(*other);
                }
            }
        }
    }
}

struct Ends {
    at: [End; 2],
    faces: [Option<FaceId>; 2],
}

struct Surroundings<'a> {
    solid: &'a Solid,
    topology: &'a Topology,
    chosen: &'a BTreeSet<EdgeId>,
    corners: &'a BTreeMap<VertexId, Corner>,
}

fn end_at(
    around: &Surroundings<'_>,
    geometry: &EdgeGeometry,
    vertex: VertexId,
    reach: f64,
) -> Result<(End, Option<FaceId>), BlendError> {
    let Surroundings {
        solid,
        topology,
        chosen,
        corners,
    } = *around;
    let edge = geometry.edge;
    if let Some(setback) = corners
        .get(&vertex)
        .and_then(|corner| corner.setbacks.get(&edge))
    {
        return Ok((End::Setback(*setback), None));
    }
    let refused = BlendError::UnsupportedEnd {
        edge,
        vertex: Some(vertex),
    };
    let out = leaving(solid, edge, vertex).ok_or(refused.clone())?;
    let continued = topology
        .edges_at(vertex)
        .iter()
        .any(|other| chosen.contains(other) && continues(solid, edge, *other, vertex));
    if continued {
        return Ok((End::Flush, None));
    }
    let own: BTreeSet<FaceId> = geometry.faces.into_iter().collect();
    let others: BTreeSet<FaceId> = topology
        .edges_at(vertex)
        .iter()
        .flat_map(|other| face_set(solid, *other))
        .filter(|face| !own.contains(face))
        .collect();
    let [end_face] = others.into_iter().collect::<Vec<_>>()[..] else {
        return Err(refused);
    };
    let point = solid.vertex(vertex).ok_or(refused.clone())?.point();
    let normal = face_normal(solid, end_face, point).ok_or(refused.clone())?;
    let cosine = normal.dot(out);
    let planar = matches!(
        solid.face(end_face).map(|face| face.surface()),
        Some(Surface::Plane(_))
    );
    if planar && cosine.abs() >= 1.0 - PERPENDICULAR_END {
        return Ok((End::Flush, Some(end_face)));
    }
    if cosine.abs() < SHALLOWEST_END {
        return Err(refused);
    }
    let extension = reach * ((1.0 - cosine * cosine).max(0.0).sqrt() / cosine.abs() + END_MARGIN);
    let free = geometry.section.convex == (cosine > 0.0);
    if free {
        return Ok((End::Extended(extension), Some(end_face)));
    }
    if !planar {
        return Err(refused);
    }
    let plane = Plane::new(point, normal * cosine.signum()).ok_or(refused)?;
    Ok((End::Clipped { extension, plane }, Some(end_face)))
}

fn ends(
    around: &Surroundings<'_>,
    geometry: &EdgeGeometry,
    reach: f64,
) -> Result<Ends, BlendError> {
    let definition = around
        .solid
        .edge(geometry.edge)
        .ok_or(BlendError::MissingEdge(geometry.edge))?;
    if definition.is_closed() {
        return Ok(Ends {
            at: [End::Flush, End::Flush],
            faces: [None, None],
        });
    }
    let (start, start_face) = end_at(around, geometry, definition.start(), reach)?;
    let (end, end_face) = end_at(around, geometry, definition.end(), reach)?;
    if let Sweep::Along { length } = geometry.sweep
        && length + start.extension() + end.extension() <= LINEAR_RESOLUTION
    {
        return Err(BlendError::TooLarge(geometry.edge));
    }
    Ok(Ends {
        at: [start, end],
        faces: [start_face, end_face],
    })
}

const FIT_FRACTIONS: [f64; 3] = [0.25, 0.5, 0.75];
const CROSSING_CLEARANCE: f64 = 10.0 * LINEAR_RESOLUTION;

fn foot_path(geometry: &EdgeGeometry, foot: Point2) -> Option<(Curve, Interval)> {
    let start = geometry.place(foot, 0.0)?;
    match geometry.sweep {
        Sweep::Along { length } => Some((
            Line::new(start, geometry.frame.normal()).ok()?.into(),
            Interval::new(0.0, length)?,
        )),
        Sweep::Around { angle, .. } => {
            let axis = geometry.frame.y_axis();
            let origin = geometry.frame.origin();
            let center = origin + axis * (start - origin).dot(axis);
            let radial = start - center;
            let frame = Plane::with_x_axis(center, axis, radial)?;
            Some((
                Circle::new(frame, radial.length()).ok()?.into(),
                Interval::new(0.0, angle)?,
            ))
        }
    }
}

fn crosses_boundary(solid: &Solid, face: FaceId, edge: EdgeId, foot: &(Curve, Interval)) -> bool {
    let (path, range) = foot;
    let Some(definition) = solid.edge(edge) else {
        return false;
    };
    let ends = [definition.start(), definition.end()];
    let path_ends = [path.point(range.start()), path.point(range.end())];
    let inside = |point: Point3| {
        path_ends
            .iter()
            .all(|end| end.distance(point) > CROSSING_CLEARANCE)
    };
    let mut uses: BTreeMap<EdgeId, usize> = BTreeMap::new();
    for used in solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
    {
        *uses.entry(used).or_default() += 1;
    }
    let reach = path.bounding_box(*range);
    uses.iter().any(|(other, used)| {
        let Some(other_definition) = solid.edge(*other) else {
            return false;
        };
        let incident =
            ends.contains(&other_definition.start()) || ends.contains(&other_definition.end());
        if *other == edge || incident || *used > 1 {
            return false;
        }
        let other_reach = other_definition
            .curve()
            .bounding_box(other_definition.interval());
        if !boxes_overlap(&reach, &other_reach, CROSSING_CLEARANCE) {
            return false;
        }
        let Ok(found) = intersect_curves(
            path,
            *range,
            other_definition.curve(),
            other_definition.interval(),
        ) else {
            return false;
        };
        !found.overlaps.is_empty() || found.points.iter().any(|hit| inside(hit.point))
    })
}

fn fits(
    classifier: &SolidClassifier<'_>,
    solid: &Solid,
    geometry: &EdgeGeometry,
    blend: &Blend,
) -> bool {
    geometry.faces.iter().zip(blend.feet).all(|(face, foot)| {
        let Some(surface) = solid.face(*face).map(|face| face.surface()) else {
            return false;
        };
        let crossed = foot_path(geometry, foot)
            .is_some_and(|path| crosses_boundary(solid, *face, geometry.edge, &path));
        !crossed
            && FIT_FRACTIONS.iter().all(|fraction| {
                let Some(point) = geometry.place(foot, *fraction) else {
                    return false;
                };
                let uv = surface.project(point, None);
                let contained = classifier.point_in_face(*face, uv);
                let accepted = if *fraction == 0.5 {
                    contained == Some(FaceContainment::Inside)
                } else {
                    matches!(
                        contained,
                        Some(FaceContainment::Inside | FaceContainment::OnBoundary)
                    )
                };
                surface.point_at(uv).distance(point) <= LINEAR_RESOLUTION && accepted
            })
    })
}

fn named(solid: &Solid, face: Option<FaceId>, fallback: Named) -> Named {
    face.and_then(|face| solid.face(face))
        .map_or(fallback, |face| Named {
            name: face.name(),
            origin: face.origin(),
        })
}

fn half_space(plane: &Plane, reach: f64, feature: u64, name: Named) -> Result<Solid, BlendError> {
    let corners = [
        Point2::new(-reach, -reach),
        Point2::new(reach, -reach),
        Point2::new(reach, reach),
        Point2::new(-reach, reach),
    ];
    let curves: Vec<ProfileCurve> = (0..corners.len())
        .filter_map(|index| {
            let from = *corners.get(index)?;
            let to = *corners.get((index + 1) % corners.len())?;
            Some(ProfileCurve::line(index as u64, from, to))
        })
        .collect();
    let regions = Profile::new(&curves)?.select(&Selection::EvenDepth)?;
    let solid = extrude(plane, &regions, LinearExtent::one_side(reach)?, feature)?;
    Ok(solid.renamed(|_, _| (name.name, name.origin)))
}

fn innermost(curves: &[ProfileCurve]) -> f64 {
    curves
        .iter()
        .map(|curve| match &curve.shape {
            ProfileShape::Line { start, end } => start.x.min(end.x),
            ProfileShape::Arc { center, start, end } => {
                let (from, to) = (*start - *center, *end - *center);
                let sweep = from.angle_to(to).rem_euclid(TAU);
                let to_axis = from.angle_to(Vector2::NEG_X).rem_euclid(TAU);
                let ends = start.x.min(end.x);
                if to_axis <= sweep {
                    ends.min(center.x - from.length())
                } else {
                    ends
                }
            }
            ProfileShape::Circle { center, radius } => center.x - radius,
            ProfileShape::Spline { control_points, .. } => control_points
                .iter()
                .map(|point| point.x)
                .fold(f64::INFINITY, f64::min),
            ProfileShape::Ellipse { .. } | ProfileShape::EllipticalArc { .. } => {
                curve.curve().map_or(f64::INFINITY, |(shape, range)| {
                    shape.bounding_box(range).min().x
                })
            }
        })
        .fold(f64::INFINITY, f64::min)
}

struct Tool {
    solid: Solid,
    convex: bool,
    edge: Option<EdgeId>,
}

fn tool(
    solid: &Solid,
    geometry: &EdgeGeometry,
    blend: &Blend,
    ends: &Ends,
    shape: BlendShape,
    feature: u64,
) -> Result<Tool, BlendError> {
    let edge_name = solid
        .edge(geometry.edge)
        .ok_or(BlendError::MissingEdge(geometry.edge))?
        .name();
    let own = Named {
        name: FaceName::blend(feature, edge_name),
        origin: Some(shape.origin(feature)),
    };
    let sides = [
        named(solid, Some(geometry.faces[0]), own),
        named(solid, Some(geometry.faces[1]), own),
    ];
    let caps = [
        named(solid, ends.faces[0], own),
        named(solid, ends.faces[1], own),
    ];
    let reach = geometry.section.reach(blend);
    let curves = if geometry.section.convex {
        geometry
            .section
            .cleared_profile(blend, CLEARANCE * reach)
            .ok_or(BlendError::TooLarge(geometry.edge))?
    } else {
        geometry.section.profile(blend)
    };
    let regions = Profile::new(&curves)?.select(&Selection::EvenDepth)?;
    let [start, end] = ends.at;
    let swept = match geometry.sweep {
        Sweep::Along { length } => extrude(
            &geometry.frame,
            &regions,
            LinearExtent::new(-start.extension(), length + end.extension())?,
            feature,
        )?,
        Sweep::Around { angle, closed } => {
            let axis = Axis2::new(Point2::ZERO, Vector2::Y)?;
            let extent = if closed {
                AngularExtent::full()
            } else {
                let innermost = innermost(&curves);
                if innermost <= SMALLEST_RADIUS {
                    return Err(BlendError::TooLarge(geometry.edge));
                }
                let (from, to) = (
                    -start.extension() / innermost,
                    angle + end.extension() / innermost,
                );
                if to - from >= TAU {
                    return Err(BlendError::WrapsAround(geometry.edge));
                }
                AngularExtent::new(from, to)?
            };
            revolve(&geometry.frame, &regions, axis, extent, feature)?
        }
    };
    let mut shaped = swept.renamed(|name, origin| {
        let chosen = match origin {
            Some(FaceOrigin::Side { entity, .. }) if entity == FIRST_SIDE => sides[0],
            Some(FaceOrigin::Side { entity, .. }) if entity == SECOND_SIDE => sides[1],
            Some(FaceOrigin::Side { entity, .. }) if entity == BLEND_CURVE => own,
            Some(FaceOrigin::Side { .. }) => own,
            Some(FaceOrigin::StartCap { .. }) => caps[0],
            Some(FaceOrigin::EndCap { .. }) => caps[1],
            _ => Named { name, origin },
        };
        (chosen.name, chosen.origin)
    });
    for (end, cap) in ends.at.iter().zip(caps) {
        if let End::Clipped { extension, plane } = end {
            let cutter = half_space(plane, CUTTER_SCALE * (reach + extension), feature, cap)?;
            shaped = boolean(&shaped, &cutter, BooleanOperation::Difference)?;
        }
    }
    Ok(Tool {
        solid: shaped,
        convex: geometry.section.convex,
        edge: Some(geometry.edge),
    })
}

pub fn blend_chain(solid: &Solid, edges: &[EdgeId]) -> Vec<EdgeId> {
    propagate(solid, &Topology::new(solid), edges).unwrap_or_default()
}

pub fn tangent_chain(solid: &Solid, edges: &[EdgeId]) -> Vec<EdgeId> {
    let mut chosen: BTreeSet<EdgeId> = edges
        .iter()
        .copied()
        .filter(|edge| solid.edge(*edge).is_some())
        .collect();
    let queue = chosen.iter().copied().collect();
    follow(
        solid,
        &Topology::new(solid),
        &mut chosen,
        queue,
        |_, _, _| true,
    );
    chosen.into_iter().collect()
}

pub fn tangent_faces(solid: &Solid, faces: &[FaceId]) -> Vec<FaceId> {
    let mut chosen: BTreeSet<FaceId> = faces
        .iter()
        .copied()
        .filter(|face| solid.face(*face).is_some())
        .collect();
    let mut queue: Vec<FaceId> = chosen.iter().copied().collect();
    while let Some(face) = queue.pop() {
        for edge in bounding_edges(solid, face) {
            if !meets_smoothly(solid, edge) {
                continue;
            }
            for (other, _) in edge_faces(solid, edge) {
                if chosen.insert(other) {
                    queue.push(other);
                }
            }
        }
    }
    chosen.into_iter().collect()
}

fn bounding_edges(solid: &Solid, face: FaceId) -> BTreeSet<EdgeId> {
    solid
        .face(face)
        .into_iter()
        .flat_map(|definition| definition.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|id| solid.coedge(*id))
        .map(|coedge| coedge.edge())
        .collect()
}

fn meets_smoothly(solid: &Solid, edge: EdgeId) -> bool {
    let faces = edge_faces(solid, edge);
    let ([(first, _), (second, _)], Some(definition)) = (faces.as_slice(), solid.edge(edge)) else {
        return false;
    };
    if first == second {
        return false;
    }
    let interval = definition.interval();
    TANGENT_FACE_SAMPLES.iter().all(|fraction| {
        let at = definition.curve().point(interval.at(*fraction));
        match (
            face_normal(solid, *first, at),
            face_normal(solid, *second, at),
        ) {
            (Some(a), Some(b)) => a.dot(b) > 0.0 && a.angle_between(b) <= TANGENT_FACE_ANGLE,
            _ => false,
        }
    })
}

pub fn blend(
    solid: &Solid,
    edges: &[EdgeId],
    shape: BlendShape,
    feature: u64,
) -> Result<Solid, BlendError> {
    blend_edges(solid, edges, shape, feature).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn blend_edges(
    solid: &Solid,
    edges: &[EdgeId],
    shape: BlendShape,
    feature: u64,
) -> Result<Solid, BlendError> {
    let size = shape.size();
    if !size.is_finite() || size <= LINEAR_RESOLUTION {
        return Err(BlendError::InvalidSize);
    }
    if edges.is_empty() {
        return Err(BlendError::NoEdges);
    }
    let topology = Topology::new(solid);
    let chosen = propagate(solid, &topology, edges)?;
    let analysed: BTreeMap<EdgeId, EdgeGeometry> = chosen
        .iter()
        .map(|edge| Ok((*edge, analyze(solid, *edge)?)))
        .collect::<Result<_, BlendError>>()?;
    let (convex, concave): (Vec<EdgeId>, Vec<EdgeId>) = chosen.iter().partition(|edge| {
        analysed
            .get(edge)
            .is_some_and(|geometry| geometry.section.convex)
    });
    if concave.is_empty() || convex.is_empty() {
        let geometries: Vec<EdgeGeometry> = analysed.into_values().collect();
        return apply_analysed(solid, &topology, &geometries, shape, feature);
    }
    let naming = EdgeNaming::new(solid);
    let references: Vec<(EdgeId, EdgeReference)> = convex
        .iter()
        .map(|edge| {
            EdgeReference::capture_in(&naming, *edge)
                .map(|reference| (*edge, reference))
                .ok_or(BlendError::MissingEdge(*edge))
        })
        .collect::<Result<_, _>>()?;
    let filling = propagate(solid, &topology, &concave)?
        .iter()
        .map(|edge| match analysed.get(edge) {
            Some(geometry) => Ok(*geometry),
            None => analyze(solid, *edge),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let filled = apply_analysed(solid, &topology, &filling, shape, feature)?;
    let original = find_again(&filled, &references)?;
    let remaining: Vec<EdgeId> = original.keys().copied().collect();
    apply(&filled, &remaining, shape, feature)
        .map_err(|error| error.remapped(|edge| original.get(&edge).copied()))
}

fn find_again(
    filled: &Solid,
    references: &[(EdgeId, EdgeReference)],
) -> Result<BTreeMap<EdgeId, EdgeId>, BlendError> {
    let naming = EdgeNaming::new(filled);
    let mut original = BTreeMap::new();
    for (edge, reference) in references {
        let found = match reference.resolve_by_names_in(&naming) {
            Ok(found) => vec![found],
            Err(ReferenceError::Ambiguous(candidates)) => candidates,
            Err(ReferenceError::Missing) => return Err(BlendError::Lost(*edge)),
        };
        for piece in found {
            original.insert(piece, *edge);
        }
    }
    Ok(original)
}

fn apply(
    solid: &Solid,
    edges: &[EdgeId],
    shape: BlendShape,
    feature: u64,
) -> Result<Solid, BlendError> {
    let topology = Topology::new(solid);
    let geometries = propagate(solid, &topology, edges)?
        .into_iter()
        .map(|edge| analyze(solid, edge))
        .collect::<Result<Vec<_>, _>>()?;
    apply_analysed(solid, &topology, &geometries, shape, feature)
}

fn apply_analysed(
    solid: &Solid,
    topology: &Topology,
    geometries: &[EdgeGeometry],
    shape: BlendShape,
    feature: u64,
) -> Result<Solid, BlendError> {
    let chosen: Vec<EdgeId> = geometries.iter().map(|geometry| geometry.edge).collect();
    let chosen_set: BTreeSet<EdgeId> = chosen.iter().copied().collect();
    let classifier = solid.classifier();
    let mut planned = Vec::with_capacity(chosen.len());
    for geometry in geometries {
        interrupt::check()?;
        let geometry = *geometry;
        let edge = geometry.edge;
        let blend = match shape {
            BlendShape::Fillet { radius } => geometry.section.fillet(radius),
            BlendShape::Chamfer { distance } => geometry.section.chamfer(distance),
        }
        .ok_or(BlendError::TooLarge(edge))?;
        if !fits(&classifier, solid, &geometry, &blend) {
            return Err(BlendError::TooLarge(edge));
        }
        let reach = geometry.section.reach(&blend);
        planned.push((geometry, blend, reach));
    }
    let feet: Vec<(EdgeGeometry, Blend)> = planned
        .iter()
        .map(|(geometry, blend, _)| (*geometry, *blend))
        .collect();
    if let Some(edge) = feet::crossing(solid, &feet) {
        return Err(BlendError::TooLarge(edge));
    }
    let corners = match shape {
        BlendShape::Fillet { radius } => {
            let convex: BTreeMap<EdgeId, bool> = planned
                .iter()
                .map(|(geometry, _, _)| (geometry.edge, geometry.section.convex))
                .collect();
            let vertices: BTreeSet<VertexId> = chosen
                .iter()
                .filter_map(|edge| solid.edge(*edge))
                .flat_map(|edge| [edge.start(), edge.end()])
                .collect();
            vertices
                .into_iter()
                .filter_map(|vertex| {
                    corner::find(solid, topology, &convex, radius, vertex)
                        .map(|corner| (vertex, corner))
                })
                .collect()
        }
        BlendShape::Chamfer { .. } => BTreeMap::new(),
    };
    let around = Surroundings {
        solid,
        topology,
        chosen: &chosen_set,
        corners: &corners,
    };
    let mut tools = Vec::with_capacity(planned.len() + corners.len());
    let mut blend_names = BTreeMap::new();
    for (geometry, blend, reach) in &planned {
        interrupt::check()?;
        let ends = ends(&around, geometry, *reach)?;
        let edge_name = solid
            .edge(geometry.edge)
            .ok_or(BlendError::MissingEdge(geometry.edge))?
            .name();
        blend_names.insert(geometry.edge, FaceName::blend(feature, edge_name));
        tools.push(
            tool(solid, geometry, blend, &ends, shape, feature)
                .map_err(|error| error.at(Some(geometry.edge)))?,
        );
    }
    for corner in corners.values() {
        interrupt::check()?;
        tools.push(Tool {
            solid: corner.tool(solid, feature, &blend_names)?,
            convex: true,
            edge: None,
        });
    }
    let (convex, concave): (Vec<Tool>, Vec<Tool>) = tools.into_iter().partition(|tool| tool.convex);
    let mut result = solid.clone();
    for (group, operation) in [
        (convex, BooleanOperation::Difference),
        (concave, BooleanOperation::Union),
    ] {
        let edges: Vec<Option<EdgeId>> = group.iter().map(|tool| tool.edge).collect();
        let solids: Vec<Solid> = group.into_iter().map(|tool| tool.solid).collect();
        result = applied(&result, &solids, &edges, operation)?;
    }
    Ok(result)
}

struct ToolGroup {
    solid: Solid,
    bounds: Option<Aabb>,
    members: Vec<usize>,
}

impl ToolGroup {
    fn apart_from(&self, other: &Self) -> Option<Aabb> {
        let (first, second) = self.bounds.zip(other.bounds)?;
        (!boxes_overlap(&first, &second, APART_TOOLS)).then(|| first.union(second))
    }

    fn united(&self, other: &Self) -> Result<Option<Self>, BooleanError> {
        let joined = match self.apart_from(other) {
            Some(bounds) => self
                .solid
                .beside(&other.solid)
                .map(|solid| (solid, Some(bounds))),
            None => match boolean(&self.solid, &other.solid, BooleanOperation::Union) {
                Ok(solid) => {
                    let bounds = solid.bounding_box();
                    Some((solid, bounds))
                }
                Err(error) => {
                    cancelled(error)?;
                    None
                }
            },
        };
        Ok(joined.map(|(solid, bounds)| Self {
            solid,
            bounds,
            members: [self.members.as_slice(), other.members.as_slice()].concat(),
        }))
    }
}

fn cancelled(error: BooleanError) -> Result<(), BooleanError> {
    match error {
        BooleanError::Cancelled(interrupted) => Err(BooleanError::Cancelled(interrupted)),
        _ => Ok(()),
    }
}

fn grouped(tools: &[Solid]) -> Result<Vec<ToolGroup>, BooleanError> {
    let mut groups: Vec<ToolGroup> = tools
        .iter()
        .enumerate()
        .map(|(index, solid)| ToolGroup {
            solid: solid.clone(),
            bounds: solid.bounding_box(),
            members: vec![index],
        })
        .collect();
    let mut refused: BTreeSet<(Vec<usize>, Vec<usize>)> = BTreeSet::new();
    loop {
        let mut progressed = false;
        let mut next = Vec::with_capacity(groups.len());
        let mut pending = groups.into_iter();
        while let Some(first) = pending.next() {
            let Some(second) = pending.next() else {
                next.push(first);
                break;
            };
            let pair = (first.members.clone(), second.members.clone());
            if refused.contains(&pair) {
                next.extend([first, second]);
                continue;
            }
            progressed = true;
            match first.united(&second)? {
                Some(group) => next.push(group),
                None => {
                    refused.insert(pair);
                    next.extend([first, second]);
                }
            }
        }
        groups = next;
        if groups.len() <= 1 || !progressed {
            return Ok(groups);
        }
        groups.rotate_left(1);
    }
}

fn applied(
    solid: &Solid,
    tools: &[Solid],
    edges: &[Option<EdgeId>],
    operation: BooleanOperation,
) -> Result<Solid, BlendError> {
    let mut result = solid.clone();
    for group in grouped(tools)? {
        if group.members.len() > 1 {
            match boolean(&result, &group.solid, operation) {
                Ok(next) => {
                    result = next;
                    continue;
                }
                Err(error) => cancelled(error)?,
            }
        }
        for member in &group.members {
            let Some(tool) = tools.get(*member) else {
                continue;
            };
            let edge = edges.get(*member).copied().flatten();
            result = boolean(&result, tool, operation)
                .map_err(|error| BlendError::from(error).at(edge))?;
        }
    }
    Ok(result)
}
