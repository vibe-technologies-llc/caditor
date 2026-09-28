use std::collections::BTreeMap;

use caditor_geometry::{Plane, Point3, Vector3};

use super::{BlendError, CLEARANCE, Topology, leaving};
use crate::{
    build::{
        SweepError,
        plan::{Plan, PlanCoedge, PlanFace},
    },
    curve::{Circle, Curve},
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin, VertexName},
    sense::Sense,
    surface::{PlaneSurface, Sphere, Surface},
    topology::{EdgeId, FaceId, Solid, VertexId},
};

const SINGULAR: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Corner {
    pub vertex: VertexId,
    pub setbacks: BTreeMap<EdgeId, f64>,
    faces: [FaceId; 3],
    normals: [Vector3; 3],
    edges: [(EdgeId, [usize; 2], Vector3); 3],
    apex: Point3,
    center: Point3,
    radius: f64,
}

fn meet(planes: [(Vector3, f64); 3]) -> Option<Point3> {
    let [(a, da), (b, db), (c, dc)] = planes;
    let determinant = a.dot(b.cross(c));
    if determinant.abs() <= SINGULAR {
        return None;
    }
    Some((b.cross(c) * da + c.cross(a) * db + a.cross(b) * dc) / determinant)
}

fn plane_normal(solid: &Solid, face: FaceId) -> Option<Vector3> {
    let definition = solid.face(face)?;
    let Surface::Plane(plane) = definition.surface() else {
        return None;
    };
    Some(plane.frame().normal() * definition.sense().sign())
}

