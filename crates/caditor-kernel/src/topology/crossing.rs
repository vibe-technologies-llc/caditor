use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Aabb, Aabb2, Point2, Point3};

use crate::{
    box_tree::BoxTree,
    interrupt::{self, Interrupted},
    intersect::{
        SurfaceIntersection, SurfacePatch, boxes_overlap, intersect_curve_surface,
        intersect_curves, intersect_surfaces, patch_bounds,
    },
    tolerance::{LINEAR_RESOLUTION, PCURVE_TOLERANCE},
    topology::{Edge, EdgeId, Face, FaceContainment, FaceId, Solid, SolidClassifier},
};

const BRANCH_SAMPLES: usize = 9;
const FACE_SAMPLES: usize = 7;
const NEAR_SHARED_VERTEX: f64 = PCURVE_TOLERANCE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub faces: [FaceId; 2],
    pub point: Point3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrossingCheck {
    Clear,
    Crossing(Crossing),
    Inconclusive { faces: [FaceId; 2] },
}

impl CrossingCheck {
    pub fn crossing(self) -> Option<Crossing> {
        match self {
            CrossingCheck::Crossing(crossing) => Some(crossing),
            CrossingCheck::Clear | CrossingCheck::Inconclusive { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Probe {
    Clear,
    Crossing(Point3),
    Inconclusive,
}

impl Probe {
    fn then_check(
        self,
        next: impl FnOnce() -> Result<Probe, Interrupted>,
    ) -> Result<Probe, Interrupted> {
        Ok(match self {
            Probe::Crossing(_) => self,
            Probe::Clear => next()?,
            Probe::Inconclusive => match next()? {
                Probe::Crossing(point) => Probe::Crossing(point),
                Probe::Clear | Probe::Inconclusive => Probe::Inconclusive,
            },
        })
    }
}

#[derive(Default)]
struct Survey {
    doubt: bool,
}

impl Survey {
    fn record(&mut self, probe: Probe) -> Option<Point3> {
        match probe {
            Probe::Clear => None,
            Probe::Crossing(point) => Some(point),
            Probe::Inconclusive => {
                self.doubt = true;
                None
            }
        }
    }

    fn outcome(self) -> Probe {
        if self.doubt {
            Probe::Inconclusive
        } else {
            Probe::Clear
        }
    }
}

struct Extent {
    id: FaceId,
    uv: Aabb2,
    bounds: Aabb,
}

impl Solid {
    pub fn find_crossing(&self) -> Result<CrossingCheck, Interrupted> {
        let extents: Vec<Extent> = self
            .faces()
            .filter_map(|(id, face)| {
                let uv = Aabb2::from_points(
                    face.loops()
                        .iter()
                        .filter_map(|loop_id| self.face_loop(*loop_id))
                        .flat_map(|face_loop| face_loop.coedges().iter())
                        .filter_map(|coedge| self.coedge(*coedge))
                        .flat_map(|coedge| {
                            coedge.pcurve().samples().iter().map(|sample| sample.uv)
                        }),
                )?;
                let bounds = patch_bounds(face.surface(), uv).expanded(LINEAR_RESOLUTION);
                Some(Extent { id, uv, bounds })
            })
            .collect();
        let mut neighbours: BTreeSet<(FaceId, FaceId)> = BTreeSet::new();
        for (_, edge) in self.edges() {
            let faces: Vec<FaceId> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| self.coedge_face(*coedge))
                .collect();
            for first in &faces {
                for second in &faces {
                    neighbours.insert((*first, *second));
                }
            }
        }
        let mut inconclusive = None;
        for (id, face) in self.faces() {
            match self.boundary_crossing(face)? {
                Probe::Clear => {}
                Probe::Crossing(point) => {
                    return Ok(CrossingCheck::Crossing(Crossing {
                        faces: [id, id],
                        point,
                    }));
                }
                Probe::Inconclusive => {
                    inconclusive.get_or_insert([id, id]);
                }
            }
        }
        let classifier = self.classifier();
        let tree = BoxTree::new(extents.iter().map(|extent| extent.bounds));
        for (index, first) in extents.iter().enumerate() {
            let later = tree
                .overlapping(&first.bounds, LINEAR_RESOLUTION)
                .into_iter()
                .filter(|other| *other > index)
                .filter_map(|other| extents.get(other));
            for second in later {
                interrupt::check()?;
                let found = if neighbours.contains(&(first.id, second.id)) {
                    self.edges_piercing(&classifier, first.id, second)?
                        .then_check(|| self.edges_piercing(&classifier, second.id, first))?
                } else {
                    self.crossing_between(&classifier, first, second)
                };
                match found {
                    Probe::Clear => {}
                    Probe::Crossing(point) => {
                        return Ok(CrossingCheck::Crossing(Crossing {
                            faces: [first.id, second.id],
                            point,
                        }));
                    }
                    Probe::Inconclusive => {
                        inconclusive.get_or_insert([first.id, second.id]);
                    }
                }
            }
        }
        Ok(match inconclusive {
            Some(faces) => CrossingCheck::Inconclusive { faces },
            None => CrossingCheck::Clear,
        })
    }

    fn edge_uses(&self, face: &Face) -> BTreeMap<EdgeId, usize> {
        let mut uses: BTreeMap<EdgeId, usize> = BTreeMap::new();
        for coedge in face
            .loops()
            .iter()
            .filter_map(|loop_id| self.face_loop(*loop_id))
            .flat_map(|face_loop| face_loop.coedges().iter())
            .filter_map(|coedge| self.coedge(*coedge))
        {
            *uses.entry(coedge.edge()).or_default() += 1;
        }
        uses
    }

    fn edges_piercing(
        &self,
        classifier: &SolidClassifier<'_>,
        source: FaceId,
        target: &Extent,
    ) -> Result<Probe, Interrupted> {
        let (Some(source_face), Some(target_face)) = (self.face(source), self.face(target.id))
        else {
            return Ok(Probe::Inconclusive);
        };
        let mut survey = Survey::default();
        let shared = self.edge_uses(target_face);
        let surface = target_face.surface();
        let inside =
            |uv: Point2| classifier.point_in_face(target.id, uv) == Some(FaceContainment::Inside);
        for edge in self
            .edge_uses(source_face)
            .keys()
            .filter(|id| !shared.contains_key(id))
            .filter_map(|id| self.edge(*id))
        {
            let bounds = edge.curve().bounding_box(edge.interval());
            if !boxes_overlap(&bounds, &target.bounds, LINEAR_RESOLUTION) {
                continue;
            }
            interrupt::check()?;
            let Ok(found) =
                intersect_curve_surface(edge.curve(), edge.interval(), surface, Some(target.uv))
            else {
                survey.record(Probe::Inconclusive);
                continue;
            };
            let piercing = found
                .points
                .iter()
                .filter(|point| !point.tangent && inside(point.uv))
                .map(|point| point.point);
            let lying = found.overlaps.iter().filter_map(|overlap| {
                let point = edge.curve().point(overlap.range.middle());
                inside(surface.project(point, Some(overlap.start_uv))).then_some(point)
            });
            if let Some(point) = piercing.chain(lying).next() {
                return Ok(Probe::Crossing(point));
            }
        }
        Ok(survey.outcome())
    }

    fn boundary_crossing(&self, face: &Face) -> Result<Probe, Interrupted> {
        let uses = self.edge_uses(face);
        let edges: Vec<(&Edge, Aabb)> = uses
            .iter()
            .filter(|(_, count)| **count == 1)
            .filter_map(|(id, _)| self.edge(*id))
            .map(|edge| (edge, edge.curve().bounding_box(edge.interval())))
            .collect();
        let mut survey = Survey::default();
        for (index, (first, first_bounds)) in edges.iter().enumerate() {
            for (second, second_bounds) in edges.iter().skip(index + 1) {
                if !boxes_overlap(first_bounds, second_bounds, LINEAR_RESOLUTION) {
                    continue;
                }
                interrupt::check()?;
                if let Some(point) = survey.record(self.edges_cross(first, second)) {
                    return Ok(Probe::Crossing(point));
                }
            }
        }
        Ok(survey.outcome())
    }

    fn edges_cross(&self, first: &Edge, second: &Edge) -> Probe {
        let Ok(found) = intersect_curves(
            first.curve(),
            first.interval(),
            second.curve(),
            second.interval(),
        ) else {
            return Probe::Inconclusive;
        };
        let shared: Vec<Point3> = [first.start(), first.end()]
            .into_iter()
            .filter(|vertex| [second.start(), second.end()].contains(vertex))
            .filter_map(|vertex| self.vertex(vertex))
            .map(|vertex| vertex.point())
            .collect();
        let transversal = found
            .points
            .iter()
            .filter(|point| !point.tangent)
            .map(|point| point.point);
        let overlapping = found
            .overlaps
            .iter()
            .map(|overlap| first.curve().point(overlap.first.middle()));
        transversal
            .chain(overlapping)
            .find(|point| {
                shared
                    .iter()
                    .all(|vertex| vertex.distance(*point) > NEAR_SHARED_VERTEX)
            })
            .map_or(Probe::Clear, Probe::Crossing)
    }

    fn crossing_between(
        &self,
        classifier: &SolidClassifier<'_>,
        first: &Extent,
        second: &Extent,
    ) -> Probe {
        let (Some(first_face), Some(second_face)) = (self.face(first.id), self.face(second.id))
        else {
            return Probe::Inconclusive;
        };
        let (Ok(first_patch), Ok(second_patch)) = (
            SurfacePatch::new(first_face.surface(), first.uv),
            SurfacePatch::new(second_face.surface(), second.uv),
        ) else {
            return Probe::Inconclusive;
        };
        let inside = |extent: &Extent, point: Point3, hint: Option<Point2>| {
            let surface = self.face(extent.id)?.surface();
            let uv = surface.project(point, hint.or(Some(extent.uv.center())));
            let on_surface = surface.point_at(uv).distance(point) <= LINEAR_RESOLUTION;
            (on_surface && classifier.point_in_face(extent.id, uv) == Some(FaceContainment::Inside))
                .then_some(())
        };
        let Ok(intersection) = intersect_surfaces(&first_patch, &second_patch) else {
            return Probe::Inconclusive;
        };
        let found = match intersection {
            SurfaceIntersection::Coincident(_) => {
                let (low, high) = (first.uv.min(), first.uv.max());
                (0..FACE_SAMPLES)
                    .flat_map(|row| (0..FACE_SAMPLES).map(move |column| (row, column)))
                    .map(|(row, column)| {
                        let fraction = |index: usize| (index as f64 + 0.5) / FACE_SAMPLES as f64;
                        Point2::new(
                            low.x + (high.x - low.x) * fraction(column),
                            low.y + (high.y - low.y) * fraction(row),
                        )
                    })
                    .filter(|uv| {
                        classifier.point_in_face(first.id, *uv) == Some(FaceContainment::Inside)
                    })
                    .map(|uv| first_face.surface().point_at(uv))
                    .find(|point| inside(second, *point, None).is_some())
            }
            SurfaceIntersection::Branches { branches, .. } => branches.iter().find_map(|branch| {
                (0..BRANCH_SAMPLES).find_map(|sample| {
                    let fraction = (sample as f64 + 0.5) / BRANCH_SAMPLES as f64;
                    let point = branch.curve.point(branch.range.at(fraction));
                    let [first_hint, second_hint] = branch.start_uv;
                    inside(first, point, Some(first_hint))?;
                    inside(second, point, Some(second_hint))?;
                    Some(point)
                })
            }),
        };
        found.map_or(Probe::Clear, Probe::Crossing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crossing_found_after_a_doubt_still_counts() {
        let point = Point3::new(1.0, 2.0, 3.0);
        let crossing = || Ok(Probe::Crossing(point));
        let clear = || Ok(Probe::Clear);
        let doubt = || Ok(Probe::Inconclusive);

        assert_eq!(
            Probe::Inconclusive.then_check(crossing),
            Ok(Probe::Crossing(point))
        );
        assert_eq!(
            Probe::Inconclusive.then_check(clear),
            Ok(Probe::Inconclusive)
        );
        assert_eq!(Probe::Clear.then_check(doubt), Ok(Probe::Inconclusive));
        assert_eq!(Probe::Clear.then_check(clear), Ok(Probe::Clear));
        assert_eq!(
            Probe::Crossing(point).then_check(doubt),
            Ok(Probe::Crossing(point))
        );
    }

    #[test]
    fn a_survey_with_any_failed_check_is_inconclusive() {
        let mut clean = Survey::default();
        assert_eq!(clean.record(Probe::Clear), None);
        assert_eq!(clean.outcome(), Probe::Clear);

        let mut doubtful = Survey::default();
        assert_eq!(doubtful.record(Probe::Inconclusive), None);
        assert_eq!(doubtful.record(Probe::Clear), None);
        assert_eq!(doubtful.outcome(), Probe::Inconclusive);
    }
}
