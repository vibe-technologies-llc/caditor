use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::{PathSegment, PathSweepError};
use crate::{
    bspline::BSpline,
    build::revolve::arc_spline,
    curve::{BSplineCurve, Curve},
    curve2::Circle2,
    interrupt,
    interval::Interval,
    tolerance::{MAX_SIZE, SamplingTolerance},
};

const ROOT_STEPS: usize = 64;
const INTERSECTION_CHORD: f64 = 1e-5;
const INTERSECTION_ANGLE: f64 = 0.02;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Leg {
    pub(super) curve: Curve,
    pub(super) range: Interval,
    pub(super) key: u128,
    pub(super) part: u8,
    pub(super) segment: usize,
}

impl Leg {
    pub(super) fn start_point(&self) -> Point3 {
        self.curve.point(self.range.start())
    }

    pub(super) fn end_point(&self) -> Point3 {
        self.curve.point(self.range.end())
    }

    pub(super) fn tangent(&self, parameter: f64) -> Option<Vector3> {
        self.curve.evaluate(parameter).first.try_normalize()
    }

    pub(super) fn start_tangent(&self) -> Option<Vector3> {
        self.tangent(self.range.start())
    }

    pub(super) fn end_tangent(&self) -> Option<Vector3> {
        self.tangent(self.range.end())
    }

    #[must_use]
    fn reversed(&self) -> Self {
        Self {
            curve: self.curve.reversed(),
            range: self.curve.reversed_range(self.range),
            key: self.key,
            part: self.part,
            segment: self.segment,
        }
    }

    fn split_at(&self, parameter: f64) -> Option<(Self, Self)> {
        let before = Interval::new(self.range.start(), parameter)?;
        let after = Interval::new(parameter, self.range.end())?;
        Some((
            Self {
                range: before,
                part: 1,
                ..self.clone()
            },
            Self {
                range: after,
                ..self.clone()
            },
        ))
    }

