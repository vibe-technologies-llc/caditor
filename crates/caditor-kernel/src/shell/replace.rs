use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Maths, Plane, Point3, RigidTransform, Vector3};
use thiserror::Error;

use super::{
    Carried, Motion, MoveError, Offsets,
    collapse::loop_edges,
    edge_faces,
    inner::{Naming, inner_solid},
    merge::{MergeFailure, MergedFaces, merged_faces},
    moving::{carrying, uncrossed},
};
use crate::{
    interrupt::{self, Interrupted},
    naming::FaceName,
    surface::Surface,
    tolerance::{ANGULAR_RESOLUTION, LINEAR_RESOLUTION},
    topology::{EdgeId, FaceId, Solid},
};

pub const STEEPEST_REPLACING_TURN: f64 = 89.0 * std::f64::consts::PI / 180.0;

const REACH_SAMPLES: usize = 8;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum ReplaceError {
    #[error("no face is to be replaced")]
    NothingToReplace,
    #[error("face {0:?} is not part of the solid")]
    MissingFace(FaceId),
    #[error("face {0:?} is not flat")]
    NotFlat(FaceId),
    #[error("face {0:?} is flat, so it cannot lie on a round target")]
    FlatOntoRound(FaceId),
    #[error(
        "face {0:?} cannot reach the round target: it needs the target's axis, and a cylinder away \
         from it the target's radius and direction"
    )]
    OffAxis(FaceId),
    #[error("the target plane is square to face {0:?}")]
    SquareToTarget(FaceId),
    #[error("the target plane lies beyond the far side of the body from face {0:?}")]
    BeyondBody(FaceId),
    #[error("faces {first:?} and {second:?} would lie on the target but cannot become one face")]
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

    if along.abs() < STEEPEST_REPLACING_TURN.cos_libm() {
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
    let turn = hinge
        .dot(normal.cross(outward))
        .atan2_libm(normal.dot(outward));
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

fn outward_along(solid: &Solid, face: FaceId, target: &Plane) -> Option<f64> {
    let definition = solid.face(face)?;
    let Surface::Plane(plane) = definition.surface() else {
        return None;
    };
    Some(plane.frame().normal().dot(target.normal()) * definition.sense().sign())
}

fn squeezed_flat(
    solid: &Solid,
    dropped: FaceId,
    landing: &BTreeSet<FaceId>,
    target: &Plane,
) -> bool {
    let beside: BTreeSet<FaceId> = loop_edges(solid, dropped)
        .into_iter()
        .flat_map(|edge| edge_faces(solid, edge))
        .filter(|face| *face != dropped)
        .filter(|face| landing.contains(face) || on_target(solid, *face, target))
        .collect();
    !landing.contains(&dropped) && beside.len() >= 2
}

struct LandedFace {
    name: FaceName,
    surface: Surface,
    group: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MergedName {
    OfReplaced,
    OfStill,
}

fn joined_on_target(
    result: &Solid,
    landed: &[LandedFace],
    target: &Plane,
    naming: MergedName,
) -> Vec<MergedFaces> {
    let group_of = |face: FaceId| {
        let definition = result.face(face)?;
        landed
            .iter()
            .find(|landed| {
                landed.name == definition.name() && landed.surface == *definition.surface()
            })
            .map(|landed| landed.group)
    };
    let mut roots: BTreeMap<FaceId, FaceId> = BTreeMap::new();
    let root = |roots: &BTreeMap<FaceId, FaceId>, mut face: FaceId| {
        while let Some(parent) = roots.get(&face).filter(|parent| **parent != face) {
            face = *parent;
        }
        face
    };

    for (edge, _) in result.edges() {
        let [first, second] = edge_faces(result, edge)[..] else {
            continue;
        };
        let lying = |face: FaceId| on_target(result, face, target);

        let same_side = outward_along(result, first, target)
            .zip(outward_along(result, second, target))
            .is_some_and(|(a, b)| a * b > 0.0);

        if first == second
            || group_of(first) == group_of(second)
            || !lying(first)
            || !lying(second)
            || !same_side
        {
            continue;
        }

        let (a, b) = (root(&roots, first), root(&roots, second));

        roots.insert(a.max(b), a.min(b));
    }

    let mut merged: BTreeMap<FaceId, BTreeSet<FaceId>> = BTreeMap::new();

    for face in roots.keys().copied().collect::<Vec<_>>() {
        merged.entry(root(&roots, face)).or_default().insert(face);
    }
    merged
        .into_iter()
        .filter_map(|(lowest, mut faces)| {
            faces.insert(lowest);

            let name = |face: &FaceId| result.face(*face).map(|definition| definition.name());
            let replaced = |face: &&FaceId| group_of(**face).is_some();
            let still = |face: &&FaceId| group_of(**face).is_none();

            let keeper = match naming {
                MergedName::OfReplaced => {
                    faces.iter().filter(replaced).min_by_key(|face| name(face))
                }
                MergedName::OfStill => faces
                    .iter()
                    .filter(still)
                    .min_by_key(|face| name(face))
                    .or_else(|| faces.iter().filter(replaced).min_by_key(|face| name(face))),
            }
            .copied()?;
            Some(MergedFaces { keeper, faces })
        })
        .collect()
}

fn landed(
    solid: &Solid,
    groups: &[Group],
    target: &Plane,
    naming: MergedName,
) -> Result<Solid, ReplaceError> {
    let carried: Vec<Carried<'_>> = groups
        .iter()
        .map(|group| Carried {
            faces: &group.faces,
            motion: Motion::rigid(group.first.motion),
        })
        .collect();
    let offsets = carrying(solid, carried)?.squeezing();
    let inner = inner_solid(&offsets, Naming::Kept).map_err(MoveError::from)?;
    let landing: BTreeSet<FaceId> = groups
        .iter()
        .flat_map(|group| group.faces.iter().copied())
        .collect();

    if let Some(face) = inner
        .dropped
        .iter()
        .find(|face| !squeezed_flat(solid, **face, &landing, target))
    {
        return Err(MoveError::Vanishes(*face).into());
    }

    let landed_faces: Vec<LandedFace> = groups
        .iter()
        .enumerate()
        .flat_map(|(index, group)| group.faces.iter().map(move |face| (*face, index)))
        .map(|(face, group)| {
            Ok(LandedFace {
                name: solid.face(face).map_or(FaceName::NONE, |face| face.name()),
                surface: offsets.surface(face).map_err(MoveError::from)?,
                group,
            })
        })
        .collect::<Result<_, MoveError>>()?;
    let joined = joined_on_target(&inner.solid, &landed_faces, target, naming);
    let merged = if joined.is_empty() {
        inner.solid
    } else {
        merged_faces(&inner.solid, &joined).map_err(|failure| match failure {
            MergeFailure::Cancelled(interrupted) => ReplaceError::from(interrupted),
            MergeFailure::Unmergeable => unmergeable(solid, &landing, &inner.dropped),
        })?
    };
    Ok(uncrossed(merged)?)
}

fn unmergeable(
    solid: &Solid,
    landing: &BTreeSet<FaceId>,
    dropped: &BTreeSet<FaceId>,
) -> ReplaceError {
    let first = landing.first().copied();
    let beside = first.and_then(|face| {
        loop_edges(solid, face)
            .into_iter()
            .flat_map(|edge| edge_faces(solid, edge))
            .find(|other| *other != face && !dropped.contains(other))
    });
    match (first, beside) {
        (Some(first), Some(second)) => ReplaceError::Merging { first, second },
        _ => MoveError::Invalid {
            faces: Vec::new(),
            edge: None,
        }
        .into(),
    }
}

pub fn replace_faces(
    solid: &Solid,
    faces: &[FaceId],
    target: &Plane,
) -> Result<Solid, ReplaceError> {
    replace_naming(solid, faces, target, MergedName::OfReplaced)
}

pub(super) fn replace_naming(
    solid: &Solid,
    faces: &[FaceId],
    target: &Plane,
    naming: MergedName,
) -> Result<Solid, ReplaceError> {
    replaced(solid, faces, target, naming).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn replaced(
    solid: &Solid,
    faces: &[FaceId],
    target: &Plane,
    naming: MergedName,
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
    landed(solid, &grouped(landings), target, naming)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RoundLanding {
    shift: Vector3,
    outward: f64,
}

impl RoundLanding {
    fn about_own_axis(growth: f64, sign: f64) -> Self {
        Self::across(Vector3::ZERO, growth, sign)
    }

    fn across(aside: Vector3, growth: f64, sign: f64) -> Self {
        let shift = if aside.length() <= LINEAR_RESOLUTION {
            Vector3::ZERO
        } else {
            aside
        };
        let outward = if growth.abs() <= LINEAR_RESOLUTION {
            0.0
        } else {
            growth * sign
        };
        Self { shift, outward }
    }

    fn shifts(&self) -> bool {
        self.shift != Vector3::ZERO
    }
}

fn same_axis_direction(first: &Plane, second: &Plane) -> Option<f64> {
    let along = first.normal().dot(second.normal());
    (first.normal().cross(second.normal()).length() <= ANGULAR_RESOLUTION).then_some(along.signum())
}

fn round_landing(
    solid: &Solid,
    face: FaceId,
    target: &Surface,
) -> Result<RoundLanding, ReplaceError> {
    let definition = solid.face(face).ok_or(ReplaceError::MissingFace(face))?;
    let sign = definition.sense().sign();
    let off_axis = ReplaceError::OffAxis(face);
    match (definition.surface(), target) {
        (Surface::Plane(_), _) => Err(ReplaceError::FlatOntoRound(face)),
        (Surface::Cylinder(own), Surface::Cylinder(wanted)) => {
            let (own_frame, wanted_frame) = (own.frame(), wanted.frame());

            same_axis_direction(own_frame, wanted_frame).ok_or(off_axis)?;

            let aside =
                (wanted_frame.origin() - own_frame.origin()).reject_from(own_frame.normal());
            Ok(RoundLanding::across(
                aside,
                wanted.radius() - own.radius(),
                sign,
            ))
        }
        (Surface::Cone(own), Surface::Cone(wanted)) => {
            let (own_frame, wanted_frame) = (own.frame(), wanted.frame());

            let turned = same_axis_direction(own_frame, wanted_frame).ok_or(off_axis.clone())?;
            let between = wanted_frame.origin() - own_frame.origin();
            let slope = own.half_angle().tan_libm();
            let wanted_slope = wanted.half_angle().tan_libm() * turned;
            let height = between.dot(own_frame.normal());
            let aside = between.reject_from(own_frame.normal()).length();

            if aside > LINEAR_RESOLUTION || (slope - wanted_slope).abs() > ANGULAR_RESOLUTION {
                return Err(off_axis);
            }

            let wanted_here = wanted.radius() - wanted_slope * height;
            Ok(RoundLanding::about_own_axis(
                (wanted_here - own.radius()) * own.half_angle().cos_libm(),
                sign,
            ))
        }
        (Surface::Sphere(own), Surface::Sphere(wanted)) => {
            let apart = own.frame().origin().distance(wanted.frame().origin());

            if apart > LINEAR_RESOLUTION {
                return Err(off_axis);
            }
            Ok(RoundLanding::about_own_axis(
                wanted.radius() - own.radius(),
                sign,
            ))
        }
        (Surface::Torus(own), Surface::Torus(wanted)) => {
            let (own_frame, wanted_frame) = (own.frame(), wanted.frame());

            same_axis_direction(own_frame, wanted_frame).ok_or(off_axis.clone())?;

            let apart = own_frame.origin().distance(wanted_frame.origin());
            let same_ring = (own.major_radius() - wanted.major_radius()).abs() <= LINEAR_RESOLUTION;

            if apart > LINEAR_RESOLUTION || !same_ring {
                return Err(off_axis);
            }
            Ok(RoundLanding::about_own_axis(
                wanted.minor_radius() - own.minor_radius(),
                sign,
            ))
        }
        _ => Err(off_axis),
    }
}

pub fn replace_faces_onto_round(
    solid: &Solid,
    faces: &[FaceId],
    target: &Surface,
) -> Result<Solid, ReplaceError> {
    onto_round(solid, faces, target).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn onto_round(solid: &Solid, faces: &[FaceId], target: &Surface) -> Result<Solid, ReplaceError> {
    if faces.is_empty() {
        return Err(ReplaceError::NothingToReplace);
    }

    let mut inward: BTreeMap<FaceId, f64> = BTreeMap::new();
    let mut shifted: BTreeMap<FaceId, Vector3> = BTreeMap::new();

    for face in faces {
        let landing = round_landing(solid, *face, target)?;

        if landing.outward != 0.0 {
            inward.insert(*face, -landing.outward);
        }
        if landing.shifts() {
            shifted.insert(*face, landing.shift);
        }
    }
    interrupt::check()?;

    let offsets = match shifted.first_key_value() {
        None if inward.is_empty() => return Err(ReplaceError::AlreadyOnTarget),
        None => Offsets::moving_each(solid, inward),
        Some((first, by)) => {
            let unlike = shifted
                .iter()
                .find(|(_, other)| other.distance(*by) > LINEAR_RESOLUTION)
                .map(|(face, _)| *face);

            if let Some(face) = unlike {
                return Err(ReplaceError::OffAxis(face));
            }

            let moved: Vec<FaceId> = shifted.keys().copied().collect();
            let motion = RigidTransform::translation(*by).ok_or(ReplaceError::OffAxis(*first))?;
            carrying(
                solid,
                vec![Carried {
                    faces: &moved,
                    motion: Motion::rigid(motion),
                }],
            )?
            .offsetting(inward)
        }
    };
    let inner = inner_solid(&offsets, Naming::Kept).map_err(MoveError::from)?;

    if let Some(face) = inner.dropped.iter().next() {
        return Err(MoveError::Vanishes(*face).into());
    }
    Ok(uncrossed(inner.solid)?)
}
