use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::{Point2, Vector2};

const MAX_SEGMENTS: usize = 4096;
const MIN_SEGMENT_ANGLE: f64 = 1e-3;
const DEFAULT_SEGMENT_ANGLE: f64 = 5.0 * TAU / 360.0;
const SPLINE_SEGMENTS_PER_SPAN: usize = 4;
pub(crate) const MAX_SPLINE_DEGREE: usize = 3;
const MIN_SEGMENTS_PER_TURN: f64 = 12.0;
const MAX_SEGMENTS_PER_TURN: f64 = 1024.0;
const CHORD_ERROR_PER_BENDING: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Faceting {
    chord: f64,
}

impl Faceting {
    pub fn within(chord: f64) -> Self {
        Self {
            chord: if chord.is_nan() {
                f64::MIN_POSITIVE
            } else {
                chord.max(f64::MIN_POSITIVE)
            },
        }
    }

    pub fn chord(self) -> f64 {
        self.chord
    }

    pub fn arc_segments(self, radius: f64, sweep: f64) -> usize {
        let sweep = if sweep.is_finite() {
            sweep.abs().min(TAU)
        } else {
            TAU
        };
        let cosine = (1.0 - self.chord / radius.abs()).max(-1.0);
        let step =
            (2.0 * cosine.acos()).clamp(TAU / MAX_SEGMENTS_PER_TURN, TAU / MIN_SEGMENTS_PER_TURN);
        whole_segments(sweep / step)
    }

    pub fn spline_segments(self, spline: &BSpline) -> usize {
        let by_chord = (spline.bending_bound() / (CHORD_ERROR_PER_BENDING * self.chord)).sqrt();
        let by_turning = spline.control_polygon_turning() * MIN_SEGMENTS_PER_TURN / TAU;
        whole_segments(by_chord.max(by_turning))
            .max(spline.spans())
            .min(MAX_SEGMENTS)
    }
}

