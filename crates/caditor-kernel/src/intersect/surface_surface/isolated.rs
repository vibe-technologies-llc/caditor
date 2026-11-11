use std::f64::consts::TAU;

use caditor_geometry::{Maths, Point2, Point3};

use crate::{
    intersect::{
        solve::{Contact, SeparationForm, parallel_normals},
        surface_surface::march::curvature_bound,
    },
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const RING_GAPS: f64 = 4.0;
const RING_CLEARANCE: f64 = 2.0 * TOLERANCE;
const RING_SAMPLES: usize = 12;
const MAX_RING_BEND: f64 = 0.1;
const SEED_RING_TURNS: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct IsolatedTouch {
    pub contact: Contact,
    pub reach: f64,
}

impl IsolatedTouch {
    pub(super) fn holds(&self, point: Point3) -> bool {
        self.contact.point.distance(point) <= self.reach
    }
}

pub(super) fn isolated_touch(surfaces: [&Surface; 2], near: &Contact) -> Option<IsolatedTouch> {
    ring(surfaces, near).filter(|ring| ring.turn <= SEED_RING_TURNS * MAX_RING_BEND)?;
    let touch = parallel_normals(surfaces, near.uv)?;
    if touch.gap > TOLERANCE {
        return None;
    }
    let contact = Contact {
        uv: touch.uv,
        point: touch.point,
    };
    let Ring { form, reach, turn } = ring(surfaces, &contact)?;
    let settled = turn <= MAX_RING_BEND && near.point.distance(contact.point) <= reach;
    (settled && closes_round(surfaces, &contact, &form, reach))
        .then_some(IsolatedTouch { contact, reach })
}

struct Ring {
    form: SeparationForm,
    reach: f64,
    turn: f64,
}

fn ring(surfaces: [&Surface; 2], contact: &Contact) -> Option<Ring> {
    let form = SeparationForm::at(surfaces, contact)?;
    let bends = form.definite()?;
    let reach = (2.0 * RING_GAPS * TOLERANCE / bends.least).sqrt();
    let [first, second] = surfaces;
    let [first_uv, second_uv] = contact.uv;
    let bending = bends
        .most
        .max(curvature_bound(first, first_uv)?)
        .max(curvature_bound(second, second_uv)?);
    let turn = reach * bending;
    turn.is_finite().then_some(Ring { form, reach, turn })
}

fn closes_round(
    surfaces: [&Surface; 2],
    contact: &Contact,
    form: &SeparationForm,
    reach: f64,
) -> bool {
    let [across, up] = form.tangent_frame();
    let [first, second] = surfaces;
    let [first_hint, second_hint] = contact.uv;
    let mut separations = (0..RING_SAMPLES).map(|index| {
        let angle = TAU * index as f64 / RING_SAMPLES as f64;
        let point = contact.point + (across * angle.cos_libm() + up * angle.sin_libm()) * reach;
        let foot = first.point_at(first.project(point, Some(first_hint)));
        separation(second, foot, second_hint)
    });
    let Some(Some(start)) = separations.next() else {
        return false;
    };
    let side = start.signum();
    start.abs() > RING_CLEARANCE
        && separations.all(|separation| {
            separation.is_some_and(|separation| separation * side > RING_CLEARANCE)
        })
}

fn separation(surface: &Surface, point: Point3, hint: Point2) -> Option<f64> {
    let uv = surface.project(point, Some(hint));
    let normal = surface.normal(uv.x, uv.y)?;
    let offset = point - surface.point_at(uv);
    let along = offset.dot(normal);
    (offset.reject_from_normalized(normal).length() <= TOLERANCE).then_some(along)
}
