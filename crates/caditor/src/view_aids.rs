use caditor_render::Reflection;

use crate::analysis::FaceAnalysis;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewAids {
    pub centres_of_mass: bool,
    pub control_polygons_hidden: bool,
    pub analysis: Option<FaceAnalysis>,
    pub reflection: Option<Reflection>,
    pub edges: EdgeLook,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeLook {
    pub weight: EdgeWeight,
    pub outlines_hidden: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EdgeWeight {
    Thin,
    #[default]
    Standard,
    Thick,
}

impl EdgeWeight {
    pub fn factor(self) -> f32 {
        match self {
            Self::Thin => 0.75,
            Self::Standard => 1.0,
            Self::Thick => 1.5,
        }
    }
}
