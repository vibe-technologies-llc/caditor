use caditor_geometry::{Aabb2, Point2, Point3};

use crate::{surface::Surface, tolerance::SamplingTolerance};

const LATTICE: usize = 8;
const MAX_SEGMENTS: usize = 1024;
const MAX_GRID_POINTS: f64 = (1 << 19) as f64;
const FLAT_CURVATURE: f64 = 1e-12;
const FLAT_ASPECT: f64 = 4.0;
const SPAN_SAMPLES: usize = 4;
const MAX_LATTICE: usize = 64;
const REFINEMENTS: usize = 4;
const REFINEMENT_MARGIN: f64 = 1.05;
const MIN_GROWTH: f64 = 1.1;
const CHECKED_CELLS: usize = 16;
const COLLAPSED_EDGE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Density {
    pub u_segments: usize,
    pub v_segments: usize,
    pub u_scale: f64,
    pub v_scale: f64,
}

fn lattice(low: f64, high: f64, knots: &[f64]) -> Vec<f64> {
    let mut values: Vec<f64> = (0..=LATTICE)
        .map(|index| low + (high - low) * index as f64 / LATTICE as f64)
        .collect();
    let inside: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|knot| *knot > low && *knot < high)
        .collect();
    let mut previous = low;
    for knot in inside.iter().copied().chain(std::iter::once(high)) {
        if knot > previous {
            values
                .extend((1..SPAN_SAMPLES).map(|index| {
                    previous + (knot - previous) * index as f64 / SPAN_SAMPLES as f64
                }));
            values.push(knot);
        }
        previous = knot;
        if values.len() >= MAX_LATTICE {
            break;
        }
    }
    values.sort_by(f64::total_cmp);
    values.dedup();
    values
}

pub(crate) fn density(surface: &Surface, bounds: Aabb2, tolerance: &SamplingTolerance) -> Density {
    let size = bounds.size();
    let mut u_density: f64 = 0.0;
    let mut v_density: f64 = 0.0;
    let (mut u_speeds, mut v_speeds, mut count) = (0.0, 0.0, 0.0);
    let (u_knots, v_knots): (&[f64], &[f64]) = match surface {
        Surface::BSpline(spline) => (spline.u_knots(), spline.v_knots()),
        _ => (&[], &[]),
    };
    let us = lattice(bounds.min().x, bounds.max().x, u_knots);
    let vs = lattice(bounds.min().y, bounds.max().y, v_knots);
    for v in &vs {
        for u in &us {
            let uv = Point2::new(*u, *v);
            let derivatives = surface.evaluate(uv.x, uv.y);
            let (u_speed, v_speed) = (derivatives.du.length(), derivatives.dv.length());
            if !u_speed.is_finite() || !v_speed.is_finite() {
                continue;
            }
            u_speeds += u_speed;
            v_speeds += v_speed;
            count += 1.0;
            let Some(normal) = derivatives.normal().or_else(|| surface.normal(uv.x, uv.y)) else {
                continue;
            };
            let twist = if u_speed > 0.0 && v_speed > 0.0 {
                derivatives.duv.dot(normal).abs() / (u_speed * v_speed)
            } else {
                0.0
            };
            if u_speed > 0.0 {
                let curvature = derivatives.duu.dot(normal).abs() / (u_speed * u_speed);
                u_density = u_density.max(u_speed / step_for(curvature.max(twist), tolerance));
            }
            if v_speed > 0.0 {
                let curvature = derivatives.dvv.dot(normal).abs() / (v_speed * v_speed);
                v_density = v_density.max(v_speed / step_for(curvature.max(twist), tolerance));
            }
        }
    }
    let (u_scale, v_scale) = (scale(u_speeds, count), scale(v_speeds, count));
    let mut u_segments = segments(u_density * size.x);
    let mut v_segments = segments(v_density * size.y);
    let (u_length, v_length) = (size.x * u_scale, size.y * v_scale);
    if u_density == 0.0 && v_segments > 1 {
        u_segments = segments(u_length * v_segments as f64 / (v_length * FLAT_ASPECT)).max(2);
    }
    if v_density == 0.0 && u_segments > 1 {
        v_segments = segments(v_length * u_segments as f64 / (u_length * FLAT_ASPECT)).max(2);
    }
    let measured = matches!(
        surface,
        Surface::BSpline(_) | Surface::Revolution(_) | Surface::Extrusion(_) | Surface::Cone(_)
    );
    for _ in 0..if measured { REFINEMENTS } else { 0 } {
        let deviation = grid_deviation(surface, bounds, u_segments, v_segments);
        if !deviation.is_finite() || deviation <= tolerance.chord() {
            break;
        }
        let factor = ((deviation / tolerance.chord()).sqrt() * REFINEMENT_MARGIN).max(MIN_GROWTH);
        u_segments = segments(u_segments as f64 * factor);
        v_segments = segments(v_segments as f64 * factor);
    }
    let total = u_segments as f64 * v_segments as f64;
    if total > MAX_GRID_POINTS {
        let shrink = (MAX_GRID_POINTS / total).sqrt();
        u_segments = segments(u_segments as f64 * shrink);
        v_segments = segments(v_segments as f64 * shrink);
    }
    Density {
        u_segments,
        v_segments,
        u_scale,
        v_scale,
    }
}

