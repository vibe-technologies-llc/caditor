use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector2};

use super::{HiddenLineError, Track, flatten::flat_vector};
use crate::{box_tree::BoxTree, interrupt, interval::Interval, tally};

const PARALLEL_SINE: f64 = 1e-12;
const CROSSING_SLACK: f64 = 1e-9;
const NEWTON_STEPS: usize = 6;
const POLL_EVERY: usize = 64;
const SINGULAR: f64 = 1e-18;

#[derive(Debug, Clone, Copy)]
struct Segment {
    track: usize,
    index: usize,
    start: (f64, Point2),
    end: (f64, Point2),
}

impl Segment {
    fn bounds(&self) -> Aabb {
        let lifted = |point: Point2| Point3::new(point.x, point.y, 0.0);
        Aabb::from_point(lifted(self.start.1)).including(lifted(self.end.1))
    }

    fn direction(&self) -> Vector2 {
        self.end.1 - self.start.1
    }

    fn parameter_at(&self, fraction: f64) -> f64 {
        self.start.0 + (self.end.0 - self.start.0) * fraction
    }

    fn range(&self) -> Option<Interval> {
        Interval::new(self.start.0, self.end.0)
    }
}

fn segments_of(tracks: &[Track]) -> Vec<Segment> {
    let mut segments = Vec::new();
    for (track, data) in tracks.iter().enumerate() {
        for (index, pair) in data.samples.windows(2).enumerate() {
            let [start, end] = pair else {
                continue;
            };
            segments.push(Segment {
                track,
                index,
                start: *start,
                end: *end,
            });
        }
    }
    segments
}

fn flat_at(track: &Track, parameter: f64, view: &Plane) -> (Point2, Vector2) {
    let at = track.curve.evaluate(parameter);
    (view.to_local(at.point), flat_vector(view, at.first))
}

fn refined_crossing(
    first: (&Track, &Segment, f64),
    second: (&Track, &Segment, f64),
    view: &Plane,
) -> (f64, f64) {
    let (track_a, segment_a, start_a) = first;
    let (track_b, segment_b, start_b) = second;
    let (Some(range_a), Some(range_b)) = (segment_a.range(), segment_b.range()) else {
        return (start_a, start_b);
    };
    let gap_at = |a: f64, b: f64| {
        flat_at(track_a, a, view)
            .0
            .distance(flat_at(track_b, b, view).0)
    };
    let (mut a, mut b) = (start_a, start_b);
    for _ in 0..NEWTON_STEPS {
        let (point_a, along_a) = flat_at(track_a, a, view);
        let (point_b, along_b) = flat_at(track_b, b, view);
        let gap = point_b - point_a;
        let determinant = along_a.perp_dot(along_b);
        if determinant.abs() <= SINGULAR {
            break;
        }
        a = range_a.clamp(a + gap.perp_dot(along_b) / determinant);
        b = range_b.clamp(b + gap.perp_dot(along_a) / determinant);
    }
    if gap_at(a, b) <= gap_at(start_a, start_b) {
        (a, b)
    } else {
        (start_a, start_b)
    }
}

fn add_crossing(
    tracks: &[Track],
    splits: &mut [Vec<f64>],
    first: &Segment,
    second: &Segment,
    view: &Plane,
) {
    let (along_a, along_b) = (first.direction(), second.direction());
    let denominator = along_a.perp_dot(along_b);
    if denominator.abs() <= PARALLEL_SINE * along_a.length() * along_b.length() {
        return;
    }
    let gap = second.start.1 - first.start.1;
    let at_a = gap.perp_dot(along_b) / denominator;
    let at_b = gap.perp_dot(along_a) / denominator;
    let within = -CROSSING_SLACK..=1.0 + CROSSING_SLACK;
    if !within.contains(&at_a) || !within.contains(&at_b) {
        return;
    }
    let (Some(track_a), Some(track_b)) = (tracks.get(first.track), tracks.get(second.track)) else {
        return;
    };
    let (a, b) = refined_crossing(
        (track_a, first, first.parameter_at(at_a.clamp(0.0, 1.0))),
        (track_b, second, second.parameter_at(at_b.clamp(0.0, 1.0))),
        view,
    );
    if let Some(split) = splits.get_mut(first.track) {
        split.push(a);
    }
    if let Some(split) = splits.get_mut(second.track) {
        split.push(b);
    }
}

