use std::collections::BTreeSet;

use caditor_geometry::{Point3, Vector3};

use crate::{
    curve::Curve,
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, parallel},
    topology::{EdgeId, FaceId, Solid},
};

const AXIS_TOLERANCE: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Axis {
    origin: Point3,
    direction: Vector3,
}

impl Axis {
    fn of(surface: &Surface) -> Option<Self> {
        let frame = match surface {
            Surface::Cylinder(cylinder) => cylinder.frame(),
            Surface::Cone(cone) => cone.frame(),
            Surface::Torus(torus) => torus.frame(),
            Surface::Plane(_)
            | Surface::Sphere(_)
            | Surface::Extrusion(_)
            | Surface::Revolution(_)
            | Surface::BSpline(_) => return None,
        };
        Some(Self {
            origin: frame.origin(),
            direction: frame.normal(),
        })
    }

    fn off_axis(&self, point: Point3) -> Vector3 {
        let offset = point - self.origin;
        offset - self.direction * offset.dot(self.direction)
    }

    fn holds(&self, point: Point3) -> bool {
        self.off_axis(point).length() <= AXIS_TOLERANCE
    }

    fn same_as(&self, other: &Self) -> bool {
        parallel(self.direction, other.direction) && self.holds(other.origin)
    }
}

pub fn hole_faces(solid: &Solid, faces: &[FaceId]) -> Vec<FaceId> {
    let mut chosen = BTreeSet::new();
    for seed in faces {
        let Some(axis) = solid
            .face(*seed)
            .and_then(|face| Axis::of(face.surface()))
            .filter(|axis| is_concave_about(solid, *seed, axis))
        else {
            continue;
        };
        let mut queue = vec![*seed];
        chosen.insert(*seed);
        while let Some(face) = queue.pop() {
            for neighbour in neighbours(solid, face) {
                if !chosen.contains(&neighbour) && belongs_to_hole(solid, neighbour, &axis) {
                    chosen.insert(neighbour);
                    queue.push(neighbour);
                }
            }
        }
    }
    chosen.into_iter().collect()
}

fn belongs_to_hole(solid: &Solid, face: FaceId, axis: &Axis) -> bool {
    let Some(definition) = solid.face(face) else {
        return false;
    };
    match definition.surface() {
        Surface::Plane(plane) => {
            parallel(plane.frame().normal(), axis.direction)
                && edges_of(solid, face).iter().all(|edge| {
                    is_circle_about(solid, *edge, axis)
                        && faces_of(solid, *edge)
                            .iter()
                            .all(|other| *other == face || is_concave_wall(solid, *other, axis))
                })
        }
        surface => {
            Axis::of(surface).is_some_and(|own| own.same_as(axis))
                && is_concave_about(solid, face, axis)
        }
    }
}

fn is_concave_wall(solid: &Solid, face: FaceId, axis: &Axis) -> bool {
    solid
        .face(face)
        .and_then(|definition| Axis::of(definition.surface()))
        .is_some_and(|own| own.same_as(axis))
        && is_concave_about(solid, face, axis)
}

fn is_concave_about(solid: &Solid, face: FaceId, axis: &Axis) -> bool {
    let Some(definition) = solid.face(face) else {
        return false;
    };
    let surface = definition.surface();
    edges_of(solid, face).iter().any(|edge| {
        let Some(edge) = solid.edge(*edge) else {
            return false;
        };
        let point = edge.curve().point(edge.interval().at(0.5));
        let radial = axis.off_axis(point);
        let uv = surface.project(point, None);
        surface
            .normal(uv.x, uv.y)
            .map(|normal| normal * definition.sense().sign())
            .is_some_and(|normal| radial.length() > AXIS_TOLERANCE && normal.dot(radial) < 0.0)
    })
}

fn is_circle_about(solid: &Solid, edge: EdgeId, axis: &Axis) -> bool {
    solid.edge(edge).is_some_and(|edge| match edge.curve() {
        Curve::Circle(circle) => {
            axis.holds(circle.center()) && parallel(circle.frame().normal(), axis.direction)
        }
        Curve::Line(_) | Curve::Ellipse(_) | Curve::BSpline(_) | Curve::Intersection(_) => false,
    })
}

fn edges_of(solid: &Solid, face: FaceId) -> BTreeSet<EdgeId> {
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

fn faces_of(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|definition| definition.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect()
}

fn neighbours(solid: &Solid, face: FaceId) -> BTreeSet<FaceId> {
    edges_of(solid, face)
        .into_iter()
        .flat_map(|edge| faces_of(solid, edge))
        .filter(|other| *other != face)
        .collect()
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2, Vector2};

    use super::*;
    use crate::{
        BooleanOperation, boolean,
        build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
        profile::{Profile, ProfileCurve, Region, Selection},
        test_support::{line, rectangle},
    };

    fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
        Profile::new(curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap()
    }

    fn counterbored_plate() -> Solid {
        let plate = extrude(
            &Plane::XY,
            &regions(&rectangle(1, (-20.0, -20.0), (20.0, 20.0))),
            LinearExtent::one_side(10.0).unwrap(),
            1,
        )
        .unwrap();
        let section = regions(&[
            line(11, (0.0, 2.0), (3.0, 2.0)),
            line(12, (3.0, 2.0), (3.0, 7.0)),
            line(13, (3.0, 7.0), (5.0, 7.0)),
            line(14, (5.0, 7.0), (5.0, 12.0)),
            line(15, (5.0, 12.0), (0.0, 12.0)),
            line(16, (0.0, 12.0), (0.0, 2.0)),
        ]);
        let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
        let drill = revolve(&Plane::XZ, &section, axis, AngularExtent::full(), 2).unwrap();
        boolean(&plate, &drill, BooleanOperation::Difference).unwrap()
    }

    fn faces_on(solid: &Solid, kind: fn(&Surface) -> bool) -> Vec<FaceId> {
        solid
            .faces()
            .filter(|(_, face)| kind(face.surface()))
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn a_counterbored_hole_is_its_walls_floor_and_bottom_but_not_the_plate_around_it() {
        let plate = counterbored_plate();
        let cylinders = faces_on(&plate, |surface| matches!(surface, Surface::Cylinder(_)));
        let planes = faces_on(&plate, |surface| matches!(surface, Surface::Plane(_)));

        let hole = hole_faces(&plate, &cylinders[..1]);

        for cylinder in &cylinders {
            assert!(hole.contains(cylinder));
        }
        assert_eq!(hole.len(), cylinders.len() + 2, "{hole:?}");
        assert_eq!(planes.len(), 8);
        assert!(hole_faces(&plate, &planes).is_empty());
    }

    #[test]
    fn a_boss_is_no_hole() {
        let rod = crate::fixtures::cylinder(5.0, 10.0);
        let walls = faces_on(&rod, |surface| matches!(surface, Surface::Cylinder(_)));

        assert!(!walls.is_empty());
        assert!(hole_faces(&rod, &walls).is_empty());
    }
}
