mod analytic;
mod coaxial;
mod march;
mod overlap;
mod recognize;

use caditor_geometry::{Aabb, Point2, Point3};

pub(crate) use self::analytic::line_window;
use crate::{
    curve::Curve,
    intersect::{IntersectionError, SurfacePatch, boxes_overlap, inside_intervals},
    interval::Interval,
    sense::Sense,
    tolerance::LINEAR_RESOLUTION,
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;
const MIN_CLIP_SAMPLES: usize = 32;
const MAX_CLIP_SAMPLES: usize = 4096;
const CLIP_SPACING: f64 = 0.02;
const PERIOD_SLACK: f64 = 1e-9;
const MIN_BRANCH_LENGTH: f64 = 10.0 * LINEAR_RESOLUTION;

#[derive(Debug, Clone, PartialEq)]
pub struct IntersectionBranch {
    pub curve: Curve,
    pub range: Interval,
    pub tangent: bool,
    pub closed: bool,
    pub start_uv: [Point2; 2],
    pub end_uv: [Point2; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntersectionPoint {
    pub point: Point3,
    pub uv: [Point2; 2],
    pub tangent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceIntersection {
    Coincident(Sense),
    Branches {
        branches: Vec<IntersectionBranch>,
        points: Vec<IntersectionPoint>,
    },
}

impl SurfaceIntersection {
    pub fn branches(&self) -> &[IntersectionBranch] {
        match self {
            Self::Coincident(_) => &[],
            Self::Branches { branches, .. } => branches,
        }
    }

    pub fn points(&self) -> &[IntersectionPoint] {
        match self {
            Self::Coincident(_) => &[],
            Self::Branches { points, .. } => points,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Branches { branches, points } if branches.is_empty() && points.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawCurve {
    pub curve: Curve,
    pub range: Interval,
    pub tangent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawPoint {
    pub point: Point3,
    pub tangent: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Raw {
    pub curves: Vec<RawCurve>,
    pub points: Vec<RawPoint>,
}

pub fn intersect_surfaces(
    first: &SurfacePatch,
    second: &SurfacePatch,
) -> Result<SurfaceIntersection, IntersectionError> {
    if let Some(sense) = first.surface().same_surface(second.surface()) {
        return Ok(SurfaceIntersection::Coincident(sense));
    }
    let (first_box, second_box) = (first.bounding_box(), second.bounding_box());
    if !boxes_overlap(&first_box, &second_box, TOLERANCE) {
        return Ok(SurfaceIntersection::Branches {
            branches: Vec::new(),
            points: Vec::new(),
        });
    }
    let sampled = overlap::sampled_kind(first.surface()) || overlap::sampled_kind(second.surface());
    if sampled && let Some(sense) = overlap::coincident_part(first, second) {
        return Ok(SurfaceIntersection::Coincident(sense));
    }
    let window = shared_window(&first_box, &second_box);
    let raw = match analytic::intersect(first, second, &window) {
        Some(raw) => raw,
        None => march::intersect(first, second)?,
    };
    Ok(finish(first, second, &window, raw))
}

pub(crate) fn shared_window(first: &Aabb, second: &Aabb) -> Aabb {
    let low = first.min().max(second.min());
    let high = first.max().min(second.max());
    Aabb::from_point(low.min(high))
        .including(high.max(low))
        .expanded(TOLERANCE)
}

fn locate_both(first: &SurfacePatch, second: &SurfacePatch, point: Point3) -> Option<[Point2; 2]> {
    Some([first.locate(point)?, second.locate(point)?])
}

fn clip(
    first: &SurfacePatch,
    second: &SurfacePatch,
    window: &Aabb,
    raw: &RawCurve,
) -> Vec<Interval> {
    let length = raw.curve.length(raw.range);
    let spacing = CLIP_SPACING * window.diagonal().max(TOLERANCE);
    let samples = (length / spacing).ceil();
    let samples = if samples.is_finite() {
        (samples as usize).clamp(MIN_CLIP_SAMPLES, MAX_CLIP_SAMPLES)
    } else {
        MIN_CLIP_SAMPLES
    };
    let inside = |parameter: f64| locate_both(first, second, raw.curve.point(parameter)).is_some();
    let mut pieces = inside_intervals(raw.range, samples, inside);
    let whole_period = raw
        .curve
        .period()
        .is_some_and(|period| raw.range.length() >= period * (1.0 - PERIOD_SLACK));
    if whole_period
        && pieces.len() >= 2
        && let (Some(first_piece), Some(last_piece)) =
            (pieces.first().copied(), pieces.last().copied())
        && first_piece.start() == raw.range.start()
        && last_piece.end() == raw.range.end()
        && let Some(joined) =
            Interval::new(last_piece.start(), first_piece.end() + raw.range.length())
    {
        pieces.retain(|piece| *piece != first_piece && *piece != last_piece);
        pieces.push(joined);
    }
    pieces
}

fn finish(
    first: &SurfacePatch,
    second: &SurfacePatch,
    window: &Aabb,
    raw: Raw,
) -> SurfaceIntersection {
    let mut branches = Vec::new();
    for curve in &raw.curves {
        for range in clip(first, second, window, curve) {
            if curve.curve.length(range) <= MIN_BRANCH_LENGTH {
                continue;
            }
            let (start, end) = (
                curve.curve.point(range.start()),
                curve.curve.point(range.end()),
            );
            let locate = |point: Point3| {
                locate_both(first, second, point).unwrap_or([
                    first.wrap(first.surface().project(point, None)),
                    second.wrap(second.surface().project(point, None)),
                ])
            };
            let closed = curve
                .curve
                .period()
                .is_some_and(|period| range.length() >= period * (1.0 - PERIOD_SLACK));
            branches.push(IntersectionBranch {
                curve: curve.curve.clone(),
                range,
                tangent: curve.tangent,
                closed,
                start_uv: locate(start),
                end_uv: locate(end),
            });
        }
    }
    let mut points: Vec<IntersectionPoint> = Vec::new();
    for point in raw.points {
        let Some(uv) = locate_both(first, second, point.point) else {
            continue;
        };
        if points
            .iter()
            .any(|existing| existing.point.distance(point.point) <= TOLERANCE)
        {
            continue;
        }
        points.push(IntersectionPoint {
            point: point.point,
            uv,
            tangent: point.tangent,
        });
    }
    SurfaceIntersection::Branches { branches, points }
}
