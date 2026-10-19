use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};
use thiserror::Error;

use super::{Carried, Motion, MoveError, collapse::loop_edges, edge_faces, moving::move_each};
use crate::{
    interrupt::{self, Interrupted},
    surface::Surface,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION},
    topology::{EdgeId, FaceId, Solid},
};

pub const STEEPEST_REPLACING_TURN: f64 = 89.0 * std::f64::consts::PI / 180.0;

const REACH_SAMPLES: usize = 8;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ReplaceError {
    #[error("no face is to be replaced")]
    NothingToReplace,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("face {0:?} is not flat")]
    NotFlat(FaceId),
    #[error("the target plane is square to face {0:?}")]
    SquareToTarget(FaceId),
    #[error("the target plane lies beyond the far side of the body from face {0:?}")]
    BeyondBody(FaceId),
    #[error("faces {first:?} and {second:?} meet along an edge and would both lie on the target")]
    Merging { first: FaceId, second: FaceId },
    #[error("every face already lies on the target plane")]
    AlreadyOnTarget,
    #[error(transparent)]
    Move(#[from] MoveError),
}

impl From<Interrupted> for ReplaceError {
    fn from(interrupted: Interrupted) -> Self {
        Self::Move(MoveError::Cancelled(interrupted))
    }
}

struct Landing {
    face: FaceId,
    normal: Vector3,
    through: Point3,
    motion: RigidTransform,
}

impl Landing {
    fn shares_plane(&self, other: &Self) -> bool {
        self.normal.dot(other.normal) >= 1.0 - ANGULAR_RESOLUTION
            && (other.through - self.through).dot(self.normal).abs() <= LINEAR_RESOLUTION
    }
}

fn outward_target(normal: Vector3, target: &Plane, face: FaceId) -> Result<Vector3, ReplaceError> {
    let along = normal.dot(target.normal());
    if along.abs() < STEEPEST_REPLACING_TURN.cos() {
        return Err(ReplaceError::SquareToTarget(face));
    }
    Ok(target.normal() * along.signum())
}

fn turn_onto(
    normal: Vector3,
    through: Point3,
    outward: Vector3,
    target: &Plane,
) -> Option<RigidTransform> {
    let hinge = normal.cross(outward);
    if hinge.length() <= ANGULAR_RESOLUTION {
        return Some(RigidTransform::IDENTITY);
    }
    let hinge = hinge.normalize();
    let across = hinge.cross(normal);
    let toward_target = outward.dot(target.origin() - through) / outward.dot(across);
    let on_hinge = through + across * toward_target;
    let turn = hinge.dot(normal.cross(outward)).atan2(normal.dot(outward));
    RigidTransform::rotation_about(on_hinge, hinge, turn)
}

fn landing(solid: &Solid, face: FaceId, target: &Plane) -> Result<Option<Landing>, ReplaceError> {
    let definition = solid.face(face).ok_or(ReplaceError::MissingFace(face))?;
    let Surface::Plane(plane) = definition.surface() else {
        return Err(ReplaceError::NotFlat(face));
    };
    let normal = plane.frame().normal() * definition.sense().sign();
    let through = plane.frame().origin();
    let outward = outward_target(normal, target, face)?;

    let turned = turn_onto(normal, through, outward, target).ok_or(ReplaceError::NotFlat(face))?;
    let landed = turned.apply_point(through);
    let gap = outward.dot(target.origin() - landed);
    let settled =
        RigidTransform::translation(outward * gap).ok_or(ReplaceError::SquareToTarget(face))?;
    let motion = turned.then(&settled);

    let still = normal.dot(outward) >= 1.0 - ANGULAR_RESOLUTION
        && (target.origin() - through).dot(outward).abs() <= LINEAR_RESOLUTION;
    if still {
        return Ok(None);
    }
    if beyond_body(solid, face, target.origin(), outward) {
        return Err(ReplaceError::BeyondBody(face));
    }
    Ok(Some(Landing {
        face,
        normal,
        through,
        motion,
    }))
}

fn beyond_body(solid: &Solid, face: FaceId, on_target: Point3, outward: Vector3) -> bool {
    let own: BTreeSet<EdgeId> = loop_edges(solid, face).into_iter().collect();
    let mut heights = solid
        .edges()
        .filter(|(id, _)| !own.contains(id))
        .flat_map(|(_, edge)| {
            edge.interval()
                .split(REACH_SAMPLES)
                .map(|parameter| (edge.curve().point(parameter) - on_target).dot(outward))
                .collect::<Vec<_>>()
        })
        .peekable();
    heights.peek().is_some() && heights.all(|height| height >= -LINEAR_RESOLUTION)
}

struct Group {
    first: Landing,
    faces: Vec<FaceId>,
}

fn grouped(landings: Vec<Landing>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for landing in landings {
        match groups
            .iter_mut()
            .find(|group| group.first.shares_plane(&landing))
        {
            Some(group) => group.faces.push(landing.face),
            None => groups.push(Group {
                faces: vec![landing.face],
                first: landing,
            }),
        }
    }
    groups
}

fn on_target(solid: &Solid, face: FaceId, target: &Plane) -> bool {
    let Some(Surface::Plane(plane)) = solid.face(face).map(|definition| definition.surface())
    else {
        return false;
    };
    let frame = plane.frame();
    frame.normal().cross(target.normal()).length() <= ANGULAR_RESOLUTION
        && target.signed_distance(frame.origin()).abs() <= LINEAR_RESOLUTION
}

fn merging(solid: &Solid, groups: &[Group], target: &Plane) -> Option<(FaceId, FaceId)> {
    let group_of: BTreeMap<FaceId, usize> = groups
        .iter()
        .enumerate()
        .flat_map(|(index, group)| group.faces.iter().map(move |face| (*face, index)))
        .collect();
    group_of.iter().find_map(|(face, index)| {
        loop_edges(solid, *face)
            .into_iter()
            .flat_map(|edge| edge_faces(solid, edge))
            .filter(|beside| beside != face)
            .find(|beside| match group_of.get(beside) {
                Some(other) => other != index,
                None => on_target(solid, *beside, target),
            })
            .map(|beside| (beside, *face))
    })
}

pub fn replace_faces(
    solid: &Solid,
    faces: &[FaceId],
    target: &Plane,
) -> Result<Solid, ReplaceError> {
    if faces.is_empty() {
        return Err(ReplaceError::NothingToReplace);
    }
    let mut landings: Vec<Landing> = Vec::with_capacity(faces.len());
    for face in faces {
        if landings.iter().any(|landing| landing.face == *face) {
            continue;
        }
        if let Some(landing) = landing(solid, *face, target)? {
            landings.push(landing);
        }
    }
    if landings.is_empty() {
        return Err(ReplaceError::AlreadyOnTarget);
    }
    interrupt::check()?;

    let groups = grouped(landings);
    if let Some((first, second)) = merging(solid, &groups, target) {
        return Err(ReplaceError::Merging { first, second });
    }
    let carried: Vec<Carried<'_>> = groups
        .iter()
        .map(|group| Carried {
            faces: &group.faces,
            motion: Motion::rigid(group.first.motion),
        })
        .collect();
    Ok(move_each(solid, carried)?)
}
