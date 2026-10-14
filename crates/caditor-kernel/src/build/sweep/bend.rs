use std::f64::consts::TAU;

use caditor_geometry::{Plane, Point3, Vector3};

use super::{PathSegment, SMOOTH_TURN, degenerate, path::Leg, pipe::PipeError};
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve},
    interval::Interval,
    tolerance::LINEAR_RESOLUTION,
};

const NEWTON_STEPS: usize = 64;
const DIFFERENCE_STEP: f64 = 1e-7;
const LEAST_MISS: f64 = 1e-10;
const RELATIVE_MISS: f64 = 1e-12;
const PLANE_SAMPLES: usize = 32;
const CHORD_STEPS: usize = 80;
const TANGENT_SLACK: f64 = 0.1 * SMOOTH_TURN;
const CUBIC: usize = 3;
const BEZIER_KNOTS: [f64; 8] = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rounding {
    pub(super) incoming_end: f64,
    pub(super) outgoing_start: f64,
    pub(super) segment: PathSegment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    End,
}

struct Corner<'a> {
    incoming: &'a Leg,
    outgoing: &'a Leg,
    point: Point3,
    normal: Vector3,
    turn: f64,
    radius: f64,
    key: u128,
}

pub(super) fn rounding(
    incoming: &Leg,
    outgoing: &Leg,
    radius: f64,
    key: u128,
) -> Result<Rounding, PipeError> {
    let leaving = incoming.end_tangent().ok_or_else(degenerate)?;
    let arriving = outgoing.start_tangent().ok_or_else(degenerate)?;
    let corner = Corner {
        incoming,
        outgoing,
        point: incoming.end_point(),
        normal: leaving
            .cross(arriving)
            .try_normalize()
            .ok_or_else(degenerate)?,
        turn: leaving.angle_between(arriving),
        radius,
        key,
    };
    let made = if corner.is_flat() {
        corner.arc()
    } else {
        corner.blend()
    };
    made.ok_or(PipeError::BendTooLong {
        segments: [incoming.segment, outgoing.segment],
    })
}

fn inside(range: Interval, parameter: f64, step: f64) -> f64 {
    if parameter - step >= range.start() {
        parameter - step
    } else {
        parameter + step
    }
}

