use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{
    AngularExtent, Axis2, FaceId, FaceReference, LINEAR_RESOLUTION, LinearExtent, Profile,
    ProfileCurve, ProfileError, ReferenceError, Solid, Surface, SweepError, extrude, revolve,
};

use crate::{
    attachment::{FaceAttachment, face_plane},
    datum::feature_name,
    describe::describe_origin,
    document::Document,
    pieces::pieces_of_one_face,
    split::{HALF_SPACE_MARGIN, HALF_SPACE_REACH, HalfSpaceError, half_space_solid},
};

const SURFACE_ENTITY: u64 = u64::MAX - 20;
const CAP_ENTITY: u64 = u64::MAX - 21;
const AXIS_ENTITY: u64 = u64::MAX - 22;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SurfaceToolError {
    #[error("the face is no longer part of its body")]
    Missing,
    #[error("the face was split into parts on different surfaces")]
    NotOneSurface,
    #[error("the face is a freeform surface")]
    Freeform,
    #[error("the body has no extent")]
    NoExtent,
    #[error("the surface does not pass through the body")]
    Misses,
    #[error("its outline could not be drawn: {0}")]
    Profile(ProfileError),
    #[error("it could not be swept: {0}")]
    Sweep(SweepError),
}

impl From<HalfSpaceError> for SurfaceToolError {
    fn from(error: HalfSpaceError) -> Self {
        match error {
            HalfSpaceError::NoExtent => Self::NoExtent,
            HalfSpaceError::Misses => Self::Misses,
            HalfSpaceError::Profile(error) => Self::Profile(error),
            HalfSpaceError::Sweep(error) => Self::Sweep(error),
        }
    }
}

pub(crate) struct SurfaceWords {
    pub reason: String,
    pub remedy: String,
}

impl SurfaceToolError {
    pub(crate) fn words(
        &self,
        document: &Document,
        attachment: &FaceAttachment,
    ) -> Option<SurfaceWords> {
        let face = describe_origin(document, attachment.face.origin());
        let holder = feature_name(document, attachment.body);
        let (reason, remedy) = match self {
            Self::Missing => (
                format!("{face}, which it splits along, is no longer part of {holder}."),
                "Choose another face to split along.".to_owned(),
            ),
            Self::NotOneSurface => (
                format!(
                    "{face}, which it splits along, was split into parts that no longer lie on \
                     one surface."
                ),
                "Choose one of its parts, or another face to split along.".to_owned(),
            ),
            Self::Freeform => (
                format!("{face} is a freeform surface, which cannot be carried on past its edges."),
                format!(
                    "Choose a flat, cylindrical, conical, spherical or toroidal face, or split \
                     along the whole body of {holder}."
                ),
            ),
            Self::NoExtent | Self::Misses | Self::Profile(_) | Self::Sweep(_) => return None,
        };
        Some(SurfaceWords { reason, remedy })
    }
}

pub(crate) struct SurfaceTool {
    pub solid: Solid,
    pub keep_inside: bool,
}

struct Reach {
    low: f64,
    high: f64,
    margin: f64,
}

fn reach(body: &Solid, origin: Point3, direction: Vector3) -> Option<Reach> {
    let bounds = body.bounding_box()?;
    let heights = bounds
        .corners()
        .map(|corner| (corner - origin).dot(direction));
    Some(Reach {
        low: heights.iter().copied().fold(f64::INFINITY, f64::min),
        high: heights.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        margin: bounds.diagonal() * HALF_SPACE_REACH + HALF_SPACE_MARGIN,
    })
}

fn chosen_face(holder: &Solid, face: &FaceReference) -> Result<FaceId, SurfaceToolError> {
    match face.resolve(holder) {
        Ok(found) => Ok(found),
        Err(ReferenceError::Ambiguous(pieces)) if pieces_of_one_face(holder, &pieces) => pieces
            .first()
            .copied()
            .ok_or(SurfaceToolError::NotOneSurface),
        Err(ReferenceError::Ambiguous(_)) => Err(SurfaceToolError::NotOneSurface),
        Err(ReferenceError::Missing) => Err(SurfaceToolError::Missing),
    }
}

