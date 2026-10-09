use std::f64::consts::TAU;

use caditor_geometry::{Aabb2, Point2};

use crate::{
    coordinates::distance_to_segment,
    curve2::Curve2,
    interval::Interval,
    numeric::integrate,
    parametric::{Parametric, refined_seeds},
};

const AREA_REFINEMENT: usize = 2;
const MAX_WINDING_DEPTH: usize = 40;
const WINDING_CLEARANCE: f64 = 4.0;

pub(crate) fn area_under(curve: &Curve2, range: Interval) -> f64 {
    match curve {
        Curve2::Line(_) => {
            0.5 * curve
                .point(range.start())
                .perp_dot(curve.point(range.end()))
        }
        Curve2::Circle(_) | Curve2::BSpline(_) | Curve2::Ellipse(_) => {
            let breaks = refined_seeds(curve, range, AREA_REFINEMENT);
            integrate(&breaks, |parameter| {
                let derivatives = curve.evaluate(parameter);
                0.5 * derivatives.point.perp_dot(derivatives.first)
            })
        }
    }
}

pub(crate) fn overlaps(a: &Aabb2, b: &Aabb2) -> bool {
    a.min().cmple(b.max()).all() && b.min().cmple(a.max()).all()
}

fn angle_seen(from: Point2, a: Point2, b: Point2) -> f64 {
    let (a, b) = (a - from, b - from);
    a.perp_dot(b).atan2(a.dot(b))
}

fn swept_angle(curve: &Curve2, range: Interval, point: Point2) -> f64 {
    if let Curve2::Line(_) = curve {
        return angle_seen(point, curve.point(range.start()), curve.point(range.end()));
    }
    let seeds = curve.seeds(range);
    let mut pending: Vec<(f64, f64, usize)> = seeds
        .windows(2)
        .filter_map(|pair| match pair {
            [low, high] => Some((*low, *high, 0)),
            _ => None,
        })
        .collect();
    let mut total = 0.0;
    while let Some((low, high, depth)) = pending.pop() {
        let (a, b) = (curve.point(low), curve.point(high));
        let chord = angle_seen(point, a, b);
        let middle = 0.5 * (low + high);
        let deviation = distance_to_segment(curve.point(middle), a, b);
        let clearance = distance_to_segment(point, a, b);
        let settled = deviation * WINDING_CLEARANCE < clearance;
        if settled || depth >= MAX_WINDING_DEPTH || middle <= low || middle >= high {
            total += chord;
        } else {
            pending.push((low, middle, depth + 1));
            pending.push((middle, high, depth + 1));
        }
    }
    total
}

pub(crate) fn winding<'a>(
    segments: impl Iterator<Item = (&'a Curve2, Interval, bool)>,
    point: Point2,
) -> i64 {
    let total: f64 = segments
        .map(|(curve, range, reversed)| {
            let angle = swept_angle(curve, range, point);
            if reversed { -angle } else { angle }
        })
        .sum();
    (total / TAU).round() as i64
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::curve2::{Circle2, Line2};

    #[test]
    fn areas_and_windings_of_a_circle() {
        let circle: Curve2 = Circle2::new(Point2::new(3.0, 1.0), 2.0).unwrap().into();
        let area = area_under(&circle, Interval::FULL_TURN);
        assert!((area - 4.0 * PI).abs() < 1e-12);
        let inside = Point2::new(3.5, 1.2);
        let loop_of = || std::iter::once((&circle, Interval::FULL_TURN, false));
        assert_eq!(winding(loop_of(), inside), 1);
        assert_eq!(winding(loop_of(), Point2::new(5.0 + 1e-9, 1.0)), 0);
        assert_eq!(winding(loop_of(), Point2::new(5.0 - 1e-9, 1.0)), 1);
        let backwards = std::iter::once((&circle, Interval::FULL_TURN, true));
        assert_eq!(winding(backwards, inside), -1);
    }

    #[test]
    fn a_line_contributes_half_the_cross_product_of_its_ends() {
        let line: Curve2 = Line2::through(Point2::new(1.0, 0.0), Point2::new(1.0, 2.0))
            .unwrap()
            .into();
        assert!((area_under(&line, Interval::new(0.0, 2.0).unwrap()) - 1.0).abs() < 1e-15);
    }
}
