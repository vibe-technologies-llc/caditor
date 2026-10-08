mod band;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use caditor_geometry::{Point3, Vector3};
use thiserror::Error;

use self::band::Band;
use crate::{
    bspline::BSpline, error::GeometryError, interrupt, interval::Interval, surface::BSplineSurface,
};

pub const MAX_SIDE_CONTROL_POINTS: usize = 8_192;
pub const MAX_BENT_CONTROL_POINTS: usize = 250_000;
const MAX_REFINEMENTS: usize = 10;
const EXTRA_SAMPLES: usize = 3;
const CHECKS_PER_SPAN: usize = 16;
const SMOOTHING: f64 = 1e-10;
const RIDGE: f64 = 1e-14;
const KNOT_SNAP: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurfaceSide {
    UStart,
    UEnd,
    VStart,
    VEnd,
}

impl SurfaceSide {
    pub fn along_u(self) -> bool {
        matches!(self, Self::VStart | Self::VEnd)
    }

    pub fn at_start(self) -> bool {
        matches!(self, Self::UStart | Self::VStart)
    }
}

pub struct BendTarget<'a> {
    pub range: Interval,
    pub point: &'a dyn Fn(f64) -> Option<Point3>,
    pub strict: bool,
}

pub struct SideBend<'a> {
    pub side: SurfaceSide,
    pub targets: Vec<BendTarget<'a>>,
    pub pins: Vec<(f64, Point3)>,
    pub knots: Vec<f64>,
    pub held: [bool; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct BentSide {
    pub surface: BSplineSurface,
    pub residual: f64,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum BendError {
    #[error("the side of the surface is a seam, a pole or a closed curve, so it cannot be bent")]
    Unbendable,
    #[error("the bent side has no target at parameter {0}")]
    NoTarget(f64),
    #[error(
        "bending the side would need {0} control points along it, more than \
         {MAX_SIDE_CONTROL_POINTS}"
    )]
    TooDetailed(usize),
    #[error(
        "bending the side would leave the surface with {0} control points, more than \
         {MAX_BENT_CONTROL_POINTS}"
    )]
    TooLarge(usize),
    #[error("the bent side came only within {0} mm of its targets")]
    FallsShort(f64),
    #[error("the bent side has no unique fit")]
    Singular,
    #[error("the bent surface is not usable ({0})")]
    Geometry(#[from] GeometryError),
    #[error("the operation was cancelled")]
    Cancelled,
}

impl BSplineSurface {
    pub fn bent_side(&self, bend: &SideBend<'_>, tolerance: f64) -> Result<BentSide, BendError> {
        let along_u = bend.side.along_u();
        let oriented = if along_u {
            self.clone()
        } else {
            self.transposed()
        };
        let (row_side, row) = if bend.side.at_start() {
            (SurfaceSide::VStart, 0)
        } else {
            (SurfaceSide::VEnd, oriented.rows().saturating_sub(1))
        };
        if oriented.u_period().is_some()
            || oriented.v_period().is_some()
            || oriented.degenerate_row(row)
        {
            return Err(BendError::Unbendable);
        }
        let oriented = oriented
            .cubic_along_u()
            .ok_or(BendError::Geometry(GeometryError::Knots))?;
        let original = oriented
            .side(row_side)
            .ok_or(BendError::Geometry(GeometryError::Knots))?;
        let (curve, residual) = Fit::new(&original, bend)?.run(bend, tolerance)?;
        let inserted = added_knots(oriented.u_knots(), curve.knots());
        let bent = oriented
            .with_knots(&inserted, &[])
            .and_then(|refined| refined.with_side(row_side, &curve))
            .ok_or(BendError::Geometry(GeometryError::Knots))?;
        let count = bent.control_points().len();
        if count > MAX_BENT_CONTROL_POINTS {
            return Err(BendError::TooLarge(count));
        }
        let surface = if along_u { bent } else { bent.transposed() };
        Ok(BentSide { surface, residual })
    }
}

struct Fit {
    reference: BSpline<Point3>,
    current: BSpline<Point3>,
    pins: Vec<(f64, Point3)>,
    held: [Option<Point3>; 2],
    cache: BTreeMap<(usize, u64), Point3>,
}

