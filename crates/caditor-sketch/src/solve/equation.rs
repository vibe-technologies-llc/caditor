use std::{
    f64::consts::{PI, TAU},
    sync::Arc,
};

use caditor_geometry::{Point2, Vector2};

use crate::{
    curve::{length_nodes, rational_basis},
    id::ConstraintId,
};

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
    Fixed(f64),
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
            RadiusHandle::Fixed(radius) => radius,
            RadiusHandle::ToArcStart { start, .. } => {
                start.at(values).distance(self.center.at(values))
            }
        }
    }

    fn push_radius(&self, values: &[f64], context: &Context, gradient: &mut Gradient, factor: f64) {
        match self.radius {
            RadiusHandle::Variable(index) => gradient.push((index, factor)),
            RadiusHandle::Fixed(_) => {}
            RadiusHandle::ToArcStart { start, fallback } => {
                let direction =
                    Direction::of(start.at(values) - self.center.at(values), fallback, context);
                start.push(gradient, direction.unit * factor);
                self.center.push(gradient, -direction.unit * factor);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EllipseHandle {
    pub center: PointHandle,
    pub major: PointHandle,
    pub minor: RadiusHandle,
    pub fallback: Vector2,
}

struct EllipseFrame {
    axis: Direction,
    major: f64,
    minor: f64,
}

struct EllipsePartials {
    axis: Vector2,
    major: f64,
    minor: f64,
}

struct EllipseAt {
    frame: EllipseFrame,
    cos: f64,
    sin: f64,
    point: Point2,
    tangent: Vector2,
    bend: Vector2,
}

impl EllipseHandle {
    pub(crate) fn minor_circle(&self) -> CircleHandle {
        CircleHandle {
            center: self.center,
            radius: self.minor,
        }
    }

    fn frame(&self, values: &[f64], context: &Context) -> EllipseFrame {
        let axis = Direction::of(
            self.major.at(values) - self.center.at(values),
            self.fallback,
            context,
        );
        let floor = context.degenerate_length;
        let minor = self.minor_circle().radius(values);
        EllipseFrame {
            major: axis.length.max(floor),
            minor: if minor.abs() > floor {
                minor
            } else {
                floor.copysign(minor)
            },
            axis,
        }
    }

    fn at(&self, values: &[f64], context: &Context, parameter: usize) -> EllipseAt {
        let frame = self.frame(values, context);
        let (sin, cos) = (TAU * value(values, parameter)).sin_cos();
        let (unit, across) = (frame.axis.unit, frame.axis.unit.perp());
        let radial = unit * (frame.major * cos) + across * (frame.minor * sin);
        EllipseAt {
            point: self.center.at(values) + radial,
            tangent: (across * (frame.minor * cos) - unit * (frame.major * sin)) * TAU,
            bend: -radial * (TAU * TAU),
            frame,
            cos,
            sin,
        }
    }

    fn push_point(
        &self,
        values: &[f64],
        context: &Context,
        at: &EllipseAt,
        gradient: &mut Gradient,
        partial: Vector2,
    ) {
        let (a, b) = (at.frame.major, at.frame.minor);
        let unit = at.frame.axis.unit;
        self.center.push(gradient, partial);
        let partials = EllipsePartials {
            axis: partial * (a * at.cos) - partial.perp() * (b * at.sin),
            major: at.cos * partial.dot(unit),
            minor: at.sin * partial.dot(unit.perp()),
        };
        self.push(values, context, &at.frame, gradient, &partials);
    }

    fn push_tangent(
        &self,
        values: &[f64],
        context: &Context,
        at: &EllipseAt,
        gradient: &mut Gradient,
        partial: Vector2,
    ) {
        let (a, b) = (at.frame.major, at.frame.minor);
        let unit = at.frame.axis.unit;
        let partials = EllipsePartials {
            axis: (-partial * (a * at.sin) - partial.perp() * (b * at.cos)) * TAU,
            major: -TAU * at.sin * partial.dot(unit),
            minor: TAU * at.cos * partial.dot(unit.perp()),
        };
        self.push(values, context, &at.frame, gradient, &partials);
    }

    fn push(
        &self,
        values: &[f64],
        context: &Context,
        frame: &EllipseFrame,
        gradient: &mut Gradient,
        partials: &EllipsePartials,
    ) {
        let along = if frame.axis.degenerate {
            Vector2::ZERO
        } else {
            frame.axis.back_from_unit(partials.axis) + frame.axis.unit * partials.major
        };
        self.major.push(gradient, along);
        self.center.push(gradient, -along);
        self.minor_circle()
            .push_radius(values, context, gradient, partials.minor);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplineHandle {
    pub points: Vec<PointHandle>,
    pub degree: usize,
    pub knots: Vec<f64>,
    pub weights: Option<Vec<f64>>,
    pub periodic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SplineEndHandle {
    pub end: PointHandle,
    pub next: PointHandle,
    pub after: PointHandle,
    pub factor: f64,
}

struct EndBend {
    value: f64,
    end: Vector2,
    next: Vector2,
    after: Vector2,
}

impl SplineEndHandle {
    fn bend(&self, values: &[f64], context: &Context) -> EndBend {
        let end = self.end.at(values);
        let next = self.next.at(values);
        let first = next - end;
        let second = self.after.at(values) - next;
        let length = first.length();
        if !(length > context.degenerate_length && length.is_finite()) {
            return EndBend {
                value: 0.0,
                end: Vector2::ZERO,
                next: Vector2::ZERO,
                after: Vector2::ZERO,
            };
        }
        let cube = length.powi(3);
        let value = self.factor * first.perp_dot(second) / cube;
        let by_first =
            -second.perp() * (self.factor / cube) - first * (3.0 * value / (length * length));
        let by_second = first.perp() * (self.factor / cube);
        EndBend {
            value,
            end: -by_first,
            next: by_first - by_second,
            after: by_second,
        }
    }

    fn push(&self, gradient: &mut Gradient, bend: &EndBend, factor: f64) {
        self.end.push(gradient, bend.end * factor);
        self.next.push(gradient, bend.next * factor);
        self.after.push(gradient, bend.after * factor);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LengthOf {
    Line(LineHandle),
    Spline(Arc<SplineHandle>),
}

impl LengthOf {
    fn measure(
        &self,
        values: &[f64],
        context: &Context,
        gradient: &mut Gradient,
        factor: f64,
    ) -> f64 {
        match self {
            Self::Line(line) => {
                let direction = line.direction(values, context);
                line.push_vector(gradient, direction.unit * factor);
                direction.length
            }
            Self::Spline(spline) => spline.length(values, gradient, factor),
        }
    }
}

struct SplineAt {
    point: Point2,
    tangent: Vector2,
    bend: Vector2,
    first: usize,
    weights: Vec<f64>,
    slopes: Vec<f64>,
}

impl SplineAt {
    fn tangent_line(&self, fallback: Vector2, context: &Context) -> Direction {
        let at_cusp = self.bend.try_normalize().unwrap_or(fallback);
        Direction::of(self.tangent, at_cusp, context)
    }
}

impl SplineHandle {
    fn at(&self, values: &[f64], parameter: usize) -> SplineAt {
        let parameter = value(values, parameter);
        let [(first, weights), (_, slopes), (_, bends)] = self.basis(parameter);
        let combine = |weights: &[f64]| {
            weights
                .iter()
                .zip(self.points.iter().skip(first))
                .fold(Vector2::ZERO, |sum, (weight, point)| {
                    sum + point.at(values) * *weight
                })
        };
        SplineAt {
            point: combine(&weights),
            tangent: combine(&slopes),
            bend: combine(&bends),
            first,
            weights,
            slopes,
        }
    }

    fn length(&self, values: &[f64], gradient: &mut Gradient, factor: f64) -> f64 {
        let count = self.points.len();
        let mut total = 0.0;
        for (parameter, weight) in length_nodes(self.degree, &self.knots, count) {
            let [_, (first, slopes), _] = self.basis(parameter);
            let tangent = slopes
                .iter()
                .zip(self.points.iter().skip(first))
                .fold(Vector2::ZERO, |sum, (slope, point)| {
                    sum + point.at(values) * *slope
                });
            let speed = tangent.length();
            total += speed * weight;
            if speed > 0.0 && speed.is_finite() {
                self.push(gradient, first, &slopes, tangent / speed * weight * factor);
            }
        }
        total
    }

    pub(crate) fn basis(&self, parameter: f64) -> [(usize, Vec<f64>); 3] {
        let parameter = if self.periodic {
            parameter.rem_euclid(1.0)
        } else {
            parameter
        };
        rational_basis(
            self.degree,
            &self.knots,
            self.weights.as_deref(),
            self.points.len(),
            parameter,
        )
    }

    fn push(&self, gradient: &mut Gradient, first: usize, weights: &[f64], partial: Vector2) {
        for (weight, point) in weights.iter().zip(self.points.iter().skip(first)) {
            point.push(gradient, partial * *weight);
        }
    }

    fn push_point(&self, at: &SplineAt, gradient: &mut Gradient, partial: Vector2) {
        self.push(gradient, at.first, &at.weights, partial);
    }

    fn push_tangent(&self, at: &SplineAt, gradient: &mut Gradient, partial: Vector2) {
        self.push(gradient, at.first, &at.slopes, partial);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CurveHandle {
    Spline(Arc<SplineHandle>),
    Ellipse(EllipseHandle),
}

enum CurveAt<'a> {
    Spline(&'a SplineHandle, SplineAt),
    Ellipse(&'a EllipseHandle, EllipseAt),
}

impl CurveHandle {
    fn at(&self, values: &[f64], context: &Context, parameter: usize) -> CurveAt<'_> {
        match self {
            Self::Spline(spline) => CurveAt::Spline(spline, spline.at(values, parameter)),
            Self::Ellipse(ellipse) => {
                CurveAt::Ellipse(ellipse, ellipse.at(values, context, parameter))
            }
        }
    }
}

impl CurveAt<'_> {
    fn point(&self) -> Point2 {
        match self {
            Self::Spline(_, at) => at.point,
            Self::Ellipse(_, at) => at.point,
        }
    }

    fn tangent(&self) -> Vector2 {
        match self {
            Self::Spline(_, at) => at.tangent,
            Self::Ellipse(_, at) => at.tangent,
        }
    }

    fn bend(&self) -> Vector2 {
        match self {
            Self::Spline(_, at) => at.bend,
            Self::Ellipse(_, at) => at.bend,
        }
    }

    fn tangent_line(&self, fallback: Vector2, context: &Context) -> Direction {
        match self {
            Self::Spline(_, at) => at.tangent_line(fallback, context),
            Self::Ellipse(_, at) => Direction::of(at.tangent, fallback, context),
        }
    }

    fn push_point(
        &self,
        values: &[f64],
        context: &Context,
        gradient: &mut Gradient,
        partial: Vector2,
    ) {
        match self {
            Self::Spline(spline, at) => spline.push_point(at, gradient, partial),
            Self::Ellipse(ellipse, at) => {
                ellipse.push_point(values, context, at, gradient, partial)
            }
        }
    }

    fn push_tangent(
        &self,
        values: &[f64],
        context: &Context,
        gradient: &mut Gradient,
        partial: Vector2,
    ) {
        match self {
            Self::Spline(spline, at) => spline.push_tangent(at, gradient, partial),
            Self::Ellipse(ellipse, at) => {
                ellipse.push_tangent(values, context, at, gradient, partial);
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

#[derive(Debug, Clone, PartialEq)]
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
    ArcLength {
        from: LineHandle,
        to: LineHandle,
        value: f64,
    },
    Middle {
        point: PointHandle,
        ends: (PointHandle, PointHandle),
        along: Vector2,
    },
    MirrorMiddle {
        first: PointHandle,
        second: PointHandle,
        line: LineHandle,
    },
    OnBisector {
        point: PointHandle,
        chord: LineHandle,
    },
    ArcBulge {
        point: PointHandle,
        chord: LineHandle,
        arc: CircleHandle,
    },
    MirrorAcross {
        first: PointHandle,
        second: PointHandle,
        line: LineHandle,
    },
    Offset {
        from: PointHandle,
        to: PointHandle,
        along: Vector2,
        side: f64,
        value: f64,
    },
    CircleDistance {
        point: PointHandle,
        circle: CircleHandle,
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    LineOffset {
        point: PointHandle,
        line: LineHandle,
        along: Vector2,
        side: f64,
        value: f64,
    },
    CircleGap {
        first: CircleHandle,
        second: CircleHandle,
        fallback: Vector2,
        contact: Contact,
        value: f64,
    },
    LineGap {
        line: LineHandle,
        circle: CircleHandle,
        side: f64,
        value: f64,
    },
    OnSpline {
        point: PointHandle,
        spline: Arc<SplineHandle>,
        parameter: usize,
        along: Vector2,
    },
    SplineOnLine {
        spline: Arc<SplineHandle>,
        parameter: usize,
        line: LineHandle,
        side: f64,
        value: f64,
    },
    SplineAlongLine {
        spline: Arc<SplineHandle>,
        parameter: usize,
        line: LineHandle,
        fallback: Vector2,
    },
    SplineOnCircle {
        spline: Arc<SplineHandle>,
        parameter: usize,
        circle: CircleHandle,
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    CurvesMeet {
        first: CurveHandle,
        second: CurveHandle,
        parameters: (usize, usize),
        along: Vector2,
    },
    CurvesAlong {
        first: CurveHandle,
        second: CurveHandle,
        parameters: (usize, usize),
        fallbacks: (Vector2, Vector2),
    },
    SameLength(LengthOf, LengthOf),
    SplineAcrossRadius {
        spline: Arc<SplineHandle>,
        parameter: usize,
        circle: CircleHandle,
        fallbacks: (Vector2, Vector2),
    },
    SplineFoot {
        point: PointHandle,
        spline: Arc<SplineHandle>,
        parameter: usize,
        fallback: Vector2,
    },
    SplineDistance {
        point: PointHandle,
        spline: Arc<SplineHandle>,
        parameter: usize,
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    EndCurvature {
        end: SplineEndHandle,
        circle: CircleHandle,
        side: f64,
    },
    MatchedCurvature(SplineEndHandle, SplineEndHandle),
    OnEllipse {
        point: PointHandle,
        ellipse: EllipseHandle,
    },
    EllipseTangent {
        line: LineHandle,
        ellipse: EllipseHandle,
        side: f64,
        value: f64,
    },
    EllipseTouch {
        line: LineHandle,
        point: PointHandle,
        ellipse: EllipseHandle,
    },
    EllipseMiddle {
        point: PointHandle,
        ends: (PointHandle, PointHandle),
        ellipse: EllipseHandle,
    },
    EllipseTouchCircle {
        circle: CircleHandle,
        point: PointHandle,
        ellipse: EllipseHandle,
        fallback: Vector2,
    },
    Through {
        point: PointHandle,
        terms: Arc<[(PointHandle, f64)]>,
        along: Vector2,
    },
    EllipseFoot {
        point: PointHandle,
        ellipse: EllipseHandle,
        parameter: usize,
        fallback: Vector2,
    },
    EllipseDistance {
        point: PointHandle,
        ellipse: EllipseHandle,
        parameter: usize,
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    EllipseOnCircle {
        ellipse: EllipseHandle,
        parameter: usize,
        circle: CircleHandle,
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    EllipseAcrossRadius {
        ellipse: EllipseHandle,
        parameter: usize,
        circle: CircleHandle,
        fallbacks: (Vector2, Vector2),
    },
    OnMinorAxis {
        point: PointHandle,
        axis: LineHandle,
    },
    CurvesFoot {
        first: CurveHandle,
        second: CurveHandle,
        parameters: (usize, usize),
        fallback: Vector2,
    },
    CurvesGap {
        first: CurveHandle,
        second: CurveHandle,
        parameters: (usize, usize),
        fallback: Vector2,
        side: f64,
        value: f64,
    },
    EllipsesTouch {
        point: PointHandle,
        first: EllipseHandle,
        second: EllipseHandle,
        fallbacks: (Vector2, Vector2),
    },
    NormalAngle {
        line: LineHandle,
        point: PointHandle,
        ellipse: EllipseHandle,
        fallback: Vector2,
        ellipse_first: bool,
        reversed: bool,
        radians: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Equation {
    pub owner: Option<ConstraintId>,
    pub form: Form,
}

impl Equation {
    pub fn residual(&self, values: &[f64], context: &Context, scratch: &mut Gradient) -> f64 {
        scratch.clear();
        self.form.evaluate(values, context, scratch)
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
            | Self::Radius { value, .. }
            | Self::Offset { value, .. }
            | Self::LineOffset { value, .. }
            | Self::CircleDistance { value, .. }
            | Self::CircleGap { value, .. }
            | Self::LineGap { value, .. }
            | Self::ArcLength { value, .. }
            | Self::SplineOnLine { value, .. }
            | Self::SplineOnCircle { value, .. }
            | Self::SplineDistance { value, .. }
            | Self::EllipseTangent { value, .. }
            | Self::EllipseDistance { value, .. }
            | Self::EllipseOnCircle { value, .. }
            | Self::CurvesGap { value, .. } => Some(value),
            Self::SameX(a, b) => fixed_coordinate(a, b, |position| position.x),
            Self::SameY(a, b) => fixed_coordinate(a, b, |position| position.y),
            Self::OnLine { .. }
            | Self::OnCircle { .. }
            | Self::Horizontal(_)
            | Self::Vertical(_)
            | Self::Parallel(..)
            | Self::Perpendicular(..)
            | Self::LineTangent { .. }
            | Self::CircleTangent { .. }
            | Self::EqualLength(..)
            | Self::EqualRadius(..)
            | Self::Angle { .. }
            | Self::Middle { .. }
            | Self::MirrorMiddle { .. }
            | Self::OnBisector { .. }
            | Self::ArcBulge { .. }
            | Self::MirrorAcross { .. }
            | Self::OnSpline { .. }
            | Self::SplineAlongLine { .. }
            | Self::SplineAcrossRadius { .. }
            | Self::CurvesMeet { .. }
            | Self::CurvesAlong { .. }
            | Self::CurvesFoot { .. }
            | Self::EllipsesTouch { .. }
            | Self::NormalAngle { .. }
            | Self::SameLength(..)
            | Self::SplineFoot { .. }
            | Self::EndCurvature { .. }
            | Self::MatchedCurvature(..)
            | Self::OnEllipse { .. }
            | Self::EllipseTouch { .. }
            | Self::EllipseMiddle { .. }
            | Self::EllipseTouchCircle { .. }
            | Self::EllipseFoot { .. }
            | Self::EllipseAcrossRadius { .. }
            | Self::OnMinorAxis { .. }
            | Self::Through { .. } => None,
        }
    }

    fn evaluate(&self, values: &[f64], context: &Context, gradient: &mut Gradient) -> f64 {
        match *self {
            Self::Through {
                point,
                ref terms,
                along,
            } => {
                point.push(gradient, along);
                let mut combined = Vector2::ZERO;
                for (term, weight) in terms.iter() {
                    term.push(gradient, -along * *weight);
                    combined += term.at(values) * *weight;
                }
                along.dot(point.at(values) - combined)
            }
            Self::OnEllipse { point, ellipse } => {
                on_ellipse(point, &ellipse, values, context, gradient)
            }
            Self::EllipseTangent {
                line,
                ellipse,
                side,
                value,
            } => ellipse_tangent(&line, &ellipse, (side, value), values, context, gradient),
            Self::OnMinorAxis { point, axis } => {
                let direction = axis.direction(values, context);
                let offset = point.at(values) - axis.start.at(values);
                point.push(gradient, direction.unit);
                axis.start.push(gradient, -direction.unit);
                axis.push_vector(gradient, direction.back_from_unit(offset));
                direction.unit.dot(offset)
            }
            Self::EllipseFoot {
                point,
                ellipse,
                parameter,
                fallback,
            } => {
                let at = ellipse.at(values, context, parameter);
                let along = Direction::of(at.tangent, fallback, context);
                let offset = point.at(values) - at.point;
                let turning = along.back_from_unit(offset);
                point.push(gradient, along.unit);
                ellipse.push_point(values, context, &at, gradient, -along.unit);
                ellipse.push_tangent(values, context, &at, gradient, turning);
                gradient.push((parameter, turning.dot(at.bend) - along.unit.dot(at.tangent)));
                along.unit.dot(offset)
            }
            Self::EllipseDistance {
                point,
                ellipse,
                parameter,
                fallback,
                side,
                value,
            } => {
                let at = ellipse.at(values, context, parameter);
                let along = Direction::of(at.tangent, fallback, context);
                let offset = point.at(values) - at.point;
                let normal = along.unit.perp() * side;
                let turning = along.back_from_unit(-offset.perp() * side);
                point.push(gradient, normal);
                ellipse.push_point(values, context, &at, gradient, -normal);
                ellipse.push_tangent(values, context, &at, gradient, turning);
                gradient.push((parameter, turning.dot(at.bend) - normal.dot(at.tangent)));
                normal.dot(offset) - value
            }
            Self::EllipseOnCircle {
                ellipse,
                parameter,
                circle,
                fallback,
                side,
                value,
            } => {
                let at = ellipse.at(values, context, parameter);
                let direction =
                    Direction::of(at.point - circle.center.at(values), fallback, context);
                let outward = direction.unit * side;
                ellipse.push_point(values, context, &at, gradient, outward);
                gradient.push((parameter, outward.dot(at.tangent)));
                circle.center.push(gradient, -outward);
                circle.push_radius(values, context, gradient, -side);
                (direction.length - circle.radius(values)) * side - value
            }
            Self::EllipseAcrossRadius {
                ellipse,
                parameter,
                circle,
                fallbacks: (radial_fallback, tangent_fallback),
            } => {
                let at = ellipse.at(values, context, parameter);
                let radial = Direction::of(
                    at.point - circle.center.at(values),
                    radial_fallback,
                    context,
                );
                let tangent = Direction::of(at.tangent, tangent_fallback, context);
                let scale = context.scale;
                let across = radial.back_from_unit(tangent.unit * scale);
                let turning = tangent.back_from_unit(radial.unit * scale);
                ellipse.push_point(values, context, &at, gradient, across);
                circle.center.push(gradient, -across);
                ellipse.push_tangent(values, context, &at, gradient, turning);
                gradient.push((parameter, across.dot(at.tangent) + turning.dot(at.bend)));
                radial.unit.dot(tangent.unit) * scale
            }
            Self::EllipseTouch {
                line,
                point,
                ellipse,
            } => ellipse_touch(&line, point, &ellipse, values, context, gradient),
            Self::EllipseMiddle {
                point,
                ends,
                ellipse,
            } => ellipse_middle(point, ends, &ellipse, values, context, gradient),
            Self::EllipseTouchCircle {
                circle,
                point,
                ellipse,
                fallback,
            } => {
                let radius = Direction::of(
                    point.at(values) - circle.center.at(values),
                    fallback,
                    context,
                );
                let (value, by_tangent) = ellipse_touch_along(
                    radius.unit.perp(),
                    point,
                    &ellipse,
                    values,
                    context,
                    gradient,
                );
                let by_radius = radius.back_from_unit(-by_tangent.perp());
                point.push(gradient, by_radius);
                circle.center.push(gradient, -by_radius);
                value
            }
            Self::SplineFoot {
                point,
                ref spline,
                parameter,
                fallback,
            } => {
                let at = spline.at(values, parameter);
                let along = at.tangent_line(fallback, context);
                let offset = point.at(values) - at.point;
                let turning = along.back_from_unit(offset);
                point.push(gradient, along.unit);
                spline.push_point(&at, gradient, -along.unit);
                spline.push_tangent(&at, gradient, turning);
                gradient.push((parameter, turning.dot(at.bend) - along.unit.dot(at.tangent)));
                along.unit.dot(offset)
            }
            Self::SplineDistance {
                point,
                ref spline,
                parameter,
                fallback,
                side,
                value,
            } => {
                let at = spline.at(values, parameter);
                let along = at.tangent_line(fallback, context);
                let offset = point.at(values) - at.point;
                let normal = along.unit.perp() * side;
                let turning = along.back_from_unit(-offset.perp() * side);
                point.push(gradient, normal);
                spline.push_point(&at, gradient, -normal);
                spline.push_tangent(&at, gradient, turning);
                gradient.push((parameter, turning.dot(at.bend) - normal.dot(at.tangent)));
                normal.dot(offset) - value
            }
            Self::EndCurvature { end, circle, side } => {
                let bend = end.bend(values, context);
                let radius = circle.radius(values);
                let scale = context.scale;
                end.push(gradient, &bend, radius * scale);
                circle.push_radius(values, context, gradient, bend.value * scale);
                (bend.value * radius - side) * scale
            }
            Self::MatchedCurvature(first, second) => {
                let squared = context.scale * context.scale;
                let (one, other) = (first.bend(values, context), second.bend(values, context));
                first.push(gradient, &one, squared);
                second.push(gradient, &other, squared);
                (one.value + other.value) * squared
            }
            Self::OnSpline {
                point,
                ref spline,
                parameter,
                along,
            } => {
                let at = spline.at(values, parameter);
                point.push(gradient, along);
                spline.push_point(&at, gradient, -along);
                gradient.push((parameter, -along.dot(at.tangent)));
                along.dot(point.at(values) - at.point)
            }
            Self::SplineOnLine {
                ref spline,
                parameter,
                line,
                side,
                value,
            } => {
                let at = spline.at(values, parameter);
                let direction = line.direction(values, context);
                let offset = at.point - line.start.at(values);
                let normal = direction.unit.perp() * side;
                spline.push_point(&at, gradient, normal);
                gradient.push((parameter, normal.dot(at.tangent)));
                line.start.push(gradient, -normal);
                line.push_vector(gradient, direction.back_from_unit(-offset.perp() * side));
                direction.unit.perp_dot(offset) * side - value
            }
            Self::CurvesMeet {
                ref first,
                ref second,
                parameters: (one, other),
                along,
            } => {
                let at = first.at(values, context, one);
                let there = second.at(values, context, other);
                at.push_point(values, context, gradient, along);
                gradient.push((one, along.dot(at.tangent())));
                there.push_point(values, context, gradient, -along);
                gradient.push((other, -along.dot(there.tangent())));
                along.dot(at.point() - there.point())
            }
            Self::CurvesAlong {
                ref first,
                ref second,
                parameters: (one, other),
                fallbacks: (first_fallback, second_fallback),
            } => {
                let at = first.at(values, context, one);
                let there = second.at(values, context, other);
                let one_way = at.tangent_line(first_fallback, context);
                let other_way = there.tangent_line(second_fallback, context);
                let scale = context.scale;
                let turning = one_way.back_from_unit(-other_way.unit.perp() * scale);
                let other_turning = other_way.back_from_unit(one_way.unit.perp() * scale);
                at.push_tangent(values, context, gradient, turning);
                gradient.push((one, turning.dot(at.bend())));
                there.push_tangent(values, context, gradient, other_turning);
                gradient.push((other, other_turning.dot(there.bend())));
                one_way.unit.perp_dot(other_way.unit) * scale
            }
            Self::CurvesFoot {
                ref first,
                ref second,
                parameters: (one, other),
                fallback,
            } => {
                let at = first.at(values, context, one);
                let there = second.at(values, context, other);
                let along = at.tangent_line(fallback, context);
                let offset = there.point() - at.point();
                let turning = along.back_from_unit(offset);
                there.push_point(values, context, gradient, along.unit);
                gradient.push((other, along.unit.dot(there.tangent())));
                at.push_point(values, context, gradient, -along.unit);
                at.push_tangent(values, context, gradient, turning);
                gradient.push((one, turning.dot(at.bend()) - along.unit.dot(at.tangent())));
                along.unit.dot(offset)
            }
            Self::CurvesGap {
                ref first,
                ref second,
                parameters: (one, other),
                fallback,
                side,
                value,
            } => {
                let at = first.at(values, context, one);
                let there = second.at(values, context, other);
                let along = at.tangent_line(fallback, context);
                let offset = there.point() - at.point();
                let normal = along.unit.perp() * side;
                let turning = along.back_from_unit(-offset.perp() * side);
                there.push_point(values, context, gradient, normal);
                gradient.push((other, normal.dot(there.tangent())));
                at.push_point(values, context, gradient, -normal);
                at.push_tangent(values, context, gradient, turning);
                gradient.push((one, turning.dot(at.bend()) - normal.dot(at.tangent())));
                normal.dot(offset) - value
            }
            Self::EllipsesTouch {
                point,
                ref first,
                ref second,
                fallbacks,
            } => ellipses_touch(point, (first, second), fallbacks, values, context, gradient),
            Self::NormalAngle {
                line,
                point,
                ref ellipse,
                fallback,
                ellipse_first,
                reversed,
                radians,
            } => {
                let mut unused = Vec::new();
                let (_, normal) =
                    ellipse_touch_along(Vector2::X, point, ellipse, values, context, &mut unused);
                let along = line.direction(values, context);
                let across = Direction::of(normal, fallback, context);
                let (first, second) = if ellipse_first {
                    (&across, &along)
                } else {
                    (&along, &across)
                };
                let scale = context.scale;
                let angle = first
                    .unit
                    .perp_dot(second.unit)
                    .atan2(first.unit.dot(second.unit));
                let by_first = first.back_from_unit(-first.unit.perp() * scale);
                let by_second = second.back_from_unit(second.unit.perp() * scale);
                let (by_line, by_normal) = if ellipse_first {
                    (by_second, by_first)
                } else {
                    (by_first, by_second)
                };
                line.push_vector(gradient, by_line);
                ellipse_touch_along(by_normal, point, ellipse, values, context, gradient);
                let turn = if reversed { PI } else { 0.0 };
                wrap_angle(angle + turn - radians) * scale
            }
            Self::SameLength(ref first, ref second) => {
                first.measure(values, context, gradient, 1.0)
                    - second.measure(values, context, gradient, -1.0)
            }
            Self::SplineAlongLine {
                ref spline,
                parameter,
                line,
                fallback,
            } => {
                let at = spline.at(values, parameter);
                let first = line.direction(values, context);
                let second = at.tangent_line(fallback, context);
                let scale = context.scale;
                line.push_vector(gradient, first.back_from_unit(-second.unit.perp() * scale));
                let turning = second.back_from_unit(first.unit.perp() * scale);
                spline.push_tangent(&at, gradient, turning);
                gradient.push((parameter, turning.dot(at.bend)));
                first.unit.perp_dot(second.unit) * scale
            }
            Self::SplineOnCircle {
                ref spline,
                parameter,
                circle,
                fallback,
                side,
                value,
            } => {
                let at = spline.at(values, parameter);
                let direction =
                    Direction::of(at.point - circle.center.at(values), fallback, context);
                let outward = direction.unit * side;
                spline.push_point(&at, gradient, outward);
                gradient.push((parameter, outward.dot(at.tangent)));
                circle.center.push(gradient, -outward);
                circle.push_radius(values, context, gradient, -side);
                (direction.length - circle.radius(values)) * side - value
            }
            Self::SplineAcrossRadius {
                ref spline,
                parameter,
                circle,
                fallbacks: (radial_fallback, tangent_fallback),
            } => {
                let at = spline.at(values, parameter);
                let radial = Direction::of(
                    at.point - circle.center.at(values),
                    radial_fallback,
                    context,
                );
                let tangent = at.tangent_line(tangent_fallback, context);
                let scale = context.scale;
                let across = radial.back_from_unit(tangent.unit * scale);
                let turning = tangent.back_from_unit(radial.unit * scale);
                spline.push_point(&at, gradient, across);
                circle.center.push(gradient, -across);
                spline.push_tangent(&at, gradient, turning);
                gradient.push((parameter, across.dot(at.tangent) + turning.dot(at.bend)));
                radial.unit.dot(tangent.unit) * scale
            }
            Self::OnBisector { point, chord } => {
                let direction = chord.direction(values, context);
                let middle = chord.start.at(values).midpoint(chord.end.at(values));
                let offset = point.at(values) - middle;
                point.push(gradient, direction.unit);
                chord.start.push(gradient, -direction.unit * 0.5);
                chord.end.push(gradient, -direction.unit * 0.5);
                chord.push_vector(gradient, direction.back_from_unit(offset));
                direction.unit.dot(offset)
            }
            Self::ArcBulge { point, chord, arc } => {
                let direction = chord.direction(values, context);
                let offset = point.at(values) - arc.center.at(values);
                let outward = direction.unit.perp();
                point.push(gradient, outward);
                arc.center.push(gradient, -outward);
                chord.push_vector(gradient, direction.back_from_unit(-offset.perp()));
                arc.push_radius(values, context, gradient, 1.0);
                direction.unit.perp_dot(offset) + arc.radius(values)
            }
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
            } => circle_tangency(
                (first, second),
                fallback,
                contact,
                values,
                context,
                gradient,
            ),
            Self::CircleGap {
                first,
                second,
                fallback,
                contact,
                value,
            } => {
                let tangency = circle_tangency(
                    (first, second),
                    fallback,
                    contact,
                    values,
                    context,
                    gradient,
                );
                match contact {
                    Contact::External => tangency - value,
                    Contact::Internal { .. } => tangency + value,
                }
            }
            Self::LineGap {
                line,
                circle,
                side,
                value,
            } => {
                let distance =
                    signed_distance(circle.center, &line, values, context, gradient, side);
                circle.push_radius(values, context, gradient, -1.0);
                distance - circle.radius(values) - value
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
            Self::ArcLength { from, to, value } => {
                let (first, second) = (
                    from.direction(values, context),
                    to.direction(values, context),
                );
                let turn = first
                    .unit
                    .perp_dot(second.unit)
                    .atan2(first.unit.dot(second.unit))
                    .rem_euclid(TAU);
                let sweep = if turn > 0.0 { turn } else { TAU };
                if first.degenerate {
                    from.push_vector(gradient, Vector2::ZERO);
                } else {
                    from.push_vector(gradient, first.unit * sweep - first.unit.perp());
                }
                if second.degenerate {
                    to.push_vector(gradient, Vector2::ZERO);
                } else {
                    to.push_vector(gradient, second.unit.perp() * first.length / second.length);
                }
                first.length * sweep - value
            }
            Self::Middle {
                point,
                ends: (a, b),
                along,
            } => {
                point.push(gradient, along);
                a.push(gradient, -along / 2.0);
                b.push(gradient, -along / 2.0);
                along.dot(point.at(values) - (a.at(values) + b.at(values)) / 2.0)
            }
            Self::MirrorMiddle {
                first,
                second,
                line,
            } => {
                let direction = line.direction(values, context);
                let middle = (first.at(values) + second.at(values)) / 2.0;
                let offset = middle - line.start.at(values);
                let normal = direction.unit.perp();
                first.push(gradient, normal / 2.0);
                second.push(gradient, normal / 2.0);
                line.start.push(gradient, -normal);
                line.push_vector(gradient, direction.back_from_unit(-offset.perp()));
                direction.unit.perp_dot(offset)
            }
            Self::MirrorAcross {
                first,
                second,
                line,
            } => {
                let direction = line.direction(values, context);
                let span = second.at(values) - first.at(values);
                second.push(gradient, direction.unit);
                first.push(gradient, -direction.unit);
                line.push_vector(gradient, direction.back_from_unit(span));
                direction.unit.dot(span)
            }
            Self::Offset {
                from,
                to,
                along,
                side,
                value,
            } => {
                to.push(gradient, along * side);
                from.push(gradient, -along * side);
                side * along.dot(to.at(values) - from.at(values)) - value
            }
            Self::LineOffset {
                point,
                line,
                along,
                side,
                value,
            } => {
                let direction = line.direction(values, context);
                let offset = point.at(values) + along * (side * value) - line.start.at(values);
                let normal = direction.unit.perp();
                point.push(gradient, normal);
                line.start.push(gradient, -normal);
                line.push_vector(gradient, direction.back_from_unit(-offset.perp()));
                direction.unit.perp_dot(offset)
            }
            Self::CircleDistance {
                point,
                circle,
                fallback,
                side,
                value,
            } => {
                let direction = Direction::of(
                    point.at(values) - circle.center.at(values),
                    fallback,
                    context,
                );
                point.push(gradient, direction.unit * side);
                circle.center.push(gradient, -direction.unit * side);
                circle.push_radius(values, context, gradient, -side);
                side * (direction.length - circle.radius(values)) - value
            }
        }
    }
}

fn fixed_coordinate(a: PointHandle, b: PointHandle, coordinate: fn(Point2) -> f64) -> Option<f64> {
    match (a, b) {
        (PointHandle::Fixed(position), _) | (_, PointHandle::Fixed(position)) => {
            Some(coordinate(position))
        }
        (PointHandle::Variable(_), PointHandle::Variable(_)) => None,
    }
}

fn circle_tangency(
    (first, second): (CircleHandle, CircleHandle),
    fallback: Vector2,
    contact: Contact,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
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
            let larger_first = if (first_radius - second_radius).abs() > context.degenerate_length {
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

fn on_ellipse(
    point: PointHandle,
    ellipse: &EllipseHandle,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
    let frame = ellipse.frame(values, context);
    let (unit, across) = (frame.axis.unit, frame.axis.unit.perp());
    let (a, b) = (frame.major, frame.minor);
    let offset = point.at(values) - ellipse.center.at(values);
    let (x, y) = (offset.dot(unit), offset.dot(across));
    let by_x = b * x / (a * a);
    let by_y = y / b;
    let toward = unit * by_x + across * by_y;
    point.push(gradient, toward);
    ellipse.center.push(gradient, -toward);
    ellipse.push(
        values,
        context,
        &frame,
        gradient,
        &EllipsePartials {
            axis: offset * by_x - offset.perp() * by_y,
            major: -b * x * x / (a * a * a),
            minor: x * x / (2.0 * a * a) - y * y / (2.0 * b * b) - 0.5,
        },
    );
    b * x * x / (2.0 * a * a) + y * y / (2.0 * b) - b / 2.0
}

fn ellipse_tangent(
    line: &LineHandle,
    ellipse: &EllipseHandle,
    (side, gap): (f64, f64),
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
    let distance = signed_distance(ellipse.center, line, values, context, gradient, 1.0);
    let direction = line.direction(values, context);
    let normal = direction.unit.perp();
    let frame = ellipse.frame(values, context);
    let (unit, across) = (frame.axis.unit, frame.axis.unit.perp());
    let (a, b) = (frame.major, frame.minor);
    let (along, sideways) = (normal.dot(unit), normal.dot(across));
    let reach = (a * a * along * along + b * b * sideways * sideways).sqrt();
    if reach > context.degenerate_length && reach.is_finite() {
        let by_along = -side * a * a * along / reach;
        let by_sideways = -side * b * b * sideways / reach;
        let by_normal = unit * by_along + across * by_sideways;
        line.push_vector(gradient, direction.back_from_unit(-by_normal.perp()));
        ellipse.push(
            values,
            context,
            &frame,
            gradient,
            &EllipsePartials {
                axis: normal * by_along - normal.perp() * by_sideways,
                major: -side * a * along * along / reach,
                minor: -side * b * sideways * sideways / reach,
            },
        );
    }
    distance - side * (reach + gap)
}

fn ellipse_touch(
    line: &LineHandle,
    point: PointHandle,
    ellipse: &EllipseHandle,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
    let direction = line.direction(values, context);
    let (value, by_tangent) =
        ellipse_touch_along(direction.unit, point, ellipse, values, context, gradient);
    line.push_vector(gradient, direction.back_from_unit(by_tangent));
    value
}

fn ellipse_touch_along(
    tangent: Vector2,
    point: PointHandle,
    ellipse: &EllipseHandle,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> (f64, Vector2) {
    let frame = ellipse.frame(values, context);
    let (unit, across) = (frame.axis.unit, frame.axis.unit.perp());
    let (a, b) = (frame.major, frame.minor);
    let offset = point.at(values) - ellipse.center.at(values);
    let (x, y) = (offset.dot(unit), offset.dot(across));
    let scale = a * b;
    let (on_unit, on_across) = (tangent.dot(unit), tangent.dot(across));
    let normal = unit * (x * b * b) + across * (y * a * a);
    let total = tangent.dot(normal);
    let by_x = b * b * on_unit / scale;
    let by_y = a * a * on_across / scale;
    let toward = unit * by_x + across * by_y;
    point.push(gradient, toward);
    ellipse.center.push(gradient, -toward);
    let direct = (tangent * (x * b * b) - tangent.perp() * (y * a * a)) / scale;
    ellipse.push(
        values,
        context,
        &frame,
        gradient,
        &EllipsePartials {
            axis: offset * by_x - offset.perp() * by_y + direct,
            major: 2.0 * a * y * on_across / scale - total / (a * scale),
            minor: 2.0 * b * x * on_unit / scale - total / (b * scale),
        },
    );
    (total / scale, normal / scale)
}

fn ellipses_touch(
    point: PointHandle,
    (first, second): (&EllipseHandle, &EllipseHandle),
    (first_fallback, second_fallback): (Vector2, Vector2),
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
    let mut unused = Vec::new();
    let (_, first_normal) =
        ellipse_touch_along(Vector2::X, point, first, values, context, &mut unused);
    let (_, second_normal) =
        ellipse_touch_along(Vector2::X, point, second, values, context, &mut unused);
    let one_way = Direction::of(first_normal, first_fallback, context);
    let other_way = Direction::of(second_normal, second_fallback, context);
    let scale = context.scale;
    let turning = one_way.back_from_unit(-other_way.unit.perp() * scale);
    let other_turning = other_way.back_from_unit(one_way.unit.perp() * scale);
    ellipse_touch_along(turning, point, first, values, context, gradient);
    ellipse_touch_along(other_turning, point, second, values, context, gradient);
    one_way.unit.perp_dot(other_way.unit) * scale
}

struct EllipseTurn {
    angle: f64,
    by_point: Vector2,
    partials: EllipsePartials,
}

fn ellipse_turn(
    point: PointHandle,
    ellipse: &EllipseHandle,
    frame: &EllipseFrame,
    values: &[f64],
) -> EllipseTurn {
    let (unit, across) = (frame.axis.unit, frame.axis.unit.perp());
    let (a, b) = (frame.major, frame.minor);
    let offset = point.at(values) - ellipse.center.at(values);
    let (x, y) = (offset.dot(unit) / a, offset.dot(across) / b);
    let spread = x * x + y * y;
    let angle = y.atan2(x);
    if !(spread > f64::EPSILON && spread.is_finite()) {
        return EllipseTurn {
            angle,
            by_point: Vector2::ZERO,
            partials: EllipsePartials {
                axis: Vector2::ZERO,
                major: 0.0,
                minor: 0.0,
            },
        };
    }
    let (by_x, by_y) = (-y / spread, x / spread);
    EllipseTurn {
        angle,
        by_point: unit * (by_x / a) + across * (by_y / b),
        partials: EllipsePartials {
            axis: offset * (by_x / a) - offset.perp() * (by_y / b),
            major: -by_x * x / a,
            minor: -by_y * y / b,
        },
    }
}

fn ellipse_middle(
    point: PointHandle,
    (start, end): (PointHandle, PointHandle),
    ellipse: &EllipseHandle,
    values: &[f64],
    context: &Context,
    gradient: &mut Gradient,
) -> f64 {
    let frame = ellipse.frame(values, context);
    let a = frame.major;
    let turns = [
        (point, 1.0, ellipse_turn(point, ellipse, &frame, values)),
        (start, -0.5, ellipse_turn(start, ellipse, &frame, values)),
        (end, -0.5, ellipse_turn(end, ellipse, &frame, values)),
    ];
    let [(_, _, at), (_, _, from), (_, _, to)] = &turns;
    let sweep = (to.angle - from.angle).rem_euclid(TAU);
    let off = wrap_angle(at.angle - from.angle - sweep / 2.0);
    let mut partials = EllipsePartials {
        axis: Vector2::ZERO,
        major: off,
        minor: 0.0,
    };
    for (handle, weight, turn) in &turns {
        let factor = a * weight;
        handle.push(gradient, turn.by_point * factor);
        ellipse.center.push(gradient, -turn.by_point * factor);
        partials.axis += turn.partials.axis * factor;
        partials.major += turn.partials.major * factor;
        partials.minor += turn.partials.minor * factor;
    }
    ellipse.push(values, context, &frame, gradient, &partials);
    a * off
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

    fn ellipse(center: usize, major: usize, minor: usize) -> EllipseHandle {
        EllipseHandle {
            center: point(center),
            major: point(major),
            minor: RadiusHandle::Variable(minor),
            fallback: Vector2::X,
        }
    }

    fn values() -> Vec<f64> {
        vec![
            0.3, -1.2, 4.1, 0.7, 2.2, 3.9, -1.5, 2.8, 1.1, 5.3, 1.7, 2.4, -0.6, 0.9, 3.3, -2.1,
            0.37, 0.81,
        ]
    }

    fn spline(first: usize, count: usize) -> Arc<SplineHandle> {
        let (degree, knots) = crate::curve::clamped_knots(count);
        Arc::new(SplineHandle {
            points: (0..count).map(|index| point(first + 2 * index)).collect(),
            degree,
            knots,
            weights: None,
            periodic: false,
        })
    }

    fn conic(first: usize) -> Arc<SplineHandle> {
        Arc::new(SplineHandle {
            points: (0..3).map(|index| point(first + 2 * index)).collect(),
            degree: 2,
            knots: crate::curve::CONIC_KNOTS.to_vec(),
            weights: Some(vec![1.0, 2.5, 1.0]),
            periodic: false,
        })
    }

    fn periodic(first: usize, count: usize) -> Arc<SplineHandle> {
        let (degree, knots) = crate::curve::periodic_knots(count);
        Arc::new(SplineHandle {
            points: (0..count + degree)
                .map(|index| point(first + 2 * (index % count)))
                .collect(),
            degree,
            knots,
            weights: None,
            periodic: true,
        })
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
            Form::ArcLength {
                from: line(4, 0),
                to: line(4, 12),
                value: 3.0,
            },
            Form::ArcLength {
                from: line(4, 12),
                to: line(4, 0),
                value: 3.0,
            },
            Form::Middle {
                point: point(12),
                ends: (point(0), point(4)),
                along: Vector2::X,
            },
            Form::Middle {
                point: point(12),
                ends: (point(0), PointHandle::Fixed(Point2::new(2.0, -1.0))),
                along: Vector2::Y,
            },
            Form::MirrorMiddle {
                first: point(0),
                second: point(12),
                line: line(4, 6),
            },
            Form::MirrorAcross {
                first: point(0),
                second: point(12),
                line: line(4, 6),
            },
            Form::OnBisector {
                point: point(12),
                chord: line(0, 4),
            },
            Form::ArcBulge {
                point: point(12),
                chord: line(2, 4),
                arc: arc(6, 2),
            },
            Form::Offset {
                from: point(2),
                to: point(8),
                along: Vector2::Y,
                side: -1.0,
                value: 1.25,
            },
            Form::LineOffset {
                point: point(8),
                line: line(0, 2),
                along: Vector2::X,
                side: -1.0,
                value: 1.5,
            },
            Form::LineOffset {
                point: circle(6, 10).center,
                line: line(2, 4),
                along: Vector2::Y,
                side: 1.0,
                value: 0.75,
            },
            Form::CircleDistance {
                point: point(0),
                circle: circle(6, 10),
                fallback: Vector2::X,
                side: -1.0,
                value: 0.5,
            },
            Form::CircleDistance {
                point: point(14),
                circle: arc(4, 12),
                fallback: Vector2::X,
                side: 1.0,
                value: 2.0,
            },
            Form::CircleGap {
                first: circle(6, 10),
                second: arc(12, 14),
                fallback: Vector2::X,
                contact: Contact::External,
                value: 0.75,
            },
            Form::CircleGap {
                first: arc(4, 0),
                second: circle(12, 11),
                fallback: Vector2::X,
                contact: Contact::Internal { larger_first: 1.0 },
                value: 0.25,
            },
            Form::LineGap {
                line: line(0, 2),
                circle: arc(6, 8),
                side: -1.0,
                value: 1.5,
            },
            Form::LineGap {
                line: line(0, 2),
                circle: circle(6, 11),
                side: 1.0,
                value: 0.5,
            },
            Form::OnSpline {
                point: point(14),
                spline: spline(0, 5),
                parameter: 16,
                along: Vector2::X,
            },
            Form::OnSpline {
                point: point(14),
                spline: spline(2, 3),
                parameter: 17,
                along: Vector2::Y,
            },
            Form::SplineOnLine {
                spline: spline(0, 4),
                parameter: 16,
                line: line(10, 12),
                side: 1.0,
                value: 0.0,
            },
            Form::SplineOnLine {
                spline: spline(2, 5),
                parameter: 17,
                line: line(0, 14),
                side: -1.0,
                value: 1.25,
            },
            Form::CurvesMeet {
                first: CurveHandle::Spline(spline(0, 4)),
                second: CurveHandle::Spline(spline(8, 4)),
                parameters: (16, 17),
                along: Vector2::X,
            },
            Form::CurvesMeet {
                first: CurveHandle::Spline(spline(2, 3)),
                second: CurveHandle::Spline(spline(6, 5)),
                parameters: (17, 16),
                along: Vector2::Y,
            },
            Form::CurvesAlong {
                first: CurveHandle::Spline(spline(0, 5)),
                second: CurveHandle::Spline(spline(6, 5)),
                parameters: (16, 17),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::CurvesMeet {
                first: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                second: CurveHandle::Spline(spline(4, 4)),
                parameters: (16, 17),
                along: Vector2::Y,
            },
            Form::CurvesMeet {
                first: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                second: CurveHandle::Ellipse(ellipse(12, 6, 11)),
                parameters: (16, 17),
                along: Vector2::X,
            },
            Form::CurvesAlong {
                first: CurveHandle::Spline(spline(4, 5)),
                second: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                parameters: (16, 17),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::CurvesAlong {
                first: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                second: CurveHandle::Ellipse(ellipse(12, 6, 11)),
                parameters: (17, 16),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::CurvesFoot {
                first: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                second: CurveHandle::Spline(spline(4, 4)),
                parameters: (16, 17),
                fallback: Vector2::X,
            },
            Form::CurvesFoot {
                first: CurveHandle::Spline(periodic(4, 4)),
                second: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                parameters: (17, 16),
                fallback: Vector2::Y,
            },
            Form::CurvesGap {
                first: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                second: CurveHandle::Ellipse(ellipse(12, 6, 11)),
                parameters: (16, 17),
                fallback: Vector2::X,
                side: -1.0,
                value: 0.75,
            },
            Form::CurvesGap {
                first: CurveHandle::Spline(conic(4)),
                second: CurveHandle::Ellipse(ellipse(0, 2, 10)),
                parameters: (17, 16),
                fallback: Vector2::Y,
                side: 1.0,
                value: 0.5,
            },
            Form::EllipsesTouch {
                point: point(14),
                first: ellipse(0, 2, 10),
                second: ellipse(4, 8, 11),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::NormalAngle {
                line: line(4, 8),
                point: point(14),
                ellipse: ellipse(0, 2, 10),
                fallback: Vector2::X,
                ellipse_first: false,
                reversed: false,
                radians: 0.4,
            },
            Form::NormalAngle {
                line: line(6, 12),
                point: point(14),
                ellipse: ellipse(0, 2, 11),
                fallback: Vector2::Y,
                ellipse_first: true,
                reversed: true,
                radians: -1.1,
            },
            Form::EllipsesTouch {
                point: point(14),
                first: ellipse(0, 2, 10),
                second: EllipseHandle {
                    center: PointHandle::Fixed(Point2::new(0.5, -0.25)),
                    major: point(6),
                    minor: RadiusHandle::Fixed(2.0),
                    fallback: Vector2::X,
                },
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::SameLength(
                LengthOf::Spline(spline(0, 5)),
                LengthOf::Spline(spline(4, 6)),
            ),
            Form::SameLength(LengthOf::Line(line(0, 14)), LengthOf::Spline(spline(2, 4))),
            Form::SplineAlongLine {
                spline: spline(0, 6),
                parameter: 17,
                line: line(12, 14),
                fallback: Vector2::X,
            },
            Form::SplineOnCircle {
                spline: spline(0, 4),
                parameter: 16,
                circle: circle(12, 10),
                fallback: Vector2::X,
                side: 1.0,
                value: 0.0,
            },
            Form::SplineOnCircle {
                spline: spline(2, 4),
                parameter: 17,
                circle: arc(12, 14),
                fallback: Vector2::X,
                side: -1.0,
                value: 0.5,
            },
            Form::SplineAcrossRadius {
                spline: spline(0, 5),
                parameter: 17,
                circle: arc(12, 14),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::OnEllipse {
                point: point(4),
                ellipse: ellipse(0, 2, 10),
            },
            Form::OnEllipse {
                point: point(12),
                ellipse: EllipseHandle {
                    center: PointHandle::Fixed(Point2::new(0.5, -0.25)),
                    major: point(6),
                    minor: RadiusHandle::Fixed(2.0),
                    fallback: Vector2::X,
                },
            },
            Form::EllipseTangent {
                line: line(6, 8),
                ellipse: ellipse(0, 2, 11),
                side: 1.0,
                value: 0.0,
            },
            Form::EllipseTangent {
                line: line(12, 4),
                ellipse: ellipse(14, 2, 10),
                side: -1.0,
                value: 0.75,
            },
            Form::EllipseTouch {
                line: line(4, 6),
                point: point(12),
                ellipse: ellipse(0, 2, 10),
            },
            Form::EllipseTouch {
                line: line(8, 12),
                point: point(8),
                ellipse: ellipse(14, 6, 11),
            },
            Form::SplineFoot {
                point: point(12),
                spline: conic(0),
                parameter: 16,
                fallback: Vector2::X,
            },
            Form::SplineOnLine {
                spline: conic(4),
                parameter: 17,
                line: line(0, 14),
                side: 1.0,
                value: 0.5,
            },
            Form::SameLength(LengthOf::Spline(conic(2)), LengthOf::Spline(periodic(6, 4))),
            Form::SplineDistance {
                point: point(14),
                spline: periodic(0, 5),
                parameter: 17,
                fallback: Vector2::Y,
                side: -1.0,
                value: 0.75,
            },
            Form::EllipseTouchCircle {
                circle: arc(14, 8),
                point: point(12),
                ellipse: ellipse(0, 2, 10),
                fallback: Vector2::X,
            },
            Form::EllipseTouchCircle {
                circle: circle(6, 11),
                point: point(4),
                ellipse: ellipse(14, 8, 10),
                fallback: Vector2::Y,
            },
            Form::EllipseMiddle {
                point: point(4),
                ends: (point(6), point(12)),
                ellipse: ellipse(0, 2, 10),
            },
            Form::EllipseMiddle {
                point: point(14),
                ends: (point(8), PointHandle::Fixed(Point2::new(-1.0, 2.5))),
                ellipse: ellipse(12, 6, 11),
            },
            Form::Through {
                point: point(14),
                terms: Arc::from([(point(0), 0.25), (point(4), 0.5), (point(8), 0.25)]),
                along: Vector2::Y,
            },
            Form::EllipseFoot {
                point: point(12),
                ellipse: ellipse(0, 2, 10),
                parameter: 16,
                fallback: Vector2::X,
            },
            Form::OnMinorAxis {
                point: point(14),
                axis: line(4, 8),
            },
            Form::EllipseDistance {
                point: point(14),
                ellipse: ellipse(4, 8, 11),
                parameter: 17,
                fallback: Vector2::Y,
                side: -1.0,
                value: 0.75,
            },
            Form::EllipseOnCircle {
                ellipse: ellipse(0, 2, 10),
                parameter: 17,
                circle: circle(12, 11),
                fallback: Vector2::X,
                side: 1.0,
                value: 0.0,
            },
            Form::EllipseOnCircle {
                ellipse: ellipse(4, 6, 11),
                parameter: 16,
                circle: arc(12, 14),
                fallback: Vector2::X,
                side: -1.0,
                value: 0.5,
            },
            Form::EllipseAcrossRadius {
                ellipse: ellipse(0, 2, 10),
                parameter: 16,
                circle: arc(12, 14),
                fallbacks: (Vector2::X, Vector2::Y),
            },
            Form::EllipseAcrossRadius {
                ellipse: EllipseHandle {
                    center: PointHandle::Fixed(Point2::new(0.5, -0.25)),
                    major: point(6),
                    minor: RadiusHandle::Fixed(2.0),
                    fallback: Vector2::X,
                },
                parameter: 17,
                circle: circle(8, 11),
                fallbacks: (Vector2::X, Vector2::Y),
            },
        ]
    }

    #[test]
    fn every_gradient_matches_central_finite_differences() {
        let base = values();
        for form in forms() {
            let equation = Equation {
                owner: None,
                form: form.clone(),
            };
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
                let numeric = (equation.residual(&plus, &CONTEXT, &mut Vec::new())
                    - equation.residual(&minus, &CONTEXT, &mut Vec::new()))
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
        let collapsed = vec![1.0; 18];
        for form in forms() {
            let equation = Equation {
                owner: None,
                form: form.clone(),
            };
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
