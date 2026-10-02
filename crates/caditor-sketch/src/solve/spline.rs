use std::{cell::OnceCell, sync::Arc};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{BSpline, clamped_knots},
    entity::{Entity, Role},
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::{CircleHandle, Form, LineHandle, SplineHandle, fallback_direction},
        system::{Joints, System},
    },
};

const SAMPLES_PER_SPAN: usize = 16;
const REFINEMENTS: usize = 32;
const TIE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    Closest,
    Touching,
}

impl System {
    pub(super) fn parameter_start(
        &self,
        sketch: &Sketch,
        joints: &OnceCell<Joints>,
        constraint: &Constraint,
    ) -> Result<Option<f64>, SketchError> {
        let role = |entity: EntityId| sketch.role(entity);
        match *constraint {
            Constraint::Coincident(a, b) => {
                let (point, spline) = match (role(a), role(b)) {
                    (Some(Role::Point), Some(Role::Spline)) => (a, b),
                    (Some(Role::Spline), Some(Role::Point)) => (b, a),
                    _ => return Ok(None),
                };
                let target = self.point(point)?.at(&self.values);
                Ok(Some(start_on(
                    &self.curve(sketch, spline)?,
                    Wanted::Closest,
                    |at| at.distance(target),
                    |at, tangent, bend| {
                        let offset = at - target;
                        (offset.dot(tangent), tangent.dot(tangent) + offset.dot(bend))
                    },
                )))
            }
            Constraint::Tangent(a, b) => {
                let (spline, other) = match (role(a), role(b)) {
                    (Some(Role::Spline), Some(Role::Line | Role::Circular)) => (a, b),
                    (Some(Role::Line | Role::Circular), Some(Role::Spline)) => (b, a),
                    _ => return Ok(None),
                };
                let joints = joints.get_or_init(|| Joints::of(sketch));
                if joints.spline_end(sketch, spline, other).is_some() {
                    return Ok(None);
                }
                self.touching_start(sketch, spline, other).map(Some)
            }
            _ => Ok(None),
        }
    }

    fn touching_start(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        other: EntityId,
    ) -> Result<f64, SketchError> {
        let curve = self.curve(sketch, spline)?;
        let values = &self.values;
        if sketch.role(other) == Some(Role::Line) {
            let line = self.line(sketch, other)?;
            let anchor = line.start.at(values);
            let normal = fallback_direction(line.end.at(values) - anchor).perp();
            return Ok(start_on(
                &curve,
                Wanted::Touching,
                |at| normal.dot(at - anchor),
                |_, tangent, bend| (normal.dot(tangent), normal.dot(bend)),
            ));
        }
        let circle = self.circle(sketch, other)?;
        let (center, radius) = (circle.center.at(values), circle.radius(values));
        Ok(start_on(
            &curve,
            Wanted::Touching,
            |at| at.distance(center) - radius,
            |at, tangent, bend| {
                let offset = at - center;
                (offset.dot(tangent), tangent.dot(tangent) + offset.dot(bend))
            },
        ))
    }

    pub(super) fn on_spline(
        &self,
        sketch: &Sketch,
        point: EntityId,
        spline: EntityId,
        parameter: usize,
    ) -> Result<Vec<Form>, SketchError> {
        let (point, spline) = (self.point(point)?, self.spline_handle(sketch, spline)?);
        Ok([Vector2::X, Vector2::Y]
            .into_iter()
            .map(|along| Form::OnSpline {
                point,
                spline: Arc::clone(&spline),
                parameter,
                along,
            })
            .collect())
    }

    pub(super) fn spline_tangent(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        (spline, other): (EntityId, EntityId),
        parameter: Option<usize>,
    ) -> Result<Vec<Form>, SketchError> {
        let is_line = sketch.role(other) == Some(Role::Line);
        if let Some((end, neighbour)) = joints.spline_end(sketch, spline, other) {
            let (end, neighbour) = (self.point(end)?, self.point(neighbour)?);
            let leg = LineHandle {
                start: end,
                end: neighbour,
                fallback: self.initial_direction(end, neighbour),
            };
            return Ok(vec![if is_line {
                Form::Parallel(self.line(sketch, other)?, leg)
            } else {
                let circle = self.circle(sketch, other)?;
                Form::Perpendicular(self.radius_line(end, circle.center), leg)
            }]);
        }
        let parameter = parameter.ok_or(SketchError::MissingEntity(spline))?;
        let handle = self.spline_handle(sketch, spline)?;
        let curve = self.curve(sketch, spline)?;
        let start = self.values.get(parameter).copied().unwrap_or(0.0);
        let [tangent, _] = curve.derivatives(start);
        let tangent_fallback = fallback_direction(tangent);
        if is_line {
            let line = self.line(sketch, other)?;
            return Ok(vec![
                Form::SplineOnLine {
                    spline: Arc::clone(&handle),
                    parameter,
                    line,
                },
                Form::SplineAlongLine {
                    spline: handle,
                    parameter,
                    line,
                    fallback: tangent_fallback,
                },
            ]);
        }
        let circle: CircleHandle = self.circle(sketch, other)?;
        let radial_fallback =
            fallback_direction(curve.point_at(start) - circle.center.at(&self.values));
        Ok(vec![
            Form::SplineOnCircle {
                spline: Arc::clone(&handle),
                parameter,
                circle,
                fallback: radial_fallback,
            },
            Form::SplineAcrossRadius {
                spline: handle,
                parameter,
                circle,
                fallbacks: (radial_fallback, tangent_fallback),
            },
        ])
    }

