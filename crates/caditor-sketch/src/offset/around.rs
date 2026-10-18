use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve::{BSpline, EllipseGeometry},
    entity::{Entity, SplineKind},
    id::EntityId,
    intersect,
    offset::{Chain, OffsetError, Side},
    sketch::{Sketch, SketchError},
};

const FIRST_FIT_POINTS: usize = 16;
const FIT_POINTS_PER_CONTROL_POINT: usize = 4;
const MIN_FIT_POINTS: usize = 4;
const MAX_FIT_POINTS: usize = 512;
const FIT_TOLERANCE: f64 = 1e-5;
const CHECKS_PER_SPAN: usize = 8;
const CURVATURE_SAMPLES: usize = 720;
const COLLAPSE_SHARE: f64 = 1e-6;
const CROSSING_SEGMENT_ANGLE: f64 = PI / 90.0;

#[derive(Debug, Clone, PartialEq)]
enum Free {
    Ellipse(EllipseGeometry),
    Spline { curve: BSpline, closed: bool },
}

impl Free {
    fn closed(&self) -> bool {
        match self {
            Self::Ellipse(shape) => shape.is_full(),
            Self::Spline { closed, .. } => *closed,
        }
    }

    fn ends(&self) -> Vec<Point2> {
        if self.closed() {
            return Vec::new();
        }
        vec![self.point_at(0.0), self.point_at(1.0)]
    }

    fn point_at(&self, share: f64) -> Point2 {
        match self {
            Self::Ellipse(shape) => shape.point_at(shape.start + shape.sweep * share),
            Self::Spline { curve, .. } => curve.point_at(share),
        }
    }

    fn tangent_at(&self, share: f64) -> Vector2 {
        match self {
            Self::Ellipse(shape) => shape.tangent_at(shape.start + shape.sweep * share),
            Self::Spline { curve, .. } => curve.derivatives(share)[0],
        }
    }

    fn signed_curvature_at(&self, share: f64) -> f64 {
        let (tangent, bend) = match self {
            Self::Ellipse(shape) => {
                let parameter = shape.start + shape.sweep * share;
                (
                    shape.tangent_at(parameter),
                    shape.center - shape.point_at(parameter),
                )
            }
            Self::Spline { curve, .. } => {
                let [tangent, bend] = curve.derivatives(share);
                (tangent, bend)
            }
        };
        let speed = tangent.length();
        if speed <= f64::MIN_POSITIVE {
            return 0.0;
        }
        tangent.perp_dot(bend) / (speed * speed * speed)
    }

    fn closest_share(&self, point: Point2) -> f64 {
        match self {
            Self::Ellipse(shape) => (shape.closest_parameter(point) - shape.start) / shape.sweep,
            Self::Spline { curve, .. } => intersect::closest_spline_parameter(curve, point),
        }
    }

    fn polyline(&self, max_segment_angle: f64) -> Vec<Point2> {
        match self {
            Self::Ellipse(shape) => shape.polyline(max_segment_angle),
            Self::Spline { curve, .. } => curve.polyline(max_segment_angle),
        }
    }

    fn reach(&self) -> f64 {
        match self {
            Self::Ellipse(shape) => shape.major_radius().max(shape.minor_radius.abs()).max(1.0),
            Self::Spline { curve, .. } => {
                let (low, high) = curve.control_points().iter().fold(
                    (
                        Point2::splat(f64::INFINITY),
                        Point2::splat(f64::NEG_INFINITY),
                    ),
                    |(low, high), point| (low.min(*point), high.max(*point)),
                );
                (high - low).max_element().max(1.0)
            }
        }
    }

    fn first_fit_points(&self) -> usize {
        match self {
            Self::Ellipse(shape) => {
                let share = shape.sweep / TAU;
                ((FIRST_FIT_POINTS as f64 * share).ceil() as usize).max(MIN_FIT_POINTS)
            }
            Self::Spline { curve, .. } => (curve.control_points().len()
                * FIT_POINTS_PER_CONTROL_POINT)
                .clamp(FIRST_FIT_POINTS, MAX_FIT_POINTS),
        }
    }

    fn collapses_under(&self, by: f64) -> bool {
        (0..=CURVATURE_SAMPLES).any(|index| {
            let share = index as f64 / CURVATURE_SAMPLES as f64;
            by * self.signed_curvature_at(share) >= 1.0 - COLLAPSE_SHARE
        })
    }

