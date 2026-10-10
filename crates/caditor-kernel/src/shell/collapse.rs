use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point3, Vector3};

use super::{OFFSET_TOLERANCE, Offsets, ShellError};
use crate::{
    curve::Curve,
    surface::Surface,
    topology::{EdgeId, FaceId, Solid},
};

const PARALLEL: f64 = 1e-6;

fn parallel(a: Vector3, b: Vector3) -> bool {
    a.cross(b).length() <= PARALLEL
}

#[derive(Debug)]
pub(super) enum Collapse {
    Axis(Vector3),
    Centre,
    Spine(Vector3),
    Apex(FaceId),
    Shrinks(BTreeSet<EdgeId>),
}

impl Collapse {
    fn vanishes(&self, edge: EdgeId, curve: &Curve) -> bool {
        match self {
            Self::Centre | Self::Apex(_) => true,
            Self::Axis(axis) => {
                !matches!(curve, Curve::Line(line) if parallel(line.direction(), *axis))
            }
            Self::Spine(axis) => {
                !matches!(curve, Curve::Circle(circle) if parallel(circle.frame().normal(), *axis))
            }
            Self::Shrinks(across) => across.contains(&edge),
        }
    }

    fn shrinks_away(&self) -> bool {
        matches!(self, Self::Apex(_) | Self::Shrinks(_))
    }
}

pub(super) fn coedge_faces(solid: &Solid, edge: EdgeId) -> Vec<FaceId> {
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter_map(|coedge| solid.coedge_face(*coedge))
        .collect()
}

pub(super) fn loop_edges(solid: &Solid, face: FaceId) -> Vec<EdgeId> {
    solid
        .face(face)
        .into_iter()
        .flat_map(|face| face.loops())
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges())
        .filter_map(|coedge| Some(solid.coedge(*coedge)?.edge()))
        .collect()
}

fn too_curved(offsets: &Offsets<'_>, face: FaceId) -> Option<Collapse> {
    let definition = offsets.solid.face(face)?;
    let along = -offsets.distance(face) * definition.sense().sign();
    let gone = |radius: f64| radius + along <= OFFSET_TOLERANCE;
    match definition.surface() {
        Surface::Cylinder(cylinder) if gone(cylinder.radius()) => {
            Some(Collapse::Axis(cylinder.frame().normal()))
        }
        Surface::Sphere(sphere) if gone(sphere.radius()) => Some(Collapse::Centre),
        Surface::Torus(torus) if gone(torus.minor_radius()) => {
            Some(Collapse::Spine(torus.frame().normal()))
        }
        _ => None,
    }
}

fn past_apex(offsets: &Offsets<'_>, face: FaceId) -> Option<Collapse> {
    let solid = offsets.solid;
    let Surface::Plane(plane) = solid.face(face)?.surface() else {
        return None;
    };
    let mut cone = None;
    for edge_id in loop_edges(solid, face) {
        let Curve::Circle(circle) = solid.edge(edge_id)?.curve() else {
            return None;
        };
        let other = coedge_faces(solid, edge_id)
            .into_iter()
            .find(|other| *other != face)?;
        if cone.is_some_and(|known| known != other) {
            return None;
        }
        let Surface::Cone(surface) = solid.face(other)?.surface() else {
            return None;
        };
        let axis = surface.frame().normal();
        let off_axis = (circle.center() - surface.frame().origin()).reject_from(axis);
        let coaxial = parallel(circle.frame().normal(), axis)
            && parallel(plane.frame().normal(), axis)
            && off_axis.length() <= OFFSET_TOLERANCE;
        if !coaxial {
            return None;
        }
        cone = Some(other);
    }
    let cone = cone?;
    let (Surface::Cone(original), Ok(Surface::Cone(offset))) =
        (solid.face(cone)?.surface(), offsets.surface(cone))
    else {
        return None;
    };
    let (beyond, _) = offsets.residual(face, original.apex(), None)?;
    let (passed, _) = offsets.residual(face, offset.apex(), None)?;
    let narrows_to_face = beyond - offsets.distance(face) > OFFSET_TOLERANCE;
    (narrows_to_face && passed < -OFFSET_TOLERANCE).then_some(Collapse::Apex(cone))
}

