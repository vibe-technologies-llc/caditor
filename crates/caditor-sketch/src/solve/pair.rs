use std::{cell::OnceCell, f64::consts::TAU};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{BSpline, EllipseGeometry},
    entity::Role,
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::{CurveHandle, Form, fallback_direction},
        spline::SplineEnd,
        system::{Joints, System},
    },
};

const SAMPLES_PER_SPAN: usize = 16;
const ELLIPSE_SAMPLES: usize = 64;
const REFINEMENTS: usize = 32;

pub(crate) enum Course {
    Spline(BSpline),
    Ellipse(EllipseGeometry),
}

impl Course {
    pub(crate) fn of(sketch: &Sketch, curve: EntityId) -> Option<Self> {
        if let Some(drawn) = sketch.ellipse(curve) {
            return Some(Self::Ellipse(EllipseGeometry::full(
                drawn.center,
                drawn.center + drawn.major,
                drawn.minor_radius,
            )));
        }
        sketch.spline(curve).map(Self::Spline)
    }

    fn at(&self, parameter: f64) -> (Point2, Vector2, Vector2) {
        match self {
            Self::Spline(curve) => {
                let [tangent, bend] = curve.derivatives(parameter);
                (curve.point_at(parameter), tangent, bend)
            }
            Self::Ellipse(shape) => {
                let angle = TAU * parameter;
                let point = shape.point_at(angle);
                (
                    point,
                    shape.tangent_at(angle) * TAU,
                    (shape.center - point) * (TAU * TAU),
                )
            }
        }
    }

    pub(crate) fn point_at(&self, parameter: f64) -> Point2 {
        self.at(parameter).0
    }

    fn tangent_at(&self, parameter: f64) -> Vector2 {
        self.at(parameter).1
    }

    fn settle(&self, parameter: f64) -> f64 {
        match self {
            Self::Spline(_) => parameter.clamp(0.0, 1.0),
            Self::Ellipse(_) => parameter.rem_euclid(1.0),
        }
    }

    fn samples(&self) -> Vec<(f64, Point2)> {
        let (count, closing) = match self {
            Self::Spline(curve) => (
                curve
                    .control_points()
                    .len()
                    .saturating_sub(curve.degree())
                    .max(1)
                    * SAMPLES_PER_SPAN,
                1,
            ),
            Self::Ellipse(_) => (ELLIPSE_SAMPLES, 0),
        };
        (0..count + closing)
            .map(|index| {
                let parameter = index as f64 / count as f64;
                (parameter, self.point_at(parameter))
            })
            .collect()
    }
}

pub(crate) fn closest_pair(first: &Course, second: &Course) -> (f64, f64) {
    let (ones, others) = (first.samples(), second.samples());
    let rough = ones
        .iter()
        .flat_map(|one| others.iter().map(move |other| (*one, *other)))
        .min_by(|a, b| {
            a.0.1
                .distance_squared(a.1.1)
                .total_cmp(&b.0.1.distance_squared(b.1.1))
        })
        .map_or((0.0, 0.0), |((one, _), (other, _))| (one, other));
    let (mut one, mut other) = rough;
    for _ in 0..REFINEMENTS {
        let (at, along, bend) = first.at(one);
        let (there, other_along, other_bend) = second.at(other);
        let gap = at - there;
        let gradient = (gap.dot(along), -gap.dot(other_along));
        let across = -along.dot(other_along);
        let hessian = (
            along.dot(along) + gap.dot(bend),
            across,
            other_along.dot(other_along) - gap.dot(other_bend),
        );
        let determinant = hessian.0 * hessian.2 - hessian.1 * hessian.1;
        if !(determinant.is_finite() && determinant.abs() > f64::EPSILON) {
            break;
        }
        let step = (
            (hessian.2 * gradient.0 - hessian.1 * gradient.1) / determinant,
            (hessian.0 * gradient.1 - hessian.1 * gradient.0) / determinant,
        );
        let next = (first.settle(one - step.0), second.settle(other - step.1));
        let moved = step.0.abs() + step.1.abs();
        (one, other) = next;
        if moved <= f64::EPSILON {
            break;
        }
    }
    let distance = |(one, other): (f64, f64)| first.point_at(one).distance(second.point_at(other));
    if distance((one, other)) <= distance(rough) {
        (one, other)
    } else {
        rough
    }
}

pub(crate) fn curve_pair_gap(
    sketch: &Sketch,
    first: EntityId,
    second: EntityId,
) -> Option<(Point2, Point2)> {
    let (one, other) = (Course::of(sketch, first)?, Course::of(sketch, second)?);
    let (at, there) = closest_pair(&one, &other);
    Some((one.point_at(at), other.point_at(there)))
}

pub(super) enum PairJoint {
    Point(EntityId),
    SplineEnd {
        spline: EntityId,
        end: SplineEnd,
        ellipse: EntityId,
    },
}

impl Joints {
    pub(super) fn pair_joint(
        &self,
        sketch: &Sketch,
        a: EntityId,
        b: EntityId,
    ) -> Option<PairJoint> {
        match (sketch.role(a)?, sketch.role(b)?) {
            (Role::Elliptic, Role::Elliptic) => self.joint(sketch, a, b).map(PairJoint::Point),
            (Role::Spline, Role::Elliptic) => {
                self.spline_end_on(sketch, a, b)
                    .map(|end| PairJoint::SplineEnd {
                        spline: a,
                        end,
                        ellipse: b,
                    })
            }
            (Role::Elliptic, Role::Spline) => {
                self.spline_end_on(sketch, b, a)
                    .map(|end| PairJoint::SplineEnd {
                        spline: b,
                        end,
                        ellipse: a,
                    })
            }
            _ => None,
        }
    }
}

