use crate::{coordinates::Coordinates, error::GeometryError, interval::Interval};

pub const MAX_SPLINE_DEGREE: usize = 9;

#[derive(Debug, Clone, PartialEq)]
pub struct BSpline<P> {
    degree: usize,
    knots: Vec<f64>,
    control_points: Vec<P>,
    weights: Option<Vec<f64>>,
    domain: Interval,
}

impl<P: Coordinates> BSpline<P> {
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<P>,
    ) -> Result<Self, GeometryError> {
        Self::build(degree, knots, control_points, None)
    }

    pub fn rational(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<P>,
        weights: Vec<f64>,
    ) -> Result<Self, GeometryError> {
        if weights.len() != control_points.len() {
            return Err(GeometryError::WeightCount {
                points: control_points.len(),
                weights: weights.len(),
            });
        }
        if let Some(bad) = weights
            .iter()
            .find(|weight| !weight.is_finite() || **weight <= 0.0)
        {
            return Err(GeometryError::Weight(*bad));
        }
        Self::build(degree, knots, control_points, Some(weights))
    }

    pub fn clamped_uniform(degree: usize, control_points: Vec<P>) -> Result<Self, GeometryError> {
        let spans = control_points.len().saturating_sub(degree).max(1);
        let knots = std::iter::repeat_n(0.0, degree + 1)
            .chain((1..spans).map(|index| index as f64 / spans as f64))
            .chain(std::iter::repeat_n(1.0, degree + 1))
            .collect();
        Self::new(degree, knots, control_points)
    }

    fn build(
        degree: usize,
        knots: Vec<f64>,
        control_points: Vec<P>,
        weights: Option<Vec<f64>>,
    ) -> Result<Self, GeometryError> {
        if degree == 0 || degree > MAX_SPLINE_DEGREE {
            return Err(GeometryError::SplineDegree(degree));
        }
        let points = control_points.len();
        if points <= degree {
            return Err(GeometryError::TooFewControlPoints { degree, points });
        }
        if knots.len() != points + degree + 1 {
            return Err(GeometryError::KnotCount {
                degree,
                points,
                knots: knots.len(),
            });
        }
        if !control_points.iter().all(|point| point.all_finite()) {
            return Err(GeometryError::NonFinite);
        }
        let domain = clamped_domain(&knots, degree).ok_or(GeometryError::Knots)?;
        Ok(Self {
            degree,
            knots,
            control_points,
            weights: weights.filter(|weights| weights.iter().any(|weight| *weight != 1.0)),
            domain,
        })
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn control_points(&self) -> &[P] {
        &self.control_points
    }

    pub fn weights(&self) -> Option<&[f64]> {
        self.weights.as_deref()
    }

    pub fn is_rational(&self) -> bool {
        self.weights.is_some()
    }

    pub fn domain(&self) -> Interval {
        self.domain
    }

    pub fn breakpoints(&self) -> Vec<f64> {
        let mut breaks: Vec<f64> = self
            .knots
            .iter()
            .copied()
            .filter(|knot| self.domain.contains(*knot))
            .collect();
        breaks.dedup();
        breaks
    }

    pub fn point(&self, parameter: f64) -> P {
        let [point, _, _] = self.derivatives(parameter);
        point
    }

    pub fn derivatives(&self, parameter: f64) -> [P; 3] {
        let parameter = self.domain.clamp(parameter);
        let span = self.span(parameter);
        let table = self.basis_table(span, parameter);
        let degree = self.degree;
        let values = table.get(degree).cloned().unwrap_or_default();
        let first = table
            .get(degree - 1)
            .map(|lower| self.derivative_row(lower, degree, span))
            .unwrap_or_default();
        let second = match degree.checked_sub(2).and_then(|index| table.get(index)) {
            Some(lower) => {
                let middle = self.derivative_row(lower, degree - 1, span);
                self.derivative_row(&middle, degree, span)
            }
            None => Vec::new(),
        };
        let (a0, w0) = self.combine(span, &values);
        let (a1, w1) = self.combine(span, &first);
        let (a2, w2) = self.combine(span, &second);
        if w0 <= 0.0 {
            return [P::ORIGIN; 3];
        }
        let point = a0 * (1.0 / w0);
        let tangent = (a1 - point * w1) * (1.0 / w0);
        let curvature = (a2 - tangent * (2.0 * w1) - point * w2) * (1.0 / w0);
        [point, tangent, curvature]
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        let pivot = self.domain.start() + self.domain.end();
        Self {
            degree: self.degree,
            knots: self.knots.iter().rev().map(|knot| pivot - knot).collect(),
            control_points: self.control_points.iter().rev().copied().collect(),
            weights: self
                .weights
                .as_ref()
                .map(|weights| weights.iter().rev().copied().collect()),
            domain: self.domain.mirrored(pivot),
        }
    }

    pub fn map_points<Q: Coordinates>(
        &self,
        map: impl Fn(P) -> Q,
    ) -> Result<BSpline<Q>, GeometryError> {
        let control_points: Vec<Q> = self
            .control_points
            .iter()
            .map(|point| map(*point))
            .collect();
        if !control_points.iter().all(|point| point.all_finite()) {
            return Err(GeometryError::NonFinite);
        }
        Ok(BSpline {
            degree: self.degree,
            knots: self.knots.clone(),
            control_points,
            weights: self.weights.clone(),
            domain: self.domain,
        })
    }

    pub(crate) fn control_points_over(&self, range: Interval) -> &[P] {
        let first = self.span(self.domain.clamp(range.start())) - self.degree;
        let last = self.span(self.domain.clamp(range.end()));
        self.control_points
            .get(first..=last)
            .unwrap_or(&self.control_points)
    }

    pub(crate) fn single_span(&self, range: Interval) -> bool {
        let start = self.domain.clamp(range.start());
        let end = self.domain.clamp(range.end());
        !self.knots.iter().any(|knot| *knot > start && *knot < end)
    }

    pub(crate) fn seeds(&self, range: Interval) -> Vec<f64> {
        let per_span = 2 * self.degree;
        let mut seeds = vec![range.start()];
        let mut breaks: Vec<f64> = self
            .breakpoints()
            .into_iter()
            .filter(|knot| *knot > range.start() && *knot < range.end())
            .collect();
        breaks.push(range.end());
        for end in breaks {
            let start = seeds.last().copied().unwrap_or(range.start());
            if let Some(piece) = Interval::new(start, end) {
                seeds.extend(piece.split(per_span).skip(1));
            }
        }
        seeds.dedup();
        seeds
    }

    fn knot(&self, index: usize) -> f64 {
        self.knots.get(index).copied().unwrap_or(0.0)
    }

    fn span(&self, parameter: f64) -> usize {
        let last = self.control_points.len() - 1;
        self.knots
            .partition_point(|knot| *knot <= parameter)
            .saturating_sub(1)
            .clamp(self.degree, last)
    }

    fn basis_table(&self, span: usize, parameter: f64) -> Vec<Vec<f64>> {
        let mut table: Vec<Vec<f64>> = vec![vec![1.0]];
        for degree in 1..=self.degree {
            let lower = table.last().cloned().unwrap_or_default();
            let row = (0..=degree)
                .map(|offset| {
                    let index = span + offset - degree;
                    let left = offset
                        .checked_sub(1)
                        .and_then(|below| lower.get(below))
                        .copied()
                        .unwrap_or(0.0);
                    let right = lower.get(offset).copied().unwrap_or(0.0);
                    let rising = ratio(
                        parameter - self.knot(index),
                        self.knot(index + degree) - self.knot(index),
                    );
                    let falling = ratio(
                        self.knot(index + degree + 1) - parameter,
                        self.knot(index + degree + 1) - self.knot(index + 1),
                    );
                    left * rising + right * falling
                })
                .collect();
            table.push(row);
        }
        table
    }

    fn derivative_row(&self, lower: &[f64], degree: usize, span: usize) -> Vec<f64> {
        (0..=degree)
            .map(|offset| {
                let index = span + offset - degree;
                let left = offset
                    .checked_sub(1)
                    .and_then(|below| lower.get(below))
                    .copied()
                    .unwrap_or(0.0);
                let right = lower.get(offset).copied().unwrap_or(0.0);
                degree as f64
                    * (ratio(left, self.knot(index + degree) - self.knot(index))
                        - ratio(right, self.knot(index + degree + 1) - self.knot(index + 1)))
            })
            .collect()
    }

    fn combine(&self, span: usize, coefficients: &[f64]) -> (P, f64) {
        let first = span - self.degree;
        coefficients.iter().enumerate().fold(
            (P::ORIGIN, 0.0),
            |(sum, total), (offset, coefficient)| {
                let index = first + offset;
                let point = self.control_points.get(index).copied().unwrap_or(P::ORIGIN);
                let weight = self
                    .weights
                    .as_ref()
                    .and_then(|weights| weights.get(index))
                    .copied()
                    .unwrap_or(1.0);
                (
                    sum + point * (coefficient * weight),
                    total + coefficient * weight,
                )
            },
        )
    }
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

pub(crate) fn clamped_domain(knots: &[f64], degree: usize) -> Option<Interval> {
    if !knots.iter().all(|knot| knot.is_finite()) {
        return None;
    }
    if knots.windows(2).any(|pair| matches!(pair, [a, b] if b < a)) {
        return None;
    }
    let start = *knots.first()?;
    let end = *knots.last()?;
    let clamped_start = knots.get(..=degree)?.iter().all(|knot| *knot == start);
    let clamped_end = knots
        .get(knots.len() - degree - 1..)?
        .iter()
        .all(|knot| *knot == end);
    let interior = knots.get(degree + 1..knots.len() - degree - 1)?;
    let limited = interior.chunk_by(|a, b| a == b).all(|run| {
        run.len() <= degree && run.first().is_some_and(|knot| *knot > start && *knot < end)
    });
    (clamped_start && clamped_end && limited && start < end).then(|| Interval::new(start, end))?
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Point2, Point3};

    use super::*;

    fn quarter_circle() -> BSpline<Point2> {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        BSpline::rational(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![Point2::X, Point2::ONE, Point2::Y],
            vec![1.0, half, 1.0],
        )
        .unwrap()
    }

    #[test]
    fn a_rational_quadratic_traces_a_circle_exactly() {
        let arc = quarter_circle();
        for index in 0..=20 {
            let parameter = index as f64 / 20.0;
            let [point, tangent, _] = arc.derivatives(parameter);
            assert!((point.length() - 1.0).abs() < 1e-14);
            assert!(point.dot(tangent).abs() < 1e-12);
        }
    }

    #[test]
    fn derivatives_match_finite_differences() {
        let spline = BSpline::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.3, 0.7, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point3::ZERO,
                Point3::new(1.0, 2.0, 0.0),
                Point3::new(2.0, -1.0, 1.0),
                Point3::new(4.0, 0.0, 3.0),
                Point3::new(5.0, 2.0, -1.0),
                Point3::new(7.0, 1.0, 0.0),
                Point3::new(8.0, 0.0, 2.0),
            ],
        )
        .unwrap();
        let splines = [spline.clone(), {
            let weights = vec![1.0, 2.0, 0.5, 1.0, 3.0, 1.0, 0.7];
            BSpline::rational(
                3,
                spline.knots().to_vec(),
                spline.control_points().to_vec(),
                weights,
            )
            .unwrap()
        }];
        let step = 1e-6;
        for curve in &splines {
            for parameter in [0.05, 0.2, 0.45, 0.5, 0.65, 0.9] {
                let [_, first, second] = curve.derivatives(parameter);
                let [ahead, ahead_first, _] = curve.derivatives(parameter + step);
                let [behind, behind_first, _] = curve.derivatives(parameter - step);
                let numeric_first = (ahead - behind) / (2.0 * step);
                let numeric_second = (ahead_first - behind_first) / (2.0 * step);
                assert!((numeric_first - first).length() < 1e-5 * (1.0 + first.length()));
                assert!((numeric_second - second).length() < 1e-4 * (1.0 + second.length()));
            }
        }
    }

    #[test]
    fn reversal_traces_the_same_points_backwards() {
        let arc = quarter_circle();
        let reversed = arc.reversed();
        for index in 0..=10 {
            let parameter = index as f64 / 10.0;
            let pivot = arc.domain().start() + arc.domain().end();
            assert!(
                reversed
                    .point(pivot - parameter)
                    .distance(arc.point(parameter))
                    < 1e-14
            );
        }
    }

    #[test]
    fn construction_rejects_malformed_input() {
        let points = vec![Point2::ZERO, Point2::X, Point2::ONE];
        assert!(matches!(
            BSpline::new(0, vec![0.0, 1.0], vec![Point2::ZERO]),
            Err(GeometryError::SplineDegree(0))
        ));
        assert!(matches!(
            BSpline::new(3, vec![0.0; 7], points.clone()),
            Err(GeometryError::TooFewControlPoints { .. })
        ));
        assert!(matches!(
            BSpline::new(2, vec![0.0, 0.0, 1.0, 1.0], points.clone()),
            Err(GeometryError::KnotCount { .. })
        ));
        assert_eq!(
            BSpline::new(2, vec![0.0, 0.0, 0.1, 1.0, 1.0, 1.0], points.clone()),
            Err(GeometryError::Knots)
        );
        assert_eq!(
            BSpline::new(2, vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0], points.clone()),
            Err(GeometryError::Knots)
        );
        assert_eq!(
            BSpline::new(1, vec![0.0, 0.0, f64::NAN, 1.0, 1.0], points.clone()),
            Err(GeometryError::Knots)
        );
        assert_eq!(
            BSpline::new(
                2,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![Point2::ZERO, Point2::NAN, Point2::X]
            ),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(
            BSpline::rational(
                2,
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                points.clone(),
                vec![1.0, 0.0, 1.0]
            ),
            Err(GeometryError::Weight(0.0))
        );
        assert!(
            !BSpline::rational(2, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0], points, vec![1.0; 3])
                .unwrap()
                .is_rational()
        );
    }

    #[test]
    fn clamped_uniform_splines_interpolate_their_end_points() {
        let points = vec![
            Point2::ZERO,
            Point2::new(1.0, 2.0),
            Point2::new(3.0, 2.0),
            Point2::new(4.0, 0.0),
            Point2::new(6.0, 1.0),
        ];
        let spline = BSpline::clamped_uniform(3, points.clone()).unwrap();
        assert_eq!(spline.breakpoints(), vec![0.0, 0.5, 1.0]);
        assert!(spline.point(0.0).distance(points[0]) < 1e-15);
        assert!(spline.point(1.0).distance(points[4]) < 1e-15);
        assert!(spline.point(7.0).distance(points[4]) < 1e-15);
        assert_eq!(
            spline
                .control_points_over(Interval::new(0.0, 0.25).unwrap())
                .len(),
            4
        );
        assert_eq!(spline.control_points_over(spline.domain()).len(), 5);
        let seeds = spline.seeds(spline.domain());
        assert_eq!(seeds.first(), Some(&0.0));
        assert_eq!(seeds.last(), Some(&1.0));
        assert!(seeds.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
