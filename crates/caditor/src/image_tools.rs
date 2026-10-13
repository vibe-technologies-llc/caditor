use std::{path::Path, sync::Arc};

use caditor_document::{
    Calibration, DEFAULT_IMAGE_OPACITY_PERCENT, Document, Edit, Evaluation, FeatureId, FeatureKind,
    FeatureResult, ImagePlane, ImageSize, ImageSpot, Picture, PlacedImage, PlaneReference,
    PrincipalPlane, ReferenceImage, Transaction,
};
use caditor_geometry::{Point2, Ray};

use crate::{
    datum_tools,
    editing::{self, EditingCommand, SketchEditing},
    field,
    model::{Action, Model, Notice},
    primitive_tools,
    selection::{Pickable, Selection},
    solid_tools,
};

pub const DEFAULT_WIDTH: f64 = 100.0;
pub const NAME_PREFIX: &str = "Image";
const GONE: &str = "The reference image no longer exists";
const NO_PLANE: &str = "Select a plane or flat face made before this reference image";
const NOT_ON_PICTURE: &str = "Click a point on the picture itself";
const NOT_PLACED: &str = "The reference image is not placed yet; fix it or recompute first";
const SAME_SPOT: &str = "Click a second point away from the first one";
const MISSED: &str = "The click does not meet the picture's plane; look at it from another side";

#[derive(Debug, Clone, PartialEq)]
pub enum ImageTarget {
    Plane { plane: PlaneReference, at: Point2 },
    Sketch(FeatureId),
}

pub fn target(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Result<ImageTarget, &'static str> {
    if let Some(sketch) = editing.feature() {
        return Ok(ImageTarget::Sketch(sketch));
    }
    let index = model.document().bar_index();
    match datum_tools::chosen_plane(model, selection, index)? {
        Some(chosen) => {
            let (plane, at) = primitive_tools::chosen_spot(model, &chosen, index)?;
            Ok(ImageTarget::Plane { plane, at })
        }
        None => Ok(ImageTarget::Plane {
            plane: PlaneReference::Principal(PrincipalPlane::Xy),
            at: Point2::ZERO,
        }),
    }
}

pub fn placed(evaluation: &Evaluation, feature: FeatureId) -> Option<PlacedImage> {
    evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::image)
        .copied()
}

fn image_name(document: &Document, path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty() && stem.lines().count() == 1);
    match stem {
        Some(stem) if document.features().all(|feature| feature.name != stem) => stem.to_owned(),
        Some(_) | None => editing::next_feature_name(document, NAME_PREFIX),
    }
}

pub fn create(
    model: &Model,
    target: &ImageTarget,
    picture: Arc<Picture>,
    path: &Path,
) -> (Transaction, FeatureId) {
    let document = model.document();
    let unit = model.length_unit();
    let (plane, at) = match target {
        ImageTarget::Plane { plane, at } => (ImagePlane::Reference(plane.clone()), *at),
        ImageTarget::Sketch(sketch) => (ImagePlane::Sketch(*sketch), Point2::ZERO),
    };
    let name = image_name(document, path);
    let mut transaction = document.transaction(format!("Insert {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::ReferenceImage(Box::new(ReferenceImage {
            picture,
            plane,
            at: [unit.measured(at.x), unit.measured(at.y)],
            turn: solid_tools::degrees(0.0),
            flipped: false,
            size: ImageSize::Width(unit.default_length(DEFAULT_WIDTH)),
            opacity: DEFAULT_IMAGE_OPACITY_PERCENT,
            in_front: false,
            locked: false,
        })),
    );
    (transaction.finish(), feature)
}

pub fn edit(document: &Document, feature: FeatureId, image: ReferenceImage) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    let transaction = Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::ReferenceImage(Box::new(image)),
        },
    );
    Some(document.carrying_owners(transaction, feature, []))
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    image: ReferenceImage,
) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, image).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

fn image_of(model: &Model, feature: FeatureId) -> Option<&ReferenceImage> {
    match &model.document().feature(feature)?.kind {
        FeatureKind::ReferenceImage(image) => Some(image),
        _ => None,
    }
}

