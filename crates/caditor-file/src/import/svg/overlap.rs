use caditor_geometry::Point2;
use caditor_kernel::{Piece, Profile, ProfileCurve, Selection};

use crate::import::svg::path::Outline;

const SAMPLES_PER_CURVE: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Segment {
    Line(Point2, Point2),
    Bezier(Vec<Point2>),
}

impl Segment {
    fn profile_curve(&self, entity: u64) -> ProfileCurve {
        match self {
            Self::Line(start, end) => ProfileCurve::line(entity, *start, *end),
            Self::Bezier(points) => {
                let count = points.len();
                let knots = [vec![0.0; count], vec![1.0; count]].concat();
                ProfileCurve::spline(entity, count.saturating_sub(1), knots, points.clone())
            }
        }
    }

    fn polyline(&self) -> Vec<Point2> {
        match self {
            Self::Line(start, end) => vec![*start, *end],
            Self::Bezier(points) => (0..=SAMPLES_PER_CURVE)
                .map(|step| bezier_point(points, step as f64 / SAMPLES_PER_CURVE as f64))
                .collect(),
        }
    }
}

pub(super) fn traced(segments: &[Segment]) -> Outline {
    let mut outline = Outline::default();
    for segment in segments {
        match segment {
            Segment::Line(start, end) => outline.line(*start, *end),
            Segment::Bezier(points) => outline.bezier(points),
        }
    }
    outline
}

pub(super) fn merged(segments: &[Segment]) -> Option<Outline> {
    let curves: Vec<ProfileCurve> = segments
        .iter()
        .zip(0_u64..)
        .map(|(segment, entity)| segment.profile_curve(entity))
        .collect();
    let profile = Profile::new(&curves).ok()?;
    let mut filled = Vec::new();
    for region in profile.regions() {
        if winding(segments, region.anchor()?) != 0 {
            filled.push(region.key());
        }
    }
    if filled.is_empty() {
        return None;
    }
    let lumps = profile.select(&Selection::Regions(filled)).ok()?;
    let mut outline = Outline::default();
    for lump in &lumps {
        for profile_loop in lump.loops() {
            trace_loop(profile_loop.pieces(), segments, &mut outline)?;
        }
    }
    Some(outline)
}

fn trace_loop(pieces: &[Piece], segments: &[Segment], outline: &mut Outline) -> Option<()> {
    let junctions: Vec<Point2> = pieces.iter().map(Piece::start).collect();
    let ends = junctions.iter().cycle().skip(1);
    for ((piece, start), end) in pieces.iter().zip(&junctions).zip(ends) {
        let segment = segments.get(usize::try_from(piece.entity()).ok()?)?;
        match segment {
            Segment::Line(..) => outline.line(*start, *end),
            Segment::Bezier(points) => {
                let range = piece.range();
                let mut part = restricted(points, range.start(), range.end());
                if piece.is_reversed() {
                    part.reverse();
                }
                if let Some(first) = part.first_mut() {
                    *first = *start;
                }
                if let Some(last) = part.last_mut() {
                    *last = *end;
                }
                outline.bezier(&part);
            }
        }
    }
    Some(())
}

fn winding(segments: &[Segment], point: Point2) -> i32 {
    let mut winding = 0;
    for segment in segments {
        let polyline = segment.polyline();
        for (from, to) in polyline.iter().zip(polyline.iter().skip(1)) {
            let side = (*to - *from).perp_dot(point - *from);
            if from.y <= point.y && to.y > point.y && side > 0.0 {
                winding += 1;
            } else if from.y > point.y && to.y <= point.y && side < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

fn bezier_point(points: &[Point2], parameter: f64) -> Point2 {
    let (left, _) = split(points, parameter);
    left.last().copied().unwrap_or(Point2::ZERO)
}

fn split(points: &[Point2], parameter: f64) -> (Vec<Point2>, Vec<Point2>) {
    let mut row = points.to_vec();
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    while let (Some(first), Some(last)) = (row.first().copied(), row.last().copied()) {
        left.push(first);
        right.push(last);
        row = row
            .iter()
            .zip(row.iter().skip(1))
            .map(|(a, b)| a.lerp(*b, parameter))
            .collect();
    }
    right.reverse();
    (left, right)
}

fn restricted(points: &[Point2], from: f64, to: f64) -> Vec<Point2> {
    let (_, after) = split(points, from);
    if from >= 1.0 {
        return after;
    }
    let (within, _) = split(&after, (to - from) / (1.0 - from));
    within
}
