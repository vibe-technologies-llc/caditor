use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, RigidTransform, Vector3};

use crate::{
    boolean::{Input, Operand, TOLERANCE},
    shell,
    surface::{Cylinder, Surface},
    tolerance::{ANGULAR_RESOLUTION, parallel},
    topology::{Face, FaceId, Solid},
};

const SNAP_FLOOR: f64 = 1e-9;
const SIDEWAYS_REACH: f64 = 4.0 * TOLERANCE + SNAP_FLOOR;

pub(super) fn snapped(input: &Input) -> Option<Solid> {
    let Some(shift) = sideways_shift(input) else {
        return offset_onto(input);
    };
    let shifted = input
        .second
        .transformed(&RigidTransform::translation(shift)?)
        .ok()?;
    let moved = Input::new(input.first, &shifted);
    if !contacts(&moved).is_superset(&contacts(input)) {
        return offset_onto(input);
    }
    let offset = offset_onto(&moved);
    drop(moved);
    offset.or(Some(shifted))
}

fn sideways_shift(input: &Input) -> Option<Vector3> {
    let shifts: Vec<Vector3> = near_pairs(input)
        .filter_map(
            |(second, first)| match (second.surface(), first.surface()) {
                (Surface::Cylinder(cylinder), Surface::Cylinder(other)) => {
                    onto_axis(cylinder, other)
                }
                _ => None,
            },
        )
        .collect();
    let (first, rest) = shifts.split_first()?;
    let agreed = rest
        .iter()
        .all(|shift| shift.distance(*first) <= SNAP_FLOOR);
    (agreed && first.length() > SNAP_FLOOR).then_some(*first)
}

fn near_pairs<'a>(input: &'a Input) -> impl Iterator<Item = (&'a Face, &'a Face)> {
    input.faces(Operand::Second).iter().flat_map(move |bounds| {
        input
            .faces_near(Operand::First, &bounds.bounds)
            .filter_map(move |near| {
                Some((input.second.face(bounds.id)?, input.first.face(near.id)?))
            })
    })
}

fn contacts(input: &Input) -> BTreeSet<(FaceId, FaceId)> {
    input
        .faces(Operand::Second)
        .iter()
        .flat_map(|bounds| {
            input
                .faces_near(Operand::First, &bounds.bounds)
                .filter(|near| {
                    let pair = input.second.face(bounds.id).zip(input.first.face(near.id));
                    pair.and_then(|(second, first)| growth_onto(second.surface(), first.surface()))
                        .is_some_and(|growth| growth.abs() <= TOLERANCE)
                })
                .map(|near| (bounds.id, near.id))
        })
        .collect()
}

fn onto_axis(cylinder: &Cylinder, other: &Cylinder) -> Option<Vector3> {
    let shift = off_axis(cylinder, other)?;
    let apart = shift.length();
    let walls_meet = (other.radius() - cylinder.radius()).abs() <= TOLERANCE + apart;
    (apart <= SIDEWAYS_REACH && walls_meet).then_some(shift)
}

fn off_axis(cylinder: &Cylinder, other: &Cylinder) -> Option<Vector3> {
    let (axis, other_axis) = (cylinder.frame(), other.frame());
    let offset: Vector3 = other_axis.origin() - axis.origin();
    parallel(axis.normal(), other_axis.normal())
        .then(|| offset - axis.normal() * offset.dot(axis.normal()))
}

fn offset_onto(input: &Input) -> Option<Solid> {
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
    let coaxial = off_axis(cylinder, other)?.length() <= SNAP_FLOOR;
    coaxial.then(|| other.radius() - cylinder.radius())
}
