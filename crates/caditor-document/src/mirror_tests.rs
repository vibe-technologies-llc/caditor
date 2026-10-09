use std::collections::BTreeSet;

use caditor_expression::Expression;
use caditor_geometry::{Point2, Point3, Vector3};
use caditor_kernel::{FaceId, FaceName, Solid, Surface};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{Pair, evaluate, pair, rectangle, volume},
    pattern_tests::{extrusion, failure, on_top, top},
    *,
};

const PLATE_VOLUME: f64 = 20.0 * 10.0 * 4.0;

fn mirrored(pair: &mut Pair, plane: PlaneReference, keep_original: bool) -> FeatureId {
    let plate = pair.plate;
    let mut transaction = pair.document.transaction("Mirror");
    let feature = transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: plate,
            plane,
            keep_original,
            mirrored: Vec::new(),
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    feature
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> (Point3, Point3) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min(), bounds.max())
}

fn near(found: Point3, expected: [f64; 3]) -> bool {
    (found - Point3::from_array(expected)).length() < 1e-6
}

fn face_names(evaluation: &Evaluation, body: FeatureId) -> BTreeSet<FaceName> {
    evaluation
        .body(body)
        .unwrap()
        .faces()
        .map(|(_, face)| face.name())
        .collect()
}

fn end_face(solid: &Solid, normal: Vector3) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                (plane.frame().normal() * face.sense().sign()).distance(normal) < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

#[test]
fn a_mirror_alone_puts_the_body_on_the_other_side_and_keeps_its_face_names() {
    let mut pair = pair();
    let mut engine = Recompute::default();
    let before = evaluate(&pair.document, &mut engine);
    let names = face_names(&before, pair.plate);

    mirrored(
        &mut pair,
        PlaneReference::Principal(PrincipalPlane::Yz),
        false,
    );
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [-20.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [0.0, 10.0, 4.0]), "{high:?}");
    assert!((volume(&after, pair.plate) - PLATE_VOLUME).abs() < 1e-6 * PLATE_VOLUME);
    assert_eq!(face_names(&after, pair.plate), names);
}

#[test]
fn keeping_the_original_joins_it_with_its_image_across_the_plane() {
    let mut pair = pair();
    let mut engine = Recompute::default();

    mirrored(
        &mut pair,
        PlaneReference::Principal(PrincipalPlane::Yz),
        true,
    );
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [-20.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [20.0, 10.0, 4.0]), "{high:?}");
    assert!((volume(&after, pair.plate) - 2.0 * PLATE_VOLUME).abs() < 1e-6 * PLATE_VOLUME);
}

#[test]
fn a_mirror_across_a_face_of_its_own_body_doubles_it_and_follows_the_face() {
    let mut pair = pair();
    let mut engine = Recompute::default();
    let before = evaluate(&pair.document, &mut engine);
    let plate = before.body(pair.plate).unwrap();
    let (attachment, _) =
        FaceAttachment::capture(pair.plate, plate, end_face(plate, Vector3::X)).unwrap();

    let mirror = mirrored(&mut pair, PlaneReference::Face(attachment), true);
    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [0.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [40.0, 10.0, 4.0]), "{high:?}");
    let feature = pair.document.feature(mirror).unwrap();
    assert!(feature.kind.dependencies().contains(&pair.plate));
    assert!(feature.kind.origin_features().contains(&pair.plate));
}

#[test]
fn a_mirror_across_a_datum_plane_apart_from_the_body_keeps_two_lumps_and_uses_the_datum() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Datum");
    let datum = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Yz),
            rotation: None,
            offset: transaction.parse("30 mm").unwrap(),
        })),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mirror = mirrored(&mut pair, PlaneReference::Datum(datum), true);
    let mut engine = Recompute::default();

    let after = evaluate(&pair.document, &mut engine);

    assert_eq!(after.failed_count(), 0);
    let (low, high) = bounds(&after, pair.plate);
    assert!(near(low, [0.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [60.0, 10.0, 4.0]), "{high:?}");
    assert!((volume(&after, pair.plate) - 2.0 * PLATE_VOLUME).abs() < 1e-6 * PLATE_VOLUME);
    assert!(
        pair.document
            .feature(mirror)
            .unwrap()
            .kind
            .planes_used()
            .contains(&datum)
    );
    assert_eq!(pair.document.dependents_of(&[datum]), vec![mirror]);
}

#[test]
fn faces_of_the_image_are_described_as_the_mirror_image_of_the_original() {
    let mut pair = pair();
    let mut engine = Recompute::default();
    mirrored(
        &mut pair,
        PlaneReference::Principal(PrincipalPlane::Yz),
        true,
    );

    let after = evaluate(&pair.document, &mut engine);
    let solid = after.body(pair.plate).unwrap();
    let image = end_face(solid, Vector3::NEG_X);
    let described = describe_origin(&pair.document, solid.face(image).unwrap().origin());

    assert_eq!(described, "Mirror 1 image of Plate side from Line 5");
}

#[test]
fn a_mirror_across_an_axis_datum_is_refused() {
    let mut pair = pair();
    let mut transaction = pair.document.transaction("Axis");
    let axis = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Z,
        )))),
    );
    pair.document.apply(transaction.finish()).unwrap();
    let mut transaction = pair.document.transaction("Mirror");
    transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: pair.plate,
            plane: PlaneReference::Datum(axis),
            keep_original: false,
            mirrored: Vec::new(),
        }),
    );

    assert_eq!(
        pair.document.apply(transaction.finish()),
        Err(EditError::NotAPlane("Axis 1".to_owned()))
    );
}

