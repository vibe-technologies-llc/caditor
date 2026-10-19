use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};
use thiserror::Error;

use super::{
    Carried, Motion, MoveError,
    collapse::loop_edges,
    moving::move_each,
    taper::{Slopes, Taper},
};
use crate::{
    interrupt::{self, Interrupted},
    surface::Surface,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION},
    topology::{Face, FaceId, Solid},
};

pub const STEEPEST_DRAFT: f64 = 89.0 * std::f64::consts::PI / 180.0;

const HEIGHT_SAMPLES: usize = 8;

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
    #[error("face {0:?} is curved in a way that cannot be drafted")]
    NotDraftable(FaceId),
    #[error("face {0:?} is round about an axis that does not run along the pull direction")]
    OffPull(FaceId),
    #[error("face {0:?} would narrow to a point")]
    NarrowsToPoint(FaceId),
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

enum Drafting {
    Tilted(Tilt),
    Tapered(Taper),
}

struct Turn {
    face: FaceId,
    drafting: Drafting,
}

impl Turn {
    fn shares_motion(&self, other: &Self) -> bool {
        match (&self.drafting, &other.drafting) {
            (Drafting::Tilted(tilt), Drafting::Tilted(other)) => tilt.shares_motion(other),
            (Drafting::Tapered(taper), Drafting::Tapered(other)) => taper.same_as(other),
            _ => false,
        }
    }

    fn motion(&self) -> Motion {
        match &self.drafting {
            Drafting::Tilted(tilt) => Motion::rigid(tilt.motion),
            Drafting::Tapered(taper) => Motion::Tapered(taper.clone()),
        }
    }
}

pub fn check_draftable(solid: &Solid, face: FaceId, neutral: &Plane) -> Result<(), DraftError> {
    let definition = solid.face(face).ok_or(DraftError::MissingFace(face))?;
    let axis = match definition.surface() {
        Surface::Plane(_) => return Ok(()),
        Surface::Cylinder(cylinder) => cylinder.frame().normal(),
        Surface::Cone(cone) => cone.frame().normal(),
        Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Extrusion(_)
        | Surface::Revolution(_)
        | Surface::BSpline(_) => return Err(DraftError::NotDraftable(face)),
    };
    if axis.dot(neutral.normal()).abs() < 1.0 - ANGULAR_RESOLUTION {
        return Err(DraftError::OffPull(face));
    }
    Ok(())
}

fn turn(solid: &Solid, drafted: DraftedFace, neutral: &Plane) -> Result<Option<Turn>, DraftError> {
    let face = drafted.face;
    let definition = solid.face(face).ok_or(DraftError::MissingFace(face))?;
    if !drafted.angle.is_finite() || drafted.angle.abs() > STEEPEST_DRAFT {
        return Err(DraftError::TooSteep(face));
    }
    check_draftable(solid, face, neutral)?;
    let drafting = match definition.surface() {
        Surface::Plane(_) => tilt(definition, drafted, neutral)?.map(Drafting::Tilted),
        _ => taper(solid, definition, drafted, neutral)?.map(Drafting::Tapered),
    };
    Ok(drafting.map(|drafting| Turn { face, drafting }))
}

fn tilt(
    definition: &Face,
    drafted: DraftedFace,
    neutral: &Plane,
) -> Result<Option<Tilt>, DraftError> {
    let face = drafted.face;
    let Surface::Plane(plane) = definition.surface() else {
        return Err(DraftError::NotDraftable(face));
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
        normal,
        through,
        turn,
        motion,
    }))
}

fn heights(solid: &Solid, face: FaceId, axis: &Plane) -> Option<(f64, f64)> {
    loop_edges(solid, face)
        .into_iter()
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            edge.interval()
                .split(HEIGHT_SAMPLES)
                .map(|parameter| axis.signed_distance(edge.curve().point(parameter)))
                .collect::<Vec<_>>()
        })
        .fold(None, |range, height| match range {
            None => Some((height, height)),
            Some((low, high)) => Some((f64::min(low, height), f64::max(high, height))),
        })
}

fn taper(
    solid: &Solid,
    definition: &Face,
    drafted: DraftedFace,
    neutral: &Plane,
) -> Result<Option<Taper>, DraftError> {
    let face = drafted.face;
    let (frame, radius, half_angle) = match definition.surface() {
        Surface::Cylinder(cylinder) => (*cylinder.frame(), cylinder.radius(), 0.0),
        Surface::Cone(cone) => (*cone.frame(), cone.radius(), cone.half_angle()),
        _ => return Err(DraftError::NotDraftable(face)),
    };
    let pull = neutral.normal();
    let along = frame.normal().dot(pull);
    let leaning = -definition.sense().sign() * along.signum();
    let current = leaning * half_angle;
    if (drafted.angle - current).abs() <= ANGULAR_RESOLUTION {
        return Ok(None);
    }
    let slopes = Slopes {
        from: half_angle.tan(),
        to: (leaning * drafted.angle).tan(),
    };
    let to_neutral = (neutral.origin() - frame.origin()).dot(pull) / along;
    let axis = Plane::from_frame(
        frame.origin() + frame.normal() * to_neutral,
        frame.normal(),
        frame.x_axis(),
    )
    .ok_or(DraftError::OffPull(face))?;
    let hinge = radius + slopes.from * to_neutral;
    let (low, high) = heights(solid, face, &axis).ok_or(DraftError::MissingFace(face))?;
    let narrowest = f64::min(hinge + low * slopes.to, hinge + high * slopes.to);
    if narrowest <= LINEAR_RESOLUTION {
        return Err(DraftError::NarrowsToPoint(face));
    }
    Taper::new(axis, hinge, slopes, 0.5 * (low + high))
        .map(Some)
        .ok_or(DraftError::NarrowsToPoint(face))
}

fn grouped(turns: Vec<Turn>) -> Vec<(Turn, Vec<FaceId>)> {
    let mut groups: Vec<(Turn, Vec<FaceId>)> = Vec::new();
    for turn in turns {
        let face = turn.face;
        match groups
            .iter_mut()
            .find(|(first, _)| first.shares_motion(&turn))
        {
            Some((_, faces)) => faces.push(face),
            None => groups.push((turn, vec![face])),
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
    let mut turns: Vec<Turn> = Vec::with_capacity(faces.len());
    for drafted in faces {
        if turns.iter().any(|turn| turn.face == drafted.face) {
            continue;
        }
        if let Some(turn) = turn(solid, *drafted, neutral)? {
            turns.push(turn);
        }
    }
    if turns.is_empty() {
        return Err(DraftError::AlreadyDrafted);
    }
    interrupt::check()?;
    let groups = grouped(turns);
    let carried: Vec<Carried<'_>> = groups
        .iter()
        .map(|(first, faces)| Carried {
            faces,
            motion: first.motion(),
        })
        .collect();
    Ok(move_each(solid, carried)?)
}