fn moved_to(
    model: &Model,
    feature: FeatureId,
    image: &ReferenceImage,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let (plane, at) = primitive_tools::clicked_spot(model, pickable, index, ray)?;
    let unit = model.length_unit();
    change(
        model,
        feature,
        ReferenceImage {
            plane: ImagePlane::Reference(plane),
            at: [unit.measured(at.x), unit.measured(at.y)],
            ..image.clone()
        },
    )
}

pub fn placed_on(
    model: &Model,
    feature: FeatureId,
    image: &ReferenceImage,
    plane: &PlaneReference,
) -> Result<Transaction, String> {
    let pickable = datum_tools::plane_pickable(plane).ok_or_else(|| NO_PLANE.to_owned())?;
    moved_to(model, feature, image, pickable, None)
}

pub fn place_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    image: &ReferenceImage,
) -> Result<Transaction, String> {
    let mut chosen = selection.iter().filter(|pickable| {
        matches!(
            pickable,
            Pickable::Plane(_)
                | Pickable::Datum(_)
                | Pickable::FramePlane { .. }
                | Pickable::Face { .. }
        )
    });
    match (chosen.next(), chosen.next()) {
        (Some(pickable), None) => moved_to(model, feature, image, pickable, None),
        (Some(_), Some(_)) => Err(datum_tools::SEVERAL_PLANES.to_owned()),
        (None, _) => Err(NO_PLANE.to_owned()),
    }
}

pub fn place_click(
    model: &Model,
    feature: FeatureId,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Vec<Action> {
    let Some(image) = image_of(model, feature) else {
        return vec![Action::Editing(EditingCommand::StopPicking)];
    };
    match moved_to(model, feature, image, pickable, ray) {
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(reason))],
    }
}

pub fn spot_under(placed: &PlacedImage, ray: Ray) -> Result<ImageSpot, &'static str> {
    let plane = placed.plane().ok_or(NOT_PLACED)?;
    let distance = ray.intersect_plane(&plane).ok_or(MISSED)?;
    let spot = placed.spot_at(ray.at(distance));
    if placed.holds(spot) {
        Ok(spot)
    } else {
        Err(NOT_ON_PICTURE)
    }
}

pub fn calibrated(
    model: &Model,
    feature: FeatureId,
    (from, to): (ImageSpot, ImageSpot),
) -> Result<Transaction, String> {
    let image = image_of(model, feature).ok_or_else(|| GONE.to_owned())?;
    let placed = placed(model.evaluation(), feature).ok_or_else(|| NOT_PLACED.to_owned())?;
    let calibration = Calibration {
        from,
        to,
        distance: caditor_expression::Expression::Number(0.0),
    };
    let gap = calibration.spot_gap(image.picture.size().aspect());
    let distance = placed.width * gap;
    if !(distance > 0.0 && distance.is_finite()) {
        return Err(SAME_SPOT.to_owned());
    }
    change(
        model,
        feature,
        ReferenceImage {
            size: ImageSize::Calibrated(Calibration {
                distance: model.length_unit().measured(distance),
                ..calibration
            }),
            ..image.clone()
        },
    )
}

pub fn calibration_click(
    model: &Model,
    editing: &SketchEditing,
    feature: FeatureId,
    ray: Option<Ray>,
) -> Vec<Action> {
    let Some(placed) = placed(model.evaluation(), feature) else {
        return vec![Action::Inform(Notice::warning(NOT_PLACED))];
    };
    let spot = match ray.ok_or(MISSED).and_then(|ray| spot_under(&placed, ray)) {
        Ok(spot) => spot,
        Err(reason) => return vec![Action::Inform(Notice::warning(reason))],
    };
    match editing.calibration_from() {
        None => vec![Action::Editing(EditingCommand::CalibrateFrom(spot))],
        Some(from) => match calibrated(model, feature, (from, spot)) {
            Ok(transaction) => vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::StopPicking),
            ],
            Err(reason) => vec![Action::Inform(Notice::warning(reason))],
        },
    }
}

pub fn calibration_prompt(editing_first: bool) -> &'static str {
    if editing_first {
        "Click the second point on the picture whose real distance from the first you know"
    } else {
        "Click a first point on the picture, at one end of a length you know"
    }
}
