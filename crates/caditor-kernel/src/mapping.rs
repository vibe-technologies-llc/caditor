use std::f64::consts::TAU;

use caditor_geometry::Point2;

use crate::interval::Interval;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine {
    scale: f64,
    offset: f64,
}

impl Affine {
    pub(crate) const IDENTITY: Self = Self {
        scale: 1.0,
        offset: 0.0,
    };
    pub(crate) const TURNED_BACK: Self = Self {
        scale: -1.0,
        offset: TAU,
    };

    pub(crate) fn scaling(scale: f64) -> Self {
        Self { scale, offset: 0.0 }
    }

    pub(crate) fn apply(&self, value: f64) -> f64 {
        value * self.scale + self.offset
    }

    pub(crate) fn interval(&self, interval: Interval) -> Option<Interval> {
        let (start, end) = (self.apply(interval.start()), self.apply(interval.end()));
        Interval::new(start.min(end), start.max(end))
    }

    fn keeps_order(&self) -> bool {
        self.scale > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct UvMap {
    pub(crate) u: Affine,
    pub(crate) v: Affine,
}

impl UvMap {
    pub(crate) const IDENTITY: Self = Self {
        u: Affine::IDENTITY,
        v: Affine::IDENTITY,
    };

    pub(crate) fn apply(&self, uv: Point2) -> Point2 {
        Point2::new(self.u.apply(uv.x), self.v.apply(uv.y))
    }

    pub(crate) fn keeps_orientation(&self) -> bool {
        self.u.keeps_order() == self.v.keeps_order()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_back_angle_stays_within_the_turn() {
        let interval = Interval::new(0.5, 2.0).unwrap();
        let mapped = Affine::TURNED_BACK.interval(interval).unwrap();

        assert!((mapped.start() - (TAU - 2.0)).abs() < 1e-15);
        assert!((mapped.end() - (TAU - 0.5)).abs() < 1e-15);
        assert!(!Affine::TURNED_BACK.keeps_order());
    }

    #[test]
    fn orientation_follows_the_sign_of_the_jacobian() {
        let stretched = UvMap {
            u: Affine::scaling(2.0),
            v: Affine::scaling(3.0),
        };
        let mirrored = UvMap {
            u: Affine::TURNED_BACK,
            v: Affine::scaling(3.0),
        };
        let both = UvMap {
            u: Affine::TURNED_BACK,
            v: Affine::scaling(-1.0),
        };

        assert!(stretched.keeps_orientation());
        assert!(!mirrored.keeps_orientation());
        assert!(both.keeps_orientation());
        assert_eq!(
            mirrored.apply(Point2::new(1.0, 2.0)),
            Point2::new(TAU - 1.0, 6.0)
        );
    }
}
