mod corner;
mod section;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};
use thiserror::Error;

use self::{
    corner::Corner,
    section::{BLEND_CURVE, Blend, FIRST_SIDE, SECOND_SIDE, Section, SectionCurve, SectionSide},
};
use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    build::{AngularExtent, Axis2, LinearExtent, SweepError, extrude, revolve},
    curve::Curve,
    naming::{EdgeReference, FaceName, FaceOrigin, ReferenceError},
    profile::{Profile, ProfileCurve, ProfileError, ProfileShape, Selection},
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceContainment, FaceId, Solid, SolidClassifier, VertexId},
};

const DIRECTION_TOLERANCE: f64 = 1e-7;
const SMOOTH_TOLERANCE: f64 = 1e-6;
const TANGENT_CONTINUITY: f64 = 1e-6;
const PERPENDICULAR_END: f64 = 1e-9;
const SHALLOWEST_END: f64 = 0.1;
const END_MARGIN: f64 = 0.25;
const CUTTER_SCALE: f64 = 4.0;
const CLEARANCE: f64 = 0.25;
const SMALLEST_RADIUS: f64 = 10.0 * LINEAR_RESOLUTION;

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
    #[error("the size is not a finite number above zero")]
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
    #[error("edge {edge:?} ends at vertex {vertex:?} where the blend cannot be closed off")]
    UnsupportedEnd {
        edge: EdgeId,
        vertex: Option<VertexId>,
    },
    #[error("edge {0:?} could not be found again after the concave edges were filled")]
    Lost(EdgeId),
    #[error("after the concave edges were filled: {0}")]
    AfterFill(Box<BlendError>),
    #[error("the blend shape could not be built: {0}")]
    Profile(#[from] ProfileError),
    #[error("the blend shape could not be swept: {0}")]
    Sweep(#[from] SweepError),
    #[error(transparent)]
    Boolean(#[from] BooleanError),
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
            Self::UnsupportedEnd { .. } => Self::UnsupportedEnd { edge, vertex: None },
            other => other,
        }
    }

    pub fn edge(&self) -> Option<EdgeId> {
        match self {
            Self::MissingEdge(edge)
            | Self::Unsupported(edge)
            | Self::Smooth(edge)
            | Self::TooLarge(edge)
            | Self::Lost(edge)
            | Self::UnsupportedEnd { edge, .. } => Some(*edge),
            Self::InvalidSize
            | Self::AfterFill(_)
            | Self::NoEdges
            | Self::Profile(_)
            | Self::Sweep(_)
            | Self::Boolean(_) => None,
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
    if parallel_within(normals[0], normals[1], SMOOTH_TOLERANCE) {
        return Err(BlendError::Smooth(edge));
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
        (Some(a), Some(b)) => !parallel_within(a, b, SMOOTH_TOLERANCE),
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
                    && is_sharp(solid, *other, point)
                {
                    chosen.insert(*other);
                    queue.push(*other);
                }
            }
        }
    }
    Ok(chosen.into_iter().collect())
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
        FIT_FRACTIONS.iter().all(|fraction| {
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
        })
        .fold(f64::INFINITY, f64::min)
}

struct Tool {
    solid: Solid,
    convex: bool,
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
                AngularExtent::new(
                    -start.extension() / innermost,
                    angle + end.extension() / innermost,
                )?
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
    })
}

pub fn blend_chain(solid: &Solid, edges: &[EdgeId]) -> Vec<EdgeId> {
    propagate(solid, &Topology::new(solid), edges).unwrap_or_default()
}

pub fn blend(
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
    let mut concave = Vec::new();
    let mut convex = Vec::new();
    for edge in chosen {
        if analyze(solid, edge)?.section.convex {
            convex.push(edge);
        } else {
            concave.push(edge);
        }
    }
    if concave.is_empty() || convex.is_empty() {
        return apply(solid, edges, shape, feature);
    }
    let references: Vec<(EdgeId, EdgeReference)> = convex
        .iter()
        .filter_map(|edge| Some((*edge, EdgeReference::capture(solid, *edge)?)))
        .collect();
    let filled = apply(solid, &concave, shape, feature)?;
    if let Some(lost) = convex
        .iter()
        .find(|edge| !references.iter().any(|(captured, _)| captured == *edge))
    {
        return Err(BlendError::Lost(*lost));
    }
    let mut original = BTreeMap::new();
    for (edge, reference) in &references {
        let found = match reference.resolve(&filled) {
            Ok(found) => vec![found],
            Err(ReferenceError::Ambiguous(candidates)) => candidates,
            Err(ReferenceError::Missing) => return Err(BlendError::Lost(*edge)),
        };
        for piece in found {
            original.insert(piece, *edge);
        }
    }
    let remaining: Vec<EdgeId> = original.keys().copied().collect();
    apply(&filled, &remaining, shape, feature)
        .map_err(|error| error.remapped(|edge| original.get(&edge).copied()))
}

fn apply(
    solid: &Solid,
    edges: &[EdgeId],
    shape: BlendShape,
    feature: u64,
) -> Result<Solid, BlendError> {
    let topology = Topology::new(solid);
    let chosen = propagate(solid, &topology, edges)?;
    let chosen_set: BTreeSet<EdgeId> = chosen.iter().copied().collect();
    let classifier = solid.classifier();
    let mut planned = Vec::with_capacity(chosen.len());
    for edge in &chosen {
        let geometry = analyze(solid, *edge)?;
        let blend = match shape {
            BlendShape::Fillet { radius } => geometry.section.fillet(radius),
            BlendShape::Chamfer { distance } => geometry.section.chamfer(distance),
        }
        .ok_or(BlendError::TooLarge(*edge))?;
        if !fits(&classifier, solid, &geometry, &blend) {
            return Err(BlendError::TooLarge(*edge));
        }
        let reach = geometry.section.reach(&blend);
        planned.push((geometry, blend, reach));
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
                    corner::find(solid, &topology, &chosen, &convex, radius, vertex)
                        .map(|corner| (vertex, corner))
                })
                .collect()
        }
        BlendShape::Chamfer { .. } => BTreeMap::new(),
    };
    let around = Surroundings {
        solid,
        topology: &topology,
        chosen: &chosen_set,
        corners: &corners,
    };
    let mut tools = Vec::with_capacity(planned.len() + corners.len());
    let mut blend_names = BTreeMap::new();
    for (geometry, blend, reach) in &planned {
        let ends = ends(&around, geometry, *reach)?;
        let edge_name = solid
            .edge(geometry.edge)
            .ok_or(BlendError::MissingEdge(geometry.edge))?
            .name();
        blend_names.insert(geometry.edge, FaceName::blend(feature, edge_name));
        tools.push(tool(solid, geometry, blend, &ends, shape, feature)?);
    }
    for corner in corners.values() {
        tools.push(Tool {
            solid: corner.tool(solid, feature, &blend_names)?,
            convex: true,
        });
    }
    tools.sort_by_key(|tool| !tool.convex);
    let mut result = solid.clone();
    for tool in tools {
        let operation = if tool.convex {
            BooleanOperation::Difference
        } else {
            BooleanOperation::Union
        };
        result = boolean(&result, &tool.solid, operation)?;
    }
    Ok(result)
}
