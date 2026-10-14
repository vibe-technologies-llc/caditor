use caditor_geometry::{Point2, Point3, Vector3};

use crate::{bspline::BSpline, curve::BSplineCurve, error::GeometryError, surface::BSplineSurface};

pub(super) type Homogeneous = [f64; 4];

const SAME_BREAK: f64 = 1e-9;
const HERMITE_DEGREE: usize = 3;

fn mix(from: Homogeneous, to: Homogeneous, along: f64) -> Homogeneous {
    let mut mixed = from;
    for (slot, target) in mixed.iter_mut().zip(to) {
        *slot += (target - *slot) * along;
    }
    mixed
}

fn scaled(point: Homogeneous, factor: f64) -> Homogeneous {
    point.map(|coordinate| coordinate * factor)
}

fn lifted(point: Point3, weight: f64) -> Homogeneous {
    [point.x * weight, point.y * weight, point.z * weight, weight]
}

fn projected(point: Homogeneous) -> Result<(Point3, f64), GeometryError> {
    let [x, y, z, weight] = point;
    if !(weight.is_finite() && weight > 0.0) {
        return Err(GeometryError::Weight(weight));
    }
    let cartesian = Point3::new(x, y, z) / weight;
    if !cartesian.is_finite() {
        return Err(GeometryError::NonFinite);
    }
    Ok((cartesian, weight))
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Beziers {
    degree: usize,
    breaks: Vec<f64>,
    points: Vec<Homogeneous>,
}

impl Beziers {
    pub(super) fn of(spline: &BSplineCurve) -> Result<Self, GeometryError> {
        let degree = spline.degree();
        let breakpoints = spline.breakpoints();
        let (Some(first), Some(last)) = (breakpoints.first(), breakpoints.last()) else {
            return Err(GeometryError::Knots);
        };
        let length = last - first;
        if !(length.is_finite() && length > 0.0) {
            return Err(GeometryError::Knots);
        }
        let interior: Vec<f64> = breakpoints
            .iter()
            .skip(1)
            .take(breakpoints.len().saturating_sub(2))
            .flat_map(|knot| std::iter::repeat_n(*knot, degree))
            .collect();
        let refined = spline.with_knots(&interior).ok_or(GeometryError::Knots)?;
        let segments = breakpoints.len() - 1;
        if refined.control_points().len() != segments * degree + 1 {
            return Err(GeometryError::Knots);
        }
        let points = refined
            .control_points()
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let weight = refined
                    .weights()
                    .and_then(|weights| weights.get(index))
                    .copied()
                    .unwrap_or(1.0);
                lifted(*point, weight)
            })
            .collect();
        Ok(Self {
            degree,
            breaks: breakpoints
                .iter()
                .map(|knot| (knot - first) / length)
                .collect(),
            points,
        })
    }

    pub(super) fn point(point: Point3, degree: usize) -> Self {
        Self {
            degree,
            breaks: vec![0.0, 1.0],
            points: vec![lifted(point, 1.0); degree + 1],
        }
    }

    pub(super) fn degree(&self) -> usize {
        self.degree
    }

    pub(super) fn breaks(&self) -> &[f64] {
        &self.breaks
    }

    pub(super) fn points(&self) -> &[Homogeneous] {
        &self.points
    }

    fn segment(&self, index: usize) -> &[Homogeneous] {
        let start = index * self.degree;
        self.points
            .get(start..=start + self.degree)
            .unwrap_or_default()
    }

    fn segments(&self) -> usize {
        self.breaks.len().saturating_sub(1)
    }

    fn assembled(degree: usize, breaks: Vec<f64>, segments: &[Vec<Homogeneous>]) -> Self {
        let mut points = Vec::with_capacity(segments.len() * degree + 1);
        for (index, segment) in segments.iter().enumerate() {
            let skip = usize::from(index > 0);
            points.extend(segment.iter().skip(skip).copied());
        }
        Self {
            degree,
            breaks,
            points,
        }
    }

    #[must_use]
    pub(super) fn elevated(&self, degree: usize) -> Self {
        if degree <= self.degree {
            return self.clone();
        }
        let segments: Vec<Vec<Homogeneous>> = (0..self.segments())
            .map(|index| {
                let mut points = self.segment(index).to_vec();
                while points.len() <= degree {
                    points = raised(&points);
                }
                points
            })
            .collect();
        Self::assembled(degree, self.breaks.clone(), &segments)
    }

    #[must_use]
    pub(super) fn refined(&self, breaks: &[f64]) -> Self {
        let mut kept = vec![0.0];
        let mut segments = Vec::with_capacity(self.segments());
        for index in 0..self.segments() {
            let (Some(start), Some(end)) = (self.breaks.get(index), self.breaks.get(index + 1))
            else {
                continue;
            };
            let mut rest = self.segment(index).to_vec();
            let mut from = *start;
            for target in breaks {
                if *target <= from + SAME_BREAK || *target >= end - SAME_BREAK {
                    continue;
                }
                let (before, after) = split(&rest, (target - from) / (end - from));
                segments.push(before);
                kept.push(*target);
                rest = after;
                from = *target;
            }
            segments.push(rest);
            kept.push(*end);
        }
        Self::assembled(self.degree, kept, &segments)
    }

    #[must_use]
    pub(super) fn normalised(&self) -> Self {
        let segments: Vec<Vec<Homogeneous>> = (0..self.segments())
            .map(|index| ends_weighted_one(self.segment(index)))
            .collect();
        Self::assembled(self.degree, self.breaks.clone(), &segments)
    }

    pub(super) fn knots(&self) -> Vec<f64> {
        let interior = self
            .breaks
            .iter()
            .skip(1)
            .take(self.breaks.len().saturating_sub(2));
        std::iter::repeat_n(0.0, self.degree + 1)
            .chain(interior.flat_map(|knot| std::iter::repeat_n(*knot, self.degree)))
            .chain(std::iter::repeat_n(1.0, self.degree + 1))
            .collect()
    }
}