pub(super) fn find(
    solid: &Solid,
    topology: &Topology,
    convex: &BTreeMap<EdgeId, bool>,
    radius: f64,
    vertex: VertexId,
) -> Option<Corner> {
    let around = topology.edges_at(vertex);
    let [first, second, third] = around else {
        return None;
    };
    let ids = [*first, *second, *third];
    let usable = ids.iter().all(|edge| {
        convex.get(edge) == Some(&true)
            && solid
                .edge(*edge)
                .is_some_and(|definition| matches!(definition.curve(), Curve::Line(_)))
    });
    if !usable {
        return None;
    }
    let mut faces: Vec<FaceId> = Vec::with_capacity(3);
    let mut edge_faces: Vec<[FaceId; 2]> = Vec::with_capacity(3);
    for edge in ids {
        let pair = super::face_set(solid, edge);
        let [a, b] = pair.iter().copied().collect::<Vec<_>>()[..] else {
            return None;
        };
        for face in [a, b] {
            if !faces.contains(&face) {
                faces.push(face);
            }
        }
        edge_faces.push([a, b]);
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
    let offset = meet([
        (normals[0], -radius),
        (normals[1], -radius),
        (normals[2], -radius),
    ])?;
    let center = apex + offset;
    let mut edges = Vec::with_capacity(3);
    let mut setbacks = BTreeMap::new();
    for (edge, pair) in ids.iter().zip(edge_faces) {
        let direction = -leaving(solid, *edge, vertex)?;
        let index = |face: FaceId| faces.iter().position(|candidate| *candidate == face);
        let pair = [index(pair[0])?, index(pair[1])?];
        let setback = offset.dot(direction);
        if setback <= 0.0 {
            return None;
        }
        setbacks.insert(*edge, setback);
        edges.push((*edge, pair, direction));
    }
    let [e0, e1, e2] = edges[..] else {
        return None;
    };
    Some(Corner {
        vertex,
        setbacks,
        faces,
        normals,
        edges: [e0, e1, e2],
        apex,
        center,
        radius,
    })
}

fn oriented(points: &[Point3], outward: Vector3) -> bool {
    let count = points.len();
    let mut normal = Vector3::ZERO;
    for index in 0..count {
        if let (Some(a), Some(b)) = (points.get(index), points.get((index + 1) % count)) {
            normal += a.cross(*b);
        }
    }
    normal.dot(outward) >= 0.0
}

struct Builder {
    plan: Plan,
    edges: BTreeMap<(usize, usize), (usize, bool)>,
}

impl Builder {
    fn line(&mut self, from: usize, to: usize) -> Result<(), BlendError> {
        let edge = self.plan.line(from, to, EdgeName::NONE)?;
        self.edges.insert((from, to), (edge, true));
        self.edges.insert((to, from), (edge, false));
        Ok(())
    }

    fn arc(
        &mut self,
        from: usize,
        to: usize,
        center: Point3,
        radius: f64,
    ) -> Result<(), BlendError> {
        let (Some(start), Some(end)) = (self.plan.point(from), self.plan.point(to)) else {
            return Err(SweepError::Unassembled.into());
        };
        let x_axis = (start - center).normalize_or_zero();
        let toward = (end - center).normalize_or_zero();
        let normal = x_axis.cross(toward).normalize_or_zero();
        let frame = Plane::from_frame(center, normal, x_axis).ok_or(SweepError::Unassembled)?;
        let angle = x_axis.dot(toward).clamp(-1.0, 1.0).acos();
        let interval = Interval::new(0.0, angle).ok_or(SweepError::Unassembled)?;
        let circle = Circle::new(frame, radius).map_err(SweepError::from)?;
        let edge = self
            .plan
            .edge(circle.into(), interval, (from, to), EdgeName::NONE);
        self.edges.insert((from, to), (edge, true));
        self.edges.insert((to, from), (edge, false));
        Ok(())
    }

    fn face(
        &mut self,
        cycle: &[usize],
        outward: Vector3,
        surface: Surface,
        sense: Sense,
        name: FaceName,
        origin: Option<FaceOrigin>,
    ) -> Result<(), BlendError> {
        let points: Vec<Point3> = cycle
            .iter()
            .filter_map(|vertex| self.plan.point(*vertex))
            .collect();
        let mut cycle = cycle.to_vec();
        if !oriented(&points, outward) {
            cycle.reverse();
        }
        let count = cycle.len();
        let mut coedges = Vec::with_capacity(count);
        for index in 0..count {
            let (Some(from), Some(to)) = (cycle.get(index), cycle.get((index + 1) % count)) else {
                return Err(SweepError::Unassembled.into());
            };
            let (edge, forward) = self
                .edges
                .get(&(*from, *to))
                .copied()
                .ok_or(SweepError::Unassembled)?;
            let sense = if forward {
                Sense::Same
            } else {
                Sense::Reversed
            };
            coedges.push(PlanCoedge::new(edge, sense));
        }
        self.plan.face(PlanFace {
            surface,
            sense,
            name,
            origin,
            loops: vec![coedges],
        });
        Ok(())
    }
}

impl Corner {
    pub(super) fn tool(
        &self,
        solid: &Solid,
        feature: u64,
        blend_names: &BTreeMap<EdgeId, FaceName>,
    ) -> Result<Solid, BlendError> {
        let failed = || BlendError::from(SweepError::Unassembled);
        let clearance = CLEARANCE * self.radius;
        let lifted = |face: usize| -> Option<(Vector3, f64)> {
            let normal = *self.normals.get(face)?;
            Some((normal, normal.dot(self.apex) + clearance))
        };
        let setback = |edge: usize| -> Option<(Vector3, f64)> {
            let (_, _, direction) = self.edges.get(edge)?;
            Some((*direction, direction.dot(self.center)))
        };
        let edges_of = |face: usize| -> Vec<usize> {
            (0..3)
                .filter(|edge| {
                    self.edges
                        .get(*edge)
                        .is_some_and(|(_, pair, _)| pair.contains(&face))
                })
                .collect()
        };
        let mut builder = Builder {
            plan: Plan::default(),
            edges: BTreeMap::new(),
        };
        let top = builder.plan.vertex(
            meet([
                lifted(0).ok_or_else(failed)?,
                lifted(1).ok_or_else(failed)?,
                lifted(2).ok_or_else(failed)?,
            ])
            .ok_or_else(failed)?,
        );
        let at = |list: &[usize], index: usize| list.get(index).copied().ok_or_else(failed);
        let mut along = Vec::with_capacity(3);
        for (edge, (_, [a, b], _)) in self.edges.iter().enumerate() {
            let point = meet([
                lifted(*a).ok_or_else(failed)?,
                lifted(*b).ok_or_else(failed)?,
                setback(edge).ok_or_else(failed)?,
            ])
            .ok_or_else(failed)?;
            along.push(builder.plan.vertex(point));
        }
        let mut outer = Vec::with_capacity(3);
        let mut touch = Vec::with_capacity(3);
        for (face, normal) in self.normals.iter().enumerate() {
            let [e1, e2] = edges_of(face)[..] else {
                return Err(failed());
            };
            let point = meet([
                lifted(face).ok_or_else(failed)?,
                setback(e1).ok_or_else(failed)?,
                setback(e2).ok_or_else(failed)?,
            ])
            .ok_or_else(failed)?;
            outer.push(builder.plan.vertex(point));
            touch.push(builder.plan.vertex(self.center + *normal * self.radius));
        }
        for (edge, (_, [a, b], _)) in self.edges.iter().enumerate() {
            let corner = at(&along, edge)?;
            builder.line(top, corner)?;
            builder.line(corner, at(&outer, *a)?)?;
            builder.line(corner, at(&outer, *b)?)?;
            builder.arc(at(&touch, *a)?, at(&touch, *b)?, self.center, self.radius)?;
        }
        for (outside, contact) in outer.iter().zip(&touch) {
            builder.line(*outside, *contact)?;
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
            let [e1, e2] = edges_of(face)[..] else {
                return Err(failed());
            };
            let plane = Plane::new(self.apex + *normal * clearance, *normal).ok_or_else(failed)?;
            builder.face(
                &[top, at(&along, e1)?, at(&outer, face)?, at(&along, e2)?],
                *normal,
                PlaneSurface::new(plane).map_err(SweepError::from)?.into(),
                Sense::Same,
                *name,
                *origin,
            )?;
        }
        for (edge, (id, [a, b], direction)) in self.edges.iter().enumerate() {
            let plane = Plane::new(self.center, *direction).ok_or_else(failed)?;
            let name = blend_names.get(id).copied().unwrap_or_default();
            builder.face(
                &[
                    at(&along, edge)?,
                    at(&outer, *a)?,
                    at(&touch, *a)?,
                    at(&touch, *b)?,
                    at(&outer, *b)?,
                ],
                *direction,
                PlaneSurface::new(plane).map_err(SweepError::from)?.into(),
                Sense::Same,
                name,
                Some(FaceOrigin::Fillet { feature }),
            )?;
        }
        let frame = Plane::new(self.center, Vector3::Z).ok_or_else(failed)?;
        let face_names = names.iter().map(|(name, _)| *name);
        let points: Vec<Point3> = touch
            .iter()
            .filter_map(|vertex| builder.plan.point(*vertex))
            .collect();
        let middle = points.iter().fold(Vector3::ZERO, |sum, point| sum + *point) / 3.0;
        builder.face(
            &touch,
            self.center - middle,
            Sphere::new(frame, self.radius)
                .map_err(SweepError::from)?
                .into(),
            Sense::Reversed,
            FaceName::corner(feature, VertexName::of_faces(face_names)),
            Some(FaceOrigin::Fillet { feature }),
        )?;
        let built = builder.plan.build().map_err(SweepError::from)?;
        Ok(built.renamed(|name, origin| (name, origin)))
    }
}
