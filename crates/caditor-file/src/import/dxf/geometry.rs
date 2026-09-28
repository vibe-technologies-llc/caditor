use std::f64::consts::TAU;

use caditor_geometry::{Point2, Point3, Vector2, Vector3};

const ARBITRARY_AXIS_LIMIT: f64 = 1.0 / 64.0;
const FULL_TURN_TOLERANCE: f64 = 1e-9;
const CIRCULAR_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Affine {
    x: Vector3,
    y: Vector3,
    z: Vector3,
    origin: Point3,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        x: Vector3::X,
        y: Vector3::Y,
        z: Vector3::Z,
        origin: Point3::ZERO,
    };

    pub fn object_system(normal: Vector3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        let helper =
            if normal.x.abs() < ARBITRARY_AXIS_LIMIT && normal.y.abs() < ARBITRARY_AXIS_LIMIT {
                Vector3::Y
            } else {
                Vector3::Z
            };
        let x = helper.cross(normal).try_normalize()?;
        let y = normal.cross(x).try_normalize()?;
        Some(Self {
            x,
            y,
            z: normal,
            origin: Point3::ZERO,
        })
    }

    pub fn translation(offset: Vector3) -> Self {
        Self {
            origin: offset,
            ..Self::IDENTITY
        }
    }

    pub fn scale(factors: Vector3) -> Self {
        Self {
            x: Vector3::X * factors.x,
            y: Vector3::Y * factors.y,
            z: Vector3::Z * factors.z,
            origin: Point3::ZERO,
        }
    }

    pub fn rotation_z(angle: f64) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            x: Vector3::new(cos, sin, 0.0),
            y: Vector3::new(-sin, cos, 0.0),
            z: Vector3::Z,
            origin: Point3::ZERO,
        }
    }

    pub fn point(&self, point: Point3) -> Point3 {
        self.origin + self.vector(point)
    }

    pub fn vector(&self, vector: Vector3) -> Vector3 {
        self.x * vector.x + self.y * vector.y + self.z * vector.z
    }

    pub fn then(&self, outer: &Self) -> Self {
        Self {
            x: outer.vector(self.x),
            y: outer.vector(self.y),
            z: outer.vector(self.z),
            origin: outer.point(self.origin),
        }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.origin.is_finite()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Nurbs {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub points: Vec<Point3>,
    pub weights: Option<Vec<f64>>,
}

impl Nurbs {
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        points: Vec<Point3>,
        weights: Option<Vec<f64>>,
    ) -> Option<Self> {
        let weights = weights.filter(|weights| weights.len() == points.len());
        let valid = degree >= 1
            && points.len() > degree
            && knots.len() == points.len() + degree + 1
            && knots.iter().all(|knot| knot.is_finite())
            && knots
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a <= b))
            && points.iter().all(|point| point.is_finite())
            && weights
                .as_ref()
                .is_none_or(|weights| weights.iter().all(|w| w.is_finite() && *w > 0.0));
        let nurbs = Self {
            degree,
            knots,
            points,
            weights: weights.filter(|weights| weights.iter().any(|weight| *weight != 1.0)),
        };
        (valid && nurbs.domain().is_some()).then_some(nurbs)
    }

    pub fn domain(&self) -> Option<(f64, f64)> {
        let start = *self.knots.get(self.degree)?;
        let end = *self.knots.get(self.points.len())?;
        (end > start).then_some((start, end))
    }

    pub fn spans(&self) -> usize {
        let Some((start, end)) = self.domain() else {
            return 0;
        };
        let mut breaks: Vec<f64> = self
            .knots
            .iter()
            .copied()
            .filter(|knot| *knot >= start && *knot <= end)
            .collect();
        breaks.dedup();
        breaks.len().saturating_sub(1).max(1)
    }

    pub fn point(&self, parameter: f64) -> Option<Point3> {
        let (start, end) = self.domain()?;
        let parameter = parameter.clamp(start, end);
        let degree = self.degree;
        let span = (degree..self.points.len())
            .rev()
            .find(
                |span| match (self.knots.get(*span), self.knots.get(span + 1)) {
                    (Some(low), Some(high)) => *low <= parameter && low < high,
                    _ => false,
                },
            )
            .unwrap_or(degree);
        let first = span.checked_sub(degree)?;
        let weight = |index: usize| {
            self.weights
                .as_ref()
                .and_then(|weights| weights.get(index))
                .copied()
                .unwrap_or(1.0)
        };
        let mut local: Vec<(Vector3, f64)> = (first..=span)
            .map(|index| {
                let w = weight(index);
                self.points.get(index).map(|point| (*point * w, w))
            })
            .collect::<Option<_>>()?;
        for level in 1..=degree {
            for index in (level..=degree).rev() {
                let low = *self.knots.get(first + index)?;
                let high = *self.knots.get(first + index + degree + 1 - level)?;
                let width = high - low;
                let alpha = if width > 0.0 {
                    (parameter - low) / width
                } else {
                    0.0
                };
                let (previous_point, previous_weight) = *local.get(index - 1)?;
                let current = local.get_mut(index)?;
                current.0 = previous_point.lerp(current.0, alpha);
                current.1 = previous_weight + (current.1 - previous_weight) * alpha;
            }
        }
        let (point, weight) = *local.get(degree)?;
        (weight > 0.0).then(|| point / weight)
    }

    pub fn transformed(&self, transform: &Affine) -> Self {
        Self {
            degree: self.degree,
            knots: self.knots.clone(),
            points: self
                .points
                .iter()
                .map(|point| transform.point(*point))
                .collect(),
            weights: self.weights.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Shape {
    Point(Point3),
    Line(Point3, Point3),
    Conic {
        center: Point3,
        major: Vector3,
        minor: Vector3,
        start: f64,
        sweep: f64,
    },
    Spline(Nurbs),
    Interpolated(Vec<Point3>),
}

impl Shape {
    pub fn transformed(&self, transform: &Affine) -> Self {
        match self {
            Self::Point(point) => Self::Point(transform.point(*point)),
            Self::Line(start, end) => Self::Line(transform.point(*start), transform.point(*end)),
            Self::Conic {
                center,
                major,
                minor,
                start,
                sweep,
            } => Self::Conic {
                center: transform.point(*center),
                major: transform.vector(*major),
                minor: transform.vector(*minor),
                start: *start,
                sweep: *sweep,
            },
            Self::Spline(nurbs) => Self::Spline(nurbs.transformed(transform)),
            Self::Interpolated(points) => {
                Self::Interpolated(points.iter().map(|point| transform.point(*point)).collect())
            }
        }
    }

    pub fn defining_points(&self) -> Vec<Point3> {
        match self {
            Self::Point(point) => vec![*point],
            Self::Line(start, end) => vec![*start, *end],
            Self::Conic {
                center,
                major,
                minor,
                ..
            } => vec![
                *center + *major,
                *center - *major,
                *center + *minor,
                *center - *minor,
            ],
            Self::Spline(nurbs) => nurbs.points.clone(),
            Self::Interpolated(points) => points.clone(),
        }
    }
}

pub(super) fn conic_arc(
    center: Point3,
    major: Vector3,
    minor: Vector3,
    start: f64,
    end: f64,
) -> Shape {
    Shape::Conic {
        center,
        major,
        minor,
        start,
        sweep: sweep_between(start, end),
    }
}

pub(super) fn sweep_between(start: f64, end: f64) -> f64 {
    let sweep = (end - start).rem_euclid(TAU);
    if sweep <= FULL_TURN_TOLERANCE || !sweep.is_finite() {
        TAU
    } else {
        sweep
    }
}

pub(super) fn is_full_turn(sweep: f64) -> bool {
    sweep >= TAU - FULL_TURN_TOLERANCE
}

pub(super) struct PlanarCircle {
    pub center: Point2,
    pub radius: f64,
    pub start_angle: f64,
    pub counter_clockwise: bool,
}

pub(super) fn planar_circle(
    center: Point3,
    major: Vector3,
    minor: Vector3,
) -> Option<PlanarCircle> {
    let (a, b) = (flat(major), flat(minor));
    let radius = a.length();
    let scale = radius.max(b.length());
    let circular = radius > 0.0
        && (a.length() - b.length()).abs() <= CIRCULAR_TOLERANCE * scale
        && a.dot(b).abs() <= CIRCULAR_TOLERANCE * scale * scale;
    circular.then(|| PlanarCircle {
        center: Point2::new(center.x, center.y),
        radius,
        start_angle: a.y.atan2(a.x),
        counter_clockwise: a.perp_dot(b) > 0.0,
    })
}

pub(super) fn flat(vector: Vector3) -> Vector2 {
    Vector2::new(vector.x, vector.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arbitrary_axis_algorithm_mirrors_a_downward_normal() {
        let up = Affine::object_system(Vector3::Z).unwrap();
        assert_eq!(
            up.point(Point3::new(1.0, 2.0, 3.0)),
            Point3::new(1.0, 2.0, 3.0)
        );
        let down = Affine::object_system(-Vector3::Z).unwrap();
        let mapped = down.point(Point3::new(1.0, 2.0, 3.0));
        assert!(mapped.distance(Point3::new(-1.0, 2.0, -3.0)) < 1e-12);
        let tilted = Affine::object_system(Vector3::X).unwrap();
        assert!(tilted.vector(Vector3::X).distance(Vector3::Y) < 1e-12);
        assert!(tilted.vector(Vector3::Y).distance(Vector3::Z) < 1e-12);
    }

    #[test]
    fn transforms_compose_inner_first() {
        let inner = Affine::translation(Vector3::X);
        let outer = Affine::rotation_z(std::f64::consts::FRAC_PI_2);
        let both = inner.then(&outer);
        assert!(both.point(Point3::ZERO).distance(Point3::Y) < 1e-12);
    }

    #[test]
    fn a_nurbs_quarter_circle_stays_on_the_circle() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let nurbs = Nurbs::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![Point3::X, Point3::new(1.0, 1.0, 0.0), Point3::Y],
            Some(vec![1.0, half, 1.0]),
        )
        .unwrap();
        for step in 0..=10 {
            let point = nurbs.point(step as f64 / 10.0).unwrap();
            assert!((point.length() - 1.0).abs() < 1e-12);
        }
        assert_eq!(nurbs.point(0.0), Some(Point3::X));
        assert!(nurbs.point(1.0).unwrap().distance(Point3::Y) < 1e-12);
    }

    #[test]
    fn an_unclamped_nurbs_runs_over_its_inner_knots() {
        let corner = Point3::new(1.0, 1.0, 0.0);
        let nurbs = Nurbs::new(
            1,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![Point3::ZERO, Point3::X, corner],
            None,
        )
        .unwrap();
        assert_eq!(nurbs.domain(), Some((1.0, 3.0)));
        assert!(nurbs.point(1.0).unwrap().distance(Point3::ZERO) < 1e-12);
        assert!(nurbs.point(2.0).unwrap().distance(Point3::X) < 1e-12);
        assert!(nurbs.point(3.0).unwrap().distance(corner) < 1e-12);
        assert!(Nurbs::new(3, vec![0.0; 4], vec![Point3::ZERO; 2], None).is_none());
    }

    #[test]
    fn a_circle_seen_from_below_turns_clockwise() {
        let circle = planar_circle(Point3::ZERO, -Vector3::X, Vector3::Y).unwrap();
        assert!(!circle.counter_clockwise);
        assert!(planar_circle(Point3::ZERO, Vector3::X * 2.0, Vector3::Y).is_none());
    }
}
