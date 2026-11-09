use std::collections::BTreeMap;

use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector2};

use super::{
    HiddenLineError, HiddenLineView, Scene, TRACKS_PER_THREAD, Track, ViewEdgeKind, ViewSource,
    Visibility, decided,
    flatten::{flat_parameter, flat_vector},
    piece_ends, tracks_of_scene,
    visibility::covered_along,
};
use crate::{
    box_tree::BoxTree,
    curve2::Curve2,
    fan_out::{self, available_threads},
    interrupt,
    interval::Interval,
    tally,
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{RayView, Solid},
};

const SIDE_STEP: f64 = 32.0 * LINEAR_RESOLUTION;
const EXACT_SAME_LINE: f64 = 4.0 * LINEAR_RESOLUTION;
const BEHIND_MARGIN: f64 = 1.0;
const SAMPLES_PER_RUN: usize = 16;
const SMALLEST_LOOP_AREA: f64 = 1e-12;

#[derive(Debug, Clone, PartialEq)]
pub struct OutlinePiece {
    pub curve: Curve2,
    pub range: Interval,
    pub source: ViewSource,
    pub exact: bool,
    pub occurrence: usize,
    pub of: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    track: usize,
    range: Interval,
    material_on_left: bool,
    rank: u8,
}

fn rank_of(source: &ViewSource) -> Option<u8> {
    match source {
        ViewSource::Edge { kind, .. } => match kind {
            ViewEdgeKind::Sharp => Some(0),
            ViewEdgeKind::Tangent => Some(1),
            ViewEdgeKind::Seam => None,
        },
        ViewSource::Silhouette { .. } => Some(2),
    }
}

struct Sides<'a> {
    scene: &'a Scene<'a>,
    view: &'a Plane,
    start_height: f64,
}

impl Sides<'_> {
    fn material_at(&self, point: Point2) -> Result<bool, HiddenLineError> {
        let origin = self.view.to_world(point) + self.view.normal() * self.start_height;
        match covered_along(self.scene, origin, self.view.normal(), None, &[]) {
            RayView::Covered => Ok(true),
            RayView::Clear => Ok(false),
            RayView::Undecided => {
                interrupt::check()?;
                Err(HiddenLineError::OutlineUndecided)
            }
        }
    }

    fn material_on_left(
        &self,
        track: &Track,
        range: Interval,
    ) -> Result<Option<bool>, HiddenLineError> {
        let parameter = range.middle();
        let at = track.curve.evaluate(parameter);
        let along = flat_vector(self.view, at.first)
            .try_normalize()
            .or_else(|| {
                let start = self.view.to_local(track.curve.point(range.start()));
                let end = self.view.to_local(track.curve.point(range.end()));
                (end - start).try_normalize()
            })
            .unwrap_or(Vector2::X);
        let middle = self.view.to_local(at.point);
        let left = middle + along.perp() * SIDE_STEP;
        let right = middle - along.perp() * SIDE_STEP;
        Ok(match (self.material_at(left)?, self.material_at(right)?) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        })
    }
}

fn start_height(scene: &Scene<'_>, view: &Plane) -> f64 {
    let bounds = scene
        .tree
        .bounds()
        .unwrap_or_else(|| Aabb::from_point(Point3::ZERO));
    bounds
        .corners()
        .iter()
        .map(|corner| view.signed_distance(*corner))
        .fold(f64::INFINITY, f64::min)
        - BEHIND_MARGIN
}

fn track_runs(index: usize, track: &Track, sides: &Sides<'_>) -> Result<Vec<Run>, HiddenLineError> {
    let Some(rank) = rank_of(&track.source) else {
        return Ok(Vec::new());
    };
    let mut runs = Vec::new();
    for pair in piece_ends(track, sides.view).windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let Some(range) = Interval::new(*start, *end) else {
            continue;
        };
        let Some(visibility) = decided(track, range, sides.scene, sides.view.normal())? else {
            return Err(HiddenLineError::OutlineUndecided);
        };
        if visibility != Visibility::Visible {
            continue;
        }
        let Some(material_on_left) = sides.material_on_left(track, range)? else {
            continue;
        };
        runs.push(Run {
            track: index,
            range,
            material_on_left,
            rank,
        });
    }
    Ok(runs)
}

fn boundary_runs(
    tracks: &[Track],
    scene: &Scene<'_>,
    view: &Plane,
    threads: usize,
) -> Result<Vec<Run>, HiddenLineError> {
    let sides = Sides {
        scene,
        view,
        start_height: start_height(scene, view),
    };
    let found =
        fan_out::each_in_order(
            tracks.len(),
            threads,
            TRACKS_PER_THREAD,
            |index| match tracks.get(index) {
                Some(track) => track_runs(index, track, &sides),
                None => Ok(Vec::new()),
            },
        );
    let mut runs = Vec::new();
    for track_runs in found {
        runs.extend(track_runs?);
    }
    Ok(runs)
}

