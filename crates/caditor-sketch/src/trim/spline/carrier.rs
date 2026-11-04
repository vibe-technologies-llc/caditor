use caditor_geometry::Point2;

use super::{MAX_DOUBLINGS, Part, REACH_MARGIN, SplineCourse, SplineSource, WINDOW_SAMPLES};
use crate::{
    curve::BSpline,
    id::EntityId,
    intersect::{Shape, closest_spline_parameter},
    sketch::Sketch,
    trim::Cutter,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SplineCarrier {
    extended: BSpline,
    low: f64,
    high: f64,
    from: f64,
    to: f64,
    ends: (EntityId, EntityId),
}

impl SplineCarrier {
    pub(crate) fn extended(&self) -> &BSpline {
        &self.extended
    }

    pub(crate) fn ends(&self) -> (EntityId, EntityId) {
        self.ends
    }

    pub(crate) fn share_at(&self, point: Point2) -> f64 {
        let parameter = closest_spline_parameter(&self.extended, point);
        let knot = self.from + (self.to - self.from) * parameter;
        (knot - self.low) / (self.high - self.low)
    }

    pub(crate) fn knot_of(&self, share: f64) -> f64 {
        self.low + share * (self.high - self.low)
    }

    pub(crate) fn crossings(&self, shape: &Shape, tolerance: f64) -> Vec<Point2> {
        Cutter::Shape(shape.clone()).spline_cut_positions(&self.extended, tolerance)
    }
}

impl Sketch {
    pub(crate) fn spline_carrier(&self, curve: EntityId, reach: f64) -> Option<SplineCarrier> {
        let source = self.spline_source(curve)?;
        if source.kind.is_closed() {
            return None;
        }
        let own = self.spline(curve)?;
        let ends = self.entity(curve)?.spline_ends()?;
        let (low, high) = source.knotted.domain()?;
        let bound = REACH_MARGIN * reach.max(SplineCourse::reach_of(&own)).max(1.0);
        let before = continuation_length(&source, &own, false, bound)?;
        let after = continuation_length(&source, &own, true, bound)?;
        let (from, to) = (low - before, high + after);
        let extended = source.knotted.between(from, to)?.spline(source.rational)?;
        Some(SplineCarrier {
            extended,
            low,
            high,
            from,
            to,
            ends,
        })
    }
}

fn continuation_length(
    source: &SplineSource,
    own: &BSpline,
    at_end: bool,
    bound: f64,
) -> Option<f64> {
    let (low, high) = source.knotted.domain()?;
    let span = source.knotted.end_span(at_end)?;
    let end_parameter = if at_end { 1.0 } else { 0.0 };
    let [tangent, _] = own.derivatives(end_parameter);
    let outward = (if at_end { tangent } else { -tangent }).try_normalize()?;
    let beyond = |along: f64| if at_end { high + along } else { low - along };
    let mut previous = own.point_at(end_parameter);
    let mut reached = 0.0;
    for doubling in 0..MAX_DOUBLINGS {
        let length = span * 2f64.powi(doubling);
        let step = (length - reached) / WINDOW_SAMPLES as f64;
        for index in 1..=WINDOW_SAMPLES {
            let along = reached + step * index as f64;
            let onward = source
                .knotted
                .homogeneous_at(beyond(along))
                .and_then(Part::position)
                .filter(|point| {
                    point.x.abs().max(point.y.abs()) <= bound
                        && (*point - previous).dot(outward) > 0.0
                });
            match onward {
                Some(point) => previous = point,
                None => return Some(reached + step * (index - 1) as f64),
            }
        }
        reached = length;
    }
    Some(reached)
}