pub(crate) fn is_elliptic_pair(sketch: &Sketch, a: EntityId, b: EntityId) -> bool {
    matches!(
        (sketch.role(a), sketch.role(b)),
        (Some(Role::Elliptic), Some(Role::Elliptic | Role::Spline))
            | (Some(Role::Spline), Some(Role::Elliptic))
    )
}

impl System {
    pub(super) fn pair_parameter_start(
        &self,
        sketch: &Sketch,
        joints: &OnceCell<Joints>,
        constraint: &Constraint,
    ) -> Result<Vec<(EntityId, f64)>, SketchError> {
        let (a, b) = match *constraint {
            Constraint::Distance { from, to, .. } => (from, to),
            Constraint::Tangent(a, b) => {
                let joints = joints.get_or_init(|| Joints::of(sketch));
                if joints.pair_joint(sketch, a, b).is_some() {
                    return Ok(Vec::new());
                }
                (a, b)
            }
            _ => return Ok(Vec::new()),
        };
        if !is_elliptic_pair(sketch, a, b) {
            return Ok(Vec::new());
        }
        let (one, other) = closest_pair(&self.course(sketch, a)?, &self.course(sketch, b)?);
        Ok(vec![(a, one), (b, other)])
    }

    pub(super) fn course(&self, sketch: &Sketch, curve: EntityId) -> Result<Course, SketchError> {
        if sketch.role(curve) == Some(Role::Elliptic) {
            Ok(Course::Ellipse(self.ellipse_shape(sketch, curve)?))
        } else {
            Ok(Course::Spline(self.curve(sketch, curve)?))
        }
    }

    fn curve_handle(&self, sketch: &Sketch, curve: EntityId) -> Result<CurveHandle, SketchError> {
        if sketch.role(curve) == Some(Role::Elliptic) {
            Ok(CurveHandle::Ellipse(self.ellipse(sketch, curve)?))
        } else {
            Ok(CurveHandle::Spline(self.spline_handle(sketch, curve)?))
        }
    }

    fn start_tangent(
        &self,
        sketch: &Sketch,
        curve: EntityId,
        parameter: usize,
    ) -> Result<Vector2, SketchError> {
        let start = self.values.get(parameter).copied().unwrap_or(0.0);
        Ok(fallback_direction(
            self.course(sketch, curve)?.tangent_at(start),
        ))
    }

    pub(super) fn curves_touch(
        &self,
        sketch: &Sketch,
        (a, b): (EntityId, EntityId),
        parameters: (usize, usize),
    ) -> Result<Vec<Form>, SketchError> {
        let (first, second) = (self.curve_handle(sketch, a)?, self.curve_handle(sketch, b)?);
        let fallbacks = (
            self.start_tangent(sketch, a, parameters.0)?,
            self.start_tangent(sketch, b, parameters.1)?,
        );
        let meet = |along| Form::CurvesMeet {
            first: first.clone(),
            second: second.clone(),
            parameters,
            along,
        };
        Ok(vec![
            meet(Vector2::X),
            meet(Vector2::Y),
            Form::CurvesAlong {
                first: first.clone(),
                second: second.clone(),
                parameters,
                fallbacks,
            },
        ])
    }

    pub(super) fn curves_gap(
        &self,
        sketch: &Sketch,
        (a, b): (EntityId, EntityId),
        parameters: (usize, usize),
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let (first, second) = (self.curve_handle(sketch, a)?, self.curve_handle(sketch, b)?);
        let fallbacks = (
            self.start_tangent(sketch, a, parameters.0)?,
            self.start_tangent(sketch, b, parameters.1)?,
        );
        let start = |parameter: usize| self.values.get(parameter).copied().unwrap_or(0.0);
        let drawn = self.course(sketch, b)?.point_at(start(parameters.1))
            - self.course(sketch, a)?.point_at(start(parameters.0));
        let side = if fallbacks.0.perp().dot(drawn) < 0.0 {
            -1.0
        } else {
            1.0
        };
        Ok(vec![
            Form::CurvesFoot {
                first: first.clone(),
                second: second.clone(),
                parameters,
                fallback: fallbacks.0,
            },
            Form::CurvesGap {
                first: first.clone(),
                second: second.clone(),
                parameters,
                fallback: fallbacks.0,
                side,
                value,
            },
            Form::CurvesAlong {
                first,
                second,
                parameters,
                fallbacks,
            },
        ])
    }

    pub(super) fn curve_pair_tangent(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        id: ConstraintId,
        (a, b): (EntityId, EntityId),
    ) -> Result<Vec<Form>, SketchError> {
        match joints.pair_joint(sketch, a, b) {
            Some(PairJoint::Point(point)) => {
                let normal = |ellipse: EntityId| -> Result<Vector2, SketchError> {
                    let shape = self.ellipse_shape(sketch, ellipse)?;
                    let at = self.point(point)?.at(&self.values);
                    Ok(fallback_direction(
                        shape.tangent_at(shape.parameter_of(at)).perp(),
                    ))
                };
                Ok(vec![Form::EllipsesTouch {
                    point: self.point(point)?,
                    first: self.ellipse(sketch, a)?,
                    second: self.ellipse(sketch, b)?,
                    fallbacks: (normal(a)?, normal(b)?),
                }])
            }
            Some(PairJoint::SplineEnd {
                spline,
                end,
                ellipse,
            }) => {
                let line = self.leg(sketch, spline, end)?;
                Ok(vec![Form::EllipseTouch {
                    line,
                    point: line.start,
                    ellipse: self.ellipse(sketch, ellipse)?,
                }])
            }
            None => self.curves_touch(sketch, (a, b), self.parameter_pair(id)?),
        }
    }
}
