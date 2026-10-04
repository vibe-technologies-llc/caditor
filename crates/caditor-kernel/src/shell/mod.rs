mod collapse;
mod edge;
mod inner;
mod split;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use thiserror::Error;

use crate::{
    boolean::{BooleanError, BooleanOperation, boolean},
    build::{LinearExtent, extrude},
    curve::Curve,
    interrupt::{self, Interrupted},
    naming::{FaceName, FaceOrigin},
    profile::{Profile, ProfileCurve, Selection},
    surface::{Cone, Cylinder, PlaneSurface, Sphere, Surface, Torus},
    tessellation::TessellationError,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const OFFSET_TOLERANCE: f64 = 10.0 * LINEAR_RESOLUTION;
const OPENING_REACH: f64 = 2.0;
const SMOOTH_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ShellError {
    #[error("the thickness is not a finite number above 0.000001 mm")]
    InvalidThickness,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("face {0:?} is curved in a way that cannot be offset")]
    UnsupportedFace(FaceId),
    #[error("the walls cannot follow edge {0:?}")]
    UnsupportedEdge(EdgeId),
    #[error("the thickness is too large for the body")]
    TooThick,
    #[error("face {0:?} curves more tightly than the thickness")]
    TooCurved(FaceId),
    #[error("the walls cannot meet at vertex {0:?}")]
    Corner(VertexId),
    #[error("the wall along edge {0:?} shrinks to nothing")]
    EdgeCollapses(EdgeId),
    #[error("the opening in face {0:?} could not be cut")]
    Opening(FaceId),
    #[error("the offset walls do not form a valid solid")]
    Walls {
        face: Option<FaceId>,
        edge: Option<EdgeId>,
    },
    #[error("the body could not be meshed to tell its voids apart: {0}")]
    Voids(TessellationError),
    #[error(transparent)]
    Boolean(BooleanError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl ShellError {
    fn names_the_cause(&self) -> bool {
        match self {
            Self::TooThick | Self::EdgeCollapses(_) | Self::InvalidThickness => false,
            Self::Walls { face, edge } => face.is_some() || edge.is_some(),
            _ => true,
        }
    }
}

impl From<BooleanError> for ShellError {
    fn from(error: BooleanError) -> Self {
        match error {
            BooleanError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            other => Self::Boolean(other),
        }
    }
}

struct Offsets<'a> {
    solid: &'a Solid,
    thickness: f64,
    outward: BTreeSet<FaceId>,
}

impl Offsets<'_> {
    fn distance(&self, face: FaceId) -> f64 {
        if self.outward.contains(&face) {
            -self.thickness
        } else {
            self.thickness
        }
    }

    fn residual(
        &self,
        face: FaceId,
        point: Point3,
        hint: Option<Point2>,
    ) -> Option<(f64, Vector3)> {
        let definition = self.solid.face(face)?;
        let surface = definition.surface();
        let uv = surface.project(point, hint);
        let normal = surface.normal(uv.x, uv.y)? * definition.sense().sign();
        let signed = (point - surface.point_at(uv)).dot(normal);
        Some((signed + self.distance(face), normal))
    }

    fn surface(&self, face: FaceId) -> Result<Surface, ShellError> {
        let definition = self.solid.face(face).ok_or(ShellError::MissingFace(face))?;
        let along_normal = -self.distance(face) * definition.sense().sign();
        let too_thick = |_| ShellError::TooCurved(face);
        Ok(match definition.surface() {
            Surface::Plane(plane) => {
                let frame = plane.frame();
                let moved = Plane::from_frame(
                    frame.origin() + frame.normal() * along_normal,
                    frame.normal(),
                    frame.x_axis(),
                )
                .ok_or(ShellError::TooCurved(face))?;
                PlaneSurface::new(moved).map_err(too_thick)?.into()
            }
            Surface::Cylinder(cylinder) => {
                Cylinder::new(*cylinder.frame(), cylinder.radius() + along_normal)
                    .map_err(too_thick)?
                    .into()
            }
            Surface::Sphere(sphere) => Sphere::new(*sphere.frame(), sphere.radius() + along_normal)
                .map_err(too_thick)?
                .into(),
            Surface::Torus(torus) => Torus::new(
                *torus.frame(),
                torus.major_radius(),
                torus.minor_radius() + along_normal,
            )
            .map_err(too_thick)?
            .into(),
            Surface::Cone(cone) => offset_cone(cone, along_normal)
                .ok_or(ShellError::TooCurved(face))?
                .into(),
            Surface::Extrusion(_) | Surface::Revolution(_) | Surface::BSpline(_) => {
                return Err(ShellError::UnsupportedFace(face));
            }
        })
    }

    fn on_both(&self, faces: &[FaceId], point: Point3) -> bool {
        faces.iter().all(|face| {
            self.residual(*face, point, None)
                .is_some_and(|(residual, _)| residual.abs() <= OFFSET_TOLERANCE)
        })
    }
}

