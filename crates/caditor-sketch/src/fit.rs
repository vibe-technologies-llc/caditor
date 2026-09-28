use caditor_geometry::{Point2, Vector2};
use nalgebra::{DMatrix, DVector};

use crate::curve::BSpline;

const MIN_CONTROL_POINTS: usize = 4;
const SAMPLES_PER_CONTROL_POINT: usize = 3;
const PARAMETER_CORRECTIONS: usize = 3;
const NEWTON_STEPS: usize = 3;
const DERIVATIVE_STEP: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq)]
pub struct FittedSpline {
    pub spline: BSpline,
    pub deviation: f64,
}

impl BSpline {
    pub fn fit(
        samples: &[Point2],
        tolerance: f64,
        max_control_points: usize,
    ) -> Option<FittedSpline> {
        let (first, last) = (*samples.first()?, *samples.last()?);
        let parameters = chord_parameters(samples)?;
        let limit = max_control_points
            .min(samples.len() / SAMPLES_PER_CONTROL_POINT)
            .max(MIN_CONTROL_POINTS);
        let mut best: Option<FittedSpline> = None;
        let mut count = MIN_CONTROL_POINTS;
        loop {
            if let Some(fitted) = fit_with(samples, &parameters, count, first, last) {
                let within = fitted.deviation <= tolerance;
                if best
                    .as_ref()
                    .is_none_or(|best| fitted.deviation < best.deviation)
                {
                    best = Some(fitted);
                }
                if within {
                    break;
                }
            }
            if count >= limit {
                break;
            }
            count = (count * 2).min(limit);
        }
        best
    }

    pub fn interpolate(points: &[Point2]) -> Option<BSpline> {
        let count = points.len();
        if count < 2 || points.iter().any(|point| !point.is_finite()) {
            return None;
        }
        let template = BSpline::clamped(vec![Point2::ZERO; count])?;
        let last = (count - 1) as f64;
        let matrix = DMatrix::from_fn(count, count, |row, column| {
            basis_value(&template, row as f64 / last, column)
        });
        let decomposition = matrix.lu();
        let solve = |coordinate: fn(&Point2) -> f64| {
            decomposition.solve(&DVector::from_iterator(
                count,
                points.iter().map(coordinate),
            ))
        };
        let (xs, ys) = (solve(|point| point.x)?, solve(|point| point.y)?);
        let control_points: Vec<Point2> = xs
            .iter()
            .zip(ys.iter())
            .map(|(x, y)| Point2::new(*x, *y))
            .collect();
        control_points
            .iter()
            .all(|point| point.is_finite())
            .then(|| BSpline::clamped(control_points))?
    }
}

fn chord_parameters(samples: &[Point2]) -> Option<Vec<f64>> {
    let mut travelled = 0.0;
    let mut parameters = Vec::with_capacity(samples.len());
    let mut previous = *samples.first()?;
    for sample in samples {
        if !sample.is_finite() {
            return None;
        }
        travelled += sample.distance(previous);
        parameters.push(travelled);
        previous = *sample;
    }
    if travelled <= 0.0 || samples.len() < 2 {
        return None;
    }
    parameters.iter_mut().for_each(|value| *value /= travelled);
    Some(parameters)
}

fn fit_with(
    samples: &[Point2],
    parameters: &[f64],
    count: usize,
    first: Point2,
    last: Point2,
) -> Option<FittedSpline> {
    let template = BSpline::clamped(vec![Point2::ZERO; count])?;
    let mut parameters = parameters.to_vec();
    let mut spline = least_squares(&template, samples, &parameters, first, last)?;
    for _ in 0..PARAMETER_CORRECTIONS {
        correct_parameters(&spline, samples, &mut parameters);
        spline = least_squares(&template, samples, &parameters, first, last)?;
    }
    correct_parameters(&spline, samples, &mut parameters);
    let deviation = samples
        .iter()
        .zip(&parameters)
        .map(|(sample, parameter)| spline.point_at(*parameter).distance(*sample))
        .fold(0.0, f64::max);
    Some(FittedSpline { spline, deviation })
}

