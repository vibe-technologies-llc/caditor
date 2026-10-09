use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use super::{BlendError, Named, half_space};
use crate::{
    boolean::{BooleanOperation, boolean},
    build::{AngularExtent, Axis2, revolve},
    profile::{Profile, ProfileCurve, Selection},
    surface::Surface,
    topology::{FaceId, Solid},
};

const SMALLEST_RADIUS: f64 = 1e-6;
const CORNER_REACH: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Round {
    Axial {
        origin: Point3,
        axis: Vector3,
        radius: f64,
        slope: f64,
    },
    Sphere {
        centre: Point3,
        radius: f64,
    },
    Torus {
        origin: Point3,
        axis: Vector3,
        major: f64,
        minor: f64,
        facing: Option<[f64; 2]>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Cut {
    pub(super) at: Point3,
    pub(super) toward: Vector3,
    pub(super) span: f64,
    pub(super) behind: f64,
}

impl Round {
    pub(super) fn of(solid: &Solid, face: FaceId) -> Option<Self> {
        match solid.face(face)?.surface() {
            Surface::Cylinder(cylinder) => Some(Self::Axial {
                origin: cylinder.frame().origin(),
                axis: cylinder.frame().normal(),
                radius: cylinder.radius(),
                slope: 0.0,
            }),
            Surface::Cone(cone) => Some(Self::Axial {
                origin: cone.frame().origin(),
                axis: cone.frame().normal(),
                radius: cone.radius(),
                slope: cone.half_angle().tan(),
            }),
            Surface::Sphere(sphere) => Some(Self::Sphere {
                centre: sphere.frame().origin(),
                radius: sphere.radius(),
            }),
            Surface::Torus(torus) => {
                let origin = torus.frame().origin();
                let axis = torus.frame().normal();
                Some(Self::Torus {
                    origin,
                    axis,
                    major: torus.major_radius(),
                    minor: torus.minor_radius(),
                    facing: facing(solid, face, origin, axis),
                })
            }
            _ => None,
        }
    }

    fn outward(&self, point: Point3) -> Option<Vector3> {
        match *self {
            Self::Axial { origin, axis, .. } => {
                let along = (point - origin).dot(axis);
                (point - origin - axis * along).try_normalize()
            }
            Self::Sphere { centre, .. } => (point - centre).try_normalize(),
            Self::Torus {
                origin,
                axis,
                major,
                ..
            } => {
                let along = (point - origin).dot(axis);
                let radial = (point - origin - axis * along).try_normalize()?;
                (point - (origin + radial * major)).try_normalize()
            }
        }
    }

    pub(super) fn region_beyond(
        &self,
        cut: Cut,
        feature: u64,
        name: Named,
        refused: &BlendError,
    ) -> Result<Solid, BlendError> {
        let Cut {
            at,
            toward,
            span,
            behind,
        } = cut;
        let refused_now = || refused.clone();
        let outward = self.outward(at).ok_or_else(refused_now)?;
        let away = toward.dot(outward) > 0.0;
        let start = Plane::new(at - toward * behind, toward).ok_or_else(refused_now)?;
        let near = half_space(&start, span, feature, name)?;
        let (shape, operation) = match (self, away) {
            (
                Self::Torus {
                    facing: Some(facing),
                    ..
                },
                true,
            ) => (
                self.corner(at, *facing, feature, refused)?,
                BooleanOperation::Intersection,
            ),
            (_, true) => (
                self.solid(at, span, feature, refused)?,
                BooleanOperation::Difference,
            ),
            (_, false) => (
                self.solid(at, span, feature, refused)?,
                BooleanOperation::Intersection,
            ),
        };
        let shape = shape.renamed(|_, _| (name.name, name.origin));
        Ok(boolean(&near, &shape, operation)?)
    }

    fn radial_frame(origin: Point3, axis: Vector3, near: Point3) -> Option<Plane> {
        let along = (near - origin).dot(axis);
        let radial = (near - origin - axis * along)
            .try_normalize()
            .unwrap_or_else(|| axis.any_orthonormal_vector());
        Plane::from_frame(origin, radial.cross(axis), radial)
    }

    fn corner(
        &self,
        at: Point3,
        [radial_sign, axial_sign]: [f64; 2],
        feature: u64,
        refused: &BlendError,
    ) -> Result<Solid, BlendError> {
        let refused_now = || refused.clone();
        let Self::Torus {
            origin,
            axis,
            major,
            minor,
            ..
        } = *self
        else {
            return Err(refused_now());
        };
        if minor >= major - SMALLEST_RADIUS {
            return Err(refused_now());
        }
        let plane = Self::radial_frame(origin, axis, at).ok_or_else(refused_now)?;
        let reach = CORNER_REACH * minor;
        let across = |distance: f64| (major + radial_sign * distance).max(0.0);
        let centre = Point2::new(major, 0.0);
        let on_rim = Point2::new(across(minor), 0.0);
        let on_side = Point2::new(major, axial_sign * minor);
        let corners = [
            on_rim,
            Point2::new(across(reach), 0.0),
            Point2::new(across(reach), axial_sign * reach),
            Point2::new(major, axial_sign * reach),
            on_side,
        ];
        let mut curves: Vec<ProfileCurve> = corners
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [from, to] => Some(ProfileCurve::line(index as u64 + 1, *from, *to)),
                _ => None,
            })
            .collect();
        curves.push(if radial_sign * axial_sign > 0.0 {
            ProfileCurve::arc(9, centre, on_rim, on_side)
        } else {
            ProfileCurve::arc(9, centre, on_side, on_rim)
        });
        swept(&plane, &curves, feature)
    }

    fn solid(
        &self,
        near: Point3,
        reach: f64,
        feature: u64,
        refused: &BlendError,
    ) -> Result<Solid, BlendError> {
        let refused = || refused.clone();
        match *self {
            Self::Axial {
                origin,
                axis,
                radius,
                slope,
            } => {
                let plane = Self::radial_frame(origin, axis, near).ok_or_else(refused)?;
                let height = (near - origin).dot(axis);
                let radius_at = |at: f64| radius + at * slope;
                let mut low = height - reach;
                let mut high = height + reach;
                if slope != 0.0 {
                    let apex = -radius / slope;
                    if apex > low && apex < high {
                        if height > apex {
                            low = apex;
                        } else {
                            high = apex;
                        }
                    }
                }
                let mut corners = vec![Point2::new(0.0, low)];
                if radius_at(low) > SMALLEST_RADIUS {
                    corners.push(Point2::new(radius_at(low), low));
                }
                if radius_at(high) > SMALLEST_RADIUS {
                    corners.push(Point2::new(radius_at(high), high));
                }
                corners.push(Point2::new(0.0, high));
                let curves: Vec<ProfileCurve> = corners
                    .iter()
                    .zip(corners.iter().cycle().skip(1))
                    .enumerate()
                    .map(|(index, (from, to))| ProfileCurve::line(index as u64 + 1, *from, *to))
                    .collect();
                swept(&plane, &curves, feature)
            }
            Self::Sphere { centre, radius } => {
                let axis = (near - centre)
                    .any_orthonormal_vector()
                    .try_normalize()
                    .ok_or_else(refused)?;
                let plane = Self::radial_frame(centre, axis, near).ok_or_else(refused)?;
                let curves = [
                    ProfileCurve::arc(
                        1,
                        Point2::ZERO,
                        Point2::new(0.0, -radius),
                        Point2::new(0.0, radius),
                    ),
                    ProfileCurve::line(2, Point2::new(0.0, radius), Point2::new(0.0, -radius)),
                ];
                swept(&plane, &curves, feature)
            }
            Self::Torus {
                origin,
                axis,
                major,
                minor,
                ..
            } => {
                if minor >= major - SMALLEST_RADIUS {
                    return Err(refused());
                }
                let plane = Self::radial_frame(origin, axis, near).ok_or_else(refused)?;
                let curves = [ProfileCurve::circle(1, Point2::new(major, 0.0), minor)];
                swept(&plane, &curves, feature)
            }
        }
    }
}

fn swept(plane: &Plane, curves: &[ProfileCurve], feature: u64) -> Result<Solid, BlendError> {
    let regions = Profile::new(curves)?.select(&Selection::EvenDepth)?;
    let axis = Axis2::new(Point2::ZERO, Vector2::Y)?;
    Ok(revolve(
        plane,
        &regions,
        axis,
        AngularExtent::full(),
        feature,
    )?)
}

const LEANING: f64 = 0.2;

fn facing(solid: &Solid, face: FaceId, origin: Point3, axis: Vector3) -> Option<[f64; 2]> {
    let definition = solid.face(face)?;
    let surface = definition.surface();
    let middle = solid.classifier().face_uv_box(face)?.center();
    let point = surface.point_at(middle);
    let normal = surface.normal(middle.x, middle.y)? * definition.sense().sign();
    let along = (point - origin).dot(axis);
    let radial = (point - origin - axis * along).try_normalize()?;
    let [across, up] = [normal.dot(radial), normal.dot(axis)];
    (across.abs() >= LEANING && up.abs() >= LEANING).then_some([across.signum(), up.signum()])
}
