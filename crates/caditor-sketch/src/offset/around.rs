use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve::{BSpline, EllipseGeometry},
    entity::{Entity, SplineKind},
    id::EntityId,
    offset::{Chain, OffsetError, Side},
    sketch::{Sketch, SketchError},
};

const FIRST_FIT_POINTS: usize = 16;
const MIN_FIT_POINTS: usize = 4;
const MAX_FIT_POINTS: usize = 512;
const FIT_TOLERANCE: f64 = 1e-5;
const CHECKS_PER_SPAN: usize = 8;
const CURVATURE_SAMPLES: usize = 720;
const COLLAPSE_SHARE: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Around {
    pub curve: EntityId,
    pub shape: EllipseGeometry,
}

impl Around {
    pub(super) fn closed(&self) -> bool {
        self.shape.is_full()
    }

    pub(super) fn distance_to(&self, point: Point2) -> f64 {
        self.shape.closest_point(point).distance(point)
    }

    pub(super) fn left_normal_near(&self, point: Point2) -> (Point2, Vector2) {
        let at = self.shape.closest_parameter(point);
        (self.shape.point_at(at), self.shape.tangent_at(at).perp())
    }

    fn offset_at(&self, parameter: f64, by: f64) -> Point2 {
        let normal = self.shape.tangent_at(parameter).perp().normalize_or_zero();
        self.shape.point_at(parameter) + normal * by
    }

    fn reach(&self) -> f64 {
        self.shape
            .major_radius()
            .max(self.shape.minor_radius.abs())
            .max(1.0)
    }

    fn tightest_bend(&self) -> f64 {
        let (a, b) = (self.shape.major_radius(), self.shape.minor_radius.abs());
        (0..=CURVATURE_SAMPLES)
            .map(|index| {
                let parameter =
                    self.shape.start + self.shape.sweep * index as f64 / CURVATURE_SAMPLES as f64;
                let (sin, cos) = parameter.sin_cos();
                (a * a * sin * sin + b * b * cos * cos).powf(1.5) / (a * b)
            })
            .fold(f64::INFINITY, f64::min)
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
        if by > 0.0 && by >= self.tightest_bend() * (1.0 - COLLAPSE_SHARE) {
            return Err(OffsetError::Collapses {
                entity: self.curve,
                label: label.to_owned(),
            });
        }
        let closed = self.closed();
        let share = self.shape.sweep / TAU;
        let mut count = ((FIRST_FIT_POINTS as f64 * share).ceil() as usize).max(MIN_FIT_POINTS);
        let tolerance = FIT_TOLERANCE * self.reach();
        loop {
            let points: Vec<Point2> = (0..count + usize::from(!closed))
                .map(|index| {
                    let parameter =
                        self.shape.start + self.shape.sweep * index as f64 / count as f64;
                    self.offset_at(parameter, by)
                })
                .collect();
            let within = SplineKind::fit(closed)
                .curve(&points)
                .is_some_and(|curve| self.strays(&curve, count, distance) <= tolerance);
            if within || count >= MAX_FIT_POINTS {
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

impl Sketch {
    pub(super) fn around(&self, curve: EntityId) -> Option<Around> {
        match self.entity(curve)? {
            Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => Some(Around {
                curve,
                shape: self.ellipse(curve)?,
            }),
            _ => None,
        }
    }

    pub(super) fn add_fitted(
        &mut self,
        chain: &Chain,
        points: &[Point2],
    ) -> Result<Vec<EntityId>, SketchError> {
        let Some(around) = chain.around else {
            return Ok(Vec::new());
        };
        let points = points.iter().map(|point| self.add_point(*point)).collect();
        let spline = self.add_entity(Entity::Spline {
            points,
            kind: SplineKind::fit(around.closed()),
        })?;
        if self.is_construction(around.curve) {
            self.set_construction(spline, true)?;
        }
        Ok(vec![spline])
    }
}
