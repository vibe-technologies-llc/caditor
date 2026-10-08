use std::{cell::OnceCell, sync::Arc};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{BSpline, clamped_knots},
    entity::{Entity, Role},
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::{
            CircleHandle, Form, LineHandle, SplineEndHandle, SplineHandle, fallback_direction,
        },
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
            Constraint::Coincident(a, b) | Constraint::Distance { from: a, to: b, .. } => {
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

    pub(super) fn spline_distance(
        &self,
        sketch: &Sketch,
        (point, spline): (EntityId, EntityId),
        parameter: usize,
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let handle = self.point(point)?;
        if value.abs() <= self.span_context(handle, handle, value).degenerate_length {
            return self.on_spline(sketch, point, spline, parameter);
        }
        let curve = self.curve(sketch, spline)?;
        let start = self.values.get(parameter).copied().unwrap_or(0.0);
        let [tangent, _] = curve.derivatives(start);
        let fallback = fallback_direction(tangent);
        let normal = fallback.perp();
        let side = if normal.dot(handle.at(&self.values) - curve.point_at(start)) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let spline = self.spline_handle(sketch, spline)?;
        Ok(vec![
            Form::SplineFoot {
                point: handle,
                spline: Arc::clone(&spline),
                parameter,
                fallback,
            },
            Form::SplineDistance {
                point: handle,
                spline,
                parameter,
                fallback,
                side,
                value,
            },
        ])
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SplineEnd {
    pub end: EntityId,
    pub next: EntityId,
    pub after: Option<EntityId>,
}

fn spline_ends(sketch: &Sketch, spline: EntityId) -> Vec<SplineEnd> {
    let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
        return Vec::new();
    };
    let backward: Vec<EntityId> = control_points.iter().rev().copied().collect();
    [control_points.as_slice(), backward.as_slice()]
        .into_iter()
        .filter_map(|points| {
            Some(SplineEnd {
                end: *points.first()?,
                next: *points.get(1)?,
                after: points.get(2).copied(),
            })
        })
        .collect()
}

pub(super) fn end_factor(count: usize) -> f64 {
    let (degree, knots) = clamped_knots(count);
    if degree < 2 {
        return 0.0;
    }
    let knot = |index: usize| knots.get(index).copied().unwrap_or(f64::NAN);
    let order = degree as f64;
    let first = order / (knot(degree + 1) - knot(1));
    let second = order / (knot(degree + 2) - knot(2));
    let bend = (order - 1.0) / (knot(degree + 1) - knot(2));
    second * bend / (first * first)
}

pub(crate) fn joined_at_end(sketch: &Sketch, a: EntityId, b: EntityId) -> bool {
    let joints = Joints::of(sketch);
    match (sketch.role(a), sketch.role(b)) {
        (Some(Role::Spline), Some(Role::Spline)) => joints.spline_joint(sketch, a, b).is_some(),
        (Some(Role::Spline), Some(_)) => joints.spline_end_on(sketch, a, b).is_some(),
        (Some(_), Some(Role::Spline)) => joints.spline_end_on(sketch, b, a).is_some(),
        _ => false,
    }
}

pub(crate) fn not_joined(
    sketch: &Sketch,
    constraint: &Constraint,
    a: EntityId,
    b: EntityId,
) -> SketchError {
    SketchError::NotJoined {
        constraint: constraint.kind_name(),
        first: sketch.entity_label(a),
        second: sketch.entity_label(b),
    }
}

pub(crate) fn straight_spline(sketch: &Sketch, spline: EntityId) -> SketchError {
    SketchError::WrongKind {
        entity: spline,
        found: sketch.entity_label(spline),
        needed: "a spline of three or more control points",
    }
}

impl Joints {
    pub(super) fn spline_end(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        other: EntityId,
    ) -> Option<(EntityId, EntityId)> {
        self.spline_end_on(sketch, spline, other)
            .map(|end| (end.end, end.next))
    }

    pub(super) fn spline_end_on(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        other: EntityId,
    ) -> Option<SplineEnd> {
        let on_other: Vec<EntityId> = self
            .points_on(sketch, other)
            .into_iter()
            .map(|point| self.class(point))
            .collect();
        spline_ends(sketch, spline)
            .into_iter()
            .find(|end| on_other.contains(&self.class(end.end)))
    }

    pub(super) fn spline_joint(
        &self,
        sketch: &Sketch,
        first: EntityId,
        second: EntityId,
    ) -> Option<(SplineEnd, SplineEnd)> {
        let seconds = spline_ends(sketch, second);
        spline_ends(sketch, first).into_iter().find_map(|end| {
            seconds
                .iter()
                .find(|other| self.class(other.end) == self.class(end.end))
                .map(|other| (end, *other))
        })
    }
}

impl System {
    fn leg(&self, from: EntityId, to: EntityId) -> Result<LineHandle, SketchError> {
        let (start, end) = (self.point(from)?, self.point(to)?);
        Ok(LineHandle {
            start,
            end,
            fallback: self.initial_direction(start, end),
        })
    }

    fn end_handle(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        end: SplineEnd,
    ) -> Result<SplineEndHandle, SketchError> {
        let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
            return Err(SketchError::MissingEntity(spline));
        };
        let after = end.after.ok_or_else(|| straight_spline(sketch, spline))?;
        Ok(SplineEndHandle {
            end: self.point(end.end)?,
            next: self.point(end.next)?,
            after: self.point(after)?,
            factor: end_factor(control_points.len()),
        })
    }

    pub(super) fn spline_pair_tangent(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        constraint: &Constraint,
        (a, b): (EntityId, EntityId),
    ) -> Result<Form, SketchError> {
        let (first, second) = joints
            .spline_joint(sketch, a, b)
            .ok_or_else(|| not_joined(sketch, constraint, a, b))?;
        Ok(Form::Parallel(
            self.leg(first.end, first.next)?,
            self.leg(second.end, second.next)?,
        ))
    }

    pub(super) fn spline_curvature(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        constraint: &Constraint,
        (spline, other): (EntityId, EntityId),
    ) -> Result<Vec<Form>, SketchError> {
        let end = joints
            .spline_end_on(sketch, spline, other)
            .ok_or_else(|| not_joined(sketch, constraint, spline, other))?;
        let handle = self.end_handle(sketch, spline, end)?;
        if sketch.role(other) == Some(Role::Line) {
            return Ok(vec![Form::Parallel(
                LineHandle {
                    start: handle.end,
                    end: handle.next,
                    fallback: self.initial_direction(handle.end, handle.next),
                },
                LineHandle {
                    start: handle.next,
                    end: handle.after,
                    fallback: self.initial_direction(handle.next, handle.after),
                },
            )]);
        }
        let circle = self.circle(sketch, other)?;
        let at = handle.end.at(&self.values);
        let leg = handle.next.at(&self.values) - at;
        let side = if leg.perp_dot(circle.center.at(&self.values) - at) < 0.0 {
            -1.0
        } else {
            1.0
        };
        Ok(vec![Form::EndCurvature {
            end: handle,
            circle,
            side,
        }])
    }

    pub(super) fn spline_pair_curvature(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        constraint: &Constraint,
        (a, b): (EntityId, EntityId),
    ) -> Result<Vec<Form>, SketchError> {
        let (first, second) = joints
            .spline_joint(sketch, a, b)
            .ok_or_else(|| not_joined(sketch, constraint, a, b))?;
        Ok(vec![Form::MatchedCurvature(
            self.end_handle(sketch, a, first)?,
            self.end_handle(sketch, b, second)?,
        )])
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
