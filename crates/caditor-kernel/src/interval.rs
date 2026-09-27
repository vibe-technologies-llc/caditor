use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    start: f64,
    end: f64,
}

impl Interval {
    pub const FULL_TURN: Self = Self::constant(0.0, TAU);
    pub const UNIT: Self = Self::constant(0.0, 1.0);

    pub(crate) const fn constant(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn new(start: f64, end: f64) -> Option<Self> {
        (start.is_finite() && end.is_finite() && start <= end).then_some(Self { start, end })
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn end(&self) -> f64 {
        self.end
    }

    pub fn length(&self) -> f64 {
        self.end - self.start
    }

    pub fn middle(&self) -> f64 {
        self.start + 0.5 * self.length()
    }

    pub fn at(&self, fraction: f64) -> f64 {
        if fraction >= 1.0 {
            self.end
        } else {
            self.start + (self.end - self.start) * fraction
        }
    }

    pub fn contains(&self, parameter: f64) -> bool {
        parameter >= self.start && parameter <= self.end
    }

    pub fn clamp(&self, parameter: f64) -> f64 {
        if parameter.is_nan() {
            self.start
        } else {
            parameter.clamp(self.start, self.end)
        }
    }

    #[must_use]
    pub fn mirrored(&self, pivot: f64) -> Self {
        Self {
            start: pivot - self.end,
            end: pivot - self.start,
        }
    }

    pub fn split(&self, pieces: usize) -> impl Iterator<Item = f64> + '_ {
        let pieces = pieces.max(1);
        (0..=pieces).map(move |index| self.at(index as f64 / pieces as f64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Domain {
    start: f64,
    end: f64,
}

impl Domain {
    pub const UNBOUNDED: Self = Self {
        start: f64::NEG_INFINITY,
        end: f64::INFINITY,
    };

    pub fn new(start: f64, end: f64) -> Option<Self> {
        (!start.is_nan()
            && !end.is_nan()
            && start <= end
            && start < f64::INFINITY
            && end > f64::NEG_INFINITY)
            .then_some(Self { start, end })
    }

    pub fn start(&self) -> f64 {
        self.start
    }

    pub fn end(&self) -> f64 {
        self.end
    }

    pub fn bounded(&self) -> Option<Interval> {
        Interval::new(self.start, self.end)
    }

    pub fn contains(&self, parameter: f64) -> bool {
        parameter >= self.start && parameter <= self.end
    }

    pub fn clamp(&self, parameter: f64) -> f64 {
        if parameter.is_nan() {
            if self.start.is_finite() {
                self.start
            } else if self.end.is_finite() {
                self.end
            } else {
                0.0
            }
        } else {
            parameter.clamp(self.start, self.end)
        }
    }

    pub fn clipped(&self, reach: f64) -> Interval {
        let start = if self.start.is_finite() {
            self.start
        } else if self.end.is_finite() {
            self.end - 2.0 * reach
        } else {
            -reach
        };
        let end = if self.end.is_finite() {
            self.end
        } else {
            start.max(0.0) + reach
        };
        Interval {
            start,
            end: end.max(start),
        }
    }
}

impl From<Interval> for Domain {
    fn from(interval: Interval) -> Self {
        Self {
            start: interval.start,
            end: interval.end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_are_finite_and_ordered() {
        assert!(Interval::new(1.0, 0.0).is_none());
        assert!(Interval::new(0.0, f64::INFINITY).is_none());
        assert!(Interval::new(f64::NAN, 1.0).is_none());
        let interval = Interval::new(2.0, 6.0).unwrap();
        assert_eq!(interval.length(), 4.0);
        assert_eq!(interval.middle(), 4.0);
        assert_eq!(interval.at(1.0), 6.0);
        assert_eq!(interval.clamp(f64::NAN), 2.0);
        assert_eq!(interval.clamp(9.0), 6.0);
        assert_eq!(interval.mirrored(0.0), Interval::new(-6.0, -2.0).unwrap());
        let points: Vec<f64> = interval.split(4).collect();
        assert_eq!(points, vec![2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn domains_may_be_unbounded_and_clip_to_a_finite_interval() {
        assert!(Domain::UNBOUNDED.bounded().is_none());
        assert_eq!(Domain::UNBOUNDED.clamp(f64::NAN), 0.0);
        assert!(Domain::new(f64::INFINITY, f64::INFINITY).is_none());
        let half = Domain::new(1.0, f64::INFINITY).unwrap();
        assert_eq!(half.clamp(-5.0), 1.0);
        assert_eq!(half.clipped(2.0), Interval::new(1.0, 3.0).unwrap());
        let lower = Domain::new(f64::NEG_INFINITY, -1.0).unwrap();
        assert_eq!(lower.clipped(2.0), Interval::new(-5.0, -1.0).unwrap());
        assert_eq!(
            Domain::UNBOUNDED.clipped(1.0),
            Interval::new(-1.0, 1.0).unwrap()
        );
    }
}
