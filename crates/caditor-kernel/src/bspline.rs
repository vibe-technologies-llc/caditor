use std::sync::Arc;

use crate::{
    coordinates::Coordinates, error::GeometryError, interval::Interval,
    tolerance::LINEAR_RESOLUTION,
};

pub const MAX_SPLINE_DEGREE: usize = 25;
const MIN_PRUNED_SPANS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct BSpline<P> {
    degree: usize,
    knots: Arc<[f64]>,
    control_points: Arc<[P]>,
    weights: Option<Arc<[f64]>>,
    domain: Interval,
}

impl<P: Coordinates> BSpline<P> {
    pub(crate) fn heap_size(&self) -> usize {
        size_of_val(&*self.knots)
            + size_of_val(&*self.control_points)
            + self
                .weights
                .as_ref()
                .map_or(0, |weights| size_of_val(&**weights))
    }

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
            knots: knots.into(),
            control_points: control_points.into(),
            weights: weights
                .filter(|weights| weights.iter().any(|weight| *weight != 1.0))
                .map(Arc::from),
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
        if self.degree < CUBIC_WIDTH {
            self.derivatives_within::<CUBIC_WIDTH>(parameter)
        } else if self.degree < NARROW_WIDTH {
            self.derivatives_within::<NARROW_WIDTH>(parameter)
        } else {
            self.derivatives_within::<WIDE_WIDTH>(parameter)
        }
    }

    fn derivatives_within<const WIDTH: usize>(&self, parameter: f64) -> [P; 3] {
        let parameter = self.domain.clamp(parameter);
        let span = self.span(parameter);
        let table = self.basis_table::<WIDTH>(span, parameter);
        let degree = self.degree;
        let values = table.get(degree).copied().unwrap_or([0.0; WIDTH]);
        let first = table.get(degree - 1).map_or([0.0; WIDTH], |lower| {
            self.derivative_row(lower, degree, span)
        });
        let second = match degree.checked_sub(2).and_then(|index| table.get(index)) {
            Some(lower) => {
                let middle = self.derivative_row(lower, degree - 1, span);
                self.derivative_row(&middle, degree, span)
            }
            None => [0.0; WIDTH],
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
            knots: Arc::clone(&self.knots),
            control_points: control_points.into(),
            weights: self.weights.clone(),
            domain: self.domain,
        })
    }

    pub fn restricted(&self, range: Interval) -> Option<Self> {
        let low = self.domain.clamp(range.start());
        let high = self.domain.clamp(range.end());
        if low >= high {
            return None;
        }
        if low == self.domain.start() && high == self.domain.end() {
            return Some(self.clone());
        }
        let degree = self.degree;
        let mut knots = self.knots.to_vec();
        let mut points: Vec<Homogeneous<P>> = self
            .control_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let weight = self.weight(index);
                Homogeneous {
                    point: *point * weight,
                    weight,
                }
            })
            .collect();
        for parameter in [low, high] {
            let present = knots.iter().filter(|knot| **knot == parameter).count();
            for _ in present..degree {
                insert_knot(&mut knots, &mut points, degree, parameter)?;
            }
        }
        let starts = knots.partition_point(|knot| *knot < low);
        let multiplicity = knots.iter().filter(|knot| **knot == low).count();
        let first = (starts + multiplicity).checked_sub(degree + 1)?;
        let last = knots.partition_point(|knot| *knot < high).checked_sub(1)?;
        let kept = points.get(first..=last)?;
        let interior = knots.iter().filter(|knot| **knot > low && **knot < high);
        let knots: Vec<f64> = std::iter::repeat_n(low, degree + 1)
            .chain(interior.copied())
            .chain(std::iter::repeat_n(high, degree + 1))
            .collect();
        let control_points = kept
            .iter()
            .map(|homogeneous| homogeneous.point * (1.0 / homogeneous.weight))
            .collect();
        let restricted = match self.weights {
            Some(_) => Self::rational(
                degree,
                knots,
                control_points,
                kept.iter().map(|homogeneous| homogeneous.weight).collect(),
            ),
            None => Self::new(degree, knots, control_points),
        };
        restricted.ok()
    }

    pub fn interpolating(degree: usize, points: &[P]) -> Result<Self, GeometryError> {
        let mut parameters = Vec::with_capacity(points.len());
        let mut travelled = 0.0;
        for (index, point) in points.iter().enumerate() {
            if let Some(previous) = index.checked_sub(1).and_then(|before| points.get(before)) {
                let step = point.distance_to(*previous);
                if step.is_nan() || step <= 0.0 {
                    return Err(GeometryError::ZeroDirection);
                }
                travelled += step;
            }
            parameters.push(travelled);
        }
        Self::interpolating_at(degree, &parameters, points)
    }

    pub fn interpolating_at(
        degree: usize,
        parameters: &[f64],
        points: &[P],
    ) -> Result<Self, GeometryError> {
        let count = points.len();
        if degree == 0 || degree > MAX_SPLINE_DEGREE {
            return Err(GeometryError::SplineDegree(degree));
        }
        if count <= degree {
            return Err(GeometryError::TooFewControlPoints {
                degree,
                points: count,
            });
        }
        if parameters.len() != count {
            return Err(GeometryError::KnotCount {
                degree,
                points: count,
                knots: parameters.len(),
            });
        }
        let increasing = parameters
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if a.is_finite() && b.is_finite() && a < b));
        if !increasing {
            return Err(GeometryError::Knots);
        }
        let (Some(first), Some(last)) = (parameters.first(), parameters.last()) else {
            return Err(GeometryError::Knots);
        };
        let mut knots = vec![*first; degree + 1];
        for start in 1..count - degree {
            let window = parameters.get(start..start + degree).unwrap_or_default();
            knots.push(window.iter().sum::<f64>() / degree as f64);
        }
        knots.extend(std::iter::repeat_n(*last, degree + 1));
        let shape = Self::new(degree, knots.clone(), vec![P::ORIGIN; count])?;
        let mut band = Vec::with_capacity(count);
        let mut firsts = Vec::with_capacity(count);
        for parameter in parameters {
            let span = shape.span(*parameter);
            band.push(shape.basis_values(span, *parameter));
            firsts.push(span - degree);
        }
        let solved =
            solve_collocation(&band, &firsts, points, degree).ok_or(GeometryError::NonFinite)?;
        Self::new(degree, knots, solved)
    }

    fn weight(&self, index: usize) -> f64 {
        self.weights
            .as_ref()
            .and_then(|weights| weights.get(index))
            .copied()
            .unwrap_or(1.0)
    }

    pub(crate) fn with_knots(&self, parameters: &[f64]) -> Option<Self> {
        let degree = self.degree;
        let mut knots = self.knots.to_vec();
        let mut points = self.homogeneous_points();
        for parameter in parameters {
            let inside = self.domain.start() < *parameter && *parameter < self.domain.end();
            let present = knots.iter().filter(|knot| **knot == *parameter).count();
            if inside && present < degree {
                insert_knot(&mut knots, &mut points, degree, *parameter)?;
            }
        }
        self.rebuilt_from(knots, &points)
    }

    pub(crate) fn with_points(&self, control_points: Vec<P>) -> Result<Self, GeometryError> {
        if control_points.len() != self.control_points.len() {
            return Err(GeometryError::KnotCount {
                degree: self.degree,
                points: control_points.len(),
                knots: self.knots.len(),
            });
        }
        if !control_points.iter().all(|point| point.all_finite()) {
            return Err(GeometryError::NonFinite);
        }
        Ok(Self {
            degree: self.degree,
            knots: Arc::clone(&self.knots),
            control_points: control_points.into(),
            weights: self.weights.clone(),
            domain: self.domain,
        })
    }

    pub(crate) fn rational_basis(&self, parameter: f64) -> (usize, Vec<f64>) {
        let parameter = self.domain.clamp(parameter);
        let span = self.span(parameter);
        let first = span - self.degree;
        let mut values = self.basis_values(span, parameter);
        let mut total = 0.0;
        for (offset, value) in values.iter_mut().enumerate() {
            *value *= self.weight(first + offset);
            total += *value;
        }
        if total > 0.0 {
            for value in &mut values {
                *value /= total;
            }
        }
        (first, values)
    }

    fn basis_values(&self, span: usize, parameter: f64) -> Vec<f64> {
        if self.degree < CUBIC_WIDTH {
            self.basis_row::<CUBIC_WIDTH>(span, parameter)
        } else if self.degree < NARROW_WIDTH {
            self.basis_row::<NARROW_WIDTH>(span, parameter)
        } else {
            self.basis_row::<WIDE_WIDTH>(span, parameter)
        }
    }

    fn basis_row<const WIDTH: usize>(&self, span: usize, parameter: f64) -> Vec<f64> {
        self.basis_table::<WIDTH>(span, parameter)
            .get(self.degree)
            .and_then(|row| row.get(..=self.degree))
            .map_or_else(|| vec![0.0; self.degree + 1], <[f64]>::to_vec)
    }

    fn homogeneous_points(&self) -> Vec<Homogeneous<P>> {
        self.control_points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let weight = self.weight(index);
                Homogeneous {
                    point: *point * weight,
                    weight,
                }
            })
            .collect()
    }

    fn rebuilt_from(&self, knots: Vec<f64>, points: &[Homogeneous<P>]) -> Option<Self> {
        let control_points = points
            .iter()
            .map(|homogeneous| homogeneous.point * (1.0 / homogeneous.weight))
            .collect();
        let built = match self.weights {
            Some(_) => Self::rational(
                self.degree,
                knots,
                control_points,
                points
                    .iter()
                    .map(|homogeneous| homogeneous.weight)
                    .collect(),
            ),
            None => Self::new(self.degree, knots, control_points),
        };
        built.ok()
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

    pub(crate) fn spans_within(&self, range: Interval) -> Vec<Interval> {
        let mut ends: Vec<f64> = vec![range.start()];
        ends.extend(
            self.breakpoints()
                .into_iter()
                .filter(|knot| *knot > range.start() && *knot < range.end()),
        );
        ends.push(range.end());
        ends.windows(2)
            .filter_map(|pair| match pair {
                [start, end] => Interval::new(*start, *end),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn nearby_runs(&self, point: P, range: Interval) -> Vec<Interval> {
        let spans = self.spans_within(range);
        if spans.len() < MIN_PRUNED_SPANS {
            return vec![range];
        }
        let reaches: Vec<(Interval, f64, f64)> = spans
            .into_iter()
            .map(|span| {
                let hull = self.control_points_over(span);
                let low = hull
                    .iter()
                    .copied()
                    .reduce(P::component_min)
                    .unwrap_or(point);
                let high = hull
                    .iter()
                    .copied()
                    .reduce(P::component_max)
                    .unwrap_or(point);
                let outside = (low - point)
                    .component_max(point - high)
                    .component_max(P::ORIGIN);
                let corner = (point - low)
                    .component_max(low - point)
                    .component_max((high - point).component_max(point - high));
                (span, outside.dot(outside), corner.dot(corner))
            })
            .collect();
        let farthest = reaches
            .iter()
            .map(|(_, _, farthest)| *farthest)
            .fold(f64::INFINITY, f64::min);
        let reach = farthest.max(0.0).sqrt() + LINEAR_RESOLUTION;
        let reach = reach * reach;
        let mut runs: Vec<Interval> = Vec::new();
        for (span, _, _) in reaches
            .into_iter()
            .filter(|(_, nearest, _)| *nearest <= reach)
        {
            match runs.last_mut() {
                Some(last) if last.end() == span.start() => {
                    *last = Interval::new(last.start(), span.end()).unwrap_or(*last);
                }
                _ => runs.push(span),
            }
        }
        if runs.is_empty() {
            return vec![range];
        }
        runs
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

    fn basis_table<const WIDTH: usize>(
        &self,
        span: usize,
        parameter: f64,
    ) -> [[f64; WIDTH]; WIDTH] {
        let mut table = [[0.0; WIDTH]; WIDTH];
        if let Some(first) = table.first_mut().and_then(|row| row.first_mut()) {
            *first = 1.0;
        }
        for degree in 1..=self.degree.min(WIDTH - 1) {
            let lower = table.get(degree - 1).copied().unwrap_or([0.0; WIDTH]);
            let Some(row) = table.get_mut(degree) else {
                break;
            };
            for (offset, slot) in row.iter_mut().enumerate().take(degree + 1) {
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
                *slot = left * rising + right * falling;
            }
        }
        table
    }

    fn derivative_row<const WIDTH: usize>(
        &self,
        lower: &[f64; WIDTH],
        degree: usize,
        span: usize,
    ) -> [f64; WIDTH] {
        let mut row = [0.0; WIDTH];
        for (offset, slot) in row.iter_mut().enumerate().take(degree + 1) {
            let index = span + offset - degree;
            let left = offset
                .checked_sub(1)
                .and_then(|below| lower.get(below))
                .copied()
                .unwrap_or(0.0);
            let right = if offset < degree {
                lower.get(offset).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *slot = degree as f64
                * (ratio(left, self.knot(index + degree) - self.knot(index))
                    - ratio(right, self.knot(index + degree + 1) - self.knot(index + 1)));
        }
        row
    }

    fn combine(&self, span: usize, coefficients: &[f64]) -> (P, f64) {
        let first = span - self.degree;
        coefficients.iter().take(self.degree + 1).enumerate().fold(
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

#[derive(Debug, Clone, Copy)]
struct Homogeneous<P> {
    point: P,
    weight: f64,
}

fn insert_knot<P: Coordinates>(
    knots: &mut Vec<f64>,
    points: &mut Vec<Homogeneous<P>>,
    degree: usize,
    parameter: f64,
) -> Option<()> {
    let count = points.len();
    let span = knots
        .partition_point(|knot| *knot <= parameter)
        .checked_sub(1)?
        .clamp(degree, count.checked_sub(1)?);
    let mut inserted = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let point = if index + degree <= span {
            *points.get(index)?
        } else if index > span {
            *points.get(index - 1)?
        } else {
            let (start, end) = (*knots.get(index)?, *knots.get(index + degree)?);
            let along = if end > start {
                (parameter - start) / (end - start)
            } else {
                0.0
            };
            let (before, after) = (points.get(index - 1)?, points.get(index)?);
            Homogeneous {
                point: before.point + (after.point - before.point) * along,
                weight: before.weight + (after.weight - before.weight) * along,
            }
        };
        inserted.push(point);
    }
    knots.insert(span + 1, parameter);
    *points = inserted;
    Some(())
}

fn solve_collocation<P: Coordinates>(
    band: &[Vec<f64>],
    firsts: &[usize],
    points: &[P],
    degree: usize,
) -> Option<Vec<P>> {
    let count = points.len();
    let width = 2 * degree + 1;
    let mut matrix = vec![0.0; count * width];
    let at = |row: usize, column: usize| (row * width + column + degree).checked_sub(row);
    for (row, (values, first)) in band.iter().zip(firsts).enumerate() {
        for (offset, value) in values.iter().take(degree + 1).enumerate() {
            let slot = at(row, first + offset)?;
            *matrix.get_mut(slot)? = *value;
        }
    }
    let mut right = points.to_vec();
    for pivot in 0..count {
        let diagonal = *matrix.get(at(pivot, pivot)?)?;
        if diagonal.abs() <= f64::MIN_POSITIVE {
            return None;
        }
        for row in pivot + 1..(pivot + degree + 1).min(count) {
            let factor = *matrix.get(at(row, pivot)?)? / diagonal;
            if factor == 0.0 {
                continue;
            }
            for column in pivot..(pivot + degree + 1).min(count) {
                let source = *matrix.get(at(pivot, column)?)?;
                *matrix.get_mut(at(row, column)?)? -= factor * source;
            }
            let source = *right.get(pivot)?;
            let target = right.get_mut(row)?;
            *target = *target - source * factor;
        }
    }
    for row in (0..count).rev() {
        let mut sum = *right.get(row)?;
        for column in row + 1..(row + degree + 1).min(count) {
            sum = sum - *right.get(column)? * *matrix.get(at(row, column)?)?;
        }
        *right.get_mut(row)? = sum * (1.0 / *matrix.get(at(row, row)?)?);
    }
    right
        .iter()
        .all(|point| point.all_finite())
        .then_some(right)
}

pub(crate) const CUBIC_WIDTH: usize = 4;
pub(crate) const NARROW_WIDTH: usize = 10;
pub(crate) const WIDE_WIDTH: usize = MAX_SPLINE_DEGREE + 1;

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

    #[test]
    fn clones_moves_and_reshapes_share_what_they_keep() {
        let quarter = quarter_circle();

        let copy = quarter.clone();
        let moved = quarter.map_points(|point| point + Point2::ONE).unwrap();
        let doubled: Vec<Point2> = quarter
            .control_points()
            .iter()
            .map(|point| *point * 2.0)
            .collect();
        let reshaped = quarter.with_points(doubled).unwrap();

        assert_eq!(
            copy.control_points().as_ptr(),
            quarter.control_points().as_ptr()
        );
        assert_eq!(moved.knots().as_ptr(), quarter.knots().as_ptr());
        assert_eq!(reshaped.knots().as_ptr(), quarter.knots().as_ptr());
        assert_eq!(
            reshaped.weights().map(<[f64]>::as_ptr),
            quarter.weights().map(<[f64]>::as_ptr)
        );
        assert!((reshaped.point(0.5) - quarter.point(0.5) * 2.0).length() < 1e-12);
        assert_eq!(moved.point(0.5), quarter.point(0.5) + Point2::ONE);
        assert!(quarter.with_points(vec![Point2::X, Point2::Y]).is_err());
        assert_eq!(
            quarter.with_points(vec![Point2::X, Point2::NAN, Point2::Y]),
            Err(GeometryError::NonFinite)
        );
    }

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

    fn casteljau(points: &[Point3], parameter: f64) -> Point3 {
        let mut points = points.to_vec();
        while points.len() > 1 {
            points = points
                .windows(2)
                .map(|pair| pair[0].lerp(pair[1], parameter))
                .collect();
        }
        points[0]
    }

    fn wavy(count: usize, seed: f64) -> Vec<Point3> {
        (0..count)
            .map(|index| {
                let along = index as f64;
                Point3::new(
                    along,
                    (along * 0.7 + seed).sin() * 3.0,
                    (along * 0.3).cos() * seed,
                )
            })
            .collect()
    }

    #[test]
    fn splines_of_every_degree_up_to_the_most_evaluate_as_bezier_curves_and_patches() {
        for degree in [1, 2, 3, 4, 9, 10, 12, MAX_SPLINE_DEGREE] {
            let knots: Vec<f64> = std::iter::repeat_n(0.0, degree + 1)
                .chain(std::iter::repeat_n(1.0, degree + 1))
                .collect();
            let points = wavy(degree + 1, 1.5);
            let differences: Vec<Point3> = points
                .windows(2)
                .map(|pair| (pair[1] - pair[0]) * degree as f64)
                .collect();
            let curve = BSpline::new(degree, knots.clone(), points.clone()).unwrap();
            let rows = 5;
            let net: Vec<Point3> = (0..rows)
                .flat_map(|row| wavy(degree + 1, row as f64))
                .collect();
            let row_knots: Vec<f64> = std::iter::repeat_n(0.0, rows)
                .chain(std::iter::repeat_n(1.0, rows))
                .collect();
            let surface = crate::BSplineSurface::new(
                degree,
                rows - 1,
                knots.clone(),
                row_knots,
                degree + 1,
                net.clone(),
                None,
            )
            .unwrap();

            for index in 0..=10 {
                let t = index as f64 / 10.0;
                let [point, tangent, _] = curve.derivatives(t);
                let along_rows: Vec<Point3> = net
                    .chunks(degree + 1)
                    .map(|row| casteljau(row, t))
                    .collect();
                let expected = casteljau(&along_rows, 1.0 - t);
                let derivative = if degree == 1 {
                    differences[0]
                } else {
                    casteljau(&differences, t)
                };
                assert!(point.distance(casteljau(&points, t)) < 1e-9, "{degree} {t}");
                assert!(tangent.distance(derivative) < 1e-7, "{degree} {t}");
                assert!(
                    surface.point(t, 1.0 - t).distance(expected) < 1e-9,
                    "{degree} {t}"
                );
                assert!(
                    surface.evaluate(t, 1.0 - t).point.distance(expected) < 1e-9,
                    "{degree} {t}"
                );
            }
        }
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
    fn a_restricted_spline_traces_exactly_the_part_of_the_original_in_its_range() {
        let spline = BSpline::rational(
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
            vec![1.0, 2.0, 0.5, 1.0, 3.0, 1.0, 0.7],
        )
        .unwrap();
        let ranges = [(0.1, 0.9), (0.0, 0.5), (0.3, 1.0), (0.31, 0.32), (0.0, 1.0)];

        for (start, end) in ranges {
            let range = Interval::new(start, end).unwrap();
            let restricted = spline.restricted(range).unwrap();

            assert_eq!(restricted.domain(), range);
            assert!(restricted.is_rational());
            for index in 0..=20 {
                let parameter = range.at(index as f64 / 20.0);
                assert!(
                    restricted
                        .point(parameter)
                        .distance(spline.point(parameter))
                        < 1e-12
                );
            }
        }

        assert!(
            spline
                .restricted(Interval::new(1.0, 2.0).unwrap())
                .is_none()
        );
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

    #[test]
    fn an_interpolating_spline_passes_through_its_points_and_follows_their_curve() {
        let on_helix = |angle: f64| Point3::new(angle.cos(), angle.sin(), 0.2 * angle);
        let points: Vec<Point3> = (0..=24)
            .map(|index| on_helix(index as f64 * 0.25))
            .collect();

        let spline = BSpline::interpolating(3, &points).unwrap();
        let domain = spline.domain();
        let curve = crate::curve::Curve::BSpline(spline.clone());
        let off = |point: Point3| {
            curve
                .point(curve.closest_parameter(point, domain))
                .distance(point)
        };

        assert!(spline.point(domain.start()).distance(points[0]) < 1e-12);
        assert!(spline.point(domain.end()).distance(points[24]) < 1e-12);
        assert!(points.iter().all(|point| off(*point) < 1e-9));
        assert!(off(on_helix(3.125)) < 1e-4, "{}", off(on_helix(3.125)));
        assert!(matches!(
            BSpline::interpolating(3, &points[..3]),
            Err(GeometryError::TooFewControlPoints { .. })
        ));
        assert!(matches!(
            BSpline::interpolating(3, &[Point3::ZERO; 5]),
            Err(GeometryError::ZeroDirection)
        ));
    }

    #[test]
    fn a_spline_interpolating_at_given_parameters_meets_each_point_there() {
        let on_helix = |angle: f64| Point3::new(angle.cos(), angle.sin(), 0.2 * angle);
        let parameters: Vec<f64> = (0..=12).map(|index| 1.0 + index as f64 * 0.5).collect();
        let points: Vec<Point3> = parameters.iter().map(|angle| on_helix(*angle)).collect();

        let spline = BSpline::interpolating_at(3, &parameters, &points).unwrap();

        assert_eq!(spline.domain(), Interval::new(1.0, 7.0).unwrap());
        for (parameter, point) in parameters.iter().zip(&points) {
            assert!(spline.point(*parameter).distance(*point) < 1e-12);
        }
        assert!(spline.point(2.25).distance(on_helix(2.25)) < 1e-3);
        assert!(matches!(
            BSpline::interpolating_at(3, &[0.0, 1.0, 1.0, 2.0, 3.0], &points[..5]),
            Err(GeometryError::Knots)
        ));
        assert!(matches!(
            BSpline::interpolating_at(3, &parameters[..4], &points[..5]),
            Err(GeometryError::KnotCount { .. })
        ));
    }
}