fn whole_segments(count: f64) -> usize {
    if count.is_finite() && count > 1.0 {
        (count.ceil() as usize).min(MAX_SEGMENTS)
    } else {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcGeometry {
    pub center: Point2,
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
}

impl ArcGeometry {
    pub fn from_points(center: Point2, start: Point2, end: Point2) -> Self {
        let start_angle = direction_angle(start - center);
        let sweep = (direction_angle(end - center) - start_angle).rem_euclid(TAU);
        Self {
            center,
            radius: start.distance(center),
            start_angle,
            sweep: if sweep > 0.0 { sweep } else { TAU },
        }
    }

    pub fn full_circle(center: Point2, radius: f64) -> Self {
        Self {
            center,
            radius,
            start_angle: 0.0,
            sweep: TAU,
        }
    }

    pub fn end_angle(&self) -> f64 {
        self.start_angle + self.sweep
    }

    pub fn point_at(&self, angle: f64) -> Point2 {
        self.center + Vector2::from_angle(angle) * self.radius
    }

    pub fn polyline(&self, max_segment_angle: f64) -> Vec<Point2> {
        self.divided(segments_for(self.sweep, max_segment_angle))
    }

    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        self.divided(faceting.arc_segments(self.radius, self.sweep))
    }

    fn divided(&self, segments: usize) -> Vec<Point2> {
        (0..=segments)
            .map(|index| {
                let fraction = index as f64 / segments as f64;
                self.point_at(self.start_angle + self.sweep * fraction)
            })
            .collect()
    }
}

pub(crate) fn direction_angle(direction: Vector2) -> f64 {
    direction.y.atan2(direction.x)
}

#[derive(Debug, Clone, PartialEq)]
pub struct BSpline {
    control_points: Vec<Point2>,
    degree: usize,
    knots: Vec<f64>,
}

impl BSpline {
    pub fn clamped(control_points: Vec<Point2>) -> Option<Self> {
        let count = control_points.len();
        if count < 2 {
            return None;
        }
        let (degree, knots) = clamped_knots(count);
        Some(Self {
            control_points,
            degree,
            knots,
        })
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn control_points(&self) -> &[Point2] {
        &self.control_points
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn point_at(&self, parameter: f64) -> Point2 {
        let parameter = if parameter.is_nan() {
            0.0
        } else {
            parameter.clamp(0.0, 1.0)
        };
        self.de_boor(parameter)
            .or_else(|| self.control_points.first().copied())
            .unwrap_or(Point2::ZERO)
    }

    pub fn derivatives(&self, parameter: f64) -> [Vector2; 2] {
        let count = self.control_points.len();
        [1, 2].map(|order| {
            let (first, weights) =
                basis_derivatives(self.degree, &self.knots, count, parameter, order);
            weights
                .iter()
                .zip(self.control_points.iter().skip(first))
                .fold(Vector2::ZERO, |sum, (weight, point)| sum + *point * *weight)
        })
    }

    pub fn polyline(&self, max_segment_angle: f64) -> Vec<Point2> {
        let turning = self.control_polygon_turning();
        let by_angle = segments_for(turning, max_segment_angle);
        self.divided((self.spans() * SPLINE_SEGMENTS_PER_SPAN + by_angle).min(MAX_SEGMENTS))
    }

    pub fn faceted(&self, faceting: Faceting) -> Vec<Point2> {
        self.divided(faceting.spline_segments(self))
    }

    fn divided(&self, segments: usize) -> Vec<Point2> {
        (0..=segments)
            .map(|index| self.point_at(index as f64 / segments as f64))
            .collect()
    }

    fn spans(&self) -> usize {
        self.control_points.len().saturating_sub(self.degree).max(1)
    }

    fn bending_bound(&self) -> f64 {
        let Some(lower) = self.degree.checked_sub(1).filter(|lower| *lower > 0) else {
            return 0.0;
        };
        let degree = self.degree;
        let knot = |index: usize| self.knots.get(index).copied().unwrap_or(0.0);
        let scaled = |a: Vector2, b: Vector2, factor: usize, width: f64| {
            if width > 0.0 {
                (b - a) * (factor as f64 / width)
            } else {
                Vector2::ZERO
            }
        };
        let tangents: Vec<Vector2> = self
            .control_points
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [a, b] => Some(scaled(
                    *a,
                    *b,
                    degree,
                    knot(index + degree + 1) - knot(index + 1),
                )),
                _ => None,
            })
            .collect();
        tangents
            .windows(2)
            .enumerate()
            .filter_map(|(index, pair)| match pair {
                [a, b] => {
                    Some(scaled(*a, *b, lower, knot(index + degree + 1) - knot(index + 2)).length())
                }
                _ => None,
            })
            .fold(0.0, f64::max)
    }

    fn control_polygon_turning(&self) -> f64 {
        let edges: Vec<Vector2> = self
            .control_points
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(*b - *a).filter(|edge| edge.length_squared() > 0.0),
                _ => None,
            })
            .collect();
        edges
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(a.angle_to(*b).abs()),
                _ => None,
            })
            .filter(|angle| angle.is_finite())
            .sum()
    }

    pub(crate) fn span_containing(&self, parameter: f64) -> usize {
        let last = self.control_points.len().saturating_sub(1);
        let above = self.knots.partition_point(|knot| *knot <= parameter);
        above.saturating_sub(1).clamp(self.degree.min(last), last)
    }

    fn de_boor(&self, parameter: f64) -> Option<Point2> {
        let degree = self.degree;
        let span = self.span_containing(parameter);
        let first = span.checked_sub(degree)?;
        let mut points: Vec<Point2> = self.control_points.get(first..=span)?.to_vec();
        for level in 1..=degree {
            for index in (level..=degree).rev() {
                let low = *self.knots.get(index + first)?;
                let high = *self.knots.get(index + 1 + span - level)?;
                let width = high - low;
                let alpha = if width > 0.0 {
                    (parameter - low) / width
                } else {
                    0.0
                };
                let previous = *points.get(index - 1)?;
                let current = points.get_mut(index)?;
                *current = previous.lerp(*current, alpha);
            }
        }
        points.get(degree).copied()
    }
}

