use caditor_geometry::Point2;

use crate::{
    intersect::SurfacePatch, sense::Sense, surface::Surface, tolerance::LINEAR_RESOLUTION,
};

const SAMPLES: usize = 9;
const LEAST_SHARED: usize = 4;
const BESIDE: f64 = 1e-3;

pub(super) fn sampled_kind(surface: &Surface) -> bool {
    matches!(
        surface,
        Surface::Extrusion(_) | Surface::Revolution(_) | Surface::BSpline(_)
    )
}

struct Apart;

fn shared_signs(from: &SurfacePatch, onto: &SurfacePatch) -> Result<Vec<Option<f64>>, Apart> {
    let mut shared = Vec::new();
    for uv in grid(from) {
        let point = from.surface().point_at(uv);
        let Some(foot) = onto.locate(point) else {
            continue;
        };
        let Some(normal) = onto.surface().normal(foot.x, foot.y) else {
            continue;
        };
        let offset = point - onto.surface().point_at(foot);
        let distance = offset.length();
        if distance > LINEAR_RESOLUTION {
            let beside = (offset - normal * offset.dot(normal)).length();
            if beside > BESIDE * distance {
                continue;
            }
            return Err(Apart);
        }
        shared.push(from.surface().normal(uv.x, uv.y).map(|own| own.dot(normal)));
    }
    Ok(shared)
}

pub(super) fn coincident_part(first: &SurfacePatch, second: &SurfacePatch) -> Option<Sense> {
    let onto_first_is_cheaper = sampled_kind(second.surface()) && !sampled_kind(first.surface());
    let (onto_second, onto_first) = if onto_first_is_cheaper {
        let onto_first = shared_signs(second, first).ok()?;
        (shared_signs(first, second).ok()?, onto_first)
    } else {
        let onto_second = shared_signs(first, second).ok()?;
        (onto_second, shared_signs(second, first).ok()?)
    };
    if onto_second.len() + onto_first.len() < LEAST_SHARED {
        return None;
    }
    onto_second
        .into_iter()
        .chain(onto_first)
        .flatten()
        .find(|sign| sign.abs() > 0.5)
        .map(Sense::from_sign)
}

fn grid(patch: &SurfacePatch) -> Vec<Point2> {
    let (u, v) = (patch.u_range(), patch.v_range());
    let fraction = |index: usize| (index as f64 + 0.5) / SAMPLES as f64;
    let mut points = Vec::with_capacity(SAMPLES * SAMPLES);
    for row in 0..SAMPLES {
        for column in 0..SAMPLES {
            points.push(Point2::new(u.at(fraction(column)), v.at(fraction(row))));
        }
    }
    points
}