fn least_squares(
    template: &BSpline,
    samples: &[Point2],
    parameters: &[f64],
    first: Point2,
    last: Point2,
) -> Option<BSpline> {
    let count = template.control_points().len();
    let interior = count.checked_sub(2)?;
    let mut normal = DMatrix::<f64>::zeros(interior, interior);
    let mut right = DMatrix::<f64>::zeros(interior, 2);
    for (sample, parameter) in samples.iter().zip(parameters) {
        let (start, values) = basis_values(template, *parameter);
        let mut target = *sample;
        let mut unknowns = Vec::with_capacity(values.len());
        for (offset, value) in values.iter().enumerate() {
            let index = start + offset;
            if index == 0 {
                target -= first * *value;
            } else if index == count - 1 {
                target -= last * *value;
            } else {
                unknowns.push((index - 1, *value));
            }
        }
        for (row, row_value) in &unknowns {
            for (column, column_value) in &unknowns {
                if let Some(entry) = normal.get_mut((*row, *column)) {
                    *entry += row_value * column_value;
                }
            }
            if let Some(entry) = right.get_mut((*row, 0)) {
                *entry += row_value * target.x;
            }
            if let Some(entry) = right.get_mut((*row, 1)) {
                *entry += row_value * target.y;
            }
        }
    }
    let solved = normal.cholesky()?.solve(&right);
    let mut control_points = Vec::with_capacity(count);
    control_points.push(first);
    for row in 0..interior {
        let point = Point2::new(*solved.get((row, 0))?, *solved.get((row, 1))?);
        if !point.is_finite() {
            return None;
        }
        control_points.push(point);
    }
    control_points.push(last);
    BSpline::clamped(control_points)
}

fn correct_parameters(spline: &BSpline, samples: &[Point2], parameters: &mut [f64]) {
    for (sample, parameter) in samples.iter().zip(parameters.iter_mut()) {
        for _ in 0..NEWTON_STEPS {
            let (point, first, second) = derivatives(spline, *parameter);
            let offset = point - *sample;
            let slope = first.length_squared() + offset.dot(second);
            if slope <= 0.0 || !slope.is_finite() {
                break;
            }
            *parameter = (*parameter - offset.dot(first) / slope).clamp(0.0, 1.0);
        }
    }
}

fn derivatives(spline: &BSpline, parameter: f64) -> (Point2, Vector2, Vector2) {
    let step = DERIVATIVE_STEP;
    let low = (parameter - step).max(0.0);
    let high = (parameter + step).min(1.0);
    let middle = (low + high) / 2.0;
    let half = (high - low) / 2.0;
    let (before, at, after) = (
        spline.point_at(low),
        spline.point_at(middle),
        spline.point_at(high),
    );
    (
        spline.point_at(parameter),
        (after - before) / (2.0 * half),
        (after - 2.0 * at + before) / (half * half),
    )
}

fn basis_value(template: &BSpline, parameter: f64, index: usize) -> f64 {
    let (start, values) = basis_values(template, parameter);
    index
        .checked_sub(start)
        .and_then(|offset| values.get(offset))
        .copied()
        .unwrap_or(0.0)
}

