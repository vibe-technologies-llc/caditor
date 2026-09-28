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

struct Extent {
    id: FaceId,
    uv: Aabb2,
    bounds: Aabb,
}

impl Solid {
    pub fn find_crossing(&self) -> Result<Option<Crossing>, Interrupted> {
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
        for (id, face) in self.faces() {
            if let Some(point) = self.boundary_crossing(face)? {
                return Ok(Some(Crossing {
                    faces: [id, id],
                    point,
                }));
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
                        .or(self.edges_piercing(&classifier, second.id, first)?)
                } else {
                    self.crossing_between(&classifier, first, second)
                };
                if let Some(point) = found {
                    return Ok(Some(Crossing {
                        faces: [first.id, second.id],
                        point,
                    }));
                }
            }
        }
        Ok(None)
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
    ) -> Result<Option<Point3>, Interrupted> {
        let (Some(source_face), Some(target_face)) = (self.face(source), self.face(target.id))
        else {
            return Ok(None);
        };
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
                return Ok(Some(point));
            }
        }
        Ok(None)
    }

    fn boundary_crossing(&self, face: &Face) -> Result<Option<Point3>, Interrupted> {
        let uses = self.edge_uses(face);
        let edges: Vec<(&Edge, Aabb)> = uses
            .iter()
            .filter(|(_, count)| **count == 1)
            .filter_map(|(id, _)| self.edge(*id))
            .map(|edge| (edge, edge.curve().bounding_box(edge.interval())))
            .collect();
        for (index, (first, first_bounds)) in edges.iter().enumerate() {
            for (second, second_bounds) in edges.iter().skip(index + 1) {
                if !boxes_overlap(first_bounds, second_bounds, LINEAR_RESOLUTION) {
                    continue;
                }
                interrupt::check()?;
                if let Some(point) = self.edges_cross(first, second) {
                    return Ok(Some(point));
                }
            }
        }
        Ok(None)
    }

    fn edges_cross(&self, first: &Edge, second: &Edge) -> Option<Point3> {
        let found = intersect_curves(
            first.curve(),
            first.interval(),
            second.curve(),
            second.interval(),
        )
        .ok()?;
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
        transversal.chain(overlapping).find(|point| {
            shared
                .iter()
                .all(|vertex| vertex.distance(*point) > NEAR_SHARED_VERTEX)
        })
    }

    fn crossing_between(
        &self,
        classifier: &SolidClassifier<'_>,
        first: &Extent,
        second: &Extent,
    ) -> Option<Point3> {
        let (first_face, second_face) = (self.face(first.id)?, self.face(second.id)?);
        let first_patch = SurfacePatch::new(first_face.surface(), first.uv).ok()?;
        let second_patch = SurfacePatch::new(second_face.surface(), second.uv).ok()?;
        let inside = |extent: &Extent, point: Point3, hint: Option<Point2>| {
            let surface = self.face(extent.id)?.surface();
            let uv = surface.project(point, hint.or(Some(extent.uv.center())));
            let on_surface = surface.point_at(uv).distance(point) <= LINEAR_RESOLUTION;
            (on_surface && classifier.point_in_face(extent.id, uv) == Some(FaceContainment::Inside))
                .then_some(())
        };
        match intersect_surfaces(&first_patch, &second_patch).ok()? {
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
        }
    }
}