impl Fit {
    fn new(original: &BSpline<Point3>, bend: &SideBend<'_>) -> Result<Self, BendError> {
        let domain = original.domain();
        let degree = original.degree();
        let mut wanted: BTreeMap<u64, usize> = BTreeMap::new();
        for knot in &bend.knots {
            if !(domain.start() < *knot && *knot < domain.end()) {
                continue;
            }
            let snapped = snapped(original.knots(), *knot);
            *wanted.entry(snapped.to_bits()).or_default() += 1;
        }
        for (parameter, point) in &bend.pins {
            if !(parameter.is_finite() && point.is_finite()) {
                return Err(BendError::NoTarget(*parameter));
            }
            if domain.start() < *parameter && *parameter < domain.end() {
                wanted.insert(parameter.to_bits(), degree);
            }
        }
        let mut inserted = Vec::new();
        for (bits, multiplicity) in wanted {
            let value = f64::from_bits(bits);
            let present = original
                .knots()
                .iter()
                .filter(|knot| **knot == value)
                .count();
            let missing = multiplicity.min(degree).saturating_sub(present);
            inserted.extend(std::iter::repeat_n(value, missing));
        }
        let reference = original
            .with_knots(&inserted)
            .ok_or(BendError::Geometry(GeometryError::Knots))?;
        let first = original.control_points().first().copied();
        let last = original.control_points().last().copied();
        let held = [
            bend.held[0].then_some(first).flatten(),
            bend.held[1].then_some(last).flatten(),
        ];
        Ok(Self {
            current: reference.clone(),
            reference,
            pins: bend.pins.clone(),
            held,
            cache: BTreeMap::new(),
        })
    }

    fn run(
        mut self,
        bend: &SideBend<'_>,
        tolerance: f64,
    ) -> Result<(BSpline<Point3>, f64), BendError> {
        for round in 0..=MAX_REFINEMENTS {
            interrupt::check().map_err(|_| BendError::Cancelled)?;
            let fixed = self.fixed();
            let samples = self.samples(bend)?;
            self.current = solved(&self.reference, &fixed, &samples)?;
            let (failing, worst) = self.failing(bend, tolerance)?;
            if failing.is_empty() {
                return Ok((self.current, worst));
            }
            if round == MAX_REFINEMENTS {
                return Err(BendError::FallsShort(worst));
            }
            let middles: Vec<f64> = failing.iter().map(Interval::middle).collect();
            self.reference = self
                .reference
                .with_knots(&middles)
                .ok_or(BendError::Geometry(GeometryError::Knots))?;
            self.current = self
                .current
                .with_knots(&middles)
                .ok_or(BendError::Geometry(GeometryError::Knots))?;
            let count = self.reference.control_points().len();
            if count > MAX_SIDE_CONTROL_POINTS {
                return Err(BendError::TooDetailed(count));
            }
        }
        Err(BendError::FallsShort(f64::INFINITY))
    }

    fn fixed(&self) -> Vec<Option<Point3>> {
        let knots = self.reference.knots();
        let domain = self.reference.domain();
        let count = self.reference.control_points().len();
        let mut fixed = vec![None; count];
        let last = count.saturating_sub(1);
        if let Some(slot) = fixed.first_mut() {
            *slot = self.held[0];
        }
        if let Some(slot) = fixed.get_mut(last) {
            *slot = self.held[1];
        }
        for (parameter, point) in &self.pins {
            let index = if *parameter <= domain.start() {
                Some(0)
            } else if *parameter >= domain.end() {
                Some(last)
            } else {
                knots
                    .iter()
                    .position(|knot| *knot == *parameter)
                    .and_then(|first| first.checked_sub(1))
            };
            if let Some(slot) = index.and_then(|index| fixed.get_mut(index)) {
                *slot = Some(*point);
            }
        }
        fixed
    }

    fn target(
        &mut self,
        index: usize,
        target: &BendTarget<'_>,
        parameter: f64,
    ) -> Result<Point3, BendError> {
        let key = (index, parameter.to_bits());
        if let Some(point) = self.cache.get(&key) {
            return Ok(*point);
        }
        let point = (target.point)(parameter)
            .filter(|point| point.is_finite())
            .ok_or(BendError::NoTarget(parameter))?;
        self.cache.insert(key, point);
        Ok(point)
    }

    fn samples(&mut self, bend: &SideBend<'_>) -> Result<Vec<(f64, Point3)>, BendError> {
        let spans = spans(&self.reference);
        let per_span = self.reference.degree() + EXTRA_SAMPLES;
        let mut samples = Vec::with_capacity(spans.len() * per_span);
        for (index, target) in bend.targets.iter().enumerate() {
            for span in &spans {
                let Some(overlap) = overlap(*span, target.range) else {
                    continue;
                };
                for step in 0..per_span {
                    let parameter = overlap.at((step as f64 + 0.5) / per_span as f64);
                    samples.push((parameter, self.target(index, target, parameter)?));
                }
            }
        }
        Ok(samples)
    }

    fn failing(
        &mut self,
        bend: &SideBend<'_>,
        tolerance: f64,
    ) -> Result<(Vec<Interval>, f64), BendError> {
        let spans = spans(&self.reference);
        let mut failing: Vec<Interval> = Vec::new();
        let mut worst = 0.0f64;
        for (index, target) in bend.targets.iter().enumerate() {
            if !target.strict {
                continue;
            }
            interrupt::check().map_err(|_| BendError::Cancelled)?;
            for span in &spans {
                let Some(overlap) = overlap(*span, target.range) else {
                    continue;
                };
                let mut span_worst = 0.0f64;
                for step in 0..=CHECKS_PER_SPAN {
                    let parameter = overlap.at(step as f64 / CHECKS_PER_SPAN as f64);
                    let wanted = self.target(index, target, parameter)?;
                    span_worst = span_worst.max(self.current.point(parameter).distance(wanted));
                }
                worst = worst.max(span_worst);
                if span_worst > tolerance && !failing.contains(span) {
                    failing.push(*span);
                }
            }
        }
        Ok((failing, worst))
    }
}