fn offset_cone(cone: &Cone, along_normal: f64) -> Option<Cone> {
    let (sin, cos) = cone.half_angle().sin_cos();
    let radius = cone.radius() + along_normal / cos;
    let frame = cone.frame();
    if radius >= 0.0 {
        return Cone::new(*frame, radius, cone.half_angle()).ok();
    }
    let past_apex = frame.normal() * (-2.0 * radius * cos / sin);
    let moved = Plane::from_frame(frame.origin() + past_apex, frame.normal(), frame.x_axis())?;
    Cone::new(moved, -radius, cone.half_angle()).ok()
}

fn planar_curves(solid: &Solid, face: FaceId, plane: &Plane) -> Option<Vec<ProfileCurve>> {
    let definition = solid.face(face)?;
    let mut curves = Vec::new();
    let coedges = definition
        .loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges());
    for (index, coedge) in coedges.enumerate() {
        let edge = solid.edge(solid.coedge(*coedge)?.edge())?;
        let entity = index as u64;
        let point = |vertex: VertexId| Some(plane.to_local(solid.vertex(vertex)?.point()));
        let (start, end) = (point(edge.start())?, point(edge.end())?);
        curves.push(match edge.curve() {
            Curve::Line(_) => ProfileCurve::line(entity, start, end),
            Curve::Circle(circle) => {
                let center = plane.to_local(circle.center());
                if edge.is_closed() {
                    ProfileCurve::circle(entity, center, circle.radius())
                } else if circle.frame().normal().dot(plane.normal()) > 0.0 {
                    ProfileCurve::arc(entity, center, start, end)
                } else {
                    ProfileCurve::arc(entity, center, end, start)
                }
            }
            _ => return None,
        });
    }
    Some(curves)
}

fn opening_behind(offsets: &Offsets<'_>, open: FaceId, feature: u64) -> Result<Solid, ShellError> {
    let definition = offsets
        .solid
        .face(open)
        .ok_or(ShellError::MissingFace(open))?;
    let Surface::Plane(surface) = definition.surface() else {
        return Err(ShellError::UnsupportedFace(open));
    };
    let into_material = if definition.sense().is_same() {
        surface.frame().flipped()
    } else {
        *surface.frame()
    };
    let curves = planar_curves(offsets.solid, open, &into_material)
        .ok_or(ShellError::UnsupportedFace(open))?;
    let regions = Profile::new(&curves)
        .and_then(|profile| profile.select(&Selection::EvenDepth))
        .map_err(|_| ShellError::Opening(open))?;
    let extent =
        LinearExtent::one_side(offsets.thickness).map_err(|_| ShellError::Opening(open))?;
    let prism = extrude(&into_material, &regions, extent, feature)
        .map_err(|_| ShellError::Opening(open))?;
    let offset_name = FaceName::shell(feature, definition.name());
    Ok(prism.renamed(|_, _| (offset_name, Some(FaceOrigin::Shell { feature }))))
}

fn opening(
    inner: &Solid,
    offsets: &Offsets<'_>,
    open: FaceId,
    feature: u64,
) -> Result<Solid, ShellError> {
    let definition = offsets
        .solid
        .face(open)
        .ok_or(ShellError::MissingFace(open))?;
    let offset_name = FaceName::shell(feature, definition.name());
    let expected = offsets.surface(open)?;
    let (offset_face, offset) = inner
        .faces()
        .find(|(_, face)| {
            face.name() == offset_name && face.surface().same_surface(&expected).is_some()
        })
        .ok_or(ShellError::Opening(open))?;
    let Surface::Plane(surface) = offset.surface() else {
        return Err(ShellError::UnsupportedFace(open));
    };
    let plane = if offset.sense().is_same() {
        *surface.frame()
    } else {
        surface.frame().flipped()
    };
    let curves =
        planar_curves(inner, offset_face, &plane).ok_or(ShellError::UnsupportedFace(open))?;
    let regions = Profile::new(&curves)
        .and_then(|profile| profile.select(&Selection::EvenDepth))
        .map_err(|_| ShellError::Opening(open))?;
    let extent = LinearExtent::one_side(OPENING_REACH * offsets.thickness)
        .map_err(|_| ShellError::Opening(open))?;
    let prism =
        extrude(&plane, &regions, extent, feature).map_err(|_| ShellError::Opening(open))?;
    Ok(prism.renamed(|_, _| (offset_name, Some(FaceOrigin::Shell { feature }))))
}

fn edge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    let mut faces: Vec<FaceId> = solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect();
    faces.dedup();
    faces
}