fn basis_values(template: &BSpline, parameter: f64) -> (usize, Vec<f64>) {
    let degree = template.degree();
    let knots = template.knots();
    let last = template.control_points().len().saturating_sub(1);
    let parameter = parameter.clamp(0.0, 1.0);
    let span = (degree..last)
        .find(|span| knots.get(span + 1).is_some_and(|next| parameter < *next))
        .unwrap_or(last);
    let knot = |index: usize| knots.get(index).copied().unwrap_or(0.0);
    let mut values = vec![0.0; degree + 1];
    let mut left = vec![0.0; degree + 1];
    let mut right = vec![0.0; degree + 1];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    for level in 1..=degree {
        if let (Some(slot_left), Some(slot_right)) = (left.get_mut(level), right.get_mut(level)) {
            *slot_left = parameter - knot((span + 1).saturating_sub(level));
            *slot_right = knot(span + level) - parameter;
        }
        let mut saved = 0.0;
        for index in 0..level {
            let low = right.get(index + 1).copied().unwrap_or(0.0);
            let high = left.get(level - index).copied().unwrap_or(0.0);
            let width = low + high;
            let value = values.get(index).copied().unwrap_or(0.0);
            let share = if width != 0.0 { value / width } else { 0.0 };
            if let Some(slot) = values.get_mut(index) {
                *slot = saved + low * share;
            }
            saved = high * share;
        }
        if let Some(slot) = values.get_mut(level) {
            *slot = saved;
        }
    }
    (span - degree, values)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, TAU};

    use super::*;

    fn quarter_ellipse(count: usize) -> Vec<Point2> {
        (0..=count)
            .map(|index| {
                let angle = FRAC_PI_2 * index as f64 / count as f64;
                Point2::new(30.0 * angle.cos(), 10.0 * angle.sin())
            })
            .collect()
    }

    #[test]
    fn basis_values_sum_to_one_and_match_the_spline() {
        let spline = BSpline::clamped(vec![
            Point2::ZERO,
            Point2::new(1.0, 2.0),
            Point2::new(3.0, 2.0),
            Point2::new(4.0, 0.0),
            Point2::new(6.0, 1.0),
            Point2::new(7.0, 3.0),
        ])
        .unwrap();
        for step in 0..=20 {
            let parameter = step as f64 / 20.0;
            let (start, values) = basis_values(&spline, parameter);
            assert!((values.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            let combined = values
                .iter()
                .enumerate()
                .fold(Point2::ZERO, |sum, (offset, value)| {
                    sum + spline.control_points()[start + offset] * *value
                });
            assert!(combined.distance(spline.point_at(parameter)) < 1e-12);
        }
    }

    #[test]
    fn a_fit_keeps_the_ends_and_reaches_the_tolerance() {
        let samples = quarter_ellipse(400);
        let fitted = BSpline::fit(&samples, 1e-4, 200).unwrap();
        assert!(fitted.deviation <= 1e-4, "{}", fitted.deviation);
        assert_eq!(fitted.spline.degree(), 3);
        assert_eq!(fitted.spline.point_at(0.0), samples[0]);
        assert_eq!(fitted.spline.point_at(1.0), *samples.last().unwrap());
        for step in 0..=90 {
            let angle = FRAC_PI_2 * step as f64 / 90.0;
            let exact = Point2::new(30.0 * angle.cos(), 10.0 * angle.sin());
            let nearest = (0..=2000)
                .map(|index| {
                    fitted
                        .spline
                        .point_at(index as f64 / 2000.0)
                        .distance(exact)
                })
                .fold(f64::INFINITY, f64::min);
            assert!(nearest < 1e-2, "{nearest}");
        }
    }

    #[test]
    fn a_fit_that_cannot_reach_the_tolerance_returns_its_best() {
        let zigzag: Vec<Point2> = (0..40)
            .map(|index| Point2::new(index as f64, if index % 2 == 0 { 0.0 } else { 1.0 }))
            .collect();
        let fitted = BSpline::fit(&zigzag, 1e-9, 8).unwrap();
        assert!(fitted.deviation > 1e-9);
        assert!(fitted.deviation.is_finite());
        assert_eq!(fitted.spline.control_points().len(), 8);
    }

    #[test]
    fn a_fit_needs_distinct_samples() {
        assert!(BSpline::fit(&[], 1e-3, 10).is_none());
        assert!(BSpline::fit(&[Point2::X; 10], 1e-3, 10).is_none());
        assert!(BSpline::fit(&[Point2::ZERO, Point2::new(f64::NAN, 0.0)], 1e-3, 10).is_none());
    }

    #[test]
    fn interpolation_passes_through_every_point() {
        let points: Vec<Point2> = (0..7)
            .map(|index| {
                let angle = TAU * index as f64 / 9.0;
                Point2::new(angle.cos() * 5.0, angle.sin() * 3.0 + index as f64)
            })
            .collect();
        let spline = BSpline::interpolate(&points).unwrap();
        assert_eq!(spline.control_points().len(), points.len());
        for (index, point) in points.iter().enumerate() {
            let parameter = index as f64 / (points.len() - 1) as f64;
            assert!(spline.point_at(parameter).distance(*point) < 1e-9);
        }
        let line = BSpline::interpolate(&[Point2::ZERO, Point2::X]).unwrap();
        assert_eq!(line.degree(), 1);
        assert!(BSpline::interpolate(&[Point2::ZERO]).is_none());
    }
}
