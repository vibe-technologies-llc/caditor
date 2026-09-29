use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};

use crate::id::ConstraintId;

pub(crate) type Gradient = Vec<(usize, f64)>;

const DEGENERATE_LENGTH: f64 = 1e-12;
const COLLAPSED_LENGTH: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Context {
    pub scale: f64,
    pub degenerate_length: f64,
}

impl Context {
    pub const UNIT: Self = Self::at_scale(1.0);

    pub const fn at_scale(scale: f64) -> Self {
        Self {
            scale,
            degenerate_length: DEGENERATE_LENGTH * scale,
        }
    }

    pub fn collapsed_length(&self) -> f64 {
        COLLAPSED_LENGTH * self.scale
    }
}

pub(crate) fn value(values: &[f64], index: usize) -> f64 {
    values.get(index).copied().unwrap_or(f64::NAN)
}

pub(crate) fn fallback_direction(vector: Vector2) -> Vector2 {
    vector.try_normalize().unwrap_or(Vector2::X)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PointHandle {
    Variable(usize),
    Fixed(Point2),
}

impl PointHandle {
    pub fn at(self, values: &[f64]) -> Point2 {
        match self {
            Self::Variable(x) => Point2::new(value(values, x), value(values, x + 1)),
            Self::Fixed(position) => position,
        }
    }

    fn push(self, gradient: &mut Gradient, partial: Vector2) {
        if let Self::Variable(x) = self {
            gradient.push((x, partial.x));
            gradient.push((x + 1, partial.y));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineHandle {
    pub start: PointHandle,
    pub end: PointHandle,
    pub fallback: Vector2,
}

impl LineHandle {
    fn direction(&self, values: &[f64], context: &Context) -> Direction {
        Direction::of(
            self.end.at(values) - self.start.at(values),
            self.fallback,
            context,
        )
    }

    fn push_vector(&self, gradient: &mut Gradient, partial: Vector2) {
        self.end.push(gradient, partial);
        self.start.push(gradient, -partial);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RadiusHandle {
    Variable(usize),
    ToArcStart {
        start: PointHandle,
        fallback: Vector2,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CircleHandle {
    pub center: PointHandle,
    pub radius: RadiusHandle,
}

impl CircleHandle {
    pub fn radius(&self, values: &[f64]) -> f64 {
        match self.radius {
            RadiusHandle::Variable(index) => value(values, index),
            RadiusHandle::ToArcStart { start, .. } => {
                start.at(values).distance(self.center.at(values))
            }
        }
    }

    fn push_radius(&self, values: &[f64], context: &Context, gradient: &mut Gradient, factor: f64) {
        match self.radius {
            RadiusHandle::Variable(index) => gradient.push((index, factor)),
            RadiusHandle::ToArcStart { start, fallback } => {
                let direction =
                    Direction::of(start.at(values) - self.center.at(values), fallback, context);
                start.push(gradient, direction.unit * factor);
                self.center.push(gradient, -direction.unit * factor);
            }
        }
    }
}

struct Direction {
    unit: Vector2,
    length: f64,
    degenerate: bool,
}

impl Direction {
    fn of(vector: Vector2, fallback: Vector2, context: &Context) -> Self {
        let length = vector.length();
        if length > context.degenerate_length && length.is_finite() {
            Self {
                unit: vector / length,
                length,
                degenerate: false,
            }
        } else {
            Self {
                unit: fallback,
                length,
                degenerate: true,
            }
        }
    }

    fn back_from_unit(&self, partial: Vector2) -> Vector2 {
        if self.degenerate {
            Vector2::ZERO
        } else {
            (partial - self.unit * self.unit.dot(partial)) / self.length
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Contact {
    External,
    Internal { larger_first: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Form {
    SameX(PointHandle, PointHandle),
    SameY(PointHandle, PointHandle),
    OnLine {
        point: PointHandle,
        line: LineHandle,
    },
    OnCircle {
        point: PointHandle,
        circle: CircleHandle,
        fallback: Vector2,
    },
    Horizontal(LineHandle),
    Vertical(LineHandle),
    Parallel(LineHandle, LineHandle),
    Perpendicular(LineHandle, LineHandle),
    LineTangent {
        line: LineHandle,
        circle: CircleHandle,
        side: f64,
    },
    CircleTangent {
        first: CircleHandle,
        second: CircleHandle,
        fallback: Vector2,
        contact: Contact,
    },
    EqualLength(LineHandle, LineHandle),
    EqualRadius(CircleHandle, CircleHandle),
    PointDistance {
        from: PointHandle,
        to: PointHandle,
        fallback: Vector2,
        value: f64,
    },
    LineDistance {
        point: PointHandle,
        line: LineHandle,
        side: f64,
        value: f64,
    },
    Angle {
        from: LineHandle,
        to: LineHandle,
        reversed: bool,
        radians: f64,
    },
    Radius {
        circle: CircleHandle,
        value: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Equation {
    pub owner: Option<ConstraintId>,
    pub form: Form,
}

impl Equation {
    pub fn residual(&self, values: &[f64], context: &Context) -> f64 {
        self.form.evaluate(values, context, &mut Vec::new())
    }

    pub fn linearize(&self, values: &[f64], context: &Context, gradient: &mut Gradient) -> f64 {
        gradient.clear();
        self.form.evaluate(values, context, gradient)
    }

    pub fn variables(&self, values: &[f64]) -> Vec<usize> {
        let mut gradient = Vec::new();
        self.linearize(values, &Context::UNIT, &mut gradient);
        let mut variables: Vec<usize> = gradient.into_iter().map(|(index, _)| index).collect();
        variables.sort_unstable();
        variables.dedup();
        variables
    }
}

impl Form {
    pub fn length(&self) -> Option<f64> {
        match *self {
            Self::PointDistance { value, .. }
            | Self::LineDistance { value, .. }
            | Self::Radius { value, .. } => Some(value),
            Self::SameX(..)
            | Self::SameY(..)
            | Self::OnLine { .. }
            | Self::OnCircle { .. }
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::LineTangent { .. }
            | Self::CircleTangent { .. }
            | Self::EqualLength(..)
            | Self::EqualRadius(..)
            | Self::Angle { .. } => None,
        }
    }

    fn evaluate(&self, values: &[f64], context: &Context, gradient: &mut Gradient) -> f64 {
        match *self {
            Self::SameX(a, b) => {
                a.push(gradient, Vector2::X);
                b.push(gradient, -Vector2::X);
                a.at(values).x - b.at(values).x
            }
            Self::SameY(a, b) => {
                a.push(gradient, Vector2::Y);
                b.push(gradient, -Vector2::Y);
                a.at(values).y - b.at(values).y
            }
            Self::OnLine { point, line } => {
                signed_distance(point, &line, values, context, gradient, 1.0)
            }
            Self::OnCircle {
                point,
                circle,
                fallback,
            } => {
                let direction = Direction::of(
                    point.at(values) - circle.center.at(values),
                    fallback,
                    context,
                );
                point.push(gradient, direction.unit);
                circle.center.push(gradient, -direction.unit);
                circle.push_radius(values, context, gradient, -1.0);
                direction.length - circle.radius(values)
            }
            Self::Horizontal(line) => {
                line.push_vector(gradient, Vector2::Y);
                line.end.at(values).y - line.start.at(values).y
            }
            Self::Vertical(line) => {
                line.push_vector(gradient, Vector2::X);
                line.end.at(values).x - line.start.at(values).x
            }
            Self::Parallel(a, b) => {
                let (first, second) = (a.direction(values, context), b.direction(values, context));
                let scale = context.scale;
                a.push_vector(gradient, first.back_from_unit(-second.unit.perp() * scale));
                b.push_vector(gradient, second.back_from_unit(first.unit.perp() * scale));
                first.unit.perp_dot(second.unit) * scale
            }
            Self::Perpendicular(a, b) => {
                let (first, second) = (a.direction(values, context), b.direction(values, context));
                let scale = context.scale;
                a.push_vector(gradient, first.back_from_unit(second.unit * scale));
                b.push_vector(gradient, second.back_from_unit(first.unit * scale));
                first.unit.dot(second.unit) * scale
            }
            Self::LineTangent { line, circle, side } => {
                let distance =
                    signed_distance(circle.center, &line, values, context, gradient, 1.0);
                circle.push_radius(values, context, gradient, -side);
                distance - side * circle.radius(values)
            }
            Self::CircleTangent {
                first,
                second,
                fallback,
                contact,
            } => {
                let direction = Direction::of(
                    first.center.at(values) - second.center.at(values),
                    fallback,
                    context,
                );
                first.center.push(gradient, direction.unit);
                second.center.push(gradient, -direction.unit);
                let (first_radius, second_radius) = (first.radius(values), second.radius(values));
                match contact {
                    Contact::External => {
                        first.push_radius(values, context, gradient, -1.0);
                        second.push_radius(values, context, gradient, -1.0);
                        direction.length - (first_radius + second_radius)
                    }
                    Contact::Internal { larger_first } => {
                        let larger_first =
                            if (first_radius - second_radius).abs() > context.degenerate_length {
                                (first_radius - second_radius).signum()
                            } else {
                                larger_first
                            };
                        first.push_radius(values, context, gradient, -larger_first);
                        second.push_radius(values, context, gradient, larger_first);
                        direction.length - larger_first * (first_radius - second_radius)
                    }
                }
            }
            Self::EqualLength(a, b) => {
                let (first, second) = (a.direction(values, context), b.direction(values, context));
                a.push_vector(gradient, first.unit);
                b.push_vector(gradient, -second.unit);
                first.length - second.length
            }
            Self::EqualRadius(a, b) => {
                a.push_radius(values, context, gradient, 1.0);
                b.push_radius(values, context, gradient, -1.0);
                a.radius(values) - b.radius(values)
            }
            Self::PointDistance {
                from,
                to,
                fallback,
                value,
            } => {
                let direction = Direction::of(to.at(values) - from.at(values), fallback, context);
                to.push(gradient, direction.unit);
                from.push(gradient, -direction.unit);
                direction.length - value
            }
            Self::LineDistance {
                point,
                line,
                side,
                value,
            } => signed_distance(point, &line, values, context, gradient, side) - value,
            Self::Angle {
                from,
                to,
                reversed,
                radians,
            } => {
                let (first, second) = (
                    from.direction(values, context),
                    to.direction(values, context),
                );
                let scale = context.scale;
                let angle = first
                    .unit
                    .perp_dot(second.unit)
                    .atan2(first.unit.dot(second.unit));
                if !first.degenerate {
                    from.push_vector(gradient, -first.unit.perp() / first.length * scale);
                } else {
                    from.push_vector(gradient, Vector2::ZERO);
                }
                if !second.degenerate {
                    to.push_vector(gradient, second.unit.perp() / second.length * scale);
                } else {
                    to.push_vector(gradient, Vector2::ZERO);
                }
                let turn = if reversed { PI } else { 0.0 };
                wrap_angle(angle + turn - radians) * scale
            }
            Self::Radius { circle, value } => {
                circle.push_radius(values, context, gradient, 1.0);
                circle.radius(values) - value
            }
        }
    }
}

fn signed_distance(
    point: PointHandle,
    line: &LineHandle,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
    factor: f64,
) -> f64 {
    let direction = line.direction(values, context);
    let offset = point.at(values) - line.start.at(values);
    let normal = direction.unit.perp();
    point.push(gradient, normal * factor);
    line.start.push(gradient, -normal * factor);
    line.push_vector(gradient, direction.back_from_unit(-offset.perp()) * factor);
    direction.unit.perp_dot(offset) * factor
}

pub(crate) fn wrap_angle(angle: f64) -> f64 {
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTEXT: Context = Context::at_scale(7.0);
    const STEP: f64 = 1e-6;

    fn point(index: usize) -> PointHandle {
        PointHandle::Variable(index)
    }

    fn line(start: usize, end: usize) -> LineHandle {
        LineHandle {
            start: point(start),
            end: point(end),
            fallback: Vector2::X,
        }
    }

    fn circle(center: usize, radius: usize) -> CircleHandle {
        CircleHandle {
            center: point(center),
            radius: RadiusHandle::Variable(radius),
        }
    }

    fn arc(center: usize, start: usize) -> CircleHandle {
        CircleHandle {
            center: point(center),
            radius: RadiusHandle::ToArcStart {
                start: point(start),
                fallback: Vector2::X,
            },
        }
    }

    fn values() -> Vec<f64> {
        vec![
            0.3, -1.2, 4.1, 0.7, 2.2, 3.9, -1.5, 2.8, 1.1, 5.3, 1.7, 2.4, -0.6, 0.9, 3.3, -2.1,
        ]
    }

    fn forms() -> Vec<Form> {
        vec![
            Form::SameX(point(0), point(2)),
            Form::SameY(point(0), PointHandle::Fixed(Point2::new(1.0, 2.0))),
            Form::OnLine {
                point: point(4),
                line: line(0, 2),
            },
            Form::OnLine {
                point: point(4),
                line: LineHandle {
                    start: PointHandle::Fixed(Point2::ZERO),
                    end: PointHandle::Fixed(Point2::X),
                    fallback: Vector2::X,
                },
            },
            Form::OnCircle {
                point: point(4),
                circle: circle(6, 10),
                fallback: Vector2::X,
            },
            Form::OnCircle {
                point: point(12),
                circle: arc(6, 8),
                fallback: Vector2::X,
            },
            Form::Horizontal(line(0, 2)),
            Form::Vertical(line(0, 2)),
            Form::Parallel(line(0, 2), line(4, 6)),
            Form::Perpendicular(line(0, 2), line(4, 6)),
            Form::LineTangent {
                line: line(0, 2),
                circle: arc(6, 8),
                side: -1.0,
            },
            Form::LineTangent {
                line: line(0, 2),
                circle: circle(6, 11),
                side: 1.0,
            },
            Form::CircleTangent {
                first: circle(6, 10),
                second: arc(12, 14),
                fallback: Vector2::X,
                contact: Contact::External,
            },
            Form::CircleTangent {
                first: arc(4, 0),
                second: circle(12, 11),
                fallback: Vector2::X,
                contact: Contact::Internal { larger_first: -1.0 },
            },
            Form::EqualLength(line(0, 2), line(4, 6)),
            Form::EqualRadius(circle(6, 10), arc(12, 14)),
            Form::PointDistance {
                from: point(0),
                to: point(4),
                fallback: Vector2::X,
                value: 2.5,
            },
            Form::LineDistance {
                point: point(8),
                line: line(0, 2),
                side: -1.0,
                value: 1.5,
            },
            Form::Angle {
                from: line(0, 2),
                to: line(4, 6),
                reversed: false,
                radians: 0.5,
            },
            Form::Angle {
                from: line(0, 2),
                to: line(4, 6),
                reversed: true,
                radians: 0.5,
            },
            Form::Radius {
                circle: arc(4, 12),
                value: 3.0,
            },
        ]
    }

    #[test]
    fn every_gradient_matches_central_finite_differences() {
        let base = values();
        for form in forms() {
            let equation = Equation { owner: None, form };
            let mut gradient = Vec::new();
            equation.linearize(&base, &CONTEXT, &mut gradient);
            let mut analytic = vec![0.0; base.len()];
            for (index, partial) in gradient {
                analytic[index] += partial;
            }
            for (index, expected) in analytic.iter().enumerate() {
                let mut plus = base.clone();
                plus[index] += STEP;
                let mut minus = base.clone();
                minus[index] -= STEP;
                let numeric = (equation.residual(&plus, &CONTEXT)
                    - equation.residual(&minus, &CONTEXT))
                    / (2.0 * STEP);
                assert!(
                    (numeric - expected).abs() < 1e-6 * (1.0 + expected.abs()),
                    "{form:?}: variable {index} is {expected} analytically but {numeric} \
                     numerically"
                );
            }
        }
    }

    #[test]
    fn degenerate_geometry_gives_finite_residuals_and_gradients() {
        let collapsed = vec![1.0; 16];
        for form in forms() {
            let equation = Equation { owner: None, form };
            let mut gradient = Vec::new();
            let residual = equation.linearize(&collapsed, &CONTEXT, &mut gradient);
            assert!(residual.is_finite(), "{form:?}");
            assert!(
                gradient.iter().all(|(_, partial)| partial.is_finite()),
                "{form:?}"
            );
        }
    }

    #[test]
    fn angles_wrap_into_a_half_open_turn() {
        assert!((wrap_angle(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12);
        assert!((wrap_angle(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-12);
        assert_eq!(wrap_angle(0.25), 0.25);
    }
}
