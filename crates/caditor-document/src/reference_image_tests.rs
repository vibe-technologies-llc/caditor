use std::sync::Arc;

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point3, Vector3};
use caditor_sketch::Sketch;

use crate::*;

const PNG_START: &[u8] = b"\x89PNG\r\n\x1a\n";

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

pub(crate) fn picture(width: u32, height: u32) -> Arc<Picture> {
    let mut bytes = PNG_START.to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    Arc::new(Picture::new(bytes, PixelSize { width, height }).unwrap())
}

pub(crate) fn image(plane: ImagePlane) -> ReferenceImage {
    ReferenceImage {
        picture: picture(200, 100),
        plane,
        at: [millimetres(10.0), millimetres(0.0)],
        turn: degrees(0.0),
        flipped: false,
        size: ImageSize::Width(millimetres(100.0)),
        opacity: DEFAULT_IMAGE_OPACITY_PERCENT,
        in_front: false,
        locked: false,
    }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn placed(document: &Document, feature: FeatureId) -> PlacedImage {
    let evaluation = evaluate(document);
    *evaluation
        .feature(feature)
        .unwrap()
        .result
        .as_deref()
        .and_then(FeatureResult::image)
        .unwrap()
}

fn failure(document: &Document, feature: FeatureId) -> String {
    let evaluation = evaluate(document);
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.reason().clone(),
        state => panic!("the image did not fail: {state:?}"),
    }
}

fn set_kind(document: &mut Document, feature: FeatureId, image: ReferenceImage) {
    document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::ReferenceImage(Box::new(image)),
            },
        ))
        .unwrap();
}

#[test]
fn an_image_lies_on_its_plane_at_its_place_with_the_picture_shape() {
    let mut document = Document::default();
    let shown = image(ImagePlane::Reference(PlaneReference::Principal(
        PrincipalPlane::Xz,
    )));
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(shown)),
    );

    let placed = placed(&document, feature);

    assert_eq!(placed.width, 100.0);
    assert_eq!(placed.height, 50.0);
    assert!(placed.centre.distance(Point3::new(10.0, 0.0, 0.0)) < 1e-9);
    assert!(placed.normal().distance(Plane::XZ.normal()) < 1e-9);
    assert!(placed.corners()[2].distance(Point3::new(60.0, 0.0, 25.0)) < 1e-9);
}

#[test]
fn turning_and_flipping_keep_each_spot_of_the_picture_where_it_was_drawn() {
    let mut document = Document::default();
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(ImagePlane::Reference(
            PlaneReference::Principal(PrincipalPlane::Xy),
        )))),
    );
    let mut turned = image(ImagePlane::Reference(PlaneReference::Principal(
        PrincipalPlane::Xy,
    )));
    turned.turn = degrees(90.0);
    turned.flipped = true;
    set_kind(&mut document, feature, turned);

    let placed = placed(&document, feature);
    let spot = ImageSpot::new(0.25, 0.75);

    assert!(placed.across.distance(Vector3::Y) < 1e-9);
    assert!(placed.point_at(ImageSpot::new(0.0, 0.5)).y > placed.centre.y);
    let back = placed.spot_at(placed.point_at(spot));
    assert!((back.across - spot.across).abs() < 1e-9);
    assert!((back.up - spot.up).abs() < 1e-9);
}

#[test]
fn calibrating_by_two_points_sizes_the_picture_so_they_lie_the_typed_distance_apart() {
    let mut document = Document::default();
    let plane = ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy));
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(plane.clone()))),
    );
    let from = ImageSpot::new(0.1, 0.2);
    let to = ImageSpot::new(0.7, 0.8);
    let mut calibrated = image(plane);
    calibrated.size = ImageSize::Calibrated(Calibration {
        from,
        to,
        distance: millimetres(42.0),
    });
    set_kind(&mut document, feature, calibrated);

    let placed = placed(&document, feature);

    let apart = placed.point_at(from).distance(placed.point_at(to));
    assert!((apart - 42.0).abs() < 1e-9);
    assert!((placed.height * 2.0 - placed.width).abs() < 1e-9);
}

#[test]
fn an_image_on_a_sketch_follows_the_sketch_plane_and_needs_the_sketch_above_it() {
    let mut document = Document::default();
    let sketch = add(
        &mut document,
        "Sketch 1",
        FeatureKind::from(Sketch::new(Plane::YZ)),
    );
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(ImagePlane::Sketch(sketch)))),
    );

    let placed = placed(&document, feature);

    assert!(placed.normal().distance(Plane::YZ.normal()) < 1e-9);
    assert_eq!(
        document.feature(feature).unwrap().kind.reference_sketches(),
        [sketch].into()
    );
    assert!(document.dependents_of(&[sketch]).contains(&feature));
}

