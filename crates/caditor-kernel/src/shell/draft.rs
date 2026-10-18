use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};
use thiserror::Error;

use super::{Carried, MoveError, moving::move_each};
use crate::{
    interrupt::{self, Interrupted},
    surface::Surface,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION},
    topology::{FaceId, Solid},
};

pub const STEEPEST_DRAFT: f64 = 89.0 * std::f64::consts::PI / 180.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DraftedFace {
    pub face: FaceId,
    pub angle: f64,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DraftError {
    #[error("no face is to be drafted")]
    NothingToDraft,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("the draft angle of face {0:?} is not a number between -89 and 89 degrees")]
    TooSteep(FaceId),
    #[error("face {0:?} is not flat")]
    NotFlat(FaceId),
    #[error("face {0:?} is parallel to the neutral plane")]
    AlongNeutral(FaceId),
    #[error("every face already makes its draft angle with the pull direction")]
    AlreadyDrafted,
    #[error(transparent)]
    Move(#[from] MoveError),
}

impl From<Interrupted> for DraftError {
    fn from(interrupted: Interrupted) -> Self {
        Self::Move(MoveError::Cancelled(interrupted))
    }
}

struct Tilt {
    face: FaceId,
    normal: Vector3,
    through: Point3,
    turn: f64,
    motion: RigidTransform,
}

impl Tilt {
    fn shares_motion(&self, other: &Self) -> bool {
        self.normal.dot(other.normal) >= 1.0 - ANGULAR_RESOLUTION
            && (other.through - self.through).dot(self.normal).abs() <= LINEAR_RESOLUTION
            && (self.turn - other.turn).abs() <= ANGULAR_RESOLUTION
    }
}

fn tilt(solid: &Solid, drafted: DraftedFace, neutral: &Plane) -> Result<Option<Tilt>, DraftError> {
    let face = drafted.face;
    let definition = solid.face(face).ok_or(DraftError::MissingFace(face))?;
    if !drafted.angle.is_finite() || drafted.angle.abs() > STEEPEST_DRAFT {
        return Err(DraftError::TooSteep(face));
    }
    let Surface::Plane(plane) = definition.surface() else {
        return Err(DraftError::NotFlat(face));
    };
    let pull = neutral.normal();
    let normal = plane.frame().normal() * definition.sense().sign();
    let hinge = normal.cross(pull);
    if hinge.length() <= ANGULAR_RESOLUTION {
        return Err(DraftError::AlongNeutral(face));
    }
    let hinge = hinge.normalize();
    let current = normal.dot(pull).clamp(-1.0, 1.0).asin();
    let turn = drafted.angle - current;
    if turn.abs() <= ANGULAR_RESOLUTION {
        return Ok(None);
    }
    let through = plane.frame().origin();
    let across = hinge.cross(normal);
    let toward_neutral = pull.dot(neutral.origin() - through) / pull.dot(across);
    let on_hinge = through + across * toward_neutral;
    let motion = RigidTransform::rotation_about(on_hinge, hinge, turn)
        .ok_or(DraftError::AlongNeutral(face))?;
    Ok(Some(Tilt {
        face,
        normal,
        through,
        turn,
        motion,
    }))
}

struct Group {
    first: Tilt,
    faces: Vec<FaceId>,
}

fn grouped(tilts: Vec<Tilt>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for tilt in tilts {
        match groups
            .iter_mut()
            .find(|group| group.first.shares_motion(&tilt))
        {
            Some(group) => group.faces.push(tilt.face),
            None => groups.push(Group {
                faces: vec![tilt.face],
                first: tilt,
            }),
        }
    }
    groups
}

pub fn draft_faces(
    solid: &Solid,
    faces: &[DraftedFace],
    neutral: &Plane,
) -> Result<Solid, DraftError> {
    if faces.is_empty() {
        return Err(DraftError::NothingToDraft);
    }
    let mut tilts = Vec::with_capacity(faces.len());
    for drafted in faces {
        if tilts.iter().any(|tilt: &Tilt| tilt.face == drafted.face) {
            continue;
        }
        if let Some(tilt) = tilt(solid, *drafted, neutral)? {
            tilts.push(tilt);
        }
    }
    if tilts.is_empty() {
        return Err(DraftError::AlreadyDrafted);
    }
    interrupt::check()?;
    let groups = grouped(tilts);
    let carried: Vec<Carried<'_>> = groups
        .iter()
        .map(|group| Carried {
            faces: &group.faces,
            motion: group.first.motion,
        })
        .collect();
    Ok(move_each(solid, &carried)?)
}
