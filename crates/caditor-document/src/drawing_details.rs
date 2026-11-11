use std::{
    f64::consts::PI,
    panic::{self, AssertUnwindSafe},
};

use caditor_geometry::{Aabb2, Point2};
use caditor_kernel::{
    Curve2, Interval, LINEAR_RESOLUTION, Line2, SamplingTolerance, ViewPiece, check_interrupt,
};

use crate::{
    drawing_views::{Disc, DrawingViewError, ViewLines},
    drawings::{DetailView, DrawnView},
};

const CHORD_PER_RADIUS: f64 = 1e-3;
const SAMPLE_ANGLE: f64 = 2.0 * PI / 180.0;
const BISECTIONS: usize = 80;

fn overlap(left: Aabb2, right: Aabb2) -> bool {
    left.min().x <= right.max().x
        && right.min().x <= left.max().x
        && left.min().y <= right.max().y
        && right.min().y <= left.max().y
}

fn outside_by(disc: &Disc, point: Point2) -> f64 {
    (point - disc.centre).length_squared() - disc.radius * disc.radius
}

fn line_ranges(line: &Line2, range: Interval, disc: &Disc) -> Vec<Interval> {
    disc.clip_segment(line.point(range.start()), line.point(range.end()))
        .and_then(|(from, to)| {
            let start = line.parameter_of(from).max(range.start());
            let end = line.parameter_of(to).min(range.end());
            Interval::new(start, end)
        })
        .into_iter()
        .collect()
}

fn root_between(curve: &Curve2, disc: &Disc, mut inside: f64, mut outside: f64) -> f64 {
    for _ in 0..BISECTIONS {
        let middle = 0.5 * (inside + outside);
        if outside_by(disc, curve.point(middle)) <= 0.0 {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    0.5 * (inside + outside)
}

fn crossings(curve: &Curve2, range: Interval, disc: &Disc) -> Vec<f64> {
    let tolerance = SamplingTolerance::new(disc.radius * CHORD_PER_RADIUS, SAMPLE_ANGLE)
        .unwrap_or_else(|| SamplingTolerance::for_extent(disc.radius));
    let samples = curve.sample(range, &tolerance);
    let mut found = Vec::new();
    for (before, after) in samples.iter().zip(samples.iter().skip(1)) {
        let middle = 0.5 * (before.parameter + after.parameter);
        let heights = [
            (before.parameter, outside_by(disc, before.point)),
            (middle, outside_by(disc, curve.point(middle))),
            (after.parameter, outside_by(disc, after.point)),
        ];
        for (from, to) in heights.iter().zip(heights.iter().skip(1)) {
            if (from.1 <= 0.0) != (to.1 <= 0.0) {
                let (inside, outside) = if from.1 <= 0.0 {
                    (from.0, to.0)
                } else {
                    (to.0, from.0)
                };
                found.push(root_between(curve, disc, inside, outside));
            }
        }
    }
    found.sort_by(f64::total_cmp);
    found
}

fn curve_ranges(curve: &Curve2, range: Interval, disc: &Disc) -> Vec<Interval> {
    let mut stops = vec![range.start()];
    stops.extend(crossings(curve, range, disc));
    stops.push(range.end());
    let mut kept: Vec<Interval> = Vec::new();
    for (from, to) in stops.iter().zip(stops.iter().skip(1)) {
        let Some(span) = Interval::new(*from, *to) else {
            continue;
        };
        let inside = outside_by(disc, curve.point(span.middle())) <= 0.0;
        if !inside || curve.length(span) < LINEAR_RESOLUTION {
            continue;
        }
        match kept.last_mut() {
            Some(last) if last.end() >= span.start() => {
                if let Some(joined) = Interval::new(last.start(), span.end()) {
                    *last = joined;
                }
            }
            _ => kept.push(span),
        }
    }
    kept
}

fn clipped(piece: &ViewPiece, disc: &Disc) -> Vec<ViewPiece> {
    if !overlap(piece.curve.bounding_box(piece.range), disc.bounds()) {
        return Vec::new();
    }
    let ranges = match &piece.curve {
        Curve2::Line(line) => line_ranges(line, piece.range, disc),
        curve => curve_ranges(curve, piece.range, disc),
    };
    ranges
        .into_iter()
        .map(|range| ViewPiece {
            range,
            ..piece.clone()
        })
        .collect()
}

fn enlarged(detail: &DetailView, parent: &ViewLines) -> Result<ViewLines, DrawingViewError> {
    let disc = Disc {
        centre: detail.centre,
        radius: detail.radius,
    };
    let mut pieces = Vec::new();
    for piece in &parent.pieces {
        check_interrupt().map_err(|_| DrawingViewError::Cancelled)?;
        pieces.extend(clipped(piece, &disc));
    }
    Ok(ViewLines {
        view: DrawnView::Detail(detail.letter),
        pieces,
        bounds: Some(disc.bounds()),
        undecided: parent.undecided,
        section: parent.section.clone(),
        clip: Some(disc),
    })
}

pub fn detail_view(detail: &DetailView, parent: &ViewLines) -> Result<ViewLines, DrawingViewError> {
    panic::catch_unwind(AssertUnwindSafe(|| enlarged(detail, parent)))
        .map_err(|_| DrawingViewError::Panicked)?
}
