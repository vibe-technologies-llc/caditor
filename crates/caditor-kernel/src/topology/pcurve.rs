use std::sync::Arc;

use caditor_geometry::{Point2, Vector2};
use thiserror::Error;

use crate::{
    coordinates::distance_to_segment,
    curve::Curve,
    interrupt::{self, Interrupted},
    interval::Interval,
    parametric::Parametric,
    sense::Sense,
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, PCURVE_TOLERANCE},
};

const MAX_PCURVE_SAMPLES: usize = 1 << 16;
const MAX_PCURVE_DEPTH: usize = 30;
const MAX_PERIOD_FRACTION_PER_STEP: f64 = 0.25;
const BOW_SHARE: f64 = 0.5;
const BOW_SAMPLES: [f64; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

#[derive(Debug, Clone, PartialEq, Error)]
pub enum PcurveError {
    #[error("a pcurve needs at least two samples")]
    TooFewSamples,
    #[error("a pcurve sample is not finite")]
    NonFinite,
    #[error("the pcurve parameters do not run in one direction")]
    NotMonotone,
    #[error("the edge image in the surface parameters needs more than {0} samples")]
    TooComplex(usize),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcurveSample {
    pub parameter: f64,
    pub uv: Point2,
}

#[derive(Debug, Clone)]
pub struct Pcurve {
    samples: Arc<[PcurveSample]>,
    tolerance: f64,
}

impl PartialEq for Pcurve {
    fn eq(&self, other: &Self) -> bool {
        self.tolerance == other.tolerance
            && (Arc::ptr_eq(&self.samples, &other.samples) || self.samples == other.samples)
    }
}

impl Pcurve {
    pub fn heap_size(&self) -> usize {
        size_of_val(&*self.samples)
    }

    pub fn new(samples: Vec<PcurveSample>, tolerance: f64) -> Result<Self, PcurveError> {
        if samples.len() < 2 {
            return Err(PcurveError::TooFewSamples);
        }
        let finite = tolerance.is_finite()
            && tolerance >= 0.0
            && samples
                .iter()
                .all(|sample| sample.parameter.is_finite() && sample.uv.is_finite());
        if !finite {
            return Err(PcurveError::NonFinite);
        }
        let increasing = samples
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if b.parameter > a.parameter));
        let decreasing = samples
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if b.parameter < a.parameter));
        if !increasing && !decreasing {
            return Err(PcurveError::NotMonotone);
        }
        Ok(Self {
            samples: samples.into(),
            tolerance,
        })
    }

    pub fn samples(&self) -> &[PcurveSample] {
        &self.samples
    }

    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }

    pub fn start(&self) -> Point2 {
        self.samples
            .first()
            .map_or(Point2::ZERO, |sample| sample.uv)
    }

    pub fn end(&self) -> Point2 {
        self.samples.last().map_or(Point2::ZERO, |sample| sample.uv)
    }

    pub fn start_parameter(&self) -> f64 {
        self.samples.first().map_or(0.0, |sample| sample.parameter)
    }

    pub fn end_parameter(&self) -> f64 {
        self.samples.last().map_or(0.0, |sample| sample.parameter)
    }

    pub fn uv_at(&self, parameter: f64) -> Point2 {
        let increasing = self.end_parameter() >= self.start_parameter();
        let beyond = self.samples.partition_point(|sample| {
            if increasing {
                sample.parameter < parameter
            } else {
                sample.parameter > parameter
            }
        });
        let after = beyond.clamp(1, self.samples.len().saturating_sub(1));
        let (Some(first), Some(second)) = (self.samples.get(after - 1), self.samples.get(after))
        else {
            return self.start();
        };
        let span = second.parameter - first.parameter;
        let fraction = if span != 0.0 {
            ((parameter - first.parameter) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        first.uv.lerp(second.uv, fraction)
    }

    #[must_use]
    pub(crate) fn with_ends(&self, start: Point2, end: Point2) -> Self {
        if self.start() == start && self.end() == end {
            return self.clone();
        }
        let mut samples = self.samples.to_vec();
        if let Some(first) = samples.first_mut() {
            first.uv = start;
        }
        if let Some(last) = samples.last_mut() {
            last.uv = end;
        }
        Self {
            samples: samples.into(),
            tolerance: self.tolerance,
        }
    }

    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            samples: self.samples.iter().rev().copied().collect(),
            tolerance: self.tolerance,
        }
    }

    pub(crate) fn refined(&self, surface: &Surface, curve: &Curve) -> Result<Self, PcurveError> {
        self.refined_within(surface, curve, self.tolerance)
    }

    pub(crate) fn refined_within(
        &self,
        surface: &Surface,
        curve: &Curve,
        tolerance: f64,
    ) -> Result<Self, PcurveError> {
        let refined = refine(surface, curve, &self.samples, tolerance)?;
        let usable = tolerance.is_finite() && tolerance >= 0.0;
        if usable && refined.len() == self.samples.len() {
            return Ok(Self {
                samples: Arc::clone(&self.samples),
                tolerance,
            });
        }
        Self::new(refined, tolerance)
    }

    #[must_use]
    pub fn shifted(&self, offset: Vector2) -> Self {
        if offset == Vector2::ZERO {
            return self.clone();
        }
        Self {
            samples: self
                .samples
                .iter()
                .map(|sample| PcurveSample {
                    parameter: sample.parameter,
                    uv: sample.uv + offset,
                })
                .collect(),
            tolerance: self.tolerance,
        }
    }
}