fn raised(points: &[Homogeneous]) -> Vec<Homogeneous> {
    let degree = points.len();
    let mut raised = Vec::with_capacity(degree + 1);
    for index in 0..=degree {
        let share = index as f64 / degree as f64;
        let point = match (
            index.checked_sub(1).and_then(|before| points.get(before)),
            points.get(index),
        ) {
            (Some(before), Some(at)) => mix(*at, *before, share),
            (None, Some(at)) => *at,
            (Some(before), None) => *before,
            (None, None) => continue,
        };
        raised.push(point);
    }
    raised
}

fn split(points: &[Homogeneous], at: f64) -> (Vec<Homogeneous>, Vec<Homogeneous>) {
    let mut level = points.to_vec();
    let mut before = Vec::with_capacity(points.len());
    let mut after = Vec::with_capacity(points.len());
    while let Some(first) = level.first() {
        before.push(*first);
        if let Some(last) = level.last() {
            after.push(*last);
        }
        level = level
            .windows(2)
            .filter_map(|pair| match pair {
                [from, to] => Some(mix(*from, *to, at)),
                _ => None,
            })
            .collect();
    }
    after.reverse();
    (before, after)
}

fn ends_weighted_one(points: &[Homogeneous]) -> Vec<Homogeneous> {
    let degree = points.len().saturating_sub(1);
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return points.to_vec();
    };
    let (start, end) = (first[3], last[3]);
    if degree == 0 || start <= 0.0 || end <= 0.0 {
        return points.to_vec();
    }
    let ratio = (start / end).powf(1.0 / degree as f64);
    points
        .iter()
        .enumerate()
        .map(|(index, point)| scaled(*point, ratio.powi(index as i32) / start))
        .collect()
}

