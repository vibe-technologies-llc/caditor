use caditor_geometry::Point2;

use crate::overlap::Segment;

const SIDE_BEARING: f64 = 0.06;
const CIRCLE_KAPPA: f64 = 0.552_284_749_830_793_4;
const RING_RADIUS: f64 = 0.5;
const SLASH_RUN: f64 = 0.6;
const SLASH_OVERSHOOT: f64 = 0.04;
const DEPTH_WIDTH: f64 = 0.6;
const DEPTH_HEAD_HEIGHT: f64 = 0.2;
const DEPTH_HEAD_SPREAD: f64 = 0.24;
const BORE_WIDTH: f64 = 0.8;
const BORE_HEIGHT: f64 = 0.7;
const SINK_WIDTH: f64 = 0.8;
const SINK_HEIGHT: f64 = 0.7;
const INSET_REFINEMENTS: usize = 4;
const NEAR_PARALLEL: f64 = 1e-3;

#[derive(Clone, Debug, PartialEq)]
pub struct Sign {
    pub advance: f64,
    pub segments: Vec<Segment>,
}

pub fn sign(character: char, stem: f64) -> Option<Sign> {
    let (width, segments) = match character {
        '\u{2300}' => (2.0 * RING_RADIUS, diameter(stem)),
        '\u{21a7}' => (DEPTH_WIDTH, depth(stem)),
        '\u{2334}' => (BORE_WIDTH, counterbore(stem)),
        '\u{2335}' => (SINK_WIDTH, countersink(stem)),
        _ => return None,
    };

    Some(Sign {
        advance: width + 2.0 * SIDE_BEARING,
        segments: shifted(segments, SIDE_BEARING),
    })
}

fn shifted(segments: Vec<Segment>, by: f64) -> Vec<Segment> {
    segments
        .into_iter()
        .map(|segment| segment.mapped(|point| Point2::new(point.x + by, point.y)))
        .collect()
}

fn diameter(stem: f64) -> Vec<Segment> {
    let centre = Point2::new(RING_RADIUS, 0.5);
    let half_width = stem / 2.0 * SLASH_RUN.hypot(1.0);
    let at = |y: f64| centre.x + (y - centre.y) * SLASH_RUN;
    let (bottom, top) = (-SLASH_OVERSHOOT, 1.0 + SLASH_OVERSHOOT);

    let mut segments = circle(centre, RING_RADIUS, true);

    segments.extend(circle(centre, (RING_RADIUS - stem).max(0.0), false));
    segments.extend(polygon(vec![
        Point2::new(at(bottom) - half_width, bottom),
        Point2::new(at(bottom) + half_width, bottom),
        Point2::new(at(top) + half_width, top),
        Point2::new(at(top) - half_width, top),
    ]));
    segments
}

fn depth(stem: f64) -> Vec<Segment> {
    let middle = DEPTH_WIDTH / 2.0;
    let apex = stem + DEPTH_HEAD_HEIGHT;

    let mut segments = stroke(&[Point2::new(middle, 1.0), Point2::new(middle, stem)], stem);

    segments.extend(stroke(
        &[
            Point2::new(middle - DEPTH_HEAD_SPREAD, apex + DEPTH_HEAD_SPREAD),
            Point2::new(middle, apex),
            Point2::new(middle + DEPTH_HEAD_SPREAD, apex + DEPTH_HEAD_SPREAD),
        ],
        stem,
    ));
    segments.extend(polygon(vec![
        Point2::new(0.0, 0.0),
        Point2::new(DEPTH_WIDTH, 0.0),
        Point2::new(DEPTH_WIDTH, stem),
        Point2::new(0.0, stem),
    ]));
    segments
}

fn counterbore(stem: f64) -> Vec<Segment> {
    let inset = stem / 2.0;

    stroke(
        &[
            Point2::new(inset, BORE_HEIGHT),
            Point2::new(inset, inset),
            Point2::new(BORE_WIDTH - inset, inset),
            Point2::new(BORE_WIDTH - inset, BORE_HEIGHT),
        ],
        stem,
    )
}

fn countersink(stem: f64) -> Vec<Segment> {
    let half = SINK_WIDTH / 2.0;
    let inset = (0..INSET_REFINEMENTS).fold(0.0, |inset, _| {
        stem / 2.0 * SINK_HEIGHT / (half - inset).hypot(SINK_HEIGHT)
    });

    let run = half - inset;
    let apex = stem / 2.0 * run.hypot(SINK_HEIGHT) / run;

    stroke(
        &[
            Point2::new(inset, SINK_HEIGHT),
            Point2::new(half, apex),
            Point2::new(SINK_WIDTH - inset, SINK_HEIGHT),
        ],
        stem,
    )
}

fn circle(centre: Point2, radius: f64, counter_clockwise: bool) -> Vec<Segment> {
    let turn = if counter_clockwise { 1.0 } else { -1.0 };
    let quarters = [
        (Point2::new(1.0, 0.0), Point2::new(0.0, 1.0)),
        (Point2::new(0.0, 1.0), Point2::new(-1.0, 0.0)),
        (Point2::new(-1.0, 0.0), Point2::new(0.0, -1.0)),
        (Point2::new(0.0, -1.0), Point2::new(1.0, 0.0)),
    ];
    let quarters: Vec<(Point2, Point2)> = if counter_clockwise {
        quarters.to_vec()
    } else {
        quarters
            .iter()
            .rev()
            .map(|(from, to)| (*to, *from))
            .collect()
    };

    quarters
        .into_iter()
        .map(|(from, to)| {
            let start = centre + from * radius;
            let end = centre + to * radius;
            let leaving = from.perp() * turn * CIRCLE_KAPPA * radius;
            let arriving = to.perp() * turn * CIRCLE_KAPPA * radius;

            Segment::Bezier(vec![start, start + leaving, end - arriving, end])
        })
        .collect()
}

fn polygon(points: Vec<Point2>) -> Vec<Segment> {
    let area: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(from, to)| from.perp_dot(*to))
        .sum();
    let mut points = points;

    if area < 0.0 {
        points.reverse();
    }
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(from, to)| Segment::Line(*from, *to))
        .collect()
}

fn stroke(path: &[Point2], width: f64) -> Vec<Segment> {
    let half = width / 2.0;
    let normals: Vec<Point2> = path
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => Some((*to - *from).normalize_or_zero().perp()),
            _ => None,
        })
        .collect();

    let offsets: Vec<Point2> = path
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let before = index
                .checked_sub(1)
                .and_then(|previous| normals.get(previous))
                .or(normals.first())?;
            let after = normals.get(index).unwrap_or(before);

            let miter = (*before + *after).normalize_or_zero();
            let along = miter.dot(*before);

            Some(if along < NEAR_PARALLEL {
                *before * half
            } else {
                miter * (half / along)
            })
        })
        .collect();

    let left = path
        .iter()
        .zip(&offsets)
        .map(|(point, offset)| *point + *offset);
    let right = path
        .iter()
        .zip(&offsets)
        .map(|(point, offset)| *point - *offset);

    polygon(left.chain(right.rev()).collect())
}
