use std::collections::BTreeSet;

use caditor_geometry::{Aabb, Aabb2, Point2, Point3};

use crate::{
    boolean::{
        BooleanError,
        imprint::Arrangement,
        trace::{Chart, Coedge, Fragment, TracedLoop, depth},
    },
    box_tree::BoxTree,
    interrupt,
    intersect::boxes_overlap,
    surface::Surface,
    tolerance::LINEAR_RESOLUTION,
    topology::{PcurveError, signed_area},
};

const MAX_ROUNDS: usize = 12;
const FINEST_TOLERANCE: f64 = 0.1 * LINEAR_RESOLUTION;
const RELIABLE_DEPTH: f64 = 2.0;
const DEEP_GATE: f64 = 8.0;

type CoedgeAt = (usize, usize);

fn flat_box([a, b]: [Point2; 2]) -> Aabb {
    Aabb::from_point(Point3::new(a.x, a.y, 0.0)).including(Point3::new(b.x, b.y, 0.0))
}

fn properly_cross([a, b]: [Point2; 2], [c, d]: [Point2; 2]) -> bool {
    let side = |from: Point2, to: Point2, point: Point2| (to - from).perp_dot(point - from);
    let (first, second) = (side(a, b, c), side(a, b, d));
    let (third, fourth) = (side(c, d, a), side(c, d, b));
    first * second < 0.0 && third * fourth < 0.0
}

struct Track {
    at: CoedgeAt,
    segments: Vec<[Point2; 2]>,
    bounds: Aabb,
}

fn tracks(loops: &[TracedLoop]) -> Vec<Track> {
    let mut found = Vec::new();
    for (loop_index, traced) in loops.iter().enumerate() {
        for (coedge_index, coedge) in traced.coedges.iter().enumerate() {
            let segments: Vec<[Point2; 2]> = coedge
                .pcurve
                .samples()
                .windows(2)
                .filter_map(|pair| match pair {
                    [a, b] if a.uv != b.uv => Some([a.uv, b.uv]),
                    _ => None,
                })
                .collect();
            let Some(bounds) = segments
                .iter()
                .map(|segment| flat_box(*segment))
                .reduce(Aabb::union)
            else {
                continue;
            };
            found.push(Track {
                at: (loop_index, coedge_index),
                segments,
                bounds,
            });
        }
    }
    found
}

fn tracks_cross(first: &Track, second: &Track) -> bool {
    let low = first.bounds.min().max(second.bounds.min());
    let high = first.bounds.max().min(second.bounds.max());
    if low.cmpgt(high).any() {
        return false;
    }
    let shared = Aabb::from_point(low).including(high);
    let near = |track: &Track| -> Vec<[Point2; 2]> {
        track
            .segments
            .iter()
            .copied()
            .filter(|segment| boxes_overlap(&flat_box(*segment), &shared, 0.0))
            .collect()
    };
    let (mine, theirs) = (near(first), near(second));
    let tree = BoxTree::new(theirs.iter().map(|segment| flat_box(*segment)));
    mine.iter().any(|segment| {
        tree.overlapping(&flat_box(*segment), 0.0)
            .into_iter()
            .filter_map(|index| theirs.get(index))
            .any(|other| properly_cross(*segment, *other))
    })
}

fn crossed_coedges(loops: &[TracedLoop]) -> Result<BTreeSet<CoedgeAt>, BooleanError> {
    let tracks = tracks(loops);
    let tree = BoxTree::new(tracks.iter().map(|track| track.bounds));
    let mut crossed = BTreeSet::new();
    for (index, track) in tracks.iter().enumerate() {
        interrupt::check()?;
        for other in tree.overlapping(&track.bounds, 0.0) {
            if other <= index {
                continue;
            }
            if let Some(other) = tracks.get(other)
                && tracks_cross(track, other)
            {
                crossed.insert(track.at);
                crossed.insert(other.at);
            }
        }
    }
    Ok(crossed)
}

fn halve(
    arrangement: &Arrangement,
    chart: &Chart,
    coedge: &mut Coedge,
) -> Result<bool, BooleanError> {
    let tolerance = 0.5 * coedge.pcurve.tolerance();
    let Some((curve, _)) = arrangement.curve(coedge.half_edge.piece) else {
        return Ok(false);
    };
    if tolerance < FINEST_TOLERANCE {
        return Ok(false);
    }
    match coedge
        .pcurve
        .refined_within(chart.surface, curve, tolerance)
    {
        Ok(pcurve) => {
            coedge.pcurve = pcurve;
            Ok(true)
        }
        Err(PcurveError::Cancelled(interrupted)) => Err(interrupted.into()),
        Err(_) => Ok(false),
    }
}

fn update_areas(loops: &mut [TracedLoop]) {
    for traced in loops {
        traced.area = signed_area(&traced.polygon());
    }
}

pub(super) fn untangle(
    arrangement: &Arrangement,
    chart: &Chart,
    loops: &mut [TracedLoop],
) -> Result<(), BooleanError> {
    for _ in 0..MAX_ROUNDS {
        let crossed = crossed_coedges(loops)?;
        let mut refined_any = false;
        for (loop_index, coedge_index) in crossed {
            if let Some(coedge) = loops
                .get_mut(loop_index)
                .and_then(|traced| traced.coedges.get_mut(coedge_index))
            {
                refined_any |= halve(arrangement, chart, coedge)?;
            }
        }
        if !refined_any {
            break;
        }
        update_areas(loops);
    }
    Ok(())
}

pub(super) fn sharpen(
    arrangement: &Arrangement,
    chart: &Chart,
    fragments: &mut [Fragment],
) -> Result<(), BooleanError> {
    for fragment in fragments {
        for _ in 0..MAX_ROUNDS {
            interrupt::check()?;
            let coarsest = fragment
                .loops
                .iter()
                .flat_map(|traced| traced.coedges.iter())
                .map(|coedge| coedge.pcurve.tolerance())
                .fold(0.0, f64::max);
            if surely_deep(fragment, chart.surface, coarsest) {
                break;
            }
            let reliable = depth(fragment, chart.surface) / RELIABLE_DEPTH;
            let mut refined_any = false;
            for coedge in fragment
                .loops
                .iter_mut()
                .flat_map(|traced| traced.coedges.iter_mut())
            {
                if coedge.pcurve.tolerance() > reliable {
                    refined_any |= halve(arrangement, chart, coedge)?;
                }
            }
            if !refined_any {
                break;
            }
            update_areas(&mut fragment.loops);
        }
    }
    Ok(())
}

fn surely_deep(fragment: &Fragment, surface: &Surface, tolerance: f64) -> bool {
    let polygons: Vec<Vec<Point2>> = fragment.loops.iter().map(TracedLoop::polygon).collect();
    let Some(bounds) = Aabb2::from_points(polygons.iter().flatten().copied()) else {
        return false;
    };
    let area: f64 = fragment.loops.iter().map(|traced| traced.area).sum();
    let perimeter: f64 = polygons
        .iter()
        .map(|polygon| {
            polygon
                .iter()
                .zip(polygon.iter().cycle().skip(1))
                .map(|(a, b)| a.distance(*b))
                .sum::<f64>()
        })
        .sum();
    let center = bounds.center();
    let derivatives = surface.evaluate(center.x, center.y);
    let speed = derivatives.du.length().min(derivatives.dv.length());
    let thickness = area.abs() / perimeter * speed;
    thickness.is_finite() && thickness >= DEEP_GATE * tolerance
}