    pub(super) fn parameter_of(&self, constraint: ConstraintId) -> Result<usize, SketchError> {
        self.parameters
            .get(&constraint)
            .copied()
            .ok_or(SketchError::MissingConstraint(constraint))
    }

    fn spline_handle(
        &self,
        sketch: &Sketch,
        spline: EntityId,
    ) -> Result<Arc<SplineHandle>, SketchError> {
        let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
            return Err(SketchError::MissingEntity(spline));
        };
        let points = control_points
            .iter()
            .map(|point| self.point(*point))
            .collect::<Result<Vec<_>, _>>()?;
        let (degree, knots) = clamped_knots(points.len());
        Ok(Arc::new(SplineHandle {
            points,
            degree,
            knots,
        }))
    }

    fn curve(&self, sketch: &Sketch, spline: EntityId) -> Result<BSpline, SketchError> {
        let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
            return Err(SketchError::MissingEntity(spline));
        };
        let points = control_points
            .iter()
            .map(|point| Ok(self.point(*point)?.at(&self.values)))
            .collect::<Result<Vec<Point2>, SketchError>>()?;
        BSpline::clamped(points).ok_or(SketchError::TooFewControlPoints)
    }
}

impl Joints {
    pub(super) fn spline_end(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        other: EntityId,
    ) -> Option<(EntityId, EntityId)> {
        let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
            return None;
        };
        let on_other: Vec<EntityId> = self
            .points_on(sketch, other)
            .into_iter()
            .map(|point| self.class(point))
            .collect();
        let last = control_points.len().checked_sub(1)?;
        [(0, 1), (last, last.checked_sub(1)?)]
            .into_iter()
            .filter_map(|(end, neighbour)| {
                Some((*control_points.get(end)?, *control_points.get(neighbour)?))
            })
            .find(|(end, _)| on_other.contains(&self.class(*end)))
    }
}

fn start_on(
    curve: &BSpline,
    wanted: Wanted,
    signed: impl Fn(Point2) -> f64,
    slope: impl Fn(Point2, Vector2, Vector2) -> (f64, f64),
) -> f64 {
    let spans = curve
        .control_points()
        .len()
        .saturating_sub(curve.degree())
        .max(1);
    let count = spans * SAMPLES_PER_SPAN;
    let samples: Vec<(f64, f64)> = (0..=count)
        .map(|index| {
            let parameter = index as f64 / count as f64;
            (parameter, signed(curve.point_at(parameter)))
        })
        .collect();
    let size = polygon_size(curve);
    let candidates: Vec<Candidate> = samples
        .iter()
        .enumerate()
        .filter(|(index, (_, at))| {
            let before = index
                .checked_sub(1)
                .and_then(|previous| samples.get(previous));
            let after = samples.get(index + 1);
            is_extremum(
                before.map(|sample| sample.1),
                *at,
                after.map(|sample| sample.1),
            )
        })
        .flat_map(|(_, (parameter, _))| [(*parameter, false), refine(curve, *parameter, &slope)])
        .map(|(parameter, stationary)| {
            let [tangent, _] = curve.derivatives(parameter);
            Candidate {
                parameter,
                stationary,
                distance: signed(curve.point_at(parameter)).abs(),
                turning: tangent.length() > TIE * size,
            }
        })
        .collect();
    let touching = wanted == Wanted::Touching && candidates.iter().any(|found| found.stationary);
    let eligible: Vec<Candidate> = candidates
        .into_iter()
        .filter(|found| !touching || found.stationary)
        .collect();
    let closest = eligible
        .iter()
        .map(|found| found.distance)
        .fold(f64::INFINITY, f64::min);
    eligible
        .into_iter()
        .filter(|found| found.distance <= closest + TIE * size)
        .min_by(|a, b| {
            b.turning
                .cmp(&a.turning)
                .then(a.distance.total_cmp(&b.distance))
        })
        .map_or(0.0, |found| found.parameter)
}

struct Candidate {
    parameter: f64,
    stationary: bool,
    distance: f64,
    turning: bool,
}

fn polygon_size(curve: &BSpline) -> f64 {
    let points = curve.control_points();
    let low = points.iter().copied().reduce(Point2::min);
    let high = points.iter().copied().reduce(Point2::max);
    low.zip(high).map_or(0.0, |(low, high)| low.distance(high))
}

fn is_extremum(before: Option<f64>, at: f64, after: Option<f64>) -> bool {
    let highest = before.is_none_or(|value| at >= value) && after.is_none_or(|value| at >= value);
    let lowest = before.is_none_or(|value| at <= value) && after.is_none_or(|value| at <= value);
    highest || lowest
}

fn refine(
    curve: &BSpline,
    start: f64,
    slope: impl Fn(Point2, Vector2, Vector2) -> (f64, f64),
) -> (f64, bool) {
    let mut parameter = start;
    for _ in 0..REFINEMENTS {
        let [tangent, bend] = curve.derivatives(parameter);
        let (value, derivative) = slope(curve.point_at(parameter), tangent, bend);
        let step = value / derivative;
        if !step.is_finite() {
            break;
        }
        let next = (parameter - step).clamp(0.0, 1.0);
        let moved = (next - parameter).abs();
        parameter = next;
        if moved <= f64::EPSILON {
            return (parameter, parameter > 0.0 && parameter < 1.0);
        }
    }
    (parameter, false)
}
