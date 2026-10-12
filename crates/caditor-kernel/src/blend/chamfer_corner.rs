use std::collections::BTreeMap;

use caditor_geometry::{Plane, Point3, Vector3};

use super::{
    BlendError, CLEARANCE, EdgeGeometry, Sweep, Topology,
    corner::{Builder, meet, plane_normal},
    leaving,
    section::Blend,
};
use crate::{
    build::SweepError,
    naming::{FaceName, FaceOrigin, VertexName},
    sense::Sense,
    surface::PlaneSurface,
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, FaceId, Solid, VertexId},
};

type Bound = (Vector3, f64);

pub(super) type Bevelled = BTreeMap<EdgeId, (EdgeGeometry, Blend, f64)>;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ChamferCorner {
    faces: [FaceId; 3],
    normals: [Vector3; 3],
    edge_faces: [[usize; 2]; 3],
    apex: Point3,
    cut_point: Point3,
    toward_apex: Vector3,
    clearance: f64,
}

fn through(point: Point3, normal: Vector3) -> Bound {
    (normal, normal.dot(point))
}

fn chamfer_plane(geometry: &EdgeGeometry, blend: &Blend) -> Option<Bound> {
    let [first, second] = blend.feet;
    let first = geometry.place(first, 0.0)?;
    let second = geometry.place(second, 0.0)?;
    let normal = (second - first)
        .cross(geometry.frame.normal())
        .try_normalize()?;
    Some(through(first, normal))
}

fn edges_of(edge_faces: &[[usize; 2]; 3], face: usize) -> Option<[usize; 2]> {
    let edges: Vec<usize> = (0..3)
        .filter(|edge| {
            edge_faces
                .get(*edge)
                .is_some_and(|pair| pair.contains(&face))
        })
        .collect();
    let [first, second] = edges[..] else {
        return None;
    };
    Some([first, second])
}

pub(super) fn find(
    solid: &Solid,
    topology: &Topology,
    bevelled: &Bevelled,
    vertex: VertexId,
) -> Option<ChamferCorner> {
    let [first, second, third] = topology.edges_at(vertex) else {
        return None;
    };
    let ids = [*first, *second, *third];

    let mut faces: Vec<FaceId> = Vec::with_capacity(3);
    let mut cuts = Vec::with_capacity(3);
    let mut pairs = Vec::with_capacity(3);
    let mut reach: f64 = 0.0;
    for edge in ids {
        let (geometry, blend, edge_reach) = bevelled.get(&edge)?;
        if !geometry.section.convex || !matches!(geometry.sweep, Sweep::Along { .. }) {
            return None;
        }
        cuts.push(chamfer_plane(geometry, blend)?);
        for face in geometry.faces {
            if !faces.contains(&face) {
                faces.push(face);
            }
        }
        pairs.push(geometry.faces);
        reach = reach.max(*edge_reach);
    }
    let [f0, f1, f2] = faces[..] else {
        return None;
    };
    let faces = [f0, f1, f2];
    let normals = [
        plane_normal(solid, f0)?,
        plane_normal(solid, f1)?,
        plane_normal(solid, f2)?,
    ];
    let apex = solid.vertex(vertex)?.point();
    let index = |face: FaceId| faces.iter().position(|candidate| *candidate == face);
    let mut edge_faces = [[0; 2]; 3];
    for (slot, [a, b]) in edge_faces.iter_mut().zip(&pairs) {
        *slot = [index(*a)?, index(*b)?];
    }
    let [c0, c1, c2] = cuts[..] else {
        return None;
    };

    let mut feet_meet = Vec::with_capacity(3);
    for (face, normal) in normals.iter().enumerate() {
        let [e1, e2] = edges_of(&edge_faces, face)?;
        feet_meet.push(meet([
            through(apex, *normal),
            *cuts.get(e1)?,
            *cuts.get(e2)?,
        ])?);
    }
    let [p0, p1, p2] = feet_meet[..] else {
        return None;
    };
    let normal = (p1 - p0).cross(p2 - p0).try_normalize()?;
    let toward_apex = if normal.dot(apex - p0) < 0.0 {
        -normal
    } else {
        normal
    };
    let apex_depth = toward_apex.dot(apex - p0);
    let tip = meet([c0, c1, c2])?;
    if apex_depth <= LINEAR_RESOLUTION || toward_apex.dot(tip - p0) <= LINEAR_RESOLUTION {
        return None;
    }

    for edge in ids {
        let along = -leaving(solid, edge, vertex)?;
        let approach = -toward_apex.dot(along);
        let length = solid.edge(edge)?.interval().length();
        if approach <= 0.0 || apex_depth / approach >= length {
            return None;
        }
    }

    Some(ChamferCorner {
        faces,
        normals,
        edge_faces,
        apex,
        cut_point: p0,
        toward_apex,
        clearance: CLEARANCE * reach,
    })
}

impl ChamferCorner {
    pub(super) fn tool(&self, solid: &Solid, feature: u64) -> Result<Solid, BlendError> {
        let failed = || BlendError::from(SweepError::Unassembled);
        let lifted = |face: usize| -> Result<Bound, BlendError> {
            let normal = *self.normals.get(face).ok_or_else(failed)?;
            Ok((normal, normal.dot(self.apex) + self.clearance))
        };
        let cut = through(self.cut_point, self.toward_apex);

        let mut builder = Builder::new();
        let top = builder
            .plan
            .vertex(meet([lifted(0)?, lifted(1)?, lifted(2)?]).ok_or_else(failed)?);
        let mut rim = Vec::with_capacity(3);
        for [a, b] in self.edge_faces {
            let point = meet([lifted(a)?, lifted(b)?, cut]).ok_or_else(failed)?;
            rim.push(builder.plan.vertex(point));
        }
        let [r0, r1, r2] = rim[..] else {
            return Err(failed());
        };
        for (from, to) in [(r0, r1), (r1, r2), (r2, r0)] {
            builder.line(from, to)?;
        }
        for corner in [r0, r1, r2] {
            builder.line(top, corner)?;
        }

        let names: Vec<(FaceName, Option<FaceOrigin>)> = self
            .faces
            .iter()
            .map(|face| {
                solid
                    .face(*face)
                    .map_or((FaceName::NONE, None), |face| (face.name(), face.origin()))
            })
            .collect();
        for (face, (normal, (name, origin))) in self.normals.iter().zip(&names).enumerate() {
            let [e1, e2] = edges_of(&self.edge_faces, face).ok_or_else(failed)?;
            let (Some(first), Some(second)) = (rim.get(e1), rim.get(e2)) else {
                return Err(failed());
            };
            let plane =
                Plane::new(self.apex + *normal * self.clearance, *normal).ok_or_else(failed)?;
            builder.face(
                &[top, *first, *second],
                *normal,
                PlaneSurface::new(plane).map_err(SweepError::from)?.into(),
                Sense::Same,
                *name,
                *origin,
            )?;
        }
        let outward = -self.toward_apex;
        let plane = Plane::new(self.cut_point, outward).ok_or_else(failed)?;
        builder.face(
            &[r0, r1, r2],
            outward,
            PlaneSurface::new(plane).map_err(SweepError::from)?.into(),
            Sense::Same,
            FaceName::corner(
                feature,
                VertexName::of_faces(names.iter().map(|(name, _)| *name)),
            ),
            Some(FaceOrigin::Chamfer { feature }),
        )?;

        let built = builder.plan.build().map_err(SweepError::from)?;
        Ok(built.renamed(|name, origin| (name, origin)))
    }
}
