use caditor_document::Rgb;
use caditor_render::Reflection;

use crate::{analysis::FaceAnalysis, body_appearance::DEFAULT_COLOUR};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewAids {
    pub centres_of_mass: bool,
    pub control_polygons_hidden: bool,
    pub analysis: Option<FaceAnalysis>,
    pub reflection: Option<Reflection>,
    pub edges: EdgeLook,
    pub body: DefaultBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultBody(pub Rgb);

impl Default for DefaultBody {
    fn default() -> Self {
        Self(DEFAULT_COLOUR)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeLook {
    pub weight: EdgeWeight,
    pub outlines_hidden: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
