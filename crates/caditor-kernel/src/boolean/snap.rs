use std::collections::BTreeMap;

use caditor_geometry::{Plane, Vector3};

use crate::{
    boolean::{Input, Operand, TOLERANCE},
    shell,
    surface::{Cylinder, Surface},
    tolerance::{ANGULAR_RESOLUTION, parallel},
    topology::{FaceId, Solid},
};

const SNAP_FLOOR: f64 = 1e-9;

pub(super) fn snapped(input: &Input) -> Option<Solid> {
    let inward: BTreeMap<FaceId, f64> = input
        .faces(Operand::Second)
        .iter()
        .filter_map(|bounds| {
            let face = input.second.face(bounds.id)?;
            let surface = face.surface();
            let outward = input
                .faces_near(Operand::First, &bounds.bounds)
                .filter_map(|near| input.first.face(near.id))
                .filter_map(|near| growth_onto(surface, near.surface()))
                .map(|growth| growth * face.sense().sign());
            Some((bounds.id, -smallest_snap(outward)?))
        })
        .collect();
    if inward.is_empty() {
        return None;
    }
    shell::snapped(input.second, inward)
}

fn smallest_snap(offsets: impl Iterator<Item = f64>) -> Option<f64> {
    let within: Vec<f64> = offsets.filter(|offset| offset.abs() <= TOLERANCE).collect();
    if within.iter().any(|offset| offset.abs() <= SNAP_FLOOR) {
        return None;
    }
    within
        .into_iter()
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
}

fn growth_onto(surface: &Surface, target: &Surface) -> Option<f64> {
    match (surface, target) {
        (Surface::Plane(plane), Surface::Plane(other)) => {
            plane_growth(plane.frame(), other.frame())
        }
        (Surface::Cylinder(cylinder), Surface::Plane(plane)) => {
            tangent_growth(cylinder, plane.frame())
        }
        (Surface::Cylinder(cylinder), Surface::Cylinder(other)) => coaxial_growth(cylinder, other),
        _ => None,
    }
}

fn plane_growth(frame: &Plane, other: &Plane) -> Option<f64> {
    parallel(frame.normal(), other.normal())
        .then(|| (other.origin() - frame.origin()).dot(frame.normal()))
}

fn tangent_growth(cylinder: &Cylinder, plane: &Plane) -> Option<f64> {
    let axis = cylinder.frame();
    let across = axis.normal().dot(plane.normal()).abs() <= ANGULAR_RESOLUTION;
    across.then(|| plane.signed_distance(axis.origin()).abs() - cylinder.radius())
}

fn coaxial_growth(cylinder: &Cylinder, other: &Cylinder) -> Option<f64> {
    let (axis, other_axis) = (cylinder.frame(), other.frame());
    let offset: Vector3 = other_axis.origin() - axis.origin();
    let off_axis = offset - axis.normal() * offset.dot(axis.normal());
    let coaxial = parallel(axis.normal(), other_axis.normal()) && off_axis.length() <= SNAP_FLOOR;
    coaxial.then(|| other.radius() - cylinder.radius())
}
