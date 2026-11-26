use caditor_geometry::{Aabb, Point3};

use super::{
    Build, HiddenLineError, Own, Scene, SolidView, Track, TrackPlan, ViewSource, tracks_of,
};
use crate::{
    fan_out,
    interrupt::{self, Interrupted},
    intersect::{
        IntersectionBranch, IntersectionError, SurfaceIntersection, SurfacePatch, boxes_overlap,
        inside_intervals, intersect_curve_surface, intersect_surfaces,
    },
    interval::Interval,
    naming::FaceName,
    tally,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceContainment, FaceId, Solid},
};

const PAIRS_PER_THREAD: usize = 8;
const CROSSING_SAMPLES: usize = 16;
const ON_BRANCH: f64 = 10.0 * LINEAR_RESOLUTION;
const AT_BOUNDARY: f64 = 100.0 * LINEAR_RESOLUTION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FacePair {
    pub solids: [usize; 2],
    pub faces: [FaceId; 2],
}

#[derive(Default)]
pub(super) struct Crossings {
    pub tracks: Vec<Track>,
    pub failed: usize,
}

struct CrossedFace<'a> {
    view: &'a SolidView<'a>,
    id: FaceId,
    name: FaceName,
    patch: SurfacePatch<'a>,
    bounds: Aabb,
}

impl<'a> CrossedFace<'a> {
    fn of(view: &'a SolidView<'a>, id: FaceId) -> Option<Self> {
        let face = view.solid.face(id)?;
        let uv_box = view.classifier.face_uv_box(id)?;
        Some(Self {
            view,
            id,
            name: face.name(),
            patch: SurfacePatch::new(face.surface(), uv_box).ok()?,
            bounds: view.classifier.face_bounds(id)?,
        })
    }

    fn holds_strictly(&self, point: Point3) -> bool {
        let uv = self.patch.place(point);
        self.view.classifier.point_in_face(self.id, uv) == Some(FaceContainment::Inside)
    }

    fn boundary_hits(&self, onto: &CrossedFace<'_>) -> Result<Vec<Point3>, Interrupted> {
        let solid = self.view.solid;
        let mut hits = Vec::new();

        for id in boundary_edges(solid, self.id) {
            let Some(edge) = solid.edge(id) else {
                continue;
            };

            let (curve, range) = (edge.curve(), edge.interval());

            if !boxes_overlap(&curve.bounding_box(range), &onto.bounds, LINEAR_RESOLUTION) {
                continue;
            }

            let found = match intersect_curve_surface(
                curve,
                range,
                onto.patch.surface(),
                Some(onto.patch.bounds()),
            ) {
                Ok(found) => found,
                Err(IntersectionError::Cancelled(interrupted)) => return Err(interrupted),
                Err(_) => continue,
            };

            hits.extend(found.points.iter().map(|hit| hit.point));
            hits.extend(found.overlaps.iter().flat_map(|overlap| {
                [
                    curve.point(overlap.range.start()),
                    curve.point(overlap.range.end()),
                ]
            }));
        }
        Ok(hits)
    }
}

fn boundary_edges(solid: &Solid, face: FaceId) -> Vec<EdgeId> {
    let mut edges: Vec<EdgeId> = solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops().iter())
        .filter_map(|loop_id| solid.face_loop(*loop_id))
        .flat_map(|face_loop| face_loop.coedges().iter())
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
        .collect();

    edges.sort_unstable();
    edges.dedup();
    edges
}

pub(super) fn face_pairs(scene: &Scene<'_>) -> Vec<FacePair> {
    let mut pairs = Vec::new();

    for (first, view) in scene.views.iter().enumerate() {
        let Some(bounds) = view.classifier.bounds() else {
            continue;
        };

        let mut others = scene.tree.overlapping(&bounds, 0.0);

        others.sort_unstable();
        for second in others.into_iter().filter(|second| *second > first) {
            let Some(other) = scene.views.get(second) else {
                continue;
            };
            let Some(other_bounds) = other.classifier.bounds() else {
                continue;
            };

            for (face, _) in view.classifier.faces_overlapping(&other_bounds) {
                let Some(face_bounds) = view.classifier.face_bounds(face) else {
                    continue;
                };

                pairs.extend(
                    other
                        .classifier
                        .faces_overlapping(&face_bounds)
                        .into_iter()
                        .map(|(theirs, _)| FacePair {
                            solids: [first, second],
                            faces: [face, theirs],
                        }),
                );
            }
        }
    }
    pairs
}

fn snapped(parameter: f64, branch: &IntersectionBranch, seeds: &[(f64, Point3)]) -> f64 {
    let at = branch.curve.point(parameter);
    seeds
        .iter()
        .map(|(seed, point)| (*seed, point.distance(at)))
        .filter(|(_, distance)| *distance <= AT_BOUNDARY)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(parameter, |(seed, _)| seed)
}