fn snapped(knots: &[f64], knot: f64) -> f64 {
    let after = knots.partition_point(|existing| *existing <= knot);
    let below = after.checked_sub(1).and_then(|index| knots.get(index));
    let above = knots.get(after);
    let (Some(below), Some(above)) = (below, above) else {
        return knot;
    };
    let reach = KNOT_SNAP * (above - below);
    if knot - below <= reach {
        *below
    } else if above - knot <= reach {
        *above
    } else {
        knot
    }
}

fn spans(curve: &BSpline<Point3>) -> Vec<Interval> {
    curve
        .breakpoints()
        .windows(2)
        .filter_map(|pair| match pair {
            [start, end] => Interval::new(*start, *end),
            _ => None,
        })
        .collect()
}

fn overlap(span: Interval, range: Interval) -> Option<Interval> {
    let start = span.start().max(range.start());
    let end = span.end().min(range.end());
    (end > start).then(|| Interval::new(start, end)).flatten()
}

fn added_knots(old: &[f64], new: &[f64]) -> Vec<f64> {
    let mut added = Vec::new();
    let mut remaining = old.iter().peekable();
    for knot in new {
        if remaining.peek().is_some_and(|existing| **existing == *knot) {
            remaining.next();
        } else {
            added.push(*knot);
        }
    }
    added
}

fn solved(
    reference: &BSpline<Point3>,
    fixed: &[Option<Point3>],
    samples: &[(f64, Point3)],
) -> Result<BSpline<Point3>, BendError> {
    let base = reference.control_points();
    let mut compact = vec![None; base.len()];
    let mut free = 0;
    for (slot, pinned) in compact.iter_mut().zip(fixed) {
        if pinned.is_none() {
            *slot = Some(free);
            free += 1;
        }
    }
    let shift = |index: usize| -> Vector3 {
        match (fixed.get(index).copied().flatten(), base.get(index)) {
            (Some(pinned), Some(start)) => pinned - *start,
            _ => Vector3::ZERO,
        }
    };
    let degree = reference.degree();
    let mut band = Band::new(free, degree.max(2));
    let mut right = vec![Vector3::ZERO; free];
    for (parameter, target) in samples {
        let (first, values) = reference.rational_basis(*parameter);
        let mut residual = *target - reference.point(*parameter);
        for (offset, value) in values.iter().enumerate().take(degree + 1) {
            residual -= shift(first + offset) * *value;
        }
        for (a_offset, a) in values.iter().enumerate().take(degree + 1) {
            let Some(row) = compact.get(first + a_offset).copied().flatten() else {
                continue;
            };
            for (b_offset, b) in values.iter().enumerate().take(degree + 1) {
                if let Some(column) = compact.get(first + b_offset).copied().flatten() {
                    band.add(row, column, a * b);
                }
            }
            if let Some(entry) = right.get_mut(row) {
                *entry += residual * *a;
            }
        }
    }
    let scale = band.largest_diagonal().max(f64::MIN_POSITIVE);
    for middle in 1..base.len().saturating_sub(1) {
        let stencil = [(middle - 1, 1.0), (middle, -2.0), (middle + 1, 1.0)];
        let known: Vector3 = stencil
            .iter()
            .filter(|(index, _)| compact.get(*index).copied().flatten().is_none())
            .map(|(index, weight)| shift(*index) * *weight)
            .sum();
        for (a_index, a) in &stencil {
            let Some(row) = compact.get(*a_index).copied().flatten() else {
                continue;
            };
            for (b_index, b) in &stencil {
                if let Some(column) = compact.get(*b_index).copied().flatten() {
                    band.add(row, column, SMOOTHING * scale * a * b);
                }
            }
            if let Some(entry) = right.get_mut(row) {
                *entry -= known * (SMOOTHING * scale * a);
            }
        }
    }
    for row in 0..free {
        band.add(row, row, RIDGE * scale);
    }
    let moves = band
        .factored()
        .map_err(|_| BendError::Singular)?
        .solve(right);
    let points: Vec<Point3> = base
        .iter()
        .enumerate()
        .map(|(index, start)| match fixed.get(index).copied().flatten() {
            Some(pinned) => pinned,
            None => {
                let step = compact
                    .get(index)
                    .copied()
                    .flatten()
                    .and_then(|row| moves.get(row))
                    .copied()
                    .unwrap_or(Vector3::ZERO);
                *start + step
            }
        })
        .collect();
    if !points.iter().all(|point| point.is_finite()) {
        return Err(BendError::Singular);
    }
    Ok(reference.with_points(points)?)
}