impl Corner<'_> {
    fn is_flat(&self) -> bool {
        [self.incoming, self.outgoing].into_iter().all(|leg| {
            leg.range.split(PLANE_SAMPLES).all(|parameter| {
                (leg.curve.point(parameter) - self.point)
                    .dot(self.normal)
                    .abs()
                    <= LINEAR_RESOLUTION
            })
        })
    }

    fn centre(&self, leg: &Leg, parameter: f64) -> Option<Point3> {
        let derivatives = leg.curve.evaluate(parameter);
        let tangent = derivatives.first.try_normalize()?;
        let inward = self.normal.cross(tangent).try_normalize()?;
        Some(derivatives.point + inward * self.radius)
    }

    fn miss(&self, incoming: f64, outgoing: f64) -> Option<Vector3> {
        Some(self.centre(self.incoming, incoming)? - self.centre(self.outgoing, outgoing)?)
    }

    fn chord_parameter(&self, leg: &Leg, end: End, distance: f64) -> Option<f64> {
        let range = leg.range;
        let (mut near, mut far) = match end {
            End::Start => (range.start(), range.end()),
            End::End => (range.end(), range.start()),
        };
        if leg.curve.point(far).distance(self.point) < distance {
            return None;
        }
        for _ in 0..CHORD_STEPS {
            let middle = 0.5 * (near + far);
            if leg.curve.point(middle).distance(self.point) < distance {
                near = middle;
            } else {
                far = middle;
            }
        }
        Some(0.5 * (near + far))
    }

    fn cut(&self) -> f64 {
        self.radius * (self.turn / 2.0).tan()
    }

    fn guess(&self, leg: &Leg, end: End) -> f64 {
        self.chord_parameter(leg, end, self.cut())
            .unwrap_or_else(|| match end {
                End::Start => leg.range.end(),
                End::End => leg.range.start(),
            })
    }

    fn tangency(&self) -> Option<(f64, f64)> {
        let (incoming_range, outgoing_range) = (self.incoming.range, self.outgoing.range);
        let mut incoming = self.guess(self.incoming, End::End);
        let mut outgoing = self.guess(self.outgoing, End::Start);
        let across = self.normal.cross(self.incoming.end_tangent()?);
        let along = across.cross(self.normal);
        let allowed = LEAST_MISS.max(self.radius * RELATIVE_MISS);
        for _ in 0..NEWTON_STEPS {
            let miss = self.miss(incoming, outgoing)?;
            if miss.length() <= allowed {
                return Some((incoming, outgoing));
            }
            let incoming_step = inside(
                incoming_range,
                incoming,
                DIFFERENCE_STEP * incoming_range.length(),
            );
            let outgoing_step = inside(
                outgoing_range,
                outgoing,
                DIFFERENCE_STEP * outgoing_range.length(),
            );
            let by_incoming =
                (self.miss(incoming_step, outgoing)? - miss) / (incoming_step - incoming);
            let by_outgoing =
                (self.miss(incoming, outgoing_step)? - miss) / (outgoing_step - outgoing);
            let [a, b, c, d] = [
                by_incoming.dot(along),
                by_outgoing.dot(along),
                by_incoming.dot(across),
                by_outgoing.dot(across),
            ];
            let determinant = a * d - b * c;
            if !determinant.is_normal() {
                return None;
            }
            let (first, second) = (miss.dot(along), miss.dot(across));
            let next_incoming =
                incoming_range.clamp(incoming - (d * first - b * second) / determinant);
            let next_outgoing =
                outgoing_range.clamp(outgoing - (a * second - c * first) / determinant);
            if next_incoming == incoming && next_outgoing == outgoing {
                return None;
            }
            (incoming, outgoing) = (next_incoming, next_outgoing);
        }
        None
    }

    fn arc(&self) -> Option<Rounding> {
        let (incoming, outgoing) = self.tangency()?;
        let centre = self.centre(self.incoming, incoming)?;
        let start = self.incoming.curve.point(incoming);
        let end = self.outgoing.curve.point(outgoing) - centre;
        let x_axis = (start - centre).try_normalize()?;
        let y_axis = self.normal.cross(x_axis);
        let sweep = end.dot(y_axis).atan2(end.dot(x_axis)).rem_euclid(TAU);
        let arriving = x_axis * -sweep.sin() + y_axis * sweep.cos();
        let tangent = self
            .incoming
            .tangent(incoming)?
            .angle_between(y_axis)
            .max(self.outgoing.tangent(outgoing)?.angle_between(arriving));
        if sweep <= SMOOTH_TURN || sweep >= TAU - SMOOTH_TURN || tangent > TANGENT_SLACK {
            return None;
        }
        let frame = Plane::from_frame(centre, self.normal, x_axis)?;
        let circle = Circle::new(frame, self.radius).ok()?;
        Some(Rounding {
            incoming_end: incoming,
            outgoing_start: outgoing,
            segment: PathSegment::new(Curve::Circle(circle), Interval::new(0.0, sweep)?, self.key),
        })
    }

    fn blend(&self) -> Option<Rounding> {
        let cut = self.cut();
        let incoming = self.chord_parameter(self.incoming, End::End, cut)?;
        let outgoing = self.chord_parameter(self.outgoing, End::Start, cut)?;
        let start = self.incoming.curve.point(incoming);
        let end = self.outgoing.curve.point(outgoing);
        let leaving = self.incoming.tangent(incoming)?;
        let arriving = self.outgoing.tangent(outgoing)?;
        let chord = start.distance(end);
        let angle = leaving.angle_between(arriving);
        let handle = if angle > SMOOTH_TURN {
            chord * 2.0 / 3.0 * (angle / 4.0).tan() / (angle / 2.0).sin()
        } else {
            chord / 3.0
        };
        let spline = BSpline::new(
            CUBIC,
            BEZIER_KNOTS.to_vec(),
            vec![
                start,
                start + leaving * handle,
                end - arriving * handle,
                end,
            ],
        )
        .ok()?;
        Some(Rounding {
            incoming_end: incoming,
            outgoing_start: outgoing,
            segment: PathSegment::new(Curve::BSpline(spline), Interval::UNIT, self.key),
        })
    }
}