fn grid_deviation(surface: &Surface, bounds: Aabb2, u_segments: usize, v_segments: usize) -> f64 {
    let size = bounds.size();
    let (u_step, v_step) = (size.x / u_segments as f64, size.y / v_segments as f64);
    let sampled = |count: usize| {
        let stride = (count / CHECKED_CELLS).max(1);
        (0..count).step_by(stride)
    };
    let mut worst: f64 = 0.0;
    for row in sampled(v_segments) {
        for column in sampled(u_segments) {
            let low = bounds.min() + Point2::new(column as f64 * u_step, row as f64 * v_step);
            let corners = [
                surface.point_at(low),
                surface.point_at(low + Point2::new(u_step, 0.0)),
                surface.point_at(low + Point2::new(0.0, v_step)),
                surface.point_at(low + Point2::new(u_step, v_step)),
            ];
            let [a, b, c, d] = corners;
            let span = a.distance(d).max(b.distance(c));
            let collapsed = |p: Point3, q: Point3| p.distance(q) <= COLLAPSED_EDGE * span;
            let deviation = if collapsed(a, b) || collapsed(c, d) {
                let (from, to, at) = if collapsed(a, b) {
                    (c, d, Point2::new(0.5, 1.0))
                } else {
                    (a, b, Point2::new(0.5, 0.0))
                };
                let on_edge = surface.point_at(low + Point2::new(u_step * at.x, v_step * at.y));
                on_edge.distance((from + to) * 0.5)
            } else {
                let middle = surface.point_at(low + Point2::new(u_step, v_step) * 0.5);
                middle
                    .distance((a + d) * 0.5)
                    .min(middle.distance((b + c) * 0.5))
            };
            worst = worst.max(deviation);
        }
    }
    worst
}

fn scale(sum: f64, count: f64) -> f64 {
    let mean = if count > 0.0 { sum / count } else { 0.0 };
    if mean.is_finite() && mean > 0.0 {
        mean
    } else {
        1.0
    }
}

fn step_for(curvature: f64, tolerance: &SamplingTolerance) -> f64 {
    if curvature.is_finite() && curvature > FLAT_CURVATURE {
        tolerance.step_on_radius(1.0 / curvature)
    } else {
        f64::INFINITY
    }
}

fn segments(count: f64) -> usize {
    let count = count.ceil();
    if count.is_finite() && count >= 1.0 {
        (count as usize).min(MAX_SEGMENTS)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::BSplineSurface;

    #[test]
    fn a_twisted_patch_is_divided_until_its_triangles_follow_it() {
        let saddle = BSplineSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
                Point3::new(10.0, 10.0, 10.0),
            ],
            None,
        )
        .unwrap();
        let surface = Surface::BSpline(saddle);
        let bounds = Aabb2::from_points([Point2::ZERO, Point2::new(1.0, 1.0)]).unwrap();
        let tolerance = SamplingTolerance::new(0.01, 0.2).unwrap();
        let found = density(&surface, bounds, &tolerance);
        assert!(found.u_segments > 4 && found.v_segments > 4, "{found:?}");
        assert!(
            grid_deviation(&surface, bounds, found.u_segments, found.v_segments)
                <= tolerance.chord()
        );
    }
}