fn nearest_on(track: &Track, segment: &Segment, end: Point2, view: &Plane) -> Option<(f64, f64)> {
    let along = segment.direction();
    let length_squared = along.length_squared();
    let fraction = if length_squared > 0.0 {
        ((end - segment.start.1).dot(along) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let range = segment.range()?;
    let mut parameter = segment.parameter_at(fraction);
    for _ in 0..NEWTON_STEPS {
        let (point, along) = flat_at(track, parameter, view);
        let speed_squared = along.length_squared();
        if speed_squared <= SINGULAR {
            break;
        }
        parameter = range.clamp(parameter + (end - point).dot(along) / speed_squared);
    }
    let distance = flat_at(track, parameter, view).0.distance(end);
    Some((parameter, distance))
}

fn add_track_ends(
    tracks: &[Track],
    splits: &mut [Vec<f64>],
    segments: &[Segment],
    tree: &BoxTree,
    reach: f64,
    view: &Plane,
) -> Result<(), HiddenLineError> {
    for (index, track) in tracks.iter().enumerate() {
        interrupt::check()?;
        if track.closed {
            continue;
        }
        for (parameter, end) in [track.samples.first(), track.samples.last()]
            .into_iter()
            .flatten()
        {
            let lifted = Aabb::from_point(Point3::new(end.x, end.y, 0.0));
            for found in tree.overlapping(&lifted, reach) {
                tally::add(1);
                let Some(segment) = segments.get(found) else {
                    continue;
                };
                let holds_this_end = segment.track == index
                    && (segment.start.0 == *parameter || segment.end.0 == *parameter);
                if holds_this_end {
                    continue;
                }
                let Some(other) = tracks.get(segment.track) else {
                    continue;
                };
                let Some((on_other, distance)) = nearest_on(other, segment, *end, view) else {
                    continue;
                };
                if distance <= reach
                    && let Some(split) = splits.get_mut(segment.track)
                {
                    split.push(on_other);
                }
            }
        }
    }
    Ok(())
}

fn add_turn_backs(tracks: &[Track], splits: &mut [Vec<f64>]) {
    for (track, split) in tracks.iter().zip(splits.iter_mut()) {
        for triple in track.samples.windows(3) {
            let [(_, first), (middle_parameter, middle), (_, last)] = triple else {
                continue;
            };
            if (*middle - *first).dot(*last - *middle) < 0.0 {
                split.push(*middle_parameter);
            }
        }
    }
}

pub(super) fn add_crossings(
    tracks: &mut [Track],
    view: &Plane,
    chord: f64,
) -> Result<(), HiddenLineError> {
    let segments = segments_of(tracks);
    let tree = BoxTree::new(segments.iter().map(Segment::bounds));
    let mut splits: Vec<Vec<f64>> = vec![Vec::new(); tracks.len()];
    let reach = 2.0 * chord;
    for (index, segment) in segments.iter().enumerate() {
        if index % POLL_EVERY == 0 {
            interrupt::check()?;
        }
        for found in tree.overlapping(&segment.bounds(), reach) {
            if found <= index {
                continue;
            }
            tally::add(1);
            let Some(other) = segments.get(found) else {
                continue;
            };
            if other.track == segment.track && other.index <= segment.index + 1 {
                continue;
            }
            add_crossing(tracks, &mut splits, segment, other, view);
        }
    }
    add_track_ends(tracks, &mut splits, &segments, &tree, reach, view)?;
    add_turn_backs(tracks, &mut splits);
    for (track, found) in tracks.iter_mut().zip(splits) {
        track.splits.extend(found);
    }
    Ok(())
}
