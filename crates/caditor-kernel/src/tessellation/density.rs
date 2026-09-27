use caditor_geometry::{Aabb2, Point2};

use crate::{surface::Surface, tolerance::SamplingTolerance};

const LATTICE: usize = 8;
const MAX_SEGMENTS: usize = 1024;
const MAX_GRID_POINTS: f64 = (1 << 19) as f64;
const FLAT_CURVATURE: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Density {
    pub u_segments: usize,
    pub v_segments: usize,
    pub u_scale: f64,
    pub v_scale: f64,
}

pub(crate) fn density(surface: &Surface, bounds: Aabb2, tolerance: &SamplingTolerance) -> Density {
    let size = bounds.size();
    let mut u_density: f64 = 0.0;
    let mut v_density: f64 = 0.0;
    let (mut u_speeds, mut v_speeds, mut count) = (0.0, 0.0, 0.0);
    for row in 0..=LATTICE {
        for column in 0..=LATTICE {
            let fraction = Point2::new(column as f64, row as f64) / LATTICE as f64;
            let uv = bounds.min() + size * fraction;
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
            if u_speed > 0.0 {
                let curvature = derivatives.duu.dot(normal).abs() / (u_speed * u_speed);
                u_density = u_density.max(u_speed / step_for(curvature, tolerance));
            }
            if v_speed > 0.0 {
                let curvature = derivatives.dvv.dot(normal).abs() / (v_speed * v_speed);
                v_density = v_density.max(v_speed / step_for(curvature, tolerance));
            }
        }
    }
    let mut u_segments = segments(u_density * size.x);
    let mut v_segments = segments(v_density * size.y);
    let total = u_segments as f64 * v_segments as f64;
    if total > MAX_GRID_POINTS {
        let shrink = (MAX_GRID_POINTS / total).sqrt();
        u_segments = segments(u_segments as f64 * shrink);
        v_segments = segments(v_segments as f64 * shrink);
    }
    let scale = |sum: f64| {
        let mean = if count > 0.0 { sum / count } else { 0.0 };
        if mean.is_finite() && mean > 0.0 {
            mean
        } else {
            1.0
        }
    };
    Density {
        u_segments,
        v_segments,
        u_scale: scale(u_speeds),
        v_scale: scale(v_speeds),
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
