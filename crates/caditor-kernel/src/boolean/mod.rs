mod assemble;
mod faces;
mod heal;
mod imprint;
mod select;
#[cfg(test)]
mod tests;
mod trace;

use caditor_geometry::{Aabb, Aabb2, Point3};
use thiserror::Error;

use crate::{
    box_tree::BoxTree,
    build::plan::PlanError,
    interrupt::{self, Interrupted},
    intersect::{IntersectionError, patch_bounds},
    tolerance::LINEAR_RESOLUTION,
    topology::{BuildError, Face, FaceId, PcurveError, Solid, SolidClassifier},
};

const TOLERANCE: f64 = LINEAR_RESOLUTION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BooleanOperation {
    Union,
    Difference,
    Intersection,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BooleanSite {
    pub first: Vec<FaceId>,
    pub second: Vec<FaceId>,
    pub point: Option<Point3>,
}

impl BooleanSite {
    pub fn is_empty(&self) -> bool {
        self.first.is_empty() && self.second.is_empty() && self.point.is_none()
    }

    fn has_faces(&self) -> bool {
        !self.first.is_empty() || !self.second.is_empty()
    }

    fn add_face(&mut self, key: FaceKey) {
        let faces = match key.operand {
            Operand::First => &mut self.first,
            Operand::Second => &mut self.second,
        };
        if !faces.contains(&key.face) {
            faces.push(key.face);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum BooleanError {
    #[error("nothing is left of the solids")]
    Empty,
    #[error("the solids could not be intersected: {error}")]
    Intersection {
        error: IntersectionError,
        site: Box<BooleanSite>,
    },
    #[error("a face could not be divided where the solids meet")]
    Split(Box<BooleanSite>),
    #[error("the solids touch where it cannot be told which side is inside")]
    Ambiguous(Box<BooleanSite>),
    #[error("the faces of the result do not join up into closed shells")]
    Open(Box<BooleanSite>),
    #[error("the result would have solids that meet only along an edge")]
    NonManifold(Box<BooleanSite>),
    #[error("the result is not a valid solid: {0}")]
    Invalid(BuildError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl From<IntersectionError> for BooleanError {
    fn from(error: IntersectionError) -> Self {
        match error {
            IntersectionError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Intersection {
                error,
                site: Box::default(),
            },
        }
    }
}

impl From<BuildError> for BooleanError {
    fn from(error: BuildError) -> Self {
        match error.interrupted() {
            Some(interrupted) => Self::Cancelled(interrupted),
            None => Self::Invalid(error),
        }
    }
}

impl BooleanError {
    pub fn site(&self) -> Option<&BooleanSite> {
        match self {
            Self::Intersection { site, .. }
            | Self::Split(site)
            | Self::Ambiguous(site)
            | Self::Open(site)
            | Self::NonManifold(site) => Some(&**site),
            Self::Empty | Self::Invalid(_) | Self::Cancelled(_) => None,
        }
    }

    fn site_mut(&mut self) -> Option<&mut BooleanSite> {
        match self {
            Self::Intersection { site, .. }
            | Self::Split(site)
            | Self::Ambiguous(site)
            | Self::Open(site)
            | Self::NonManifold(site) => Some(&mut **site),
            Self::Empty | Self::Invalid(_) | Self::Cancelled(_) => None,
        }
    }

    fn split() -> Self {
        Self::Split(Box::default())
    }

    fn unfitted(error: PcurveError) -> Self {
        match error {
            PcurveError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            _ => Self::split(),
        }
    }

    fn or_faces(mut self, keys: impl IntoIterator<Item = FaceKey>) -> Self {
        if let Some(site) = self.site_mut()
            && !site.has_faces()
        {
            for key in keys {
                site.add_face(key);
            }
        }
        self
    }

    fn or_point(mut self, point: impl FnOnce() -> Option<Point3>) -> Self {
        if let Some(site) = self.site_mut()
            && site.point.is_none()
        {
            site.point = point();
        }
        self
    }
}

impl From<PlanError> for BooleanError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::Unassembled => Self::Open(Box::default()),
            PlanError::Build(error) => error.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Operand {
    First,
    Second,
}

impl Operand {
    const BOTH: [Self; 2] = [Self::First, Self::Second];

    fn other(self) -> Self {
        match self {
            Self::First => Self::Second,
            Self::Second => Self::First,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FaceKey {
    operand: Operand,
    face: FaceId,
}

#[derive(Debug, Clone, Copy)]
struct FaceBounds {
    id: FaceId,
    uv: Aabb2,
    bounds: Aabb,
}

struct Input<'a> {
    first: &'a Solid,
    second: &'a Solid,
    classifiers: [SolidClassifier<'a>; 2],
    faces: [Vec<FaceBounds>; 2],
    trees: [BoxTree; 2],
    positions: [Vec<Option<usize>>; 2],
}

fn bounds_positions(faces: &[FaceBounds]) -> Vec<Option<usize>> {
    let size = faces
        .iter()
        .map(|face| face.id.index() + 1)
        .max()
        .unwrap_or(0);
    let mut positions = vec![None; size];
    for (position, face) in faces.iter().enumerate() {
        if let Some(slot) = positions.get_mut(face.id.index()) {
            *slot = Some(position);
        }
    }
    positions
}

fn face_bounds(solid: &Solid) -> Vec<FaceBounds> {
    solid
        .faces()
        .filter_map(|(id, face)| {
            let uv = Aabb2::from_points(
                face.loops()
                    .iter()
                    .filter_map(|loop_id| solid.face_loop(*loop_id))
                    .flat_map(|face_loop| face_loop.coedges().iter())
                    .filter_map(|coedge| solid.coedge(*coedge))
                    .flat_map(|coedge| coedge.pcurve().samples().iter().map(|sample| sample.uv)),
            )?;
            Some(FaceBounds {
                id,
                uv,
                bounds: patch_bounds(face.surface(), uv).expanded(TOLERANCE),
            })
        })
        .collect()
}

impl<'a> Input<'a> {
    fn new(first: &'a Solid, second: &'a Solid) -> Self {
        let (first_faces, second_faces) = (face_bounds(first), face_bounds(second));
        Self {
            first,
            second,
            classifiers: [first.classifier(), second.classifier()],
            positions: [
                bounds_positions(&first_faces),
                bounds_positions(&second_faces),
            ],
            trees: [
                BoxTree::new(first_faces.iter().map(|face| face.bounds)),
                BoxTree::new(second_faces.iter().map(|face| face.bounds)),
            ],
            faces: [first_faces, second_faces],
        }
    }

    fn solid(&self, operand: Operand) -> &'a Solid {
        match operand {
            Operand::First => self.first,
            Operand::Second => self.second,
        }
    }

    fn classifier(&self, operand: Operand) -> &SolidClassifier<'a> {
        let [first, second] = &self.classifiers;
        match operand {
            Operand::First => first,
            Operand::Second => second,
        }
    }

    fn faces(&self, operand: Operand) -> &[FaceBounds] {
        let [first, second] = &self.faces;
        match operand {
            Operand::First => first,
            Operand::Second => second,
        }
    }

    fn faces_near(&self, operand: Operand, bounds: &Aabb) -> impl Iterator<Item = &FaceBounds> {
        let [first, second] = &self.trees;
        let tree = match operand {
            Operand::First => first,
            Operand::Second => second,
        };
        let faces = self.faces(operand);
        tree.overlapping(bounds, TOLERANCE)
            .into_iter()
            .filter_map(|index| faces.get(index))
    }

    fn face(&self, key: FaceKey) -> Option<&'a Face> {
        self.solid(key.operand).face(key.face)
    }

    fn bounds(&self, key: FaceKey) -> Option<&FaceBounds> {
        let [first, second] = &self.positions;
        let positions = match key.operand {
            Operand::First => first,
            Operand::Second => second,
        };
        let position = (*positions.get(key.face.index())?)?;
        self.faces(key.operand).get(position)
    }
}

pub fn boolean(
    first: &Solid,
    second: &Solid,
    operation: BooleanOperation,
) -> Result<Solid, BooleanError> {
    combine(first, second, operation).or_else(|error| {
        interrupt::check()?;
        Err(error)
    })
}

fn combine(
    first: &Solid,
    second: &Solid,
    operation: BooleanOperation,
) -> Result<Solid, BooleanError> {
    interrupt::check()?;
    let input = Input::new(first, second);
    let mut arrangement = imprint::imprint(&input)?;
    interrupt::check()?;
    let split = faces::split(&input, &arrangement)?;
    interrupt::check()?;
    let kept = select::select(&input, &arrangement, split, operation)?;
    if kept.is_empty() {
        return Err(BooleanError::Empty);
    }
    interrupt::check()?;
    let healed = heal::heal(&mut arrangement, kept)?;
    interrupt::check()?;
    assemble::assemble(&arrangement, healed)
}
