use std::collections::BTreeSet;

use caditor_geometry::{Aabb, Aabb2, Point2, Point3};

use crate::{
    interrupt::{self, Interrupted},
    intersect::{
        SurfaceIntersection, SurfacePatch, boxes_overlap, intersect_surfaces, patch_bounds,
    },
    tolerance::LINEAR_RESOLUTION,
    topology::{FaceContainment, FaceId, Solid, SolidClassifier},
};

const BRANCH_SAMPLES: usize = 9;
const FACE_SAMPLES: usize = 7;

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
        let classifier = self.classifier();
        for (index, first) in extents.iter().enumerate() {
            for second in extents.iter().skip(index + 1) {
                if neighbours.contains(&(first.id, second.id))
                    || !boxes_overlap(&first.bounds, &second.bounds, LINEAR_RESOLUTION)
                {
                    continue;
                }
                interrupt::check()?;
                if let Some(point) = self.crossing_between(&classifier, first, second) {
                    return Ok(Some(Crossing {
                        faces: [first.id, second.id],
                        point,
                    }));
                }
            }
        }
        Ok(None)
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
