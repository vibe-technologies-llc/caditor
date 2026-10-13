use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_document::{
    Calibration, FeatureId, ImagePlane, ImageSize, ImageSpot, MIN_IMAGE_OPACITY_PERCENT, Picture,
    PictureDigest, PixelSize, PlaneReference, PrincipalPlane, ReferenceImage,
};
use caditor_expression::Expression;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    base64,
    format::{
        FeatureKindRecord, Lenient, PlaneReferenceRecord, plane_reference_record,
        restore_plane_reference, restore_value,
    },
};

#[derive(Debug, Clone)]
pub(crate) enum PictureField {
    Read(String),
    Kept(Arc<Picture>),
}

impl PictureField {
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Read(text) => base64::decode(text),
            Self::Kept(picture) => Some(picture.bytes().to_vec()),
        }
    }
}

impl PartialEq for PictureField {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Kept(own), Self::Kept(theirs)) => own.digest() == theirs.digest(),
            (Self::Read(own), Self::Read(theirs)) => own == theirs,
            _ => self
                .bytes()
                .is_some_and(|bytes| other.bytes() == Some(bytes)),
        }
    }
}

impl Serialize for PictureField {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Read(text) => serializer.serialize_str(text),
            Self::Kept(picture) => serializer.serialize_str(&base64::encode(picture.bytes())),
        }
    }
}