#[derive(Debug)]
pub(super) struct Collapses(BTreeMap<FaceId, Collapse>);

impl Collapses {
    pub(super) fn of(offsets: &Offsets<'_>) -> Self {
        let solid = offsets.solid;
        let mut collapses: BTreeMap<FaceId, Collapse> = solid
            .faces()
            .filter_map(|(id, _)| Some((id, too_curved(offsets, id)?)))
            .collect();
        let apexes: Vec<(FaceId, Collapse)> = solid
            .faces()
            .filter(|(id, _)| !collapses.contains_key(id))
            .filter_map(|(id, _)| Some((id, past_apex(offsets, id)?)))
            .collect();
        collapses.extend(apexes);
        Self(collapses)
    }

    pub(super) fn contains(&self, face: FaceId) -> bool {
        self.0.contains_key(&face)
    }

    pub(super) fn apex_of(&self, face: FaceId) -> Option<FaceId> {
        match self.0.get(&face)? {
            Collapse::Apex(cone) => Some(*cone),
            _ => None,
        }
    }

    pub(super) fn add_shrinking(&mut self, shrinking: BTreeMap<FaceId, BTreeSet<EdgeId>>) {
        self.0.extend(
            shrinking
                .into_iter()
                .map(|(face, across)| (face, Collapse::Shrinks(across))),
        );
    }

    pub(super) fn shrinking(&self) -> impl Iterator<Item = (FaceId, &BTreeSet<EdgeId>)> {
        self.0.iter().filter_map(|(face, collapse)| match collapse {
            Collapse::Shrinks(across) => Some((*face, across)),
            _ => None,
        })
    }

    pub(super) fn faces(&self) -> BTreeSet<FaceId> {
        self.0.keys().copied().collect()
    }

    pub(super) fn vanishing_edges(&self, solid: &Solid) -> BTreeSet<EdgeId> {
        solid
            .edges()
            .filter(|(id, edge)| {
                coedge_faces(solid, *id).iter().any(|face| {
                    self.0
                        .get(face)
                        .is_some_and(|collapse| collapse.vanishes(*id, edge.curve()))
                })
            })
            .map(|(id, _)| id)
            .collect()
    }

    pub(super) fn refusal(&self, solid: &Solid, face: FaceId) -> Option<ShellError> {
        let collapse = self.0.get(&face)?;
        if !collapse.shrinks_away() {
            return Some(ShellError::TooCurved(face));
        }
        let vanishing = loop_edges(solid, face)
            .into_iter()
            .filter(|edge| {
                solid
                    .edge(*edge)
                    .is_some_and(|definition| collapse.vanishes(*edge, definition.curve()))
            })
            .min()?;
        Some(ShellError::EdgeCollapses(vanishing))
    }

    pub(super) fn closes_over(
        &self,
        offsets: &Offsets<'_>,
        faces: &BTreeSet<FaceId>,
        point: Point3,
    ) -> Result<(), ShellError> {
        for face in faces {
            let Some(collapse) = self.0.get(face) else {
                continue;
            };
            let covered = !collapse.shrinks_away()
                || offsets
                    .residual(*face, point, None)
                    .is_some_and(|(residual, _)| {
                        let inside = residual < -OFFSET_TOLERANCE;
                        let through_opening = residual - offsets.distance(*face) > OFFSET_TOLERANCE;
                        inside && (through_opening || !offsets.is_outward(*face))
                    });
            if !covered {
                return Err(self
                    .refusal(offsets.solid, *face)
                    .unwrap_or_else(|| ShellError::walls_at([*face])));
            }
        }
        Ok(())
    }
}
