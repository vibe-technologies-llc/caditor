use std::collections::BTreeMap;

use super::ShellError;
use crate::{
    tolerance::LINEAR_RESOLUTION,
    topology::{FaceId, Solid},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellSide {
    #[default]
    Inside,
    Outside,
    BothSides,
}

impl ShellSide {
    pub const ALL: [Self; 3] = [Self::Inside, Self::Outside, Self::BothSides];

    pub(super) fn inward_share(self) -> f64 {
        match self {
            Self::Inside => 1.0,
            Self::Outside => 0.0,
            Self::BothSides => 0.5,
        }
    }

    pub(super) fn outward_share(self) -> f64 {
        1.0 - self.inward_share()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Walls {
    pub thickness: f64,
    pub side: ShellSide,
    pub faces: BTreeMap<FaceId, f64>,
}

impl Walls {
    pub fn inside(thickness: f64) -> Self {
        Self {
            thickness,
            side: ShellSide::Inside,
            faces: BTreeMap::new(),
        }
    }

    pub(super) fn of(&self, face: FaceId) -> f64 {
        self.faces.get(&face).copied().unwrap_or(self.thickness)
    }

    pub(super) fn reach(&self) -> f64 {
        self.faces.values().copied().fold(self.thickness, f64::max)
    }

    pub(super) fn check(&self, solid: &Solid, open: &[FaceId]) -> Result<(), ShellError> {
        let usable = |thickness: f64| thickness.is_finite() && thickness > LINEAR_RESOLUTION;
        if !usable(self.thickness) {
            return Err(ShellError::InvalidThickness);
        }
        for (face, thickness) in &self.faces {
            solid.face(*face).ok_or(ShellError::MissingFace(*face))?;
            if open.contains(face) {
                return Err(ShellError::OpenedFaceThickness(*face));
            }
            if !usable(*thickness) {
                return Err(ShellError::InvalidFaceThickness(*face));
            }
        }
        Ok(())
    }
}
