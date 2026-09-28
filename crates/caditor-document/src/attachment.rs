use caditor_geometry::Plane;
use caditor_kernel::{FaceId, FaceReference, ReferenceError, Solid, Surface};
use caditor_sketch::Sketch;

use crate::{
    datum::DatumResult,
    document::{Feature, FeatureId},
    recompute::{Failure, FeatureError, FeatureResult, FixTarget, Inputs},
};

#[derive(Debug, Clone, PartialEq)]
pub struct FaceAttachment {
    pub body: FeatureId,
    pub face: FaceReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentError {
    Missing,
    Ambiguous,
    NotFlat,
}

impl FaceAttachment {
    pub fn capture(body: FeatureId, solid: &Solid, face: FaceId) -> Option<(Self, Plane)> {
        let plane = face_plane(solid, face)?;
        let face = FaceReference::capture(solid, face)?;
        Some((Self { body, face }, plane))
    }

    pub fn resolve(&self, solid: &Solid) -> Result<Plane, AttachmentError> {
        let candidates = match self.face.resolve(solid) {
            Ok(face) => vec![face],
            Err(ReferenceError::Ambiguous(candidates)) => candidates,
            Err(ReferenceError::Missing) => return Err(AttachmentError::Missing),
        };
        let planes: Vec<Option<Plane>> = candidates
            .iter()
            .map(|face| face_plane(solid, *face))
            .collect();
        match planes.split_first() {
            Some((Some(first), rest)) if rest.iter().all(|other| *other == Some(*first)) => {
                Ok(*first)
            }
            Some((None, [])) => Err(AttachmentError::NotFlat),
            Some(_) => Err(AttachmentError::Ambiguous),
            None => Err(AttachmentError::Missing),
        }
    }
}

pub fn face_plane(solid: &Solid, face: FaceId) -> Option<Plane> {
    let face = solid.face(face)?;
    let Surface::Plane(surface) = face.surface() else {
        return None;
    };
    let frame = *surface.frame();
    Some(if face.sense().is_same() {
        frame
    } else {
        frame.flipped()
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum SketchAttachment {
    Face(FaceAttachment),
    Datum(FeatureId),
}

impl SketchAttachment {
    pub fn body(&self) -> Option<FeatureId> {
        match self {
            Self::Face(attachment) => Some(attachment.body),
            Self::Datum(_) => None,
        }
    }

    pub fn datum(&self) -> Option<FeatureId> {
        match self {
            Self::Datum(datum) => Some(*datum),
            Self::Face(_) => None,
        }
    }

    pub fn face(&self) -> Option<&FaceAttachment> {
        match self {
            Self::Face(attachment) => Some(attachment),
            Self::Datum(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SketchFeature {
    pub sketch: Sketch,
    pub attachment: Option<SketchAttachment>,
}

impl SketchFeature {
    pub fn on_face(sketch: Sketch, attachment: FaceAttachment) -> Self {
        Self {
            sketch,
            attachment: Some(SketchAttachment::Face(attachment)),
        }
    }

    pub fn on_datum(sketch: Sketch, datum: FeatureId) -> Self {
        Self {
            sketch,
            attachment: Some(SketchAttachment::Datum(datum)),
        }
    }
}

impl From<Sketch> for SketchFeature {
    fn from(sketch: Sketch) -> Self {
        Self {
            sketch,
            attachment: None,
        }
    }
}

pub(crate) fn attached_plane(
    feature: &Feature,
    attachment: &SketchAttachment,
    inputs: &Inputs<'_>,
) -> Result<Plane, Failure> {
    match attachment {
        SketchAttachment::Face(face) => face_attached_plane(feature, face, inputs),
        SketchAttachment::Datum(datum) => {
            let name = inputs
                .document
                .feature(*datum)
                .map(|datum| datum.name.clone())
                .unwrap_or_default();
            match inputs.features.get(datum).map(AsRef::as_ref) {
                Some(FeatureResult::Datum(DatumResult::Plane(plane))) => Ok(*plane),
                _ => Err(Failure::Error(FeatureError {
                    reason: format!("The plane this sketch lies on, {name}, is not available."),
                    remedy: format!("Fix {name} first, or place the sketch on another plane."),
                    fix: Some(FixTarget::Feature(*datum)),
                    constraints: Vec::new(),
                })),
            }
        }
    }
}

fn face_attached_plane(
    feature: &Feature,
    attachment: &FaceAttachment,
    inputs: &Inputs<'_>,
) -> Result<Plane, Failure> {
    let body_name = inputs
        .document
        .feature(attachment.body)
        .map(|body| body.name.clone())
        .unwrap_or_default();
    let Some(solid) = inputs.body(attachment.body) else {
        return Err(Failure::Error(FeatureError {
            reason: format!("The body of {body_name}, which this sketch lies on, has no shape."),
            remedy: format!("Fix {body_name} first."),
            fix: Some(FixTarget::Feature(attachment.body)),
            constraints: Vec::new(),
        }));
    };
    attachment.resolve(solid).map_err(|error| {
        let reason = match error {
            AttachmentError::Missing => {
                format!("The face this sketch lies on is no longer part of {body_name}.")
            }
            AttachmentError::Ambiguous => format!(
                "The face of {body_name} that this sketch lies on was split into parts that no \
                 longer lie in one plane."
            ),
            AttachmentError::NotFlat => {
                format!("The face of {body_name} that this sketch lies on is no longer flat.")
            }
        };
        Failure::Error(FeatureError {
            reason,
            remedy: "Select a flat face and place the sketch on it from its row in the tree, or \
                     detach the sketch."
                .to_owned(),
            fix: Some(FixTarget::Feature(feature.id())),
            constraints: Vec::new(),
        })
    })
}