#[test]
fn a_mirror_changes_its_body_without_hiding_the_result_and_is_dependent_on_it() {
    let mut pair = pair();
    let mirror = mirrored(
        &mut pair,
        PlaneReference::Principal(PrincipalPlane::Xz),
        true,
    );
    let feature = pair.document.feature(mirror).unwrap();

    assert!(!feature.kind.modifies_body());
    assert!(!feature.makes_body());
    assert_eq!(feature.body(), Some(pair.plate));
    assert_eq!(pair.document.dependents_of(&[pair.plate]), vec![mirror]);
}

struct Featured {
    document: Document,
    plate: FeatureId,
    hole: FeatureId,
    boss: FeatureId,
    mirror: FeatureId,
}

fn hole_of(points: FeatureId, plate: FeatureId, diameter: &str) -> FeatureKind {
    FeatureKind::Hole(Hole {
        sketch: points,
        body: plate,
        diameter: Expression::parse(diameter, &|_| None).unwrap(),
        depth: HoleDepth::ThroughAll,
        style: HoleStyle::Plain,
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
        sizing: HoleSizing::Typed,
        bottom: HoleBottom::Flat,
        thread: TappedThread::default(),
    })
}

fn featured(mirrored: impl FnOnce(&Featured) -> Vec<FeatureId>) -> Featured {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((-30.0, 0.0), (30.0, 10.0))),
    );
    let plate =
        transaction.add_feature("Plate", extrusion(outline, "4 mm", BodyOperation::NewBody));
    let mut points = Sketch::new(top());
    points.add_point(Point2::new(-25.0, 5.0));
    let points = transaction.add_feature("Hole sketch", FeatureKind::from(points));
    let hole = transaction.add_feature("Hole 1", hole_of(points, plate, "2 mm"));
    let boss_outline = transaction.add_feature(
        "Boss outline",
        FeatureKind::from(on_top((-22.0, 2.0), (-18.0, 8.0))),
    );
    let boss = transaction.add_feature(
        "Boss",
        extrusion(boss_outline, "2 mm", BodyOperation::Add(plate)),
    );
    document.apply(transaction.finish()).unwrap();
    let mut featured = Featured {
        document,
        plate,
        hole,
        boss,
        mirror: FeatureId::from_raw(0),
    };
    let mirror = Mirror::new(plate, PlaneReference::Principal(PrincipalPlane::Yz))
        .mirroring(mirrored(&featured));
    let mut transaction = featured.document.transaction("Mirror");
    featured.mirror = transaction.add_feature("Mirror 1", FeatureKind::Mirror(mirror));
    featured.document.apply(transaction.finish()).unwrap();
    featured
}

fn hole_area(diameter: f64) -> f64 {
    std::f64::consts::PI * diameter * diameter / 4.0
}

fn assert_near_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "volume {found} instead of {expected}"
    );
}

#[test]
fn mirroring_features_reflects_their_hole_and_boss_and_follows_edits_to_them() {
    let mut featured = featured(|featured| vec![featured.hole, featured.boss]);
    let mut engine = Recompute::default();
    let plate = 60.0 * 10.0 * 4.0;
    let boss = 4.0 * 6.0 * 2.0;

    let evaluation = evaluate(&featured.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert_near_volume(
        &evaluation,
        featured.plate,
        plate - 2.0 * hole_area(2.0) * 4.0 + 2.0 * boss,
    );
    assert_eq!(evaluation.cuts(featured.mirror).len(), 1);
    let (low, high) = bounds(&evaluation, featured.plate);
    assert!(near(low, [-30.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [30.0, 10.0, 6.0]), "{high:?}");
    let solid = evaluation.body(featured.plate).unwrap();
    let images: BTreeSet<[u32; 2]> = solid
        .faces()
        .filter_map(|(_, face)| face.origin()?.copy())
        .filter(|copy| copy.pattern == featured.mirror.raw())
        .map(|copy| copy.index)
        .collect();
    assert_eq!(images, BTreeSet::from([MIRROR_IMAGE]));
    let used = featured
        .document
        .feature(featured.mirror)
        .unwrap()
        .kind
        .features();
    assert!(used.contains(&featured.hole) && used.contains(&featured.boss));

    let points = featured
        .document
        .feature(featured.hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .sketch;
    featured
        .document
        .apply(Transaction::single(
            "Widen",
            Edit::SetFeatureKind {
                id: featured.hole,
                kind: hole_of(points, featured.plate, "4 mm"),
            },
        ))
        .unwrap();
    let widened = evaluate(&featured.document, &mut engine);

    assert_eq!(widened.failed_count(), 0);
    assert_near_volume(
        &widened,
        featured.plate,
        plate - 2.0 * hole_area(4.0) * 4.0 + 2.0 * boss,
    );
}

#[test]
fn a_mirrored_feature_that_made_the_body_fails_the_mirror_naming_it() {
    let featured = featured(|featured| vec![featured.plate]);
    let evaluation = evaluate(&featured.document, &mut Recompute::default());

    let error = failure(&evaluation, featured.mirror);
    assert_eq!(
        error.reason,
        "Plate neither adds to nor removes from a body, so it cannot be mirrored."
    );
    assert_eq!(
        error.remedy,
        "Leave it out of the mirror, or mirror the whole body."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(featured.plate)));
    assert_eq!(evaluation.failed_count(), 1);
}
