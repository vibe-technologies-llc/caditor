use std::{cell::OnceCell, sync::Arc};

use caditor_geometry::{Point2, Vector2};

use crate::{
    constraint::Constraint,
    curve::{
        BSpline, CONIC_DEGREE, CONIC_KNOTS, MAX_SPLINE_DEGREE, clamped_knots, conic_weight,
        periodic_knots,
    },
    entity::{Entity, FitSpacing, Role, SplineKind},
    id::{ConstraintId, EntityId},
    sketch::{Sketch, SketchError},
    solve::{
        equation::{
            CircleHandle, Equation, Form, LengthOf, LineHandle, PointHandle, SplineEndHandle,
            SplineHandle, fallback_direction,
        },
        pair::{Course, closest_pair, curve_pair_gap, is_elliptic_pair},
        system::{Joints, System},
    },
    spacing::{FitLayout, fit_controls},
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
    ) -> Result<Vec<(EntityId, f64)>, SketchError> {
        let role = |entity: EntityId| sketch.role(entity);
        let along = |spline: EntityId, starts: Vec<f64>| -> Vec<(EntityId, f64)> {
            starts.into_iter().map(|start| (spline, start)).collect()
        };
        match *constraint {
            Constraint::Coincident(a, b) | Constraint::Distance { from: a, to: b, .. } => {
                match (role(a), role(b)) {
                    (Some(Role::Point), Some(Role::Spline)) => {
                        Ok(along(b, self.closest_start(sketch, a, b)?))
                    }
                    (Some(Role::Spline), Some(Role::Point)) => {
                        Ok(along(a, self.closest_start(sketch, b, a)?))
                    }
                    (Some(Role::Spline), Some(Role::Line | Role::Circular))
                        if matches!(constraint, Constraint::Distance { .. }) =>
                    {
                        Ok(vec![(a, self.touching_start(sketch, a, b)?)])
                    }
                    (Some(Role::Line | Role::Circular), Some(Role::Spline))
                        if matches!(constraint, Constraint::Distance { .. }) =>
                    {
                        Ok(vec![(b, self.touching_start(sketch, b, a)?)])
                    }
                    _ => Ok(Vec::new()),
                }
            }
            Constraint::Tangent(a, b) => {
                let joints = joints.get_or_init(|| Joints::of(sketch));
                match (role(a), role(b)) {
                    (Some(Role::Spline), Some(Role::Spline)) => {
                        if joints.spline_joint(sketch, a, b).is_some() {
                            return Ok(Vec::new());
                        }
                        let (first, second) = (self.curve(sketch, a)?, self.curve(sketch, b)?);
                        let (one, other) =
                            closest_pair(&Course::Spline(first), &Course::Spline(second));
                        Ok(vec![(a, one), (b, other)])
                    }
                    (Some(Role::Spline), Some(Role::Line | Role::Circular)) => {
                        Ok(along(a, self.tangent_start(sketch, joints, a, b)?))
                    }
                    (Some(Role::Line | Role::Circular), Some(Role::Spline)) => {
                        Ok(along(b, self.tangent_start(sketch, joints, b, a)?))
                    }
                    _ => Ok(Vec::new()),
                }
            }
            _ => Ok(Vec::new()),
        }
    }

    fn closest_start(
        &self,
        sketch: &Sketch,
        point: EntityId,
        spline: EntityId,
    ) -> Result<Vec<f64>, SketchError> {
        let target = self.point(point)?.at(&self.values);
        Ok(vec![start_on(
            &self.curve(sketch, spline)?,
            Wanted::Closest,
            |at| at.distance(target),
            |at, tangent, bend| {
                let offset = at - target;
                (offset.dot(tangent), tangent.dot(tangent) + offset.dot(bend))
            },
        )])
    }

    fn tangent_start(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        spline: EntityId,
        other: EntityId,
    ) -> Result<Vec<f64>, SketchError> {
        if joints.spline_end_on(sketch, spline, other).is_some() {
            return Ok(Vec::new());
        }
        Ok(vec![self.touching_start(sketch, spline, other)?])
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
        if let Some(found) = joints.spline_end_on(sketch, spline, other) {
            let legs = self.end_legs(sketch, spline, found)?;
            let (end, neighbour) = (legs.end, legs.next);
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
        self.spline_contact(sketch, (spline, other), parameter, Contact::Touching)
    }

    pub(super) fn spline_gap(
        &self,
        sketch: &Sketch,
        (spline, other): (EntityId, EntityId),
        parameter: usize,
        value: f64,
    ) -> Result<Vec<Form>, SketchError> {
        let size = self.span_context(
            PointHandle::Fixed(Point2::ZERO),
            PointHandle::Fixed(Point2::ZERO),
            value,
        );
        let contact = if value.abs() <= size.degenerate_length {
            Contact::Touching
        } else {
            Contact::Apart(value)
        };
        self.spline_contact(sketch, (spline, other), parameter, contact)
    }

    fn spline_contact(
        &self,
        sketch: &Sketch,
        (spline, other): (EntityId, EntityId),
        parameter: usize,
        contact: Contact,
    ) -> Result<Vec<Form>, SketchError> {
        let handle = self.spline_handle(sketch, spline)?;
        let curve = self.curve(sketch, spline)?;
        let values = &self.values;
        let start = values.get(parameter).copied().unwrap_or(0.0);
        let at = curve.point_at(start);
        let [tangent, _] = curve.derivatives(start);
        let tangent_fallback = fallback_direction(tangent);
        if sketch.role(other) == Some(Role::Line) {
            let line = self.line(sketch, other)?;
            let normal = fallback_direction(line.end.at(values) - line.start.at(values)).perp();
            let (side, value) = contact.side_and_value(normal.dot(at - line.start.at(values)));
            return Ok(vec![
                Form::SplineOnLine {
                    spline: Arc::clone(&handle),
                    parameter,
                    line,
                    side,
                    value,
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
        let center = circle.center.at(values);
        let radial_fallback = fallback_direction(at - center);
        let (side, value) = contact.side_and_value(at.distance(center) - circle.radius(values));
        Ok(vec![
            Form::SplineOnCircle {
                spline: Arc::clone(&handle),
                parameter,
                circle,
                fallback: radial_fallback,
                side,
                value,
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
        self.first_parameter(constraint)
            .ok_or(SketchError::MissingConstraint(constraint))
    }

    pub(super) fn first_parameter(&self, constraint: ConstraintId) -> Option<usize> {
        self.parameters.get(&constraint)?.first().copied()
    }

    pub(super) fn parameter_pair(
        &self,
        constraint: ConstraintId,
    ) -> Result<(usize, usize), SketchError> {
        match self.parameters.get(&constraint).map(Vec::as_slice) {
            Some(&[first, second]) => Ok((first, second)),
            _ => Err(SketchError::MissingConstraint(constraint)),
        }
    }

    pub(super) fn spline_handle(
        &self,
        _sketch: &Sketch,
        spline: EntityId,
    ) -> Result<Arc<SplineHandle>, SketchError> {
        self.splines
            .get(&spline)
            .cloned()
            .ok_or(SketchError::MissingEntity(spline))
    }

    pub(super) fn curve(&self, sketch: &Sketch, spline: EntityId) -> Result<BSpline, SketchError> {
        let handle = self.spline_handle(sketch, spline)?;
        let points = handle
            .points
            .iter()
            .map(|point| point.at(&self.values))
            .collect();
        Ok(BSpline::from_parts(
            points,
            handle.degree,
            handle.knots.clone(),
            handle.weights.clone(),
        ))
    }

    pub(super) fn add_splines(&mut self, sketch: &Sketch) -> Result<(), SketchError> {
        for (id, entity) in sketch.entities() {
            let Entity::Spline { points, kind } = entity else {
                continue;
            };
            let handles = points
                .iter()
                .map(|point| self.point(*point))
                .collect::<Result<Vec<_>, _>>()?;
            let handle = match *kind {
                SplineKind::Control { closed: false } => clamped_handle(handles),
                SplineKind::Control { closed: true } => {
                    periodic_handle(&handles, periodic_knots(handles.len()).1)
                }
                SplineKind::Conic { rho } => {
                    let driven = self.rhos.get(&id).copied().unwrap_or(rho);
                    let weight = conic_weight(driven).ok_or(SketchError::InvalidRho)?;
                    SplineHandle {
                        points: handles,
                        degree: CONIC_DEGREE,
                        knots: CONIC_KNOTS.to_vec(),
                        weights: Some(vec![1.0, weight, 1.0]),
                        periodic: false,
                    }
                }
                SplineKind::Fit { closed, spacing } => {
                    self.fit_handle(id, &handles, closed, spacing)
                }
            };
            self.splines.insert(id, Arc::new(handle));
        }
        Ok(())
    }

    fn fit_handle(
        &mut self,
        id: EntityId,
        fits: &[PointHandle],
        closed: bool,
        spacing: FitSpacing,
    ) -> SplineHandle {
        let positions: Vec<Point2> = fits.iter().map(|fit| fit.at(&self.values)).collect();
        let layout = if !closed && fits.len() <= 2 {
            None
        } else {
            FitLayout::of(&positions, closed, spacing)
                .map(|layout| (layout, spacing))
                .or_else(|| {
                    FitLayout::of(&positions, closed, FitSpacing::Even)
                        .map(|layout| (layout, FitSpacing::Even))
                })
        };
        let Some((layout, spacing)) = layout else {
            return clamped_handle(fits.to_vec());
        };
        let starts =
            fit_controls(&positions, closed, spacing, &layout).unwrap_or_else(|| positions.clone());
        let hidden_range = if closed {
            0..fits.len()
        } else {
            1..fits.len() - 1
        };
        let mut hidden = Vec::with_capacity(hidden_range.len());
        let mut controls = fits.to_vec();
        for index in hidden_range {
            let variable = self.values.len();
            let start = starts.get(index).copied().unwrap_or(Point2::ZERO);
            self.values.extend([start.x, start.y]);
            hidden.push(variable);
            if let Some(slot) = controls.get_mut(index) {
                *slot = PointHandle::Variable(variable);
            }
        }
        self.hidden.insert(id, hidden);
        let handle = if closed {
            periodic_handle(&controls, layout.knots)
        } else {
            SplineHandle {
                points: controls,
                degree: layout.degree,
                knots: layout.knots,
                weights: None,
                periodic: false,
            }
        };
        for (index, (fit, parameter)) in fits.iter().zip(&layout.parameters).enumerate() {
            if !closed && (index == 0 || index + 1 == fits.len()) {
                continue;
            }
            let [(first, weights), _, _] = handle.basis(*parameter);
            let terms: Arc<[(PointHandle, f64)]> = weights
                .iter()
                .zip(handle.points.iter().skip(first))
                .filter(|(weight, _)| **weight != 0.0)
                .map(|(weight, point)| (*point, *weight))
                .collect();
            for along in [Vector2::X, Vector2::Y] {
                let form = Form::Through {
                    point: *fit,
                    terms: Arc::clone(&terms),
                    along,
                };
                self.equations.push(Equation { owner: None, form });
            }
        }
        handle
    }
}

fn clamped_handle(points: Vec<PointHandle>) -> SplineHandle {
    let (degree, knots) = clamped_knots(points.len());
    SplineHandle {
        points,
        degree,
        knots,
        weights: None,
        periodic: false,
    }
}

fn periodic_handle(points: &[PointHandle], knots: Vec<f64>) -> SplineHandle {
    let count = points.len();
    let degree = MAX_SPLINE_DEGREE;
    SplineHandle {
        points: (0..count + degree)
            .filter_map(|index| points.get(index % count.max(1)).copied())
            .collect(),
        degree,
        knots,
        weights: None,
        periodic: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Contact {
    Touching,
    Apart(f64),
}

impl Contact {
    fn side_and_value(self, drawn: f64) -> (f64, f64) {
        match self {
            Self::Touching => (1.0, 0.0),
            Self::Apart(value) if drawn < 0.0 => (-1.0, value),
            Self::Apart(value) => (1.0, value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SplineEnd {
    pub end: EntityId,
    pub from_start: bool,
}

struct EndLegs {
    end: PointHandle,
    next: PointHandle,
    after: Option<PointHandle>,
    factor: f64,
}

fn spline_ends(sketch: &Sketch, spline: EntityId) -> Vec<SplineEnd> {
    let Some((first, last)) = sketch.entity(spline).and_then(Entity::spline_ends) else {
        return Vec::new();
    };
    vec![
        SplineEnd {
            end: first,
            from_start: true,
        },
        SplineEnd {
            end: last,
            from_start: false,
        },
    ]
}

pub(super) fn end_factor(degree: usize, knots: &[f64], from_start: bool) -> f64 {
    if degree < 2 {
        return 0.0;
    }
    let last = knots.len().saturating_sub(1);
    let knot = |index: usize| {
        if from_start {
            knots.get(index).copied().unwrap_or(f64::NAN)
        } else {
            last.checked_sub(index)
                .and_then(|mirrored| knots.get(mirrored))
                .map_or(f64::NAN, |knot| 1.0 - knot)
        }
    };
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

pub(crate) fn joined_ends(
    sketch: &Sketch,
    spline: EntityId,
    other: EntityId,
) -> Option<(EntityId, Option<EntityId>)> {
    let joints = Joints::of(sketch);
    match sketch.role(other)? {
        Role::Spline => joints
            .spline_joint(sketch, spline, other)
            .map(|(own, theirs)| (own.end, Some(theirs.end))),
        Role::Line | Role::Circular => joints
            .spline_end_on(sketch, spline, other)
            .map(|end| (end.end, None)),
        Role::Point | Role::Elliptic => None,
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
    fn end_legs(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        end: SplineEnd,
    ) -> Result<EndLegs, SketchError> {
        let handle = self.spline_handle(sketch, spline)?;
        let count = handle.points.len();
        let at = |step: usize| {
            let index = if end.from_start {
                Some(step)
            } else {
                count.checked_sub(step + 1)
            };
            index.and_then(|index| Some((*handle.points.get(index)?, index)))
        };
        let missing = || SketchError::MissingEntity(spline);
        let (own, own_index) = at(0).ok_or_else(missing)?;
        let (next, next_index) = at(1).ok_or_else(missing)?;
        let after = at(2);
        let weight = |index: usize| {
            handle
                .weights
                .as_ref()
                .and_then(|weights| weights.get(index))
                .copied()
                .unwrap_or(1.0)
        };
        let factor = after.map_or(0.0, |(_, after_index)| {
            end_factor(handle.degree, &handle.knots, end.from_start)
                * weight(own_index)
                * weight(after_index)
                / weight(next_index).powi(2)
        });
        Ok(EndLegs {
            end: own,
            next,
            after: after.map(|(point, _)| point),
            factor,
        })
    }

    pub(super) fn leg(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        end: SplineEnd,
    ) -> Result<LineHandle, SketchError> {
        let legs = self.end_legs(sketch, spline, end)?;
        Ok(LineHandle {
            start: legs.end,
            end: legs.next,
            fallback: self.initial_direction(legs.end, legs.next),
        })
    }

    fn end_handle(
        &self,
        sketch: &Sketch,
        spline: EntityId,
        end: SplineEnd,
    ) -> Result<SplineEndHandle, SketchError> {
        let legs = self.end_legs(sketch, spline, end)?;
        let after = legs.after.ok_or_else(|| straight_spline(sketch, spline))?;
        Ok(SplineEndHandle {
            end: legs.end,
            next: legs.next,
            after,
            factor: legs.factor,
        })
    }

    pub(super) fn spline_pair_tangent(
        &self,
        sketch: &Sketch,
        joints: &Joints,
        id: ConstraintId,
        (a, b): (EntityId, EntityId),
    ) -> Result<Vec<Form>, SketchError> {
        if let Some((first, second)) = joints.spline_joint(sketch, a, b) {
            return Ok(vec![Form::Parallel(
                self.leg(sketch, a, first)?,
                self.leg(sketch, b, second)?,
            )]);
        }
        self.curves_touch(sketch, (a, b), self.parameter_pair(id)?)
    }

    pub(super) fn spline_length(
        &self,
        sketch: &Sketch,
        entity: EntityId,
    ) -> Result<LengthOf, SketchError> {
        match sketch.role(entity) {
            Some(Role::Spline) => Ok(LengthOf::Spline(self.spline_handle(sketch, entity)?)),
            _ => Ok(LengthOf::Line(self.line(sketch, entity)?)),
        }
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

pub(crate) fn spline_gap(
    sketch: &Sketch,
    spline: EntityId,
    other: EntityId,
) -> Option<(Point2, Point2)> {
    if is_elliptic_pair(sketch, spline, other) {
        return curve_pair_gap(sketch, spline, other);
    }
    let curve = sketch.spline(spline)?;
    let slope_from = |center: Point2| {
        move |at: Point2, tangent: Vector2, bend: Vector2| {
            let offset = at - center;
            (offset.dot(tangent), tangent.dot(tangent) + offset.dot(bend))
        }
    };
    if let Some(direction) = sketch.line_direction(other) {
        let normal = direction.try_normalize()?.perp();
        let anchor = sketch
            .line_endpoints(other)
            .map_or(Point2::ZERO, |(start, _)| start);
        let parameter = start_on(
            &curve,
            Wanted::Touching,
            |at| normal.dot(at - anchor),
            |_, tangent, bend| (normal.dot(tangent), normal.dot(bend)),
        );
        let at = curve.point_at(parameter);
        return Some((at, at - normal * normal.dot(at - anchor)));
    }
    let (center, radius) = sketch.circle(other)?;
    let parameter = start_on(
        &curve,
        Wanted::Touching,
        |at| at.distance(center) - radius,
        slope_from(center),
    );
    let at = curve.point_at(parameter);
    let outward = (at - center).try_normalize().unwrap_or(Vector2::X);
    Some((at, center + outward * radius))
}