fn joined_round_the_seam(branch: &IntersectionBranch, mut kept: Vec<Interval>) -> Vec<Interval> {
    let Some(period) = branch.curve.period().filter(|_| branch.closed) else {
        return kept;
    };

    if kept.len() < 2 {
        return kept;
    }

    let (Some(first), Some(last)) = (kept.first().copied(), kept.last().copied()) else {
        return kept;
    };

    if first.start() != branch.range.start() || last.end() != branch.range.end() {
        return kept;
    }

    if let Some(joined) = Interval::new(last.start(), first.end() + period) {
        kept.remove(0);
        if let Some(end) = kept.last_mut() {
            *end = joined;
        }
    }
    kept
}

fn kept_ranges(
    branch: &IntersectionBranch,
    faces: [&CrossedFace<'_>; 2],
    hits: &[Point3],
) -> Vec<Interval> {
    let (curve, range) = (&branch.curve, branch.range);

    let seeds: Vec<(f64, Point3)> = hits
        .iter()
        .filter_map(|point| {
            let parameter = curve.closest_parameter(*point, range);
            (curve.point(parameter).distance(*point) <= ON_BRANCH).then_some((parameter, *point))
        })
        .collect();
    let parameters: Vec<f64> = seeds.iter().map(|(parameter, _)| *parameter).collect();

    let inside = |parameter: f64| {
        let point = curve.point(parameter);
        faces.iter().all(|face| face.holds_strictly(point))
    };
    let kept = inside_intervals(range, CROSSING_SAMPLES, &parameters, inside)
        .into_iter()
        .filter_map(|kept| {
            Interval::new(
                snapped(kept.start(), branch, &seeds),
                snapped(kept.end(), branch, &seeds),
            )
        })
        .filter(|kept| curve.is_longer_than(*kept, LINEAR_RESOLUTION))
        .collect();
    joined_round_the_seam(branch, kept)
}

impl Build<'_> {
    pub(super) fn crossing_tracks(&self, scene: &Scene<'_>) -> Result<Crossings, HiddenLineError> {
        if scene.views.len() < 2 {
            return Ok(Crossings::default());
        }

        let pairs = face_pairs(scene);
        let made =
            fan_out::each_in_order(
                pairs.len(),
                self.threads,
                PAIRS_PER_THREAD,
                |at| match pairs.get(at) {
                    Some(pair) => self.pair_tracks(scene, *pair),
                    None => Ok(Crossings::default()),
                },
            );
        let mut crossings = Crossings::default();

        for found in made {
            let found = found?;

            crossings.tracks.extend(found.tracks);
            crossings.failed += found.failed;
        }
        Ok(crossings)
    }

    fn pair_tracks(&self, scene: &Scene<'_>, pair: FacePair) -> Result<Crossings, HiddenLineError> {
        interrupt::check()?;

        let [first, second] = pair.solids;
        let [mine, theirs] = pair.faces;

        let (Some(one), Some(two)) = (scene.views.get(first), scene.views.get(second)) else {
            return Ok(Crossings::default());
        };

        let (Some(near), Some(far)) = (CrossedFace::of(one, mine), CrossedFace::of(two, theirs))
        else {
            return Ok(Crossings::default());
        };

        tally::add(1);

        let found = match intersect_surfaces(&near.patch, &far.patch) {
            Ok(found) => found,
            Err(IntersectionError::Cancelled(interrupted)) => return Err(interrupted.into()),
            Err(_) => {
                return Ok(Crossings {
                    tracks: Vec::new(),
                    failed: 1,
                });
            }
        };
        let SurfaceIntersection::Branches { branches, .. } = found else {
            return Ok(Crossings::default());
        };
        let crossing: Vec<&IntersectionBranch> =
            branches.iter().filter(|branch| !branch.tangent).collect();

        if crossing.is_empty() {
            return Ok(Crossings::default());
        }

        let mut hits = near.boundary_hits(&far)?;

        hits.extend(far.boundary_hits(&near)?);

        let source = ViewSource::Crossing {
            solids: pair.solids,
            faces: pair.faces,
            names: [near.name, far.name],
        };
        let mut tracks = Vec::new();

        for branch in crossing {
            for range in kept_ranges(branch, [&near, &far], &hits) {
                let plan = TrackPlan {
                    closed: branch.closed && range == branch.range,
                    sampled: false,
                    source,
                    own: Own::Crossing {
                        other: second,
                        faces: pair.faces,
                    },
                    solid: first,
                    faces: vec![mine],
                    splits: Vec::new(),
                };

                tracks.extend(tracks_of(
                    &branch.curve,
                    range,
                    &plan,
                    self.view,
                    self.tolerance,
                )?);
            }
        }
        Ok(Crossings { tracks, failed: 0 })
    }
}