fn meets_smoothly(solid: &Solid, edge: EdgeId, faces: &[FaceId]) -> bool {
    let [first, second] = faces else {
        return false;
    };
    let Some(definition) = solid.edge(edge) else {
        return true;
    };
    let middle = definition.curve().point(definition.interval().middle());
    let normal = |face: FaceId| {
        let face = solid.face(face)?;
        let surface = face.surface();
        let uv = surface.project(middle, None);
        Some(surface.normal(uv.x, uv.y)? * face.sense().sign())
    };
    match (normal(*first), normal(*second)) {
        (Some(a), Some(b)) => a.cross(b).length() <= SMOOTH_TOLERANCE && a.dot(b) > 0.0,
        _ => true,
    }
}

fn extendable(solid: &Solid, open: &[FaceId], voids: &BTreeSet<FaceId>) -> BTreeSet<FaceId> {
    let opened: BTreeSet<FaceId> = open.iter().copied().collect();
    let mut blocked = voids.clone();
    for (edge, _) in solid.edges() {
        let faces = edge_faces(solid, edge);
        let closed_neighbour = faces.iter().any(|face| !opened.contains(face));
        if closed_neighbour && meets_smoothly(solid, edge, &faces) {
            blocked.extend(faces.iter().filter(|face| opened.contains(face)).copied());
        }
    }
    opened.difference(&blocked).copied().collect()
}

fn void_faces(solid: &Solid, open: &[FaceId]) -> Result<BTreeSet<FaceId>, ShellError> {
    let voids = match solid.void_shells() {
        Ok(voids) => voids,
        Err(TessellationError::Cancelled(interrupted)) => {
            return Err(ShellError::Cancelled(interrupted));
        }
        Err(error) => return Err(ShellError::Voids(error)),
    };
    Ok(open
        .iter()
        .filter(|face| {
            solid
                .face(**face)
                .is_some_and(|definition| voids.contains(&definition.shell()))
        })
        .copied()
        .collect())
}

struct Hollowed {
    solid: Solid,
    dropped: BTreeSet<FaceId>,
}

impl Hollowed {
    fn keeps_every_wall(&self, offsets: &Offsets<'_>, open: &[FaceId], feature: u64) -> bool {
        let present: BTreeSet<FaceName> = self.solid.faces().map(|(_, face)| face.name()).collect();
        offsets
            .solid
            .faces()
            .filter(|(id, _)| !open.contains(id) && !self.dropped.contains(id))
            .all(|(_, face)| present.contains(&FaceName::shell(feature, face.name())))
    }
}

fn hollow(
    offsets: &Offsets<'_>,
    open: &[FaceId],
    voids: &BTreeSet<FaceId>,
    feature: u64,
) -> Result<Hollowed, ShellError> {
    let inner::Inner {
        solid: mut inner,
        dropped,
    } = inner::inner_solid(offsets, feature)?;
    for face in open.iter().filter(|face| !offsets.outward.contains(face)) {
        let prism = if voids.contains(face) {
            opening_behind(offsets, *face, feature)?
        } else {
            opening(&inner, offsets, *face, feature)?
        };
        inner = boolean(&inner, &prism, BooleanOperation::Union)?;
    }
    Ok(Hollowed {
        solid: boolean(offsets.solid, &inner, BooleanOperation::Difference)?,
        dropped,
    })
}

pub fn shell(
    solid: &Solid,
    open: &[FaceId],
    thickness: f64,
    feature: u64,
) -> Result<Solid, ShellError> {
    hollow_out(solid, open, thickness, feature).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn hollow_out(
    solid: &Solid,
    open: &[FaceId],
    thickness: f64,
    feature: u64,
) -> Result<Solid, ShellError> {
    if !thickness.is_finite() || thickness <= LINEAR_RESOLUTION {
        return Err(ShellError::InvalidThickness);
    }
    for face in open {
        let definition = solid.face(*face).ok_or(ShellError::MissingFace(*face))?;
        if !matches!(definition.surface(), Surface::Plane(_)) {
            return Err(ShellError::UnsupportedFace(*face));
        }
    }
    let voids = void_faces(solid, open)?;
    let outward = extendable(solid, open, &voids);
    let inward = Offsets {
        solid,
        thickness,
        outward: BTreeSet::new(),
    };
    if outward.is_empty() {
        return hollow(&inward, open, &voids, feature).map(|hollowed| hollowed.solid);
    }
    let extended = Offsets {
        solid,
        thickness,
        outward,
    };
    let first = match hollow(&extended, open, &voids, feature) {
        Ok(result) if result.keeps_every_wall(&extended, open, feature) => return Ok(result.solid),
        Ok(_) => ShellError::TooThick,
        Err(error @ ShellError::Cancelled(_)) => return Err(error),
        Err(error) => error,
    };
    hollow(&inward, open, &voids, feature)
        .map(|hollowed| hollowed.solid)
        .map_err(|inward_error| match first {
            ShellError::TooThick if inward_error.names_the_cause() => inward_error,
            other => other,
        })
}