pub(crate) fn basis_values(
    degree: usize,
    knots: &[f64],
    count: usize,
    parameter: f64,
) -> (usize, Vec<f64>) {
    let parameter = parameter.clamp(0.0, 1.0);
    let span = span_of(degree, knots, count, parameter);
    (span - degree, local_basis(degree, knots, span, parameter))
}

pub(crate) fn basis_derivatives(
    degree: usize,
    knots: &[f64],
    count: usize,
    parameter: f64,
    order: usize,
) -> (usize, Vec<f64>) {
    let parameter = parameter.clamp(0.0, 1.0);
    let span = span_of(degree, knots, count, parameter);
    (
        span - degree,
        local_derivatives(degree, knots, span, parameter, order),
    )
}

pub(crate) fn clamped_knots(count: usize) -> (usize, Vec<f64>) {
    let degree = MAX_SPLINE_DEGREE.min(count.saturating_sub(1));
    let spans = count.saturating_sub(degree).max(1);
    let knots = std::iter::repeat_n(0.0, degree + 1)
        .chain((1..spans).map(|index| index as f64 / spans as f64))
        .chain(std::iter::repeat_n(1.0, degree + 1))
        .collect();
    (degree, knots)
}

fn span_of(degree: usize, knots: &[f64], count: usize, parameter: f64) -> usize {
    let last = count.saturating_sub(1);
    let above = knots.partition_point(|knot| *knot <= parameter);
    above
        .saturating_sub(1)
        .clamp(degree.min(last), last)
        .max(degree)
}

fn local_derivatives(
    degree: usize,
    knots: &[f64],
    span: usize,
    parameter: f64,
    order: usize,
) -> Vec<f64> {
    if order == 0 {
        return local_basis(degree, knots, span, parameter);
    }
    let Some(lower_degree) = degree.checked_sub(1) else {
        return vec![0.0];
    };
    let lower = local_derivatives(lower_degree, knots, span, parameter, order - 1);
    let knot = |index: usize| knots.get(index).copied().unwrap_or(0.0);
    let share = |value: f64, low: usize, high: usize| {
        let width = knot(high) - knot(low);
        if width > 0.0 { value / width } else { 0.0 }
    };
    (0..=degree)
        .map(|offset| {
            let index = span + offset - degree;
            let own = offset
                .checked_sub(1)
                .and_then(|below| lower.get(below))
                .copied()
                .unwrap_or(0.0);
            let next = lower.get(offset).copied().unwrap_or(0.0);
            degree as f64
                * (share(own, index, index + degree) - share(next, index + 1, index + degree + 1))
        })
        .collect()
}

fn local_basis(degree: usize, knots: &[f64], span: usize, parameter: f64) -> Vec<f64> {
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
    values
}

