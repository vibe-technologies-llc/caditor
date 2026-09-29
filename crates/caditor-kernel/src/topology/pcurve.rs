use caditor_geometry::{Point2, Vector2};
use thiserror::Error;

use crate::{
    curve::Curve, interval::Interval, parametric::Parametric, sense::Sense, surface::Surface,
    tolerance::PCURVE_TOLERANCE,
};

const MAX_PCURVE_SAMPLES: usize = 1 << 16;
const MAX_PCURVE_DEPTH: usize = 30;

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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcurveSample {
    pub parameter: f64,
    pub uv: Point2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pcurve {
    samples: Vec<PcurveSample>,
    tolerance: f64,
}

impl Pcurve {
    pub fn heap_size(&self) -> usize {
        size_of_val(self.samples.as_slice())
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
        Ok(Self { samples, tolerance })
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
        let mut samples = self.samples.clone();
        if let Some(first) = samples.first_mut() {
            first.uv = start;
        }
        if let Some(last) = samples.last_mut() {
            last.uv = end;
        }
        Self {
            samples,
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

    #[must_use]
    pub fn shifted(&self, offset: Vector2) -> Self {
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
    let mut previous = hint;
    let mut samples: Vec<PcurveSample> = parameters
        .into_iter()
        .map(|parameter| {
            let uv = surface.project(curve.point(parameter), previous);
            previous = Some(uv);
            PcurveSample { parameter, uv }
        })
        .collect();
    settle_pole_ends(surface, &mut samples);
    let refined = refine(surface, curve, &samples)?;
    Pcurve::new(refined, PCURVE_TOLERANCE)
}

fn settle_pole_ends(surface: &Surface, samples: &mut [PcurveSample]) {
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
            if splittable && (deviation.is_nan() || deviation > PCURVE_TOLERANCE) {
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
