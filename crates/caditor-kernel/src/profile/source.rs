use std::f64::consts::{PI, TAU};

use caditor_geometry::{Aabb2, Point2};

use crate::{
    bspline::BSpline,
    curve2::{Circle2, Curve2, Line2},
    error::GeometryError,
    interval::Interval,
    parametric::refined_seeds,
    profile::{ProfileCurve, ProfileError, ProfileShape},
};

const DERIVATIVE_REFINEMENT: usize = 4;
const BISECTION_STEPS: usize = 60;
const MAX_QUARTER_BREAKS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    pub entity: u64,
    pub curve: Curve2,
    pub range: Interval,
    pub closed: bool,
}

impl Source {
    pub fn from_curve(curve: &ProfileCurve) -> Result<Self, ProfileError> {
        let entity = curve.entity;
        let degenerate = || ProfileError::Degenerate { entity };
        let invalid = |error: GeometryError| match error {
            GeometryError::ZeroDirection
            | GeometryError::NonPositive(_)
            | GeometryError::BelowResolution(_) => degenerate(),
            error => ProfileError::InvalidCurve { entity, error },
        };
        let non_finite = ProfileError::InvalidCurve {
            entity,
            error: GeometryError::NonFinite,
        };
        match &curve.shape {
            ProfileShape::Line { start, end } => {
                if !start.is_finite() || !end.is_finite() {
                    return Err(non_finite);
                }
                let line = Line2::through(*start, *end).map_err(invalid)?;
                let range = Interval::new(0.0, start.distance(*end)).ok_or(non_finite)?;
                Ok(Self::open(entity, line.into(), range))
            }
            ProfileShape::Circle { center, radius } => {
                let circle = Circle2::new(*center, *radius).map_err(invalid)?;
                Ok(Self {
                    entity,
                    curve: circle.into(),
                    range: Interval::FULL_TURN,
                    closed: true,
                })
            }
            ProfileShape::Arc { center, start, end } => {
                if !center.is_finite() || !start.is_finite() || !end.is_finite() {
                    return Err(non_finite);
                }
                let circle = Circle2::new(*center, center.distance(*start)).map_err(invalid)?;
                let angle = |point: Point2| {
                    let offset = point - *center;
                    offset.y.atan2(offset.x)
                };
                let first = angle(*start);
                let sweep = (angle(*end) - first).rem_euclid(TAU);
                if sweep <= 0.0 {
                    return Err(degenerate());
                }
                let range = Interval::new(first, first + sweep).ok_or(non_finite)?;
                Ok(Self::open(entity, circle.into(), range))
            }
            ProfileShape::Spline {
                degree,
                knots,
                control_points,
            } => {
                let spline = BSpline::new(*degree, knots.clone(), control_points.clone())
                    .map_err(|error| ProfileError::InvalidCurve { entity, error })?;
                let range = spline.domain();
                Ok(Self::open(entity, spline.into(), range))
            }
        }
    }

    fn open(entity: u64, curve: Curve2, range: Interval) -> Self {
        Self {
            entity,
            curve,
            range,
            closed: false,
        }
    }

    pub fn point(&self, parameter: f64) -> Point2 {
        self.curve.point(parameter)
    }

    pub fn bounds(&self) -> Aabb2 {
        self.curve.bounding_box(self.range)
    }

    pub fn is_line(&self) -> bool {
        matches!(self.curve, Curve2::Line(_))
    }

    pub fn is_spline(&self) -> bool {
        matches!(self.curve, Curve2::BSpline(_))
    }

    pub fn longer_between(&self, from: f64, to: f64, bound: f64) -> bool {
        Interval::new(from.min(to), from.max(to))
            .is_some_and(|range| self.curve.is_longer_than(range, bound))
    }

    pub fn period(&self) -> Option<f64> {
        self.closed.then(|| self.range.length())
    }

    pub fn wrapped(&self, parameter: f64) -> f64 {
        match self.period() {
            Some(period) => {
                self.range.start() + (parameter - self.range.start()).rem_euclid(period)
            }
            None => self.range.clamp(parameter),
        }
    }

    pub fn monotone_segments(&self) -> Vec<Interval> {
        let mut breaks = match &self.curve {
            Curve2::Line(_) => Vec::new(),
            Curve2::Circle(circle) => circle_breaks(circle, self.range),
            Curve2::BSpline(_) => spline_breaks(&self.curve, self.range),
        };
        breaks.retain(|parameter| *parameter > self.range.start() && *parameter < self.range.end());
        breaks.sort_by(f64::total_cmp);
        breaks.dedup();
        let mut segments = Vec::with_capacity(breaks.len() + 1);
        let mut start = self.range.start();
        for end in breaks.into_iter().chain(std::iter::once(self.range.end())) {
            if let Some(segment) = Interval::new(start, end)
                && segment.length() > 0.0
            {
                segments.push(segment);
                start = end;
            }
        }
        if segments.is_empty() {
            segments.push(self.range);
        }
        segments
    }
}

fn circle_breaks(circle: &Circle2, range: Interval) -> Vec<f64> {
    let (x, y) = (circle.x_axis(), circle.y_axis());
    let bases = [y.x.atan2(x.x), y.y.atan2(x.y)];
    let mut breaks = Vec::new();
    for base in bases {
        let first = ((range.start() - base) / PI).ceil();
        for step in 0..MAX_QUARTER_BREAKS {
            let parameter = base + (first + step as f64) * PI;
            if parameter >= range.end() {
                break;
            }
            breaks.push(parameter);
        }
    }
    breaks
}

fn spline_breaks(curve: &Curve2, range: Interval) -> Vec<f64> {
    let samples = refined_seeds(curve, range, DERIVATIVE_REFINEMENT);
    let component = |parameter: f64, axis: usize| {
        let first = curve.evaluate(parameter).first;
        if axis == 0 { first.x } else { first.y }
    };
    let mut breaks = Vec::new();
    for axis in 0..2 {
        for pair in samples.windows(2) {
            let [low, high] = pair else {
                continue;
            };
            let (at_low, at_high) = (component(*low, axis), component(*high, axis));
            if at_low == 0.0 {
                breaks.push(*low);
            } else if at_low * at_high < 0.0 {
                let (mut below, mut above) = (*low, *high);
                for _ in 0..BISECTION_STEPS {
                    let middle = 0.5 * (below + above);
                    if component(middle, axis) * at_low > 0.0 {
                        below = middle;
                    } else {
                        above = middle;
                    }
                }
                breaks.push(0.5 * (below + above));
            }
        }
    }
    breaks
}
