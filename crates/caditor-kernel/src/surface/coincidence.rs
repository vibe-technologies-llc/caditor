use caditor_geometry::{Point2, Point3, Vector3};

use crate::{
    sense::Sense,
    surface::Surface,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION, parallel, same_direction, same_point},
};

const PATCH_SAMPLES: usize = 7;
const PATCH_REACH: f64 = 1.0;
const MAX_NET_SAMPLES: usize = 64;

pub(crate) fn same_surface(a: &Surface, b: &Surface) -> Option<Sense> {
    match (a, b) {
        (Surface::Plane(first), Surface::Plane(second)) => {
            let (first, second) = (first.frame(), second.frame());
            (parallel(first.normal(), second.normal())
                && first.signed_distance(second.origin()).abs() <= LINEAR_RESOLUTION)
                .then(|| Sense::from_sign(first.normal().dot(second.normal())))
        }
        (Surface::Cylinder(first), Surface::Cylinder(second)) => {
            let coaxial = same_axis(
                first.frame().origin(),
                first.frame().normal(),
                second.frame().origin(),
                second.frame().normal(),
            );
            (coaxial && same_length(first.radius(), second.radius())).then_some(Sense::Same)
        }
        (Surface::Cone(first), Surface::Cone(second)) => {
            let matching = same_point(first.apex(), second.apex())
                && same_direction(first.opening_direction(), second.opening_direction())
                && (first.half_angle().abs() - second.half_angle().abs()).abs()
                    <= ANGULAR_RESOLUTION;
            matching.then_some(Sense::Same)
        }
        (Surface::Sphere(first), Surface::Sphere(second)) => {
            (same_point(first.center(), second.center())
                && same_length(first.radius(), second.radius()))
            .then_some(Sense::Same)
        }
        (Surface::Torus(first), Surface::Torus(second)) => {
            let matching = same_point(first.frame().origin(), second.frame().origin())
                && parallel(first.frame().normal(), second.frame().normal())
                && same_length(first.major_radius(), second.major_radius())
                && same_length(first.minor_radius(), second.minor_radius());
            matching.then_some(Sense::Same)
        }
        (Surface::Extrusion(_) | Surface::Revolution(_) | Surface::BSpline(_), _)
        | (_, Surface::Extrusion(_) | Surface::Revolution(_) | Surface::BSpline(_)) => {
            sampled(a, b)
        }
        _ => None,
    }
}

fn same_length(a: f64, b: f64) -> bool {
    (a - b).abs() <= LINEAR_RESOLUTION
}

fn same_axis(
    first_origin: Point3,
    first_axis: Vector3,
    second_origin: Point3,
    second_axis: Vector3,
) -> bool {
    let offset = second_origin - first_origin;
    let off_axis = offset - first_axis * offset.dot(first_axis);
    parallel(first_axis, second_axis) && off_axis.length() <= LINEAR_RESOLUTION
}

fn bounded(surface: &Surface) -> bool {
    let u = surface.u_period().is_some() || surface.u_domain().bounded().is_some();
    let v = surface.v_period().is_some() || surface.v_domain().bounded().is_some();
    u && v
}

fn greville(knots: &[f64], degree: usize) -> Vec<f64> {
    let count = knots.len().saturating_sub(degree + 1);
    let all: Vec<f64> = (0..count)
        .filter_map(|index| {
            let window = knots.get(index + 1..index + 1 + degree)?;
            Some(window.iter().sum::<f64>() / degree.max(1) as f64)
        })
        .collect();
    let stride = all.len().div_ceil(MAX_NET_SAMPLES).max(1);
    all.into_iter().step_by(stride).collect()
}

fn net(surface: &Surface) -> (Vec<f64>, Vec<f64>) {
    match surface {
        Surface::BSpline(spline) => (
            greville(spline.u_knots(), spline.u_degree()),
            greville(spline.v_knots(), spline.v_degree()),
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

fn patch(surface: &Surface) -> Vec<Point2> {
    let u_range = surface.u_domain().clipped(PATCH_REACH);
    let v_range = surface.v_domain().clipped(PATCH_REACH);
    let fraction = |index: usize| (index as f64 + 0.5) / PATCH_SAMPLES as f64;
    let (net_u, net_v) = net(surface);
    let spread = |range: crate::interval::Interval, net: Vec<f64>| -> Vec<f64> {
        (0..PATCH_SAMPLES)
            .map(|index| range.at(fraction(index)))
            .chain(net)
            .collect()
    };
    let columns = spread(u_range, net_u);
    let rows = spread(v_range, net_v);
    rows.iter()
        .flat_map(|v| columns.iter().map(move |u| Point2::new(*u, *v)))
        .collect()
}

fn lies_on(from: &Surface, onto: &Surface) -> bool {
    patch(from).into_iter().all(|uv| {
        let point = from.point_at(uv);
        onto.distance(point) <= LINEAR_RESOLUTION
    })
}

fn sampled(a: &Surface, b: &Surface) -> Option<Sense> {
    let coincident = match (bounded(a), bounded(b)) {
        (true, false) => lies_on(a, b),
        (false, true) => lies_on(b, a),
        (true, true) | (false, false) => lies_on(a, b) && lies_on(b, a),
    };
    if !coincident {
        return None;
    }
    patch(a).into_iter().find_map(|uv| {
        let point = a.point_at(uv);
        let on_b = b.project(point, None);
        let first = a.normal(uv.x, uv.y)?;
        let second = b.normal(on_b.x, on_b.y)?;
        Some(Sense::from_sign(first.dot(second)))
    })
}