pub(super) fn compatible(curves: &[Beziers]) -> Vec<Beziers> {
    let degree = curves.iter().map(Beziers::degree).max().unwrap_or(1);
    let mut breaks: Vec<f64> = curves
        .iter()
        .flat_map(|curve| curve.breaks().iter().copied())
        .collect();
    breaks.sort_by(f64::total_cmp);
    breaks.dedup_by(|later, earlier| *later - *earlier <= SAME_BREAK);
    curves
        .iter()
        .map(|curve| curve.elevated(degree).refined(&breaks).normalised())
        .collect()
}

pub(super) fn ruled(bottom: &Beziers, top: &Beziers) -> Result<BSplineSurface, GeometryError> {
    net(
        bottom,
        &[bottom.points().to_vec(), top.points().to_vec()],
        1,
        vec![0.0, 0.0, 1.0, 1.0],
    )
}

pub(super) fn skinned(rows: &[Beziers], stations: &[f64]) -> Result<BSplineSurface, GeometryError> {
    let first = rows.first().ok_or(GeometryError::DegenerateSurface)?;
    let degree = (rows.len().saturating_sub(1)).clamp(1, 3);
    let width = first.points().len();
    let mut columns: Vec<Vec<Homogeneous>> = Vec::with_capacity(width);
    let mut knots = Vec::new();
    for column in 0..width {
        let data: Vec<Homogeneous> = rows
            .iter()
            .map(|row| {
                row.points()
                    .get(column)
                    .copied()
                    .ok_or(GeometryError::DegenerateSurface)
            })
            .collect::<Result<_, _>>()?;
        let spatial: Vec<Point3> = data
            .iter()
            .map(|[x, y, z, _]| Point3::new(*x, *y, *z))
            .collect();
        let weights: Vec<Point2> = data
            .iter()
            .map(|[.., weight]| Point2::new(*weight, 0.0))
            .collect();
        let through = BSpline::interpolating_at(degree, stations, &spatial)?;
        let weighted = BSpline::interpolating_at(degree, stations, &weights)?;
        knots = through.knots().to_vec();
        columns.push(
            through
                .control_points()
                .iter()
                .zip(weighted.control_points())
                .map(|(point, weight)| [point.x, point.y, point.z, weight.x])
                .collect(),
        );
    }
    let height = columns.first().map_or(0, Vec::len);
    let lines: Vec<Vec<Homogeneous>> = (0..height)
        .map(|row| {
            columns
                .iter()
                .filter_map(|column| column.get(row).copied())
                .collect()
        })
        .collect();
    net(first, &lines, degree, knots)
}

