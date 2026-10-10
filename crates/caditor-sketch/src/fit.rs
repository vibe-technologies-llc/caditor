use caditor_geometry::{Point2, Vector2};

use crate::{
    banded::Banded,
    curve::{BSpline, MAX_SPLINE_DEGREE, basis_values},
};

const MIN_CONTROL_POINTS: usize = 4;
const SAMPLES_PER_CONTROL_POINT: usize = 3;
const PARAMETER_CORRECTIONS: usize = 3;
const NEWTON_STEPS: usize = 3;
const DERIVATIVE_STEP: f64 = 1e-6;
const SAMPLES_PER_SPAN: usize = 16;
const MAX_THROUGH_SAMPLES: usize = 60_000;
const REPEATED_POINT_FRACTION: f64 = 1e-9;

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
        let parameters: Vec<f64> = (0..count).map(|index| index as f64 / last).collect();
        let control_points = collocate(template.degree(), template.knots(), points, &parameters)?;
        BSpline::clamped(control_points)
    }

    pub fn through(
        points: &[Point2],
        tolerance: f64,
        max_control_points: usize,
    ) -> Option<FittedSpline> {
        let points = &distinct(points)?;
        let parameters = chord_parameters(points)?;
        let count = points.len();
        let degree = MAX_SPLINE_DEGREE.min(count - 1);
        if degree == 1 {
            return Some(FittedSpline {
                spline: BSpline::clamped(points.to_vec())?,
                deviation: 0.0,
            });
        }
        let knots = averaged_knots(&parameters, degree);
        let control_points = collocate(degree, &knots, points, &parameters)?;
        let total = ((count - degree) * SAMPLES_PER_SPAN)
            .max(max_control_points.saturating_mul(SAMPLES_PER_CONTROL_POINT))
            .min(MAX_THROUGH_SAMPLES);
        let samples: Vec<Point2> = (0..=total)
            .map(|index| {
                let parameter = index as f64 / total as f64;
                let (start, values) = basis_values(degree, &knots, count, parameter);
                values
                    .iter()
                    .enumerate()
                    .fold(Point2::ZERO, |sum, (offset, value)| {
                        sum + control_points
                            .get(start + offset)
                            .copied()
                            .unwrap_or(Point2::ZERO)
                            * *value
                    })
            })
            .collect();
        Self::fit(&samples, tolerance, max_control_points)
    }
}

fn distinct(points: &[Point2]) -> Option<Vec<Point2>> {
    if points.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let length: f64 = points
        .windows(2)
        .map(|pair| match pair {
            [a, b] => a.distance(*b),
            _ => 0.0,
        })
        .sum();
    let repeated = length * REPEATED_POINT_FRACTION;
    let mut kept: Vec<Point2> = Vec::with_capacity(points.len());
    for point in points {
        match kept.last() {
            Some(last) if last.distance(*point) <= repeated => {}
            _ => kept.push(*point),
        }
    }
    let last = *points.last()?;
    if let [.., before, end] = kept.as_mut_slice()
        && *end != last
        && before.distance(last) > repeated
    {
        *end = last;
    }
    Some(kept)
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

pub(crate) fn least_squares(
    template: &BSpline,
    samples: &[Point2],
    parameters: &[f64],
    first: Point2,
    last: Point2,
) -> Option<BSpline> {
    let count = template.control_points().len();
    let interior = count.checked_sub(2)?;
    let mut normal = Banded::zeros(interior, template.degree());
    let mut right = vec![[0.0; 2]; interior];
    for (sample, parameter) in samples.iter().zip(parameters) {
        let (start, values) = template_basis(template, *parameter);
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
                normal.add(*row, *column, row_value * column_value)?;
            }
            let entry = right.get_mut(*row)?;
            entry[0] += row_value * target.x;
            entry[1] += row_value * target.y;
        }
    }
    let solved = normal.solve(right)?;
    let mut control_points = Vec::with_capacity(count);
    control_points.push(first);
    control_points.extend(solved.iter().map(|[x, y]| Point2::new(*x, *y)));
    control_points.push(last);
    BSpline::clamped(control_points)
}