fn segments_for(angle: f64, max_segment_angle: f64) -> usize {
    let step = if max_segment_angle.is_finite() && max_segment_angle > 0.0 {
        max_segment_angle.clamp(MIN_SEGMENT_ANGLE, FRAC_PI_2)
    } else {
        DEFAULT_SEGMENT_ANGLE
    };
    let angle = if angle.is_finite() {
        angle.abs().min(TAU)
    } else {
        TAU
    };
    let count = (angle / step).ceil();
    if count >= 1.0 {
        (count as usize).min(MAX_SEGMENTS)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-12;

    #[test]
    fn an_arc_runs_counter_clockwise_from_start_to_end() {
        let arc = ArcGeometry::from_points(Point2::ZERO, Point2::X, Point2::Y);
        assert!((arc.sweep - FRAC_PI_2).abs() < EPSILON);
        let reversed = ArcGeometry::from_points(Point2::ZERO, Point2::Y, Point2::X);
        assert!((reversed.sweep - 3.0 * FRAC_PI_2).abs() < EPSILON);

        let points = arc.polyline(FRAC_PI_2 / 4.0);
        assert_eq!(points.len(), 5);
        assert!(points[0].distance(Point2::X) < EPSILON);
        assert!(points[4].distance(Point2::Y) < EPSILON);
        assert!(
            points
                .iter()
                .all(|point| (point.length() - 1.0).abs() < EPSILON)
        );
    }

    fn sagitta(radius: f64, segments: usize) -> f64 {
        radius * (1.0 - (TAU / segments as f64 / 2.0).cos())
    }

    fn distance_to_polyline(point: Point2, polyline: &[Point2]) -> f64 {
        polyline
            .windows(2)
            .map(|pair| {
                let (start, end) = (pair[0], pair[1]);
                let along = end - start;
                let fraction = if along.length_squared() > 0.0 {
                    ((point - start).dot(along) / along.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                point.distance(start + along * fraction)
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn circles_are_faceted_to_their_chord_tolerance_with_as_few_segments_as_that_needs() {
        for radius in [0.5, 3.0, 40.0, 900.0] {
            for chord in [0.001, 0.01, 0.1] {
                let segments = Faceting::within(chord).arc_segments(radius, TAU);
                let within_bounds = (12..=1024).contains(&segments);

                assert!(within_bounds, "{radius} at {chord}: {segments}");
                if segments > 12 && segments < 1024 {
                    assert!(sagitta(radius, segments) <= chord * (1.0 + 1e-9));
                    assert!(sagitta(radius, segments - 1) > chord);
                }
            }
        }

        let circle = ArcGeometry::full_circle(Point2::new(5.0, -2.0), 40.0);
        let points = circle.faceted(Faceting::within(0.01));
        let middles = points
            .windows(2)
            .map(|pair| pair[0].lerp(pair[1], 0.5).distance(circle.center));

        assert_eq!(
            points.len(),
            Faceting::within(0.01).arc_segments(40.0, TAU) + 1
        );
        assert!(points.first().unwrap().distance(*points.last().unwrap()) < 1e-9);
        assert!(middles.clone().all(|middle| 40.0 - middle <= 0.01 + 1e-9));
    }

    #[test]
    fn small_curves_keep_a_minimum_and_huge_ones_a_maximum_per_turn() {
        let fine = Faceting::within(1e-6);
        let coarse = Faceting::within(10.0);

        assert_eq!(coarse.arc_segments(1.0, TAU), 12);
        assert_eq!(coarse.arc_segments(1.0, TAU / 4.0), 3);
        assert_eq!(fine.arc_segments(1e6, TAU), 1024);
        assert_eq!(fine.arc_segments(1e6, TAU / 2.0), 512);
        assert_eq!(coarse.arc_segments(1.0, 1e-9), 1);
        assert!(
            Faceting::within(0.01).arc_segments(10.0, TAU)
                < Faceting::within(0.01).arc_segments(1000.0, TAU)
        );
    }

    #[test]
    fn faceting_bad_values_stays_finite_and_never_empty() {
        for chord in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let faceting = Faceting::within(chord);
            assert!(faceting.chord() > 0.0);
            for radius in [0.0, -3.0, f64::NAN, f64::INFINITY, 2.0] {
                for sweep in [0.0, f64::NAN, -TAU, 100.0, 1.0] {
                    let segments = faceting.arc_segments(radius, sweep);
                    assert!((1..=1024).contains(&segments), "{chord} {radius} {sweep}");
                }
            }
            let collapsed = ArcGeometry::from_points(Point2::ZERO, Point2::ZERO, Point2::ZERO);
            assert!(
                collapsed
                    .faceted(faceting)
                    .iter()
                    .all(|point| point.is_finite())
            );
        }
    }

    #[test]
    fn splines_are_faceted_within_their_chord_tolerance() {
        let control = vec![
            Point2::ZERO,
            Point2::new(10.0, 30.0),
            Point2::new(20.0, -30.0),
            Point2::new(35.0, 10.0),
            Point2::new(40.0, 0.0),
            Point2::new(60.0, 25.0),
        ];
        let spline = BSpline::clamped(control).unwrap();
        let dense: Vec<Point2> = (0..=4000)
            .map(|index| spline.point_at(f64::from(index) / 4000.0))
            .collect();

        let mut previous = usize::MAX;
        for chord in [0.001, 0.01, 0.1, 1.0] {
            let points = spline.faceted(Faceting::within(chord));
            let farthest = dense
                .iter()
                .map(|point| distance_to_polyline(*point, &points))
                .fold(0.0, f64::max);

            assert!(farthest <= chord, "{chord}: {farthest}");
            assert!(points.len() <= previous);
            assert!(points.len() > 3);
            previous = points.len();
        }
        let line = BSpline::clamped(vec![Point2::ZERO, Point2::new(5.0, 5.0)]).unwrap();
        assert_eq!(line.faceted(Faceting::within(1e-9)).len(), 2);
    }

    #[test]
    fn degenerate_arcs_and_bad_segment_angles_stay_finite() {
        let collapsed = ArcGeometry::from_points(Point2::ZERO, Point2::ZERO, Point2::ZERO);
        assert_eq!(collapsed.radius, 0.0);
        assert_eq!(collapsed.sweep, TAU);
        for angle in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let points = collapsed.polyline(angle);
            assert!(points.len() >= 2);
            assert!(points.iter().all(|point| point.is_finite()));
        }
    }

    #[test]
    fn spline_derivatives_match_finite_differences() {
        let step = 1e-5;
        for count in 2..=6 {
            let control: Vec<Point2> = (0..count)
                .map(|index| {
                    let x = index as f64;
                    Point2::new(3.0 * x, (x * 1.7).sin() * 4.0)
                })
                .collect();
            let spline = BSpline::clamped(control).unwrap();
            for parameter in [0.05, 0.3, 0.5, 0.71, 0.95] {
                let [tangent, bend] = spline.derivatives(parameter);
                let ahead = spline.point_at(parameter + step);
                let behind = spline.point_at(parameter - step);
                let here = spline.point_at(parameter);
                let slope = (ahead - behind) / (2.0 * step);
                let curvature = (ahead - here * 2.0 + behind) / (step * step);
                assert!(
                    tangent.distance(slope) < 1e-5 * (1.0 + slope.length()),
                    "{count} points at {parameter}: {tangent} against {slope}"
                );
                assert!(
                    bend.distance(curvature) < 1e-2 * (1.0 + curvature.length()),
                    "{count} points at {parameter}: {bend} against {curvature}"
                );
            }
        }
    }

    #[test]
    fn a_clamped_spline_starts_and_ends_at_its_end_control_points() {
        let control = vec![
            Point2::ZERO,
            Point2::new(1.0, 2.0),
            Point2::new(3.0, 2.0),
            Point2::new(4.0, 0.0),
            Point2::new(6.0, 1.0),
        ];
        let spline = BSpline::clamped(control.clone()).unwrap();
        assert_eq!(spline.degree(), 3);
        assert!(spline.point_at(0.0).distance(control[0]) < EPSILON);
        assert!(spline.point_at(1.0).distance(control[4]) < EPSILON);
        assert!(spline.point_at(2.0).distance(control[4]) < EPSILON);
        assert!(spline.point_at(f64::NAN).distance(control[0]) < EPSILON);

        let line = BSpline::clamped(vec![Point2::ZERO, Point2::new(2.0, 0.0)]).unwrap();
        assert_eq!(line.degree(), 1);
        assert!(line.point_at(0.25).distance(Point2::new(0.5, 0.0)) < EPSILON);

        let quadratic = BSpline::clamped(vec![Point2::ZERO, Point2::Y, Point2::X]).unwrap();
        assert_eq!(quadratic.degree(), 2);
        assert!(quadratic.point_at(0.5).distance(Point2::new(0.25, 0.5)) < EPSILON);

        assert_eq!(BSpline::clamped(vec![Point2::ZERO]), None);
        let stacked = BSpline::clamped(vec![Point2::ZERO; 4]).unwrap();
        assert!(
            stacked
                .polyline(0.1)
                .iter()
                .all(|point| *point == Point2::ZERO)
        );
    }
}