    pub(super) fn spline(&self) -> Result<BSplineCurve, PathSweepError> {
        let unsupported = || PathSweepError::UnsupportedPath {
            segment: self.segment,
        };
        match &self.curve {
            Curve::Line(_) => {
                let (start, end) = (self.range.start(), self.range.end());
                Ok(BSpline::new(
                    1,
                    vec![start, start, end, end],
                    vec![self.start_point(), self.end_point()],
                )?)
            }
            Curve::Circle(circle) => {
                let unit = Circle2::new(Point2::ZERO, circle.radius())?;
                let frame = *circle.frame();
                Ok(arc_spline(&unit, self.range)?.map_points(|point| frame.to_world(point))?)
            }
            Curve::Ellipse(ellipse) => {
                let unit = Circle2::new(Point2::ZERO, 1.0)?;
                let frame = *ellipse.frame();
                let (major, minor) = (ellipse.major_radius(), ellipse.minor_radius());
                Ok(arc_spline(&unit, self.range)?.map_points(|point| {
                    frame.origin()
                        + frame.x_axis() * (major * point.x)
                        + frame.y_axis() * (minor * point.y)
                })?)
            }
            Curve::BSpline(spline) => spline.restricted(self.range).ok_or_else(unsupported),
            Curve::Intersection(_) => {
                let tolerance = SamplingTolerance::new(INTERSECTION_CHORD, INTERSECTION_ANGLE)
                    .ok_or_else(unsupported)?;
                let mut points: Vec<Point3> = Vec::new();
                for sample in self.curve.sample(self.range, &tolerance) {
                    if points
                        .last()
                        .is_none_or(|last| last.distance(sample.point) > 0.0)
                    {
                        points.push(sample.point);
                    }
                }
                let degree = 3.min(points.len().saturating_sub(1)).max(1);
                Ok(BSpline::interpolating(degree, &points)?)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Chain {
    pub(super) legs: Vec<Leg>,
    pub(super) closed: bool,
}

struct Ends {
    nodes: Vec<Point3>,
    of_segment: Vec<[usize; 2]>,
}

impl Ends {
    fn node_of(nodes: &mut Vec<Point3>, point: Point3, tolerance: f64) -> usize {
        match nodes
            .iter()
            .position(|node| node.distance(point) <= tolerance)
        {
            Some(found) => found,
            None => {
                nodes.push(point);
                nodes.len() - 1
            }
        }
    }

    fn of(segments: &[PathSegment], tolerance: f64) -> Self {
        let mut nodes = Vec::new();
        let of_segment = segments
            .iter()
            .map(|segment| {
                let start = segment.curve.point(segment.range.start());
                let end = segment.curve.point(segment.range.end());
                [
                    Self::node_of(&mut nodes, start, tolerance),
                    Self::node_of(&mut nodes, end, tolerance),
                ]
            })
            .collect();
        Self { nodes, of_segment }
    }

    fn users(&self, node: usize) -> Vec<usize> {
        self.of_segment
            .iter()
            .enumerate()
            .flat_map(|(segment, ends)| {
                ends.iter()
                    .filter(move |end| **end == node)
                    .map(move |_| segment)
            })
            .collect()
    }
}

pub(super) fn chain(
    segments: &[PathSegment],
    plane: &Plane,
    centre: Point3,
    tolerance: f64,
) -> Result<Chain, PathSweepError> {
    if segments.is_empty() {
        return Err(PathSweepError::NoPath);
    }
    for (index, segment) in segments.iter().enumerate() {
        let bounds = segment.curve.bounding_box(segment.range);
        let reach = bounds.min().abs().max(bounds.max().abs()).max_element();
        if !reach.is_finite() {
            return Err(PathSweepError::Sweep(super::SweepError::NonFinite));
        }
        if reach > MAX_SIZE {
            return Err(PathSweepError::TooLong);
        }
        if !segment.curve.is_longer_than(segment.range, tolerance) {
            return Err(PathSweepError::SegmentTooShort { segment: index });
        }
    }
    let ends = Ends::of(segments, tolerance);
    for node in 0..ends.nodes.len() {
        let users = ends.users(node);
        if users.len() > 2 {
            let mut branching: Vec<usize> = users;
            branching.sort_unstable();
            branching.dedup();
            return Err(PathSweepError::Branches {
                segments: branching,
            });
        }
    }
    let ordered = walk(segments, &ends)?;
    let closed = ordered
        .first()
        .zip(ordered.last())
        .is_some_and(|(first, last)| first.start_point().distance(last.end_point()) <= tolerance);
    let legs = if closed {
        opened_at_profile(ordered, plane, centre, tolerance)?
    } else {
        started_near_profile(ordered, plane, centre, tolerance)
    };
    Ok(Chain { legs, closed })
}

fn walk(segments: &[PathSegment], ends: &Ends) -> Result<Vec<Leg>, PathSweepError> {
    let degree_one = (0..ends.nodes.len()).find(|node| ends.users(*node).len() == 1);
    let start_node = match degree_one {
        Some(node) => node,
        None => ends
            .of_segment
            .first()
            .map(|[start, _]| *start)
            .ok_or(PathSweepError::NoPath)?,
    };
    let mut used = vec![false; segments.len()];
    let mut legs = Vec::with_capacity(segments.len());
    let mut node = start_node;
    loop {
        interrupt::check()?;
        let next = ends
            .users(node)
            .into_iter()
            .find(|segment| !used.get(*segment).copied().unwrap_or(true));
        let Some(index) = next else {
            break;
        };
        if let Some(slot) = used.get_mut(index) {
            *slot = true;
        }
        let (Some(segment), Some([from, to])) = (segments.get(index), ends.of_segment.get(index))
        else {
            break;
        };
        let leg = Leg {
            curve: segment.curve.clone(),
            range: segment.range,
            key: segment.key,
            part: 0,
            segment: index,
        };
        if *from == node {
            legs.push(leg);
            node = *to;
        } else {
            legs.push(leg.reversed());
            node = *from;
        }
    }
    let apart: Vec<usize> = used
        .iter()
        .enumerate()
        .filter(|(_, used)| !**used)
        .map(|(index, _)| index)
        .collect();
    if !apart.is_empty() {
        return Err(PathSweepError::Apart { segments: apart });
    }
    Ok(legs)
}

fn reversed_chain(legs: Vec<Leg>) -> Vec<Leg> {
    legs.iter().rev().map(Leg::reversed).collect()
}

fn started_near_profile(legs: Vec<Leg>, plane: &Plane, centre: Point3, tolerance: f64) -> Vec<Leg> {
    let (Some(first), Some(last)) = (legs.first(), legs.last()) else {
        return legs;
    };
    let (start, end) = (first.start_point(), last.end_point());
    let (from_start, from_end) = (
        plane.signed_distance(start).abs(),
        plane.signed_distance(end).abs(),
    );
    let reverse = if (from_start - from_end).abs() > tolerance {
        from_end < from_start
    } else {
        end.distance(centre) < start.distance(centre)
    };
    if reverse { reversed_chain(legs) } else { legs }
}

struct Crossing {
    leg: usize,
    parameter: f64,
    point: Point3,
}

fn crossings(legs: &[Leg], plane: &Plane, size: f64) -> Result<Vec<Crossing>, PathSweepError> {
    let sampling = SamplingTolerance::for_extent(size);
    let mut found = Vec::new();
    for (index, leg) in legs.iter().enumerate() {
        interrupt::check()?;
        let height = |parameter: f64| plane.signed_distance(leg.curve.point(parameter));
        let samples = leg.curve.sample(leg.range, &sampling);
        for pair in samples.windows(2) {
            let [from, to] = pair else {
                continue;
            };
            let (low, high) = (height(from.parameter), height(to.parameter));
            if low == 0.0 {
                found.push(Crossing {
                    leg: index,
                    parameter: from.parameter,
                    point: from.point,
                });
                continue;
            }
            if low.signum() == high.signum() || high == 0.0 {
                continue;
            }
            let (mut a, mut b) = (from.parameter, to.parameter);
            for _ in 0..ROOT_STEPS {
                let middle = 0.5 * (a + b);
                if height(middle).signum() == low.signum() {
                    a = middle;
                } else {
                    b = middle;
                }
            }
            let parameter = 0.5 * (a + b);
            found.push(Crossing {
                leg: index,
                parameter,
                point: leg.curve.point(parameter),
            });
        }
    }
    Ok(found)
}

fn opened_at_profile(
    legs: Vec<Leg>,
    plane: &Plane,
    centre: Point3,
    tolerance: f64,
) -> Result<Vec<Leg>, PathSweepError> {
    let size = legs
        .iter()
        .map(|leg| leg.curve.bounding_box(leg.range).diagonal())
        .fold(0.0, f64::max);
    let nearest = crossings(&legs, plane, size)?
        .into_iter()
        .min_by(|a, b| {
            a.point
                .distance(centre)
                .total_cmp(&b.point.distance(centre))
        })
        .ok_or(PathSweepError::ClosedPathMissesProfile)?;
    let count = legs.len();
    let leg = legs
        .get(nearest.leg)
        .ok_or(PathSweepError::ClosedPathMissesProfile)?;
    let rotated: Vec<Leg> = if nearest.point.distance(leg.start_point()) <= tolerance {
        legs.iter()
            .cycle()
            .skip(nearest.leg)
            .take(count)
            .cloned()
            .collect()
    } else if nearest.point.distance(leg.end_point()) <= tolerance {
        legs.iter()
            .cycle()
            .skip(nearest.leg + 1)
            .take(count)
            .cloned()
            .collect()
    } else {
        let (before, after) = leg
            .split_at(nearest.parameter)
            .ok_or(PathSweepError::ClosedPathMissesProfile)?;
        let mut rotated = vec![after];
        rotated.extend(
            legs.iter()
                .cycle()
                .skip(nearest.leg + 1)
                .take(count - 1)
                .cloned(),
        );
        rotated.push(before);
        rotated
    };
    let leaves_forward = rotated
        .first()
        .and_then(Leg::start_tangent)
        .is_some_and(|tangent| tangent.dot(plane.normal()) >= 0.0);
    Ok(if leaves_forward {
        rotated
    } else {
        reversed_chain(rotated)
    })
}