impl<'de> Deserialize<'de> for PictureField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::Read)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReferenceImageRecord {
    pub picture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<PictureField>,
    pub pixels: [u32; 2],
    pub plane: Lenient<ImagePlaneRecord>,
    pub at: [String; 2],
    pub turn: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flipped: bool,
    pub size: Lenient<ImageSizeRecord>,
    pub opacity: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_front: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImagePlaneRecord {
    Plane(PlaneReferenceRecord),
    Sketch(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImageSizeRecord {
    Width(String),
    Calibrated {
        from: [f64; 2],
        to: [f64; 2],
        distance: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct Pictures {
    by_digest: BTreeMap<PictureDigest, Arc<Picture>>,
}

impl Pictures {
    pub fn of(features: impl IntoIterator<Item = Arc<Picture>>) -> Self {
        Self {
            by_digest: features
                .into_iter()
                .map(|picture| (picture.digest(), picture))
                .collect(),
        }
    }

    fn keep(&mut self, picture: &Arc<Picture>) {
        self.by_digest
            .entry(picture.digest())
            .or_insert_with(|| Arc::clone(picture));
    }
}

pub(crate) fn reference_image_record(image: &ReferenceImage) -> ReferenceImageRecord {
    let mut image = image.clone();
    image.settle();

    let text = Expression::to_stored_text;
    let size = image.picture.size();
    let spot = |spot: ImageSpot| [spot.across, spot.up];
    ReferenceImageRecord {
        picture: image.picture.digest().hex(),
        data: Some(PictureField::Kept(Arc::clone(&image.picture))),
        pixels: [size.width, size.height],
        plane: Lenient::Read(match &image.plane {
            ImagePlane::Reference(plane) => ImagePlaneRecord::Plane(plane_reference_record(plane)),
            ImagePlane::Sketch(sketch) => ImagePlaneRecord::Sketch(sketch.raw()),
        }),
        at: image.at.each_ref().map(text),
        turn: text(&image.turn),
        flipped: image.flipped,
        size: Lenient::Read(match &image.size {
            ImageSize::Width(width) => ImageSizeRecord::Width(text(width)),
            ImageSize::Calibrated(calibration) => ImageSizeRecord::Calibrated {
                from: spot(calibration.from),
                to: spot(calibration.to),
                distance: text(&calibration.distance),
            },
        }),
        opacity: image.opacity,
        in_front: image.in_front,
        locked: image.locked,
    }
}

pub(crate) fn share_picture(record: &mut FeatureKindRecord, written: &mut BTreeSet<String>) {
    let FeatureKindRecord::ReferenceImage(image) = record else {
        return;
    };
    if !written.insert(image.picture.clone()) {
        image.data = None;
    }
}

pub(crate) fn restore_reference_image(
    record: &ReferenceImageRecord,
    name: &str,
    pictures: &mut Pictures,
    issues: &mut Vec<String>,
) -> ReferenceImage {
    let picture = restore_picture(record, name, pictures, issues);
    let plane = match &record.plane {
        Lenient::Read(ImagePlaneRecord::Sketch(sketch)) => {
            Some(ImagePlane::Sketch(FeatureId::from_raw(*sketch)))
        }
        Lenient::Read(ImagePlaneRecord::Plane(plane)) => {
            restore_plane_reference(plane).map(ImagePlane::Reference)
        }
        Lenient::Unreadable(_) => None,
    };
    let plane = plane.unwrap_or_else(|| {
        issues.push(format!(
            "The plane, face or sketch “{name}” lies on could not be read, so it lies on the XY \
             plane."
        ));
        ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy))
    });
    let size = match &record.size {
        Lenient::Read(ImageSizeRecord::Width(width)) => {
            ImageSize::Width(restore_value(width, "width", "100 mm", name, issues))
        }
        Lenient::Read(ImageSizeRecord::Calibrated { from, to, distance }) => {
            let spot = |[across, up]: [f64; 2]| ImageSpot::new(across, up).settled();
            ImageSize::Calibrated(Calibration {
                from: spot(*from),
                to: spot(*to),
                distance: restore_value(
                    distance,
                    "distance between the calibration points",
                    "100 mm",
                    name,
                    issues,
                ),
            })
        }
        Lenient::Unreadable(_) => {
            issues.push(format!(
                "The size of “{name}” could not be read, so it is 100 mm wide."
            ));
            ImageSize::Width(restore_value("100 mm", "width", "100 mm", name, issues))
        }
    };
    if record.opacity < MIN_IMAGE_OPACITY_PERCENT {
        issues.push(format!(
            "The opacity of “{name}” was below {MIN_IMAGE_OPACITY_PERCENT}%, so it was raised to \
             it."
        ));
    }
    let [along, across] = &record.at;
    let mut image = ReferenceImage {
        picture,
        plane,
        at: [
            restore_value(along, "position along X", "0 mm", name, issues),
            restore_value(across, "position along Y", "0 mm", name, issues),
        ],
        turn: restore_value(&record.turn, "angle", "0 deg", name, issues),
        flipped: record.flipped,
        size,
        opacity: record.opacity,
        in_front: record.in_front,
        locked: record.locked,
    };
    image.settle();
    image
}

fn restore_picture(
    record: &ReferenceImageRecord,
    name: &str,
    pictures: &mut Pictures,
    issues: &mut Vec<String>,
) -> Arc<Picture> {
    let recorded = PictureDigest::from_hex(&record.picture);
    let read = match &record.data {
        Some(PictureField::Kept(picture)) => Some(Arc::clone(picture)),
        Some(field @ PictureField::Read(_)) => {
            let [width, height] = record.pixels;
            field
                .bytes()
                .and_then(|bytes| Picture::new(bytes, PixelSize { width, height }).ok())
                .map(Arc::new)
        }
        None => recorded.and_then(|digest| pictures.by_digest.get(&digest).cloned()),
    };
    match read {
        Some(picture) => {
            pictures.keep(&picture);
            if let Some(digest) = recorded
                && digest != picture.digest()
            {
                pictures.by_digest.insert(digest, Arc::clone(&picture));
            }
            picture
        }
        None => {
            issues.push(match &record.data {
                Some(_) => format!(
                    "The picture of “{name}” could not be read, so it shows as a grey square; \
                     insert the picture again."
                ),
                None => format!(
                    "The picture of “{name}” was kept with another reference image that could \
                     not be read, so it shows as a grey square; insert the picture again."
                ),
            });
            Arc::new(Picture::placeholder())
        }
    }
}
