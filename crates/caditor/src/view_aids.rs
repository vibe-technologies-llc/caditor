use crate::analysis::FaceAnalysis;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewAids {
    pub centres_of_mass: bool,
    pub analysis: Option<FaceAnalysis>,
}