#[test]
fn a_size_or_calibration_that_cannot_place_the_picture_fails_it_alone_in_words() {
    let mut document = Document::default();
    let plane = ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy));
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(plane.clone()))),
    );
    let mut flat = image(plane.clone());
    flat.size = ImageSize::Width(millimetres(0.0));
    set_kind(&mut document, feature, flat);
    assert_eq!(
        failure(&document, feature),
        "The width of the image must be more than zero."
    );

    let mut same = image(plane);
    same.size = ImageSize::Calibrated(Calibration {
        from: ImageSpot::new(0.5, 0.5),
        to: ImageSpot::new(0.5, 0.5),
        distance: millimetres(10.0),
    });
    set_kind(&mut document, feature, same);
    assert_eq!(
        failure(&document, feature),
        "The two calibration points are at the same place on the image."
    );
}

#[test]
fn opacity_is_held_within_its_range_and_every_change_undoes() {
    let mut editor = Editor::new(Document::default());
    let plane = ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy));
    let mut transaction = editor.document().transaction("Insert");
    let feature = transaction.add_feature(
        "Photo",
        FeatureKind::ReferenceImage(Box::new(image(plane.clone()))),
    );
    editor.apply(transaction.finish()).unwrap();
    let mut faint = image(plane);
    faint.opacity = 0;
    faint.locked = true;
    editor
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::ReferenceImage(Box::new(faint)),
            },
        ))
        .unwrap();

    let FeatureKind::ReferenceImage(kept) = &editor.document().feature(feature).unwrap().kind
    else {
        panic!("not an image");
    };
    assert_eq!(kept.opacity, MIN_IMAGE_OPACITY_PERCENT);
    assert!(kept.locked);
    editor.undo().unwrap();
    let FeatureKind::ReferenceImage(undone) = &editor.document().feature(feature).unwrap().kind
    else {
        panic!("not an image");
    };
    assert_eq!(undone.opacity, DEFAULT_IMAGE_OPACITY_PERCENT);
    assert!(!undone.locked);
    editor.undo().unwrap();
    assert!(editor.document().feature(feature).is_none());
    editor.redo().unwrap();
    assert!(editor.document().feature(feature).is_some());
}

#[test]
fn an_inserted_image_is_settled_as_a_kind_change_settles_it_so_undoing_a_change_restores_it() {
    let mut document = Document::default();
    let plane = ImagePlane::Reference(PlaneReference::Principal(PrincipalPlane::Xy));
    let mut faint = image(plane.clone());
    faint.opacity = 0;
    faint.size = ImageSize::Calibrated(Calibration {
        from: ImageSpot {
            across: -1.0,
            up: f64::NAN,
        },
        to: ImageSpot {
            across: 2.0,
            up: 0.5,
        },
        distance: millimetres(30.0),
    });
    let feature = add(
        &mut document,
        "Photo",
        FeatureKind::ReferenceImage(Box::new(faint)),
    );
    let inserted = document.clone();

    let inverse = document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::ReferenceImage(Box::new(image(plane))),
            },
        ))
        .unwrap();
    document.apply(inverse).unwrap();

    let FeatureKind::ReferenceImage(kept) = &inserted.feature(feature).unwrap().kind else {
        panic!("not an image");
    };
    assert_eq!(kept.opacity, MIN_IMAGE_OPACITY_PERCENT);
    assert_eq!(
        kept.size,
        ImageSize::Calibrated(Calibration {
            from: ImageSpot {
                across: 0.0,
                up: 0.5,
            },
            to: ImageSpot {
                across: 1.0,
                up: 0.5,
            },
            distance: millimetres(30.0),
        })
    );
    assert!(document.same_content(&inserted));
}

#[test]
fn a_picture_is_known_by_its_content_and_refuses_what_it_cannot_be() {
    let first = picture(4, 3);
    let same = picture(4, 3);

    assert_eq!(first.digest(), same.digest());
    assert_eq!(first.format(), PictureFormat::Png);
    assert_eq!(
        PictureDigest::from_hex(&first.digest().hex()),
        Some(first.digest())
    );
    assert_eq!(
        Picture::new(
            b"not a picture".to_vec(),
            PixelSize {
                width: 1,
                height: 1
            }
        )
        .unwrap_err(),
        PictureError::UnknownFormat
    );
    assert_eq!(
        Picture::new(
            b"\0\0\0\x1cftypavif".to_vec(),
            PixelSize {
                width: 1,
                height: 1
            }
        )
        .unwrap_err(),
        PictureError::Unsupported(UnsupportedPicture::Avif)
    );
    assert_eq!(
        Picture::new(
            PNG_START.to_vec(),
            PixelSize {
                width: MAX_PICTURE_SIDE + 1,
                height: 1
            }
        )
        .unwrap_err(),
        PictureError::TooWide {
            width: MAX_PICTURE_SIDE + 1,
            height: 1
        }
    );
    assert!(Picture::placeholder().is_placeholder());
}