    fn may_cross_itself(&self) -> bool {
        matches!(self, Self::Spline { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Around {
    pub curve: EntityId,
    shape: Free,
}

impl Around {
    pub(super) fn closed(&self) -> bool {
        self.shape.closed()
    }

    pub(super) fn ends(&self) -> Vec<Point2> {
        self.shape.ends()
    }

    pub(super) fn polyline(&self, max_segment_angle: f64) -> Vec<Point2> {
        self.shape.polyline(max_segment_angle)
    }

    pub(super) fn distance_to(&self, point: Point2) -> f64 {
        self.shape
            .point_at(self.shape.closest_share(point))
            .distance(point)
    }

    pub(super) fn left_normal_near(&self, point: Point2) -> (Point2, Vector2) {
        let at = self.shape.closest_share(point);
        (self.shape.point_at(at), self.shape.tangent_at(at).perp())
    }

    fn offset_at(&self, share: f64, by: f64) -> Point2 {
        let normal = self.shape.tangent_at(share).perp().normalize_or_zero();
        self.shape.point_at(share) + normal * by
    }

    pub(super) fn fit_points(
        &self,
        label: &str,
        side: Side,
        distance: f64,
    ) -> Result<Vec<Point2>, OffsetError> {
        if !(distance.is_finite() && distance > 0.0) {
            return Err(OffsetError::NotPositive);
        }
        let by = side.sign() * distance;
        if self.shape.collapses_under(by) {
            return Err(OffsetError::Collapses {
                entity: self.curve,
                label: label.to_owned(),
            });
        }
        let closed = self.closed();
        let mut count = self.shape.first_fit_points();
        let tolerance = FIT_TOLERANCE * self.shape.reach();
        loop {
            let points: Vec<Point2> = (0..count + usize::from(!closed))
                .map(|index| self.offset_at(index as f64 / count as f64, by))
                .collect();
            let curve = SplineKind::fit(closed).curve(&points);
            let within = curve
                .as_ref()
                .is_some_and(|curve| self.strays(curve, count, distance) <= tolerance);
            if within || count >= MAX_FIT_POINTS {
                if self.shape.may_cross_itself()
                    && curve
                        .as_ref()
                        .is_some_and(|curve| crosses_itself(curve, closed))
                {
                    return Err(OffsetError::CrossesItself);
                }
                return Ok(points);
            }
            count *= 2;
        }
    }

    fn strays(&self, curve: &BSpline, spans: usize, distance: f64) -> f64 {
        let checks = spans * CHECKS_PER_SPAN;
        (0..=checks)
            .map(|index| {
                let point = curve.point_at(index as f64 / checks as f64);
                (self.distance_to(point) - distance).abs()
            })
            .fold(0.0, f64::max)
    }
}

fn crosses_itself(curve: &BSpline, closed: bool) -> bool {
    let points = curve.polyline(CROSSING_SEGMENT_ANGLE);
    let segments: Vec<(Point2, Point2)> = points
        .iter()
        .copied()
        .zip(points.iter().copied().skip(1))
        .collect();
    let count = segments.len();
    let adjacent = |first: usize, second: usize| {
        second - first <= 1 || (closed && first == 0 && second + 1 == count)
    };
    segments.iter().enumerate().any(|(first, a)| {
        segments
            .iter()
            .enumerate()
            .skip(first + 1)
            .any(|(second, b)| !adjacent(first, second) && properly_cross(*a, *b))
    })
}

fn properly_cross((a, b): (Point2, Point2), (c, d): (Point2, Point2)) -> bool {
    let side = |from: Point2, to: Point2, point: Point2| (to - from).perp_dot(point - from);

    side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
}

impl Sketch {
    pub(super) fn around(&self, curve: EntityId) -> Option<Around> {
        let shape = match self.entity(curve)? {
            Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => {
                Free::Ellipse(self.ellipse(curve)?)
            }
            Entity::Spline { kind, .. } => Free::Spline {
                curve: self.spline(curve)?,
                closed: kind.is_closed(),
            },
            _ => return None,
        };
        Some(Around { curve, shape })
    }

    pub(super) fn add_fitted(
        &mut self,
        chain: &Chain,
        points: &[Point2],
    ) -> Result<Vec<EntityId>, SketchError> {
        let Some(around) = &chain.around else {
            return Ok(Vec::new());
        };
        let closed = around.closed();
        let source = around.curve;
        let points = points.iter().map(|point| self.add_point(*point)).collect();
        let spline = self.add_entity(Entity::Spline {
            points,
            kind: SplineKind::fit(closed),
        })?;
        if self.is_construction(source) {
            self.set_construction(spline, true)?;
        }
        Ok(vec![spline])
    }
}
