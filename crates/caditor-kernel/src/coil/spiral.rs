use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2};
use thiserror::Error;

use super::{AXIS_CLEARANCE, LEAST_COIL_TURNS, LEAST_GAP, MAX_COIL_TURNS, helix::HelixTurn};
use crate::{
    bspline::BSpline,
    curve::Curve,
    error::GeometryError,
    interval::Interval,
    thread::{Handedness, SPANS_PER_TURN, helix_knots},
    tolerance::{LINEAR_RESOLUTION, MAX_SIZE},
};

pub const SPIRAL_TOLERANCE: f64 = LINEAR_RESOLUTION;
const MOST_SPANS_PER_TURN: usize = 16 * SPANS_PER_TURN;
const ROUNDING_SLACK: f64 = 1e-9;
const LEAST_PITCH_STEPS: usize = 64;
const CROSSING_HALVINGS: usize = 60;
const CHECKS_PER_SPAN: [f64; 4] = [0.125, 0.25, 0.75, 0.875];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpiralSpec {
    pub frame: Plane,
    pub radius: f64,
    pub pitch: f64,
    pub turns: f64,
    pub hand: Handedness,
    pub start_angle: f64,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SpiralError {
    #[error("a spiral dimension is not a finite number")]
    NonFinite,
    #[error("the spiral's inner radius must be more than zero")]
    NoRadius,
    #[error("the pitch must be more than zero")]
    NoPitch,
    #[error("the spiral must make at least {LEAST_COIL_TURNS} of a turn, not {turns}")]
    TooFewTurns { turns: f64 },
    #[error("the spiral would turn {turns} times, more than {MAX_COIL_TURNS}")]
    TooManyTurns { turns: f64 },
    #[error("the spiral reaches farther than {MAX_SIZE} mm")]
    TooLarge,
    #[error("the spiral cannot be followed closely enough by its curve")]
    Unfitted,
    #[error("the spiral's geometry cannot be built: {0}")]
    Geometry(#[from] GeometryError),
}

impl SpiralSpec {
    fn values(&self) -> [f64; 4] {
        [self.radius, self.pitch, self.turns, self.start_angle]
    }

    fn growth(&self) -> f64 {
        self.pitch / TAU
    }

    pub fn start_frame(&self) -> Option<Plane> {
        let turn = RigidTransform::rotation_about(
            self.frame.origin(),
            self.frame.normal(),
            self.start_angle,
        )?;
        Some(self.frame.transformed(&turn))
    }

    pub fn radius_at(&self, angle: f64) -> f64 {
        self.radius + self.growth() * angle
    }

    pub fn outer_radius(&self) -> f64 {
        self.radius + self.pitch * self.turns
    }

    pub fn sweep(&self) -> f64 {
        TAU * self.turns
    }

    fn local(&self, angle: f64) -> Point2 {
        Point2::new(angle.cos(), angle.sin()) * self.radius_at(angle)
    }

    fn local_tangent(&self, angle: f64) -> Vector2 {
        let (sin, cos) = angle.sin_cos();
        Vector2::new(cos, sin) * self.growth() + Vector2::new(-sin, cos) * self.radius_at(angle)
    }

    fn placed(&self, frame: &Plane, at: Point2) -> Point3 {
        frame.origin() + frame.x_axis() * at.x + frame.y_axis() * (self.hand.sign() * at.y)
    }

    pub fn point_at(&self, angle: f64) -> Option<Point3> {
        let frame = self.start_frame()?;
        Some(self.placed(&frame, self.local(angle)))
    }

    pub fn start(&self) -> Option<Point3> {
        self.point_at(0.0)
    }

    fn reach_between_turns(&self, across: f64, pitch: f64) -> f64 {
        let growth = pitch / TAU;
        let stretch = |radius: f64| radius.hypot(growth) / radius;
        let gap = (LEAST_GAP * across).max(AXIS_CLEARANCE);
        0.5 * across * (stretch(self.radius) + stretch(self.radius + pitch)) + gap
    }

    pub fn turns_clear_of(&self, across: f64) -> bool {
        self.turns <= 1.0 || self.pitch > self.reach_between_turns(across, self.pitch)
    }

    pub fn least_pitch_for(&self, across: f64) -> f64 {
        (0..LEAST_PITCH_STEPS).fold(across, |pitch, _| self.reach_between_turns(across, pitch))
    }

    pub fn length(&self) -> f64 {
        let growth = self.growth();
        let primitive = |radius: f64| {
            let reach = radius.hypot(growth);
            radius * reach + growth * growth * (radius + reach).ln()
        };
        (primitive(self.outer_radius()) - primitive(self.radius)) / (2.0 * growth)
    }
}

fn checked(spec: &SpiralSpec) -> Result<(), SpiralError> {
    if !spec.values().into_iter().all(f64::is_finite) {
        return Err(SpiralError::NonFinite);
    }
    if spec.radius <= LINEAR_RESOLUTION {
        return Err(SpiralError::NoRadius);
    }
    if spec.pitch <= LINEAR_RESOLUTION {
        return Err(SpiralError::NoPitch);
    }
    if spec.turns < LEAST_COIL_TURNS {
        return Err(SpiralError::TooFewTurns { turns: spec.turns });
    }
    if spec.turns > MAX_COIL_TURNS {
        return Err(SpiralError::TooManyTurns { turns: spec.turns });
    }
    if spec.outer_radius() + spec.frame.origin().abs().max_element() > MAX_SIZE {
        return Err(SpiralError::TooLarge);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: Point2,
    shoulder: Point2,
    weight: f64,
    end: Point2,
}

impl Span {
    fn point(&self, t: f64) -> Point2 {
        let (low, middle, high) = (
            (1.0 - t) * (1.0 - t),
            2.0 * t * (1.0 - t) * self.weight,
            t * t,
        );
        (self.start * low + self.shoulder * middle + self.end * high) / (low + middle + high)
    }
}

fn cross(first: Vector2, second: Vector2) -> f64 {
    first.x * second.y - first.y * second.x
}

fn crossing(
    spec: &SpiralSpec,
    (from, to): (f64, f64),
    middle: Point2,
    towards: Vector2,
) -> Option<f64> {
    let side = |angle: f64| cross(spec.local(angle) - middle, towards);
    let (mut low, mut high) = (from, to);
    let low_side = side(low);
    if low_side * side(high) >= 0.0 {
        return None;
    }
    for _ in 0..CROSSING_HALVINGS {
        let half = 0.5 * (low + high);
        if side(half) * low_side > 0.0 {
            low = half;
        } else {
            high = half;
        }
    }
    Some(0.5 * (low + high))
}

fn fitted_span(spec: &SpiralSpec, from: f64, to: f64) -> Option<Span> {
    let (start, end) = (spec.local(from), spec.local(to));
    let (leaving, arriving) = (spec.local_tangent(from), spec.local_tangent(to));
    let chord = end - start;
    let turning = cross(leaving, arriving);
    if turning.abs() <= f64::EPSILON * leaving.length() * arriving.length() {
        return None;
    }
    let out = cross(chord, arriving) / turning;
    let back = cross(leaving, chord) / turning;
    if !(out > 0.0 && back > 0.0) {
        return None;
    }
    let shoulder = start + leaving * out;
    let middle = (start + end) * 0.5;
    let towards = shoulder - middle;
    let crossing = crossing(spec, (from, to), middle, towards)?;
    let reach = (spec.local(crossing) - middle).dot(towards) / towards.length_squared();
    if !(reach > 0.0 && reach < 1.0) {
        return None;
    }
    Some(Span {
        start,
        shoulder,
        weight: reach / (1.0 - reach),
        end,
    })
}

fn deviation(spec: &SpiralSpec, span: &Span, from: f64) -> f64 {
    CHECKS_PER_SPAN
        .into_iter()
        .map(|t| {
            let point = span.point(t);
            let turned = (point.y.atan2(point.x) - from + PI).rem_euclid(TAU) - PI;
            (point.length() - spec.radius_at(from + turned)).abs()
        })
        .fold(0.0, f64::max)
}

struct Piece {
    start: f64,
    spans: usize,
    step: f64,
}

fn pieces(turns: f64, spans_per_turn: usize) -> Vec<Piece> {
    let count = (turns - ROUNDING_SLACK).ceil().max(1.0);
    let length = turns / count;
    let spans = ((length * spans_per_turn as f64 - ROUNDING_SLACK).ceil() as usize).max(1);
    (0..count as usize)
        .map(|index| Piece {
            start: TAU * length * index as f64,
            spans,
            step: TAU * length / spans as f64,
        })
        .collect()
}

fn fitted_piece(spec: &SpiralSpec, piece: &Piece) -> Option<Vec<Span>> {
    (0..piece.spans)
        .map(|index| {
            let from = piece.start + piece.step * index as f64;
            let span = fitted_span(spec, from, from + piece.step)?;
            (deviation(spec, &span, from) <= SPIRAL_TOLERANCE).then_some(span)
        })
        .collect()
}

fn fitted(spec: &SpiralSpec) -> Result<Vec<Vec<Span>>, SpiralError> {
    let mut spans_per_turn = SPANS_PER_TURN;
    while spans_per_turn <= MOST_SPANS_PER_TURN {
        let fits: Option<Vec<Vec<Span>>> = pieces(spec.turns, spans_per_turn)
            .iter()
            .map(|piece| fitted_piece(spec, piece))
            .collect();
        if let Some(fits) = fits {
            return Ok(fits);
        }
        spans_per_turn *= 2;
    }
    Err(SpiralError::Unfitted)
}

fn evenly_paced(spans: &[Span]) -> Vec<f64> {
    let mut weights = Vec::with_capacity(2 * spans.len() + 1);
    let mut joint = 1.0;
    let mut arriving = None;
    for span in spans {
        let shoulder = arriving.map_or(span.weight, |pace: f64| {
            pace / span.shoulder.distance(span.start)
        });
        weights.extend([joint, shoulder]);
        arriving = Some(shoulder * span.end.distance(span.shoulder));
        joint = shoulder * shoulder / (span.weight * span.weight * joint);
    }
    weights.push(joint);
    weights
}

fn turn_of(spec: &SpiralSpec, frame: &Plane, spans: &[Span]) -> Result<HelixTurn, SpiralError> {
    let mut points = Vec::with_capacity(2 * spans.len() + 1);
    for span in spans {
        points.extend([
            spec.placed(frame, span.start),
            spec.placed(frame, span.shoulder),
        ]);
    }
    let last = spans.last().ok_or(SpiralError::Unfitted)?;
    points.push(spec.placed(frame, last.end));
    let weights = evenly_paced(spans);
    let curve = Curve::BSpline(BSpline::rational(
        2,
        helix_knots(spans.len()),
        points,
        weights,
    )?);
    let range = Interval::new(0.0, spans.len() as f64).ok_or(GeometryError::NonFinite)?;
    Ok(HelixTurn::new(
        curve,
        range,
        (spec.frame.origin(), spec.frame.normal()),
    ))
}

pub fn spiral(spec: &SpiralSpec) -> Result<Vec<HelixTurn>, SpiralError> {
    checked(spec)?;
    let frame = spec.start_frame().ok_or(GeometryError::ZeroDirection)?;
    fitted(spec)?
        .iter()
        .map(|spans| turn_of(spec, &frame, spans))
        .collect()
}