fn revolved(
    centre: Point3,
    across: Vector3,
    up: Vector3,
    curves: &[ProfileCurve],
    feature: u64,
) -> Result<Solid, SurfaceToolError> {
    let plane =
        Plane::from_frame(centre, across.cross(up), across).ok_or(SurfaceToolError::NoExtent)?;
    let profile = Profile::new(curves).map_err(SurfaceToolError::Profile)?;
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).map_err(SurfaceToolError::Sweep)?;
    revolve(
        &plane,
        profile.regions(),
        axis,
        AngularExtent::full(),
        feature,
    )
    .map_err(SurfaceToolError::Sweep)
}

pub(crate) fn surface_tool(
    holder: &Solid,
    face: &FaceReference,
    body: &Solid,
    flipped: bool,
    feature: u64,
) -> Result<SurfaceTool, SurfaceToolError> {
    let id = chosen_face(holder, face)?;
    let chosen = holder.face(id).ok_or(SurfaceToolError::Missing)?;
    let outward = chosen.sense().is_same();
    let solid = match chosen.surface() {
        Surface::Plane(_) => {
            let plane = face_plane(holder, id).ok_or(SurfaceToolError::Missing)?;
            return Ok(SurfaceTool {
                solid: half_space_solid(body, &plane, flipped, feature)?,
                keep_inside: true,
            });
        }
        Surface::Cylinder(cylinder) => {
            let frame = cylinder.frame();
            let along = frame.normal();
            let Reach { low, high, margin } =
                reach(body, frame.origin(), along).ok_or(SurfaceToolError::NoExtent)?;
            let base = Plane::from_frame(
                frame.origin() + along * (low - margin),
                along,
                frame.x_axis(),
            )
            .ok_or(SurfaceToolError::NoExtent)?;
            let curves = [ProfileCurve::circle(
                SURFACE_ENTITY,
                Point2::ZERO,
                cylinder.radius(),
            )];
            let profile = Profile::new(&curves).map_err(SurfaceToolError::Profile)?;
            let extent = LinearExtent::new(0.0, high - low + 2.0 * margin)
                .map_err(SurfaceToolError::Sweep)?;
            extrude(&base, profile.regions(), extent, feature).map_err(SurfaceToolError::Sweep)?
        }
        Surface::Cone(cone) => {
            let apex = cone.apex();
            let up = cone
                .opening_direction()
                .try_normalize()
                .ok_or(SurfaceToolError::NoExtent)?;
            let Reach { high, margin, .. } =
                reach(body, apex, up).ok_or(SurfaceToolError::NoExtent)?;
            if high <= LINEAR_RESOLUTION {
                return Err(SurfaceToolError::Misses);
            }
            let height = high + margin;
            let rim = Point2::new(height * cone.half_angle().abs().tan(), height);
            let top = Point2::new(0.0, height);
            let curves = [
                ProfileCurve::line(SURFACE_ENTITY, Point2::ZERO, rim),
                ProfileCurve::line(CAP_ENTITY, rim, top),
                ProfileCurve::line(AXIS_ENTITY, top, Point2::ZERO),
            ];
            revolved(apex, cone.frame().x_axis(), up, &curves, feature)?
        }
        Surface::Sphere(sphere) => {
            let radius = sphere.radius();
            let frame = sphere.frame();
            let curves = [
                ProfileCurve::arc(
                    SURFACE_ENTITY,
                    Point2::ZERO,
                    Point2::new(0.0, -radius),
                    Point2::new(0.0, radius),
                ),
                ProfileCurve::line(
                    AXIS_ENTITY,
                    Point2::new(0.0, radius),
                    Point2::new(0.0, -radius),
                ),
            ];
            revolved(
                sphere.center(),
                frame.x_axis(),
                frame.normal(),
                &curves,
                feature,
            )?
        }
        Surface::Torus(torus) => {
            let frame = torus.frame();
            let curves = [ProfileCurve::circle(
                SURFACE_ENTITY,
                Point2::new(torus.major_radius(), 0.0),
                torus.minor_radius(),
            )];
            revolved(
                frame.origin(),
                frame.x_axis(),
                frame.normal(),
                &curves,
                feature,
            )?
        }
        _ => return Err(SurfaceToolError::Freeform),
    };
    Ok(SurfaceTool {
        solid,
        keep_inside: outward == flipped,
    })
}