fn net(
    across: &Beziers,
    lines: &[Vec<Homogeneous>],
    degree: usize,
    knots: Vec<f64>,
) -> Result<BSplineSurface, GeometryError> {
    let columns = across.points().len();
    let mut points = Vec::with_capacity(columns * lines.len());
    let mut weights = Vec::with_capacity(columns * lines.len());
    for line in lines {
        if line.len() != columns {
            return Err(GeometryError::DegenerateSurface);
        }
        for point in line {
            let (cartesian, weight) = projected(*point)?;
            points.push(cartesian);
            weights.push(weight);
        }
    }
    BSplineSurface::new(
        across.degree(),
        degree,
        across.knots(),
        knots,
        columns,
        points,
        Some(weights),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SkinEnd {
    Natural,
    Along(Vector3),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SkinEnds {
    pub(super) start: SkinEnd,
    pub(super) end: SkinEnd,
    pub(super) closed: bool,
}

fn plus(from: Homogeneous, to: Homogeneous) -> Homogeneous {
    let mut sum = from;
    for (slot, added) in sum.iter_mut().zip(to) {
        *slot += added;
    }
    sum
}

fn minus(from: Homogeneous, to: Homogeneous) -> Homogeneous {
    plus(from, scaled(to, -1.0))
}

fn slope(from: (f64, Homogeneous), to: (f64, Homogeneous)) -> Homogeneous {
    scaled(minus(to.1, from.1), 1.0 / (to.0 - from.0))
}

fn quadratic_slopes(
    first: (f64, Homogeneous),
    second: (f64, Homogeneous),
    third: (f64, Homogeneous),
) -> [Homogeneous; 3] {
    let low = slope(first, second);
    let high = slope(second, third);
    let bend = scaled(minus(high, low), 1.0 / (third.0 - first.0));
    [
        minus(low, scaled(bend, second.0 - first.0)),
        plus(low, scaled(bend, second.0 - first.0)),
        plus(
            low,
            scaled(bend, (third.0 - first.0) + (third.0 - second.0)),
        ),
    ]
}

fn along(direction: Vector3, at: Homogeneous) -> Homogeneous {
    let [.., weight] = at;
    [
        direction.x * weight,
        direction.y * weight,
        direction.z * weight,
        0.0,
    ]
}

fn end_slopes(
    data: &[Homogeneous],
    stations: &[f64],
    ends: SkinEnds,
) -> Result<[Homogeneous; 2], GeometryError> {
    let at = |index: usize| -> Result<(f64, Homogeneous), GeometryError> {
        Ok((
            *stations
                .get(index)
                .ok_or(GeometryError::DegenerateSurface)?,
            *data.get(index).ok_or(GeometryError::DegenerateSurface)?,
        ))
    };
    let last = data.len().saturating_sub(1);
    if ends.closed {
        let (before, point) = at(last.saturating_sub(1))?;
        let seam = at(0)?;
        let [_, slope, _] = quadratic_slopes((before - 1.0, point), seam, at(1)?);
        return Ok([slope, slope]);
    }
    let start = match ends.start {
        SkinEnd::Along(direction) => along(direction, at(0)?.1),
        SkinEnd::Natural if last >= 2 => quadratic_slopes(at(0)?, at(1)?, at(2)?)[0],
        SkinEnd::Natural => slope(at(0)?, at(1)?),
    };
    let end = match ends.end {
        SkinEnd::Along(direction) => along(direction, at(last)?.1),
        SkinEnd::Natural if last >= 2 => {
            quadratic_slopes(at(last - 2)?, at(last - 1)?, at(last)?)[2]
        }
        SkinEnd::Natural => slope(at(last.saturating_sub(1))?, at(last)?),
    };
    Ok([start, end])
}

fn basis(knots: &[f64], index: usize, degree: usize, at: f64) -> f64 {
    let knot = |position: usize| knots.get(position).copied().unwrap_or(f64::NAN);
    if degree == 0 {
        return if knot(index) <= at && at < knot(index + 1) {
            1.0
        } else {
            0.0
        };
    }
    let rising = knot(index + degree) - knot(index);
    let falling = knot(index + degree + 1) - knot(index + 1);
    let left = if rising > 0.0 {
        (at - knot(index)) / rising * basis(knots, index, degree - 1, at)
    } else {
        0.0
    };
    let right = if falling > 0.0 {
        (knot(index + degree + 1) - at) / falling * basis(knots, index + 1, degree - 1, at)
    } else {
        0.0
    };
    left + right
}

fn hermite(
    data: &[Homogeneous],
    stations: &[f64],
    [start, end]: [Homogeneous; 2],
) -> Result<(Vec<Homogeneous>, Vec<f64>), GeometryError> {
    let (Some(first), Some(last)) = (data.first(), data.last()) else {
        return Err(GeometryError::DegenerateSurface);
    };
    let (Some(low), Some(high)) = (stations.first(), stations.last()) else {
        return Err(GeometryError::DegenerateSurface);
    };
    let count = data.len();
    if count < 2 || stations.len() != count {
        return Err(GeometryError::DegenerateSurface);
    }
    let interior = stations.get(1..count - 1).unwrap_or_default();
    let knots: Vec<f64> = std::iter::repeat_n(*low, HERMITE_DEGREE + 1)
        .chain(interior.iter().copied())
        .chain(std::iter::repeat_n(*high, HERMITE_DEGREE + 1))
        .collect();
    let second_station = stations.get(1).copied().unwrap_or(*high);
    let before_last = stations.get(count - 2).copied().unwrap_or(*low);
    let lead = plus(*first, scaled(start, (second_station - low) / 3.0));
    let trail = minus(*last, scaled(end, (high - before_last) / 3.0));
    let mut lower = Vec::with_capacity(count);
    let mut diagonal = Vec::with_capacity(count);
    let mut upper = Vec::with_capacity(count);
    let mut sides = Vec::with_capacity(count);
    for (row, (station, point)) in interior.iter().zip(data.iter().skip(1)).enumerate() {
        let index = row + 1;
        let below = basis(&knots, index, HERMITE_DEGREE, *station);
        let at = basis(&knots, index + 1, HERMITE_DEGREE, *station);
        let above = basis(&knots, index + 2, HERMITE_DEGREE, *station);
        let mut side = *point;
        if row == 0 {
            side = minus(side, scaled(lead, below));
        }
        if row + 1 == interior.len() {
            side = minus(side, scaled(trail, above));
        }
        lower.push(below);
        diagonal.push(at);
        upper.push(above);
        sides.push(side);
    }
    let mut factors: Vec<f64> = Vec::with_capacity(sides.len());
    let mut carried: Vec<Homogeneous> = Vec::with_capacity(sides.len());
    for (row, ((below, at), (above, side))) in lower
        .iter()
        .zip(&diagonal)
        .zip(upper.iter().zip(&sides))
        .enumerate()
    {
        let (previous_factor, previous_side) = match (factors.last(), carried.last()) {
            (Some(factor), Some(side)) if row > 0 => (*factor, *side),
            _ => (0.0, [0.0; 4]),
        };
        let pivot = at - below * previous_factor;
        if !(pivot.is_finite() && pivot.abs() > f64::EPSILON) {
            return Err(GeometryError::NonFinite);
        }
        factors.push(above / pivot);
        carried.push(scaled(
            minus(*side, scaled(previous_side, *below)),
            1.0 / pivot,
        ));
    }
    let mut solved: Vec<Homogeneous> = Vec::with_capacity(carried.len());
    for (factor, side) in factors.iter().zip(&carried).rev() {
        let next = solved.last().copied().unwrap_or([0.0; 4]);
        solved.push(minus(*side, scaled(next, *factor)));
    }
    solved.reverse();
    let mut points = Vec::with_capacity(count + 2);
    points.push(*first);
    points.push(lead);
    points.extend(solved);
    points.push(trail);
    points.push(*last);
    Ok((points, knots))
}

pub(super) fn skinned_through(
    rows: &[Beziers],
    stations: &[f64],
    ends: SkinEnds,
) -> Result<BSplineSurface, GeometryError> {
    let first = rows.first().ok_or(GeometryError::DegenerateSurface)?;
    let width = first.points().len();
    let mut columns: Vec<Vec<Homogeneous>> = Vec::with_capacity(width);
    let mut knots = Vec::new();
    for column in 0..width {
        let data: Vec<Homogeneous> = rows
            .iter()
            .map(|row| {
                row.points()
                    .get(column)
                    .copied()
                    .ok_or(GeometryError::DegenerateSurface)
            })
            .collect::<Result<_, _>>()?;
        let slopes = end_slopes(&data, stations, ends)?;
        let (points, column_knots) = hermite(&data, stations, slopes)?;
        knots = column_knots;
        columns.push(points);
    }
    let height = columns.first().map_or(0, Vec::len);
    let lines: Vec<Vec<Homogeneous>> = (0..height)
        .map(|row| {
            columns
                .iter()
                .filter_map(|column| column.get(row).copied())
                .collect()
        })
        .collect();
    net(first, &lines, HERMITE_DEGREE, knots)
}