pub(crate) fn collocate(
    degree: usize,
    knots: &[f64],
    points: &[Point2],
    parameters: &[f64],
) -> Option<Vec<Point2>> {
    let count = points.len();
    let mut matrix = Banded::zeros(count, degree);
    for (row, parameter) in parameters.iter().enumerate() {
        let (start, values) = basis_values(degree, knots, count, *parameter);
        for (offset, value) in values.iter().enumerate() {
            if *value != 0.0 {
                matrix.add(row, start + offset, *value)?;
            }
        }
    }
    let right = points.iter().map(|point| [point.x, point.y]).collect();
    let solved = matrix.solve(right)?;
    Some(solved.iter().map(|[x, y]| Point2::new(*x, *y)).collect())
}

pub(crate) fn averaged_knots(parameters: &[f64], degree: usize) -> Vec<f64> {
    let count = parameters.len();
    let interior = (1..count - degree).map(|first| {
        parameters
            .get(first..first + degree)
            .map_or(0.0, |window| window.iter().sum::<f64>() / degree as f64)
    });
    std::iter::repeat_n(0.0, degree + 1)
        .chain(interior)
        .chain(std::iter::repeat_n(1.0, degree + 1))
        .collect()
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

fn template_basis(template: &BSpline, parameter: f64) -> (usize, Vec<f64>) {
    basis_values(
        template.degree(),
        template.knots(),
        template.control_points().len(),
        parameter,
    )
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
            let (start, values) = template_basis(&spline, parameter);
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

    #[test]
    fn unevenly_spaced_points_are_interpolated_without_loops() {
        let points: Vec<Point2> = [0.0, 0.1, 0.2, 0.3, 6.0, 12.0]
            .iter()
            .map(|x| Point2::new(*x, x * x / 10.0))
            .collect();
        let fitted = BSpline::through(&points, 1e-5, 400).unwrap();
        assert!(fitted.deviation <= 1e-5, "{}", fitted.deviation);
        let spline = fitted.spline;
        for point in &points {
            let nearest = (0..=100_000)
                .map(|index| spline.point_at(index as f64 / 100_000.0).distance(*point))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest < 1e-3, "{nearest}");
        }
        let xs: Vec<f64> = (0..=400)
            .map(|index| spline.point_at(index as f64 / 400.0).x)
            .collect();
        assert!(
            xs.windows(2).all(|pair| pair[1] >= pair[0] - 1e-9),
            "the interpolated curve turns back"
        );
    }

    #[test]
    fn repeated_points_are_passed_through_once() {
        let points = [
            Point2::ZERO,
            Point2::ZERO,
            Point2::new(1.0, 1.0),
            Point2::new(1.0, 1.0 + 1e-13),
            Point2::new(2.0, 0.0),
            Point2::new(3.0, 1.0),
            Point2::new(3.0, 1.0),
        ];
        let fitted = BSpline::through(&points, 1e-6, 400).unwrap();
        assert!(fitted.deviation <= 1e-6, "{}", fitted.deviation);
        assert_eq!(fitted.spline.point_at(0.0), Point2::ZERO);
        assert_eq!(fitted.spline.point_at(1.0), Point2::new(3.0, 1.0));

        let line = BSpline::through(&[Point2::ZERO, Point2::X, Point2::X], 1e-6, 400).unwrap();
        assert_eq!(line.spline.degree(), 1);
        assert!(BSpline::through(&[Point2::X, Point2::X, Point2::X], 1e-6, 400).is_none());
    }

    #[test]
    fn long_fits_and_interpolations_stay_fast() {
        let samples: Vec<Point2> = (0..=4_000)
            .map(|index| {
                let angle = index as f64 / 40.0;
                Point2::new(angle * 10.0, angle.sin() * 5.0)
            })
            .collect();
        let started = std::time::Instant::now();
        let fitted = BSpline::fit(&samples, 1e-3, 400).unwrap();
        assert!(fitted.deviation.is_finite());
        let through: Vec<Point2> = samples.iter().step_by(10).copied().collect();
        assert!(BSpline::interpolate(&through).is_some());
        assert!(BSpline::through(&through, 1e-3, 400).is_some());
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }
}
