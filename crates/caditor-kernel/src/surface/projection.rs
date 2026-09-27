use caditor_geometry::{Point2, Point3};

use crate::surface::Surface;

pub(crate) const AXIS_EPSILON: f64 = 1e-14;
const PERIODIC_SNAP: f64 = 1e-12;
const MAX_REFINE_ITERATIONS: usize = 40;
const MAX_HALVINGS: usize = 30;
const STEP_EPSILON: f64 = 1e-15;

pub(crate) fn periodic_near(value: f64, period: f64, hint: Option<f64>) -> f64 {
    match hint.filter(|hint| hint.is_finite()) {
        Some(hint) => value + ((hint - value) / period).round() * period,
        None => {
            let wrapped = value.rem_euclid(period);
            if period - wrapped <= PERIODIC_SNAP * period {
                0.0
            } else {
                wrapped
            }
        }
    }
}

pub(crate) fn refine(surface: &Surface, point: Point3, start: Point2) -> Point2 {
    let clamp = |uv: Point2| {
        let u = match surface.u_period() {
            Some(_) => uv.x,
            None => surface.u_domain().clamp(uv.x),
        };
        let v = match surface.v_period() {
            Some(_) => uv.y,
            None => surface.v_domain().clamp(uv.y),
        };
        Point2::new(u, v)
    };
    let distance = |uv: Point2| surface.point(uv.x, uv.y).distance_squared(point);
    let mut uv = clamp(start);
    let mut current = distance(uv);
    for _ in 0..MAX_REFINE_ITERATIONS {
        if !current.is_finite() || current == 0.0 {
            break;
        }
        let Some(step) = newton_step(surface, point, uv) else {
            break;
        };
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = clamp(uv + step * scale);
            let value = distance(candidate);
            if value < current {
                accepted = Some((candidate, value));
                break;
            }
            scale *= 0.5;
        }
        let Some((next, value)) = accepted else {
            break;
        };
        let moved = (next - uv).length();
        uv = next;
        current = value;
        if moved <= STEP_EPSILON * (1.0 + uv.length()) {
            break;
        }
    }
    uv
}

fn newton_step(surface: &Surface, point: Point3, uv: Point2) -> Option<Point2> {
    let derivatives = surface.evaluate(uv.x, uv.y);
    let offset = derivatives.point - point;
    let gradient = Point2::new(derivatives.du.dot(offset), derivatives.dv.dot(offset));
    let (uu, uv_, vv) = (
        derivatives.du.dot(derivatives.du),
        derivatives.du.dot(derivatives.dv),
        derivatives.dv.dot(derivatives.dv),
    );
    let full = (
        uu + derivatives.duu.dot(offset),
        uv_ + derivatives.duv.dot(offset),
        vv + derivatives.dvv.dot(offset),
    );
    solve_positive(full, gradient).or_else(|| solve_positive((uu, uv_, vv), gradient))
}

fn solve_positive((a, b, c): (f64, f64, f64), gradient: Point2) -> Option<Point2> {
    let determinant = a * c - b * b;
    let scale = (a * a + 2.0 * b * b + c * c).sqrt();
    if a > 0.0 && determinant > 1e-14 * scale * scale {
        let step = Point2::new(
            -(c * gradient.x - b * gradient.y) / determinant,
            -(a * gradient.y - b * gradient.x) / determinant,
        );
        step.is_finite().then_some(step)
    } else if a > 0.0 && c <= 1e-14 * scale {
        Some(Point2::new(-gradient.x / a, 0.0))
    } else if c > 0.0 && a <= 1e-14 * scale {
        Some(Point2::new(0.0, -gradient.y / c))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;

    #[test]
    fn periodic_values_land_near_the_hint_or_in_the_principal_range() {
        assert_eq!(periodic_near(-1e-17, TAU, None), 0.0);
        assert!((periodic_near(-1.0, TAU, None) - (TAU - 1.0)).abs() < 1e-12);
        assert!((periodic_near(0.5, TAU, Some(TAU)) - (TAU + 0.5)).abs() < 1e-12);
        assert!((periodic_near(0.1, TAU, Some(-6.0)) - (0.1 - TAU)).abs() < 1e-12);
        assert_eq!(periodic_near(0.3, TAU, Some(f64::NAN)), 0.3);
    }
}