fn samples_along(track: &Track, range: Interval, view: &Plane) -> Vec<Point2> {
    range
        .split(SAMPLES_PER_RUN)
        .map(|parameter| view.to_local(track.curve.point(parameter)))
        .collect()
}

fn distance_to_chain(point: Point2, chain: &[Point2]) -> f64 {
    chain
        .windows(2)
        .filter_map(|pair| match pair {
            [start, end] => {
                let along = *end - *start;
                let length_squared = along.length_squared();
                let fraction = if length_squared > 0.0 {
                    ((point - *start).dot(along) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Some(point.distance(*start + along * fraction))
            }
            _ => None,
        })
        .fold(f64::INFINITY, f64::min)
}

fn box_of(points: &[Point2]) -> Aabb {
    let lifted = |point: &Point2| Point3::new(point.x, point.y, 0.0);
    Aabb::from_points(points.iter().map(lifted)).unwrap_or_else(|| Aabb::from_point(Point3::ZERO))
}

fn without_coincident(
    runs: Vec<Run>,
    tracks: &[Track],
    view: &Plane,
    chord: f64,
) -> Result<Vec<Run>, HiddenLineError> {
    let chains: Vec<Vec<Point2>> = runs
        .iter()
        .map(|run| {
            tracks
                .get(run.track)
                .map_or_else(Vec::new, |track| samples_along(track, run.range, view))
        })
        .collect();
    let reach = 2.0 * chord + EXACT_SAME_LINE;
    let tree = BoxTree::new(chains.iter().map(|chain| box_of(chain)));
    let mut kept = Vec::with_capacity(runs.len());
    for (index, run) in runs.iter().enumerate() {
        interrupt::check()?;
        let (Some(chain), Some(track)) = (chains.get(index), tracks.get(run.track)) else {
            continue;
        };
        let middle = view.to_local(track.curve.point(run.range.middle()));
        let lifted = Aabb::from_point(Point3::new(middle.x, middle.y, 0.0));
        let covered = tree.overlapping(&lifted, reach).into_iter().any(|other| {
            tally::add(1);
            let (Some(other_run), Some(other_chain)) = (runs.get(other), chains.get(other)) else {
                return false;
            };
            let Some(other_track) = tracks.get(other_run.track) else {
                return false;
            };
            let outranks =
                other_run.rank < run.rank || (other_run.rank == run.rank && other < index);
            let allowed = if track.sampled || other_track.sampled {
                reach
            } else {
                EXACT_SAME_LINE
            };
            other != index
                && outranks
                && other_run.material_on_left_agrees(run, track, other_track, view)
                && distance_to_chain(middle, other_chain) <= allowed
                && !chain.is_empty()
        });
        if !covered {
            kept.push(*run);
        }
    }
    Ok(kept)
}

impl Run {
    fn material_on_left_agrees(
        &self,
        other: &Self,
        own_track: &Track,
        other_track: &Track,
        view: &Plane,
    ) -> bool {
        let direction = |track: &Track, run: &Self| {
            let at = track.curve.evaluate(run.range.middle());
            flat_vector(view, at.first) * if run.material_on_left { 1.0 } else { -1.0 }
        };
        direction(own_track, self).dot(direction(other_track, other)) >= 0.0
    }
}

fn joined_runs(runs: Vec<Run>, tracks: &[Track]) -> Vec<Run> {
    let mut joined: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs {
        match joined.last_mut() {
            Some(last)
                if last.track == run.track
                    && last.material_on_left == run.material_on_left
                    && last.range.end() == run.range.start() =>
            {
                if let Some(range) = Interval::new(last.range.start(), run.range.end()) {
                    last.range = range;
                }
            }
            _ => joined.push(run),
        }
    }
    let mut seam_joined: Vec<Run> = Vec::with_capacity(joined.len());
    for run in joined {
        let earlier = tracks
            .get(run.track)
            .filter(|track| track.closed)
            .and_then(|track| track.curve.period().map(|period| (track, period)))
            .and_then(|(track, period)| {
                seam_joined
                    .iter()
                    .position(|first| {
                        first.track == run.track
                            && first.material_on_left == run.material_on_left
                            && first.range.start() == track.flat.range.start()
                            && run.range.end() == track.flat.range.end()
                    })
                    .map(|position| (position, period))
            });
        match earlier {
            Some((position, period)) => {
                if let Some(first) = seam_joined.get_mut(position)
                    && let Some(range) =
                        Interval::new(run.range.start(), first.range.end() + period)
                {
                    first.range = range;
                }
            }
            None => seam_joined.push(run),
        }
    }
    seam_joined
}

fn oriented_chain(run: &Run, tracks: &[Track], view: &Plane) -> Vec<Point2> {
    let mut points = tracks
        .get(run.track)
        .map_or_else(Vec::new, |track| samples_along(track, run.range, view));
    if !run.material_on_left {
        points.reverse();
    }
    points
}

fn signed_area(chain: &[Point2]) -> f64 {
    let Some(first) = chain.first() else {
        return 0.0;
    };
    let closing = chain.iter().skip(1).chain(std::iter::once(first));
    chain
        .iter()
        .zip(closing)
        .map(|(from, to)| from.x * to.y - to.x * from.y)
        .sum::<f64>()
        * 0.5
}

fn outer_loops(
    runs: &[Run],
    tracks: &[Track],
    view: &Plane,
    chord: f64,
) -> Result<Vec<usize>, HiddenLineError> {
    let chains: Vec<Vec<Point2>> = runs
        .iter()
        .map(|run| oriented_chain(run, tracks, view))
        .collect();
    let reach = 2.0 * chord + EXACT_SAME_LINE;
    let mut used = vec![false; runs.len()];
    let mut kept = Vec::new();
    for start in 0..runs.len() {
        interrupt::check()?;
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let Some(begin) = chains.get(start).and_then(|chain| chain.first().copied()) else {
            continue;
        };
        let mut members = vec![start];
        let mut walked: Vec<Point2> = Vec::new();
        let mut current = start;
        if let Some(slot) = used.get_mut(start) {
            *slot = true;
        }
        loop {
            let Some(chain) = chains.get(current) else {
                return Err(HiddenLineError::OutlineOpen);
            };
            walked.extend(chain.iter().copied());
            let Some(tip) = chain.last().copied() else {
                return Err(HiddenLineError::OutlineOpen);
            };
            if tip.distance(begin) <= reach {
                break;
            }
            let next = chains
                .iter()
                .enumerate()
                .filter(|(index, _)| !used.get(*index).copied().unwrap_or(true))
                .filter_map(|(index, candidate)| {
                    let distance = candidate.first()?.distance(tip);
                    (distance <= reach).then_some((index, distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((index, _)) = next else {
                return Err(HiddenLineError::OutlineOpen);
            };
            if let Some(slot) = used.get_mut(index) {
                *slot = true;
            }
            members.push(index);
            current = index;
        }
        if signed_area(&walked) > SMALLEST_LOOP_AREA {
            kept.extend(members);
        }
    }
    Ok(kept)
}

fn piece_of(run: &Run, track: &Track, view: &Plane) -> Option<OutlinePiece> {
    let to_flat = |parameter: f64| flat_parameter(&track.flat, &track.curve, parameter, view);
    let (start, end) = (to_flat(run.range.start()), to_flat(run.range.end()));
    Some(OutlinePiece {
        curve: track.flat.curve.clone(),
        range: Interval::new(start.min(end), start.max(end))?,
        source: track.source,
        exact: !track.sampled,
        occurrence: 0,
        of: 0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourceKey {
    Edge(usize),
    Face(usize),
}

fn source_key(source: &ViewSource) -> SourceKey {
    match source {
        ViewSource::Edge { edge, .. } => SourceKey::Edge(edge.index()),
        ViewSource::Silhouette { face, .. } => SourceKey::Face(face.index()),
    }
}

fn numbered(pieces: Vec<OutlinePiece>) -> Vec<OutlinePiece> {
    let mut groups: BTreeMap<SourceKey, Vec<OutlinePiece>> = BTreeMap::new();
    for piece in pieces {
        groups
            .entry(source_key(&piece.source))
            .or_default()
            .push(piece);
    }
    let mut numbered = Vec::new();
    for group in groups.into_values() {
        let mut group = group;
        group.sort_by(|a, b| {
            let middle = |piece: &OutlinePiece| piece.curve.point(piece.range.middle());
            let (a, b) = (middle(a), middle(b));
            a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
        });
        let of = group.len();
        for (occurrence, mut piece) in group.into_iter().enumerate() {
            piece.occurrence = occurrence;
            piece.of = of;
            numbered.push(piece);
        }
    }
    numbered
}

pub fn outer_outline(
    solid: &Solid,
    view: &HiddenLineView,
    tolerance: &SamplingTolerance,
) -> Result<Vec<OutlinePiece>, HiddenLineError> {
    outer_outline_on(solid, view, tolerance, available_threads())
}

pub(super) fn outer_outline_on(
    solid: &Solid,
    view: &HiddenLineView,
    tolerance: &SamplingTolerance,
    threads: usize,
) -> Result<Vec<OutlinePiece>, HiddenLineError> {
    let plane = view.frame();
    let scene = Scene::of(&[solid]);
    let (tracks, _) = tracks_of_scene(&scene, plane, tolerance)?;
    let runs = boundary_runs(&tracks, &scene, plane, threads)?;
    let runs = without_coincident(runs, &tracks, plane, tolerance.chord())?;
    let runs = joined_runs(runs, &tracks);
    let outer = outer_loops(&runs, &tracks, plane, tolerance.chord())?;
    let pieces = outer
        .into_iter()
        .filter_map(|index| {
            let run = runs.get(index)?;
            piece_of(run, tracks.get(run.track)?, plane)
        })
        .collect();
    Ok(numbered(pieces))
}