pub(crate) fn fit(
    surface: &Surface,
    curve: &Curve,
    interval: Interval,
    sense: Sense,
    hint: Option<Point2>,
) -> Result<Pcurve, PcurveError> {
    let mut parameters = curve.seeds(interval);
    if !sense.is_same() {
        parameters.reverse();
    }
    let mut samples: Vec<PcurveSample> = Vec::with_capacity(parameters.len());
    let mut parameters = parameters.into_iter();
    if let Some(parameter) = parameters.next() {
        samples.push(PcurveSample {
            parameter,
            uv: surface.project(curve.point(parameter), hint),
        });
    }
    for parameter in parameters {
        let Some(from) = samples.last().copied() else {
            break;
        };
        follow(surface, curve, from, parameter, &mut samples)?;
    }
    settle_pole_ends(surface, &mut samples);
    let tolerance = shape_tolerance(curve, interval);
    let refined = refine(surface, curve, &samples, tolerance)?;
    Pcurve::new(refined, tolerance)
}

pub(crate) fn shape_tolerance(curve: &Curve, interval: Interval) -> f64 {
    let (start, end) = (curve.point(interval.start()), curve.point(interval.end()));
    let bow = BOW_SAMPLES
        .iter()
        .map(|fraction| distance_to_segment(curve.point(interval.at(*fraction)), start, end))
        .fold(0.0, f64::max);
    (BOW_SHARE * bow).clamp(LINEAR_RESOLUTION, PCURVE_TOLERANCE)
}

fn follow(
    surface: &Surface,
    curve: &Curve,
    from: PcurveSample,
    to: f64,
    samples: &mut Vec<PcurveSample>,
) -> Result<(), PcurveError> {
    let mut pending = vec![(to, 0)];
    let mut last = from;
    while let Some((parameter, depth)) = pending.pop() {
        interrupt::check()?;
        if samples.len() + pending.len() >= MAX_PCURVE_SAMPLES {
            return Err(PcurveError::TooComplex(MAX_PCURVE_SAMPLES));
        }
        let uv = surface.project(curve.point(parameter), Some(last.uv));
        let middle = 0.5 * (last.parameter + parameter);
        let divisible = depth < MAX_PCURVE_DEPTH && middle != last.parameter && middle != parameter;
        if divisible && turns_too_far(surface, last.uv, uv) {
            pending.push((parameter, depth + 1));
            pending.push((middle, depth + 1));
            continue;
        }
        last = PcurveSample { parameter, uv };
        samples.push(last);
    }
    Ok(())
}

fn turns_too_far(surface: &Surface, from: Point2, to: Point2) -> bool {
    if surface.pole_at(from).is_some() || surface.pole_at(to).is_some() {
        return false;
    }
    let beyond = |step: f64, period: Option<f64>| {
        period.is_some_and(|period| step.abs() > period * MAX_PERIOD_FRACTION_PER_STEP)
    };
    beyond(to.x - from.x, surface.u_period()) || beyond(to.y - from.y, surface.v_period())
}

pub(crate) fn settle_pole_ends(surface: &Surface, samples: &mut [PcurveSample]) {
    let count = samples.len();
    for (end, neighbour) in [(0, 1), (count.saturating_sub(1), count.saturating_sub(2))] {
        let Some(next_u) = samples.get(neighbour).map(|sample| sample.uv.x) else {
            continue;
        };
        if let Some(sample) = samples.get_mut(end)
            && let Some(pole) = surface.pole_at(sample.uv)
        {
            sample.uv = Point2::new(next_u, pole.v);
        }
    }
}

fn refine(
    surface: &Surface,
    curve: &Curve,
    samples: &[PcurveSample],
    tolerance: f64,
) -> Result<Vec<PcurveSample>, PcurveError> {
    let Some(first) = samples.first() else {
        return Err(PcurveError::TooFewSamples);
    };
    let mut refined = vec![*first];
    for pair in samples.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let mut pending = vec![(*start, *end, 0)];
        while let Some((low, high, depth)) = pending.pop() {
            interrupt::check()?;
            if refined.len() + pending.len() >= MAX_PCURVE_SAMPLES {
                return Err(PcurveError::TooComplex(MAX_PCURVE_SAMPLES));
            }
            let parameter = 0.5 * (low.parameter + high.parameter);
            let chord = low.uv.lerp(high.uv, 0.5);
            let uv = surface.project(curve.point(parameter), Some(chord));
            let deviation = surface.point_at(chord).distance(surface.point_at(uv));
            let splittable = depth < MAX_PCURVE_DEPTH
                && parameter != low.parameter
                && parameter != high.parameter;
            if splittable && (deviation.is_nan() || deviation > tolerance) {
                let middle = PcurveSample { parameter, uv };
                pending.push((middle, high, depth + 1));
                pending.push((low, middle, depth + 1));
            } else {
                refined.push(high);
            }
        }
    }
    Ok(refined)
}
