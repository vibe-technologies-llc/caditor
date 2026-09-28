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

pub(super) fn coincident_part(first: &SurfacePatch, second: &SurfacePatch) -> Option<Sense> {
    let mut shared = Vec::new();
    for (from, onto) in [(first, second), (second, first)] {
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
                return None;
            }
            let sign = from.surface().normal(uv.x, uv.y).map(|own| own.dot(normal));
            shared.push(sign);
        }
    }
    if shared.len() < LEAST_SHARED {
        return None;
    }
    shared
        .into_iter()
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
