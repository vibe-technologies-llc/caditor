use std::f64::consts::PI;

use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceId, RegionReference, SamplingTolerance, Solid, Surface};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::*;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
    add_rectangle(&mut sketch, min, max);
    sketch
}

fn add_rectangle(sketch: &mut Sketch, min: (f64, f64), max: (f64, f64)) {
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
}

fn circle(plane: Plane, center: (f64, f64), radius: f64) -> Sketch {
    let mut sketch = Sketch::new(plane);
    sketch.add_circle(Point2::new(center.0, center.1), radius);
    sketch
}

fn at(height: f64) -> Plane {
    Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap()
}

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction(format!("Add {name}"));
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn extrusion(sketch: FeatureId, extent: ExtrudeExtent, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent,
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
    }))
}

fn one_side(end: ExtrudeEnd, reversed: bool) -> ExtrudeExtent {
    ExtrudeExtent::OneSide { end, reversed }
}

fn set(document: &mut Document, parameter: ParameterId, text: &str) {
    let expression = document.parse(text).unwrap();
    document
        .apply(Transaction::single(
            "Edit",
            Edit::SetParameterExpression {
                id: parameter,
                expression,
            },
        ))
        .unwrap();
}

struct Plate {
    document: Document,
    thickness: ParameterId,
    plate: FeatureId,
}

fn plate_at(height: f64) -> Plate {
    let mut document = Document::default();
    let mut transaction = document.transaction("Plate");
    let thickness = transaction.add_parameter("thickness", transaction.parse("10 mm").unwrap());
    let outline = transaction.add_feature(
        "Plate sketch",
        FeatureKind::from(rectangle(at(height), (0.0, 0.0), (40.0, 40.0))),
    );
    let plate = transaction.add_feature(
        "Plate",
        extrusion(
            outline,
            ExtrudeExtent::one_side(Expression::Parameter(thickness), false),
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();
    Plate {
        document,
        thickness,
        plate,
    }
}

fn flat_face(solid: &Solid, normal: Vector3, height: f64) -> FaceId {
    solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => {
                plane.frame().normal() * face.sense().sign() == normal
                    && (plane.frame().origin().z - height).abs() < 1e-9
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .unwrap()
}

fn face_reference(
    document: &Document,
    body: FeatureId,
    normal: Vector3,
    height: f64,
) -> PlaneReference {
    let evaluation = evaluate(document, &mut Recompute::default());
    let solid = evaluation.body(body).unwrap();
    let (attachment, _) =
        FaceAttachment::capture(body, solid, flat_face(solid, normal, height)).unwrap();
    PlaneReference::Face(attachment)
}

#[test]
fn through_all_cuts_through_the_whole_body_whatever_its_thickness() {
    let mut model = plate_at(0.0);
    let hole = add(
        &mut model.document,
        "Hole sketch",
        FeatureKind::from(circle(Plane::XY, (20.0, 20.0), 5.0)),
    );
    add(
        &mut model.document,
        "Hole",
        extrusion(
            hole,
            one_side(ExtrudeEnd::ThroughAll, false),
            BodyOperation::Remove(model.plate),
        ),
    );
    let mut engine = Recompute::default();

    let thin = evaluate(&model.document, &mut engine);
    set(&mut model.document, model.thickness, "25 mm");
    let thick = evaluate(&model.document, &mut engine);

    assert_eq!(thin.failed_count(), 0);
    assert!((volume(&thin, model.plate) - (16_000.0 - 250.0 * PI)).abs() < 1.0);
    assert_eq!(thick.failed_count(), 0);
    assert!((volume(&thick, model.plate) - (40_000.0 - 625.0 * PI)).abs() < 2.0);
}

#[test]
fn through_all_both_ways_cuts_from_a_sketch_inside_the_body() {
    let mut model = plate_at(0.0);
    let slot = add(
        &mut model.document,
        "Slot sketch",
        FeatureKind::from(rectangle(at(4.0), (10.0, 10.0), (20.0, 15.0))),
    );
    add(
        &mut model.document,
        "Slot",
        extrusion(
            slot,
            ExtrudeExtent::TwoSides {
                forward: ExtrudeEnd::ThroughAll,
                backward: ExtrudeEnd::ThroughAll,
            },
            BodyOperation::Remove(model.plate),
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.plate) - (16_000.0 - 500.0)).abs() < 1e-6);
}

#[test]
fn through_all_only_cuts_and_needs_something_of_the_body_ahead() {
    let mut model = plate_at(0.0);
    let hole = add(
        &mut model.document,
        "Hole sketch",
        FeatureKind::from(circle(Plane::XY, (20.0, 20.0), 5.0)),
    );
    let adding = add(
        &mut model.document,
        "Adding",
        extrusion(
            hole,
            one_side(ExtrudeEnd::ThroughAll, false),
            BodyOperation::Add(model.plate),
        ),
    );
    let backwards = add(
        &mut model.document,
        "Backwards",
        extrusion(
            hole,
            one_side(ExtrudeEnd::ThroughAll, true),
            BodyOperation::Remove(model.plate),
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let refused = failure(&evaluation, adding);
    assert_eq!(
        refused.reason,
        "Through all only cuts into a body or intersects with it, so it cannot add material."
    );
    assert_eq!(refused.fix, Some(FixTarget::Feature(adding)));
    let behind = failure(&evaluation, backwards);
    assert_eq!(
        behind.reason,
        "No part of the body of Plate lies ahead of the sketch, so there is nothing to go through."
    );
    assert_eq!(
        behind.remedy,
        "Turn Reversed on or off to go the other way, or enter a distance."
    );
    assert!((volume(&evaluation, model.plate) - 16_000.0).abs() < 1e-6);
}

#[test]
fn up_to_next_fills_the_gap_up_to_the_body_and_follows_it() {
    let mut model = plate_at(10.0);
    let boss = add(
        &mut model.document,
        "Boss sketch",
        FeatureKind::from(rectangle(Plane::XY, (5.0, 5.0), (15.0, 15.0))),
    );
    add(
        &mut model.document,
        "Boss",
        extrusion(
            boss,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Add(model.plate),
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.plate) - (16_000.0 + 1_000.0)).abs() < 1e-6);
}

#[test]
fn up_to_next_cuts_until_the_profile_comes_out_of_the_body() {
    let mut model = plate_at(0.0);
    let pocket = add(
        &mut model.document,
        "Pocket sketch",
        FeatureKind::from(rectangle(at(10.0), (5.0, 5.0), (15.0, 15.0))),
    );
    add(
        &mut model.document,
        "Pocket",
        extrusion(
            pocket,
            one_side(ExtrudeEnd::up_to_next(), true),
            BodyOperation::Remove(model.plate),
        ),
    );
    let mut engine = Recompute::default();

    let evaluation = evaluate(&model.document, &mut engine);
    set(&mut model.document, model.thickness, "30 mm");
    let thicker = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.plate) - (16_000.0 - 1_000.0)).abs() < 1e-6);
    assert_eq!(thicker.failed_count(), 0);
    assert!((volume(&thicker, model.plate) - (48_000.0 - 1_000.0)).abs() < 1e-6);
}

#[test]
fn up_to_next_that_misses_meets_several_faces_or_changes_nothing_is_refused() {
    let mut model = plate_at(10.0);
    let below = add(
        &mut model.document,
        "Below sketch",
        FeatureKind::from(rectangle(Plane::XY, (30.0, 5.0), (50.0, 15.0))),
    );
    let beside = add(
        &mut model.document,
        "Beside",
        extrusion(
            below,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Add(model.plate),
        ),
    );
    let separate = add(
        &mut model.document,
        "Separate",
        extrusion(
            below,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::NewBody,
        ),
    );
    let under = add(
        &mut model.document,
        "Under sketch",
        FeatureKind::from(rectangle(Plane::XY, (5.0, 5.0), (15.0, 15.0))),
    );
    let removing = add(
        &mut model.document,
        "Removing",
        extrusion(
            under,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Remove(model.plate),
        ),
    );
    let inner = add(
        &mut model.document,
        "Inner sketch",
        FeatureKind::from(rectangle(at(15.0), (5.0, 5.0), (15.0, 15.0))),
    );
    let inside = add(
        &mut model.document,
        "Inside",
        extrusion(
            inner,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Add(model.plate),
        ),
    );
    let step = add(
        &mut model.document,
        "Step sketch",
        FeatureKind::from(rectangle(at(5.0), (0.0, 0.0), (20.0, 40.0))),
    );
    add(
        &mut model.document,
        "Step",
        extrusion(
            step,
            ExtrudeExtent::one_side(millimetres(5.0), false),
            BodyOperation::Add(model.plate),
        ),
    );
    let across = add(
        &mut model.document,
        "Across sketch",
        FeatureKind::from(rectangle(Plane::XY, (15.0, 5.0), (25.0, 15.0))),
    );
    let stepped = add(
        &mut model.document,
        "Stepped",
        extrusion(
            across,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Add(model.plate),
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(
        failure(&evaluation, beside).reason,
        "Part of the profile of Below sketch passes beside the body of Plate, so there is no one \
         next face to stop at."
    );
    assert_eq!(
        failure(&evaluation, separate).reason,
        "Up to next stops at the body this extrusion changes, and it makes a new body instead."
    );
    assert_eq!(
        failure(&evaluation, removing).reason,
        "The profile of Under sketch first meets the body of Plate where it enters it, so \
         extruding up to that face removes nothing."
    );
    assert_eq!(
        failure(&evaluation, inside).reason,
        "The profile of Inner sketch starts inside the body of Plate, so extruding it up to the \
         next face adds nothing."
    );
    assert!(matches!(
        evaluation.feature(stepped).unwrap().state,
        FeatureState::UpToDate
    ));
    assert!((volume(&evaluation, model.plate) - (16_000.0 + 4_000.0 + 750.0)).abs() < 1e-6);
}

fn rod() -> FeatureKind {
    let across = Plane::from_frame(Point3::new(-10.0, 0.0, 20.0), Vector3::X, Vector3::Y).unwrap();
    FeatureKind::from(circle(across, (0.0, 0.0), 5.0))
}

fn under_the_rod(width: f64) -> f64 {
    let half = width / 2.0;
    2.0 * (half / 2.0 * (25.0 - half * half).sqrt() + 12.5 * (half / 5.0).asin())
}

#[test]
fn up_to_a_curved_next_face_follows_the_face() {
    let mut document = Document::default();
    let rod_sketch = add(&mut document, "Rod sketch", rod());
    let rod = add(
        &mut document,
        "Rod",
        extrusion(
            rod_sketch,
            ExtrudeExtent::one_side(millimetres(20.0), false),
            BodyOperation::NewBody,
        ),
    );
    let strip = add(
        &mut document,
        "Strip sketch",
        FeatureKind::from(rectangle(Plane::XY, (1.0, -1.0), (2.0, 1.0))),
    );
    let stand = add(
        &mut document,
        "Stand",
        extrusion(
            strip,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Add(rod),
        ),
    );
    let groove = add(
        &mut document,
        "Groove sketch",
        FeatureKind::from(rectangle(at(20.0), (-6.0, -1.0), (-4.0, 1.0))),
    );
    let groove_cut = add(
        &mut document,
        "Groove",
        extrusion(
            groove,
            one_side(ExtrudeEnd::up_to_next(), false),
            BodyOperation::Remove(rod),
        ),
    );
    let offset = add(
        &mut document,
        "Offset",
        extrusion(
            strip,
            one_side(
                ExtrudeEnd::up_to_next().with_offset(Some(millimetres(1.0))),
                false,
            ),
            BodyOperation::Add(rod),
        ),
    );
    let both = add(
        &mut document,
        "Both ways",
        extrusion(
            strip,
            ExtrudeExtent::TwoSides {
                forward: ExtrudeEnd::up_to_next(),
                backward: ExtrudeEnd::Distance(millimetres(2.0)),
            },
            BodyOperation::Add(rod),
        ),
    );

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert!(matches!(
        evaluation.feature(stand).unwrap().state,
        FeatureState::UpToDate
    ));
    assert!(matches!(
        evaluation.feature(groove_cut).unwrap().state,
        FeatureState::UpToDate
    ));
    let added = 40.0 - under_the_rod(2.0);
    let removed = 2.0 * under_the_rod(2.0);
    let expected = 500.0 * PI + added - removed;
    let found = volume(&evaluation, rod);
    assert!(
        (found - expected).abs() < 2e-3 * expected,
        "{found} {expected}"
    );
    assert_eq!(
        failure(&evaluation, offset).remedy,
        "Clear the end offset, or use Up to face with a flat face or plane."
    );
    assert_eq!(
        failure(&evaluation, both).reason,
        "The profile of Strip sketch first meets curved or several faces, and an extrusion to two \
         sides can only stop at one flat face."
    );
}

#[test]
fn up_to_face_ends_on_the_face_and_follows_it_when_the_body_changes() {
    let mut model = plate_at(0.0);
    let top = face_reference(&model.document, model.plate, Vector3::Z, 10.0);
    let tower = add(
        &mut model.document,
        "Tower sketch",
        FeatureKind::from(rectangle(at(30.0), (5.0, 5.0), (15.0, 15.0))),
    );
    add(
        &mut model.document,
        "Tower",
        extrusion(
            tower,
            one_side(ExtrudeEnd::up_to_face(top), true),
            BodyOperation::Add(model.plate),
        ),
    );
    let mut engine = Recompute::default();

    let evaluation = evaluate(&model.document, &mut engine);
    set(&mut model.document, model.thickness, "15 mm");
    let thicker = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.plate) - (16_000.0 + 2_000.0)).abs() < 1e-6);
    assert_eq!(thicker.failed_count(), 0);
    assert!((volume(&thicker, model.plate) - (24_000.0 + 1_500.0)).abs() < 1e-6);
}

fn region_at(sketch: &Sketch, x: f64) -> RegionReference {
    let region = sketch_regions(sketch)
        .unwrap()
        .into_iter()
        .find(|region| {
            let bounds = region.bounds().unwrap();
            bounds.min().x <= x && x <= bounds.max().x
        })
        .unwrap();
    RegionReference::capture(&region, region.anchor())
}

#[test]
fn up_to_a_face_that_an_upstream_edit_removes_fails_alone_and_keeps_its_last_shape() {
    let mut model = plate_at(0.0);
    let mut lugs = rectangle(at(10.0), (5.0, 5.0), (15.0, 15.0));
    add_rectangle(&mut lugs, (25.0, 25.0), (35.0, 35.0));
    let (first, second) = (region_at(&lugs, 10.0), region_at(&lugs, 30.0));
    let lug_sketch = add(&mut model.document, "Lug sketch", FeatureKind::from(lugs));
    let lug_kind = |region: RegionReference| {
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: lug_sketch,
            regions: RegionChoice::Chosen(vec![region]),
            extent: ExtrudeExtent::one_side(millimetres(5.0), false),
            operation: BodyOperation::Add(model.plate),
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
        }))
    };
    let lug = add(&mut model.document, "Lug", lug_kind(first));
    let lug_top = face_reference(&model.document, model.plate, Vector3::Z, 15.0);
    let post = add(
        &mut model.document,
        "Post sketch",
        FeatureKind::from(rectangle(at(40.0), (8.0, 8.0), (12.0, 12.0))),
    );
    let tower = add(
        &mut model.document,
        "Post",
        extrusion(
            post,
            one_side(ExtrudeEnd::up_to_face(lug_top), true),
            BodyOperation::NewBody,
        ),
    );
    let block = add(
        &mut model.document,
        "Block sketch",
        FeatureKind::from(rectangle(at(50.0), (0.0, 0.0), (2.0, 2.0))),
    );
    let other = add(
        &mut model.document,
        "Block",
        extrusion(
            block,
            ExtrudeExtent::one_side(millimetres(2.0), false),
            BodyOperation::NewBody,
        ),
    );
    let mut engine = Recompute::default();
    let before = evaluate(&model.document, &mut engine);

    model
        .document
        .apply(Transaction::single(
            "Move the lug",
            Edit::SetFeatureKind {
                id: lug,
                kind: lug_kind(second),
            },
        ))
        .unwrap();
    let after = evaluate(&model.document, &mut engine);

    assert_eq!(before.failed_count(), 0);
    assert!((volume(&before, tower) - 16.0 * 25.0).abs() < 1e-6);
    let error = failure(&after, tower);
    assert_eq!(
        error.reason,
        "The face this extrusion runs up to is no longer part of the body of Plate."
    );
    assert_eq!(
        error.remedy,
        "Select a flat face or plane and use it for this end, or choose another end."
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(tower)));
    assert!(after.feature(tower).unwrap().result.is_some());
    assert!((volume(&after, tower) - 16.0 * 25.0).abs() < 1e-6);
    assert_eq!(after.feature(other).unwrap().state, FeatureState::UpToDate);
    assert_eq!(after.failed_count(), 1);
}

fn datum_plane(base: PlaneReference, rotation: Option<PlaneRotation>, offset: f64) -> FeatureKind {
    FeatureKind::Datum(Datum::Plane(DatumPlane {
        base,
        rotation,
        offset: millimetres(offset),
    }))
}

#[test]
fn up_to_a_face_behind_across_or_along_the_direction_is_refused() {
    let mut model = plate_at(0.0);
    let top = face_reference(&model.document, model.plate, Vector3::Z, 10.0);
    let high = add(
        &mut model.document,
        "High sketch",
        FeatureKind::from(rectangle(at(30.0), (5.0, 5.0), (15.0, 15.0))),
    );
    let upwards = add(
        &mut model.document,
        "Upwards",
        extrusion(
            high,
            one_side(ExtrudeEnd::up_to_face(top.clone()), false),
            BodyOperation::Add(model.plate),
        ),
    );
    let forward_behind = add(
        &mut model.document,
        "Forward behind",
        extrusion(
            high,
            ExtrudeExtent::TwoSides {
                forward: ExtrudeEnd::up_to_face(top),
                backward: ExtrudeEnd::Distance(millimetres(1.0)),
            },
            BodyOperation::NewBody,
        ),
    );
    let slope = add(
        &mut model.document,
        "Slope",
        datum_plane(
            PlaneReference::Principal(PrincipalPlane::Xy),
            Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::Y),
                angle: degrees(45.0),
            }),
            0.0,
        ),
    );
    let centred = add(
        &mut model.document,
        "Centred sketch",
        FeatureKind::from(rectangle(Plane::XY, (-10.0, -10.0), (10.0, 10.0))),
    );
    let across = add(
        &mut model.document,
        "Across",
        extrusion(
            centred,
            one_side(ExtrudeEnd::up_to_face(PlaneReference::Datum(slope)), false),
            BodyOperation::NewBody,
        ),
    );
    let along = add(
        &mut model.document,
        "Along",
        extrusion(
            centred,
            one_side(
                ExtrudeEnd::up_to_face(PlaneReference::Principal(PrincipalPlane::Xz)),
                false,
            ),
            BodyOperation::NewBody,
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let behind = failure(&evaluation, upwards);
    assert_eq!(
        behind.reason,
        "Plate end face does not lie ahead of the sketch."
    );
    assert_eq!(
        behind.remedy,
        "Turn Reversed on or off to go the other way, or choose a face or plane beyond the sketch."
    );
    assert_eq!(
        failure(&evaluation, forward_behind).reason,
        "Plate end face does not lie on the forward side of the sketch."
    );
    assert_eq!(
        failure(&evaluation, across).reason,
        "Slope cuts across the profile of Centred sketch, so the extrusion would end before it \
         starts in places."
    );
    assert_eq!(
        failure(&evaluation, along).reason,
        "The XZ plane runs along the direction of the extrusion, so the extrusion never reaches \
         it."
    );
}

#[test]
fn up_to_a_tilted_datum_plane_ends_on_it() {
    let mut document = Document::default();
    let slope = add(
        &mut document,
        "Slope",
        datum_plane(
            PlaneReference::Principal(PrincipalPlane::Xy),
            Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::X),
                angle: degrees(30.0),
            }),
            20.0,
        ),
    );
    let base = add(
        &mut document,
        "Base sketch",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 10.0))),
    );
    let wedge = add(
        &mut document,
        "Wedge",
        extrusion(
            base,
            one_side(ExtrudeEnd::up_to_face(PlaneReference::Datum(slope)), false),
            BodyOperation::NewBody,
        ),
    );

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let plane = evaluation
        .feature(slope)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::datum)
        .and_then(DatumResult::plane)
        .unwrap();
    let centre = Point3::new(5.0, 5.0, 0.0);
    let height = plane.signed_distance(centre) / -plane.normal().z;
    assert!((volume(&evaluation, wedge) - 100.0 * height).abs() < 1e-6);
    let solid = evaluation.body(wedge).unwrap();
    let top = solid
        .faces()
        .find(|(_, face)| {
            face.origin()
                == Some(caditor_kernel::FaceOrigin::EndCap {
                    feature: wedge.raw(),
                })
        })
        .map(|(_, face)| face.surface().clone())
        .unwrap();
    let Surface::Plane(top) = top else {
        panic!("the end is flat");
    };
    assert!(plane.signed_distance(top.frame().origin()).abs() < 1e-9);
    assert!(top.frame().normal().cross(plane.normal()).length() < 1e-9);
}

#[test]
fn each_side_of_a_two_sided_extrusion_takes_its_own_end() {
    let mut model = plate_at(0.0);
    let middle = add(
        &mut model.document,
        "Middle sketch",
        FeatureKind::from(rectangle(at(20.0), (5.0, 5.0), (15.0, 15.0))),
    );
    let column = add(
        &mut model.document,
        "Column",
        extrusion(
            middle,
            ExtrudeExtent::TwoSides {
                forward: ExtrudeEnd::Distance(millimetres(5.0)),
                backward: ExtrudeEnd::up_to_face(PlaneReference::Principal(PrincipalPlane::Xy)),
            },
            BodyOperation::NewBody,
        ),
    );
    let cut = add(
        &mut model.document,
        "Cut sketch",
        FeatureKind::from(rectangle(at(5.0), (30.0, 30.0), (35.0, 35.0))),
    );
    add(
        &mut model.document,
        "Cut",
        extrusion(
            cut,
            ExtrudeExtent::TwoSides {
                forward: ExtrudeEnd::ThroughAll,
                backward: ExtrudeEnd::up_to_next(),
            },
            BodyOperation::Remove(model.plate),
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, column) - 100.0 * 25.0).abs() < 1e-6);
    let reach = evaluation.body(column).unwrap().bounding_box().unwrap();
    assert!(reach.min().z.abs() < 1e-9 && (reach.max().z - 25.0).abs() < 1e-9);
    assert!((volume(&evaluation, model.plate) - (16_000.0 - 250.0)).abs() < 1e-6);
}

#[test]
fn a_new_body_up_to_a_face_of_another_body_is_recomputed_when_that_body_changes() {
    let mut model = plate_at(0.0);
    let top = face_reference(&model.document, model.plate, Vector3::Z, 10.0);
    let tower_sketch = add(
        &mut model.document,
        "Tower sketch",
        FeatureKind::from(rectangle(at(30.0), (5.0, 5.0), (15.0, 15.0))),
    );
    let tower = add(
        &mut model.document,
        "Tower",
        extrusion(
            tower_sketch,
            one_side(ExtrudeEnd::up_to_face(top), true),
            BodyOperation::NewBody,
        ),
    );
    let mut engine = Recompute::default();

    let first = evaluate(&model.document, &mut engine);
    set(&mut model.document, model.thickness, "12 mm");
    let second = evaluate(&model.document, &mut engine);

    assert!(
        model
            .document
            .feature(tower)
            .unwrap()
            .kind
            .bodies_used()
            .contains(&model.plate)
    );
    assert!((volume(&first, tower) - 2_000.0).abs() < 1e-6);
    assert!(second.recomputed().contains(&tower));
    assert!((volume(&second, tower) - 1_800.0).abs() < 1e-6);
}

#[test]
fn a_datum_plane_an_extrusion_runs_up_to_is_one_of_its_features_and_must_stay_above_it() {
    let mut document = Document::default();
    let level = add(
        &mut document,
        "Level",
        datum_plane(PlaneReference::Principal(PrincipalPlane::Xy), None, 20.0),
    );
    let base = add(
        &mut document,
        "Base sketch",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 10.0))),
    );
    let axis = add(
        &mut document,
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::X,
        )))),
    );
    let block = add(
        &mut document,
        "Block",
        extrusion(
            base,
            one_side(ExtrudeEnd::up_to_face(PlaneReference::Datum(level)), false),
            BodyOperation::NewBody,
        ),
    );

    let moved = document.apply(Transaction::single(
        "Move",
        Edit::MoveFeature {
            id: level,
            index: 3,
        },
    ));
    let not_a_plane = document.check(&Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: block,
            kind: extrusion(
                base,
                one_side(ExtrudeEnd::up_to_face(PlaneReference::Datum(axis)), false),
                BodyOperation::NewBody,
            ),
        },
    ));

    assert!(matches!(moved, Err(EditError::BelowDependent { .. })));
    assert_eq!(not_a_plane, Err(EditError::NotAPlane("Axis 1".to_owned())));
    assert!(
        document
            .feature(block)
            .unwrap()
            .kind
            .features()
            .contains(&level)
    );
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert!((volume(&evaluation, block) - 2_000.0).abs() < 1e-6);
}

#[test]
fn a_revolve_turns_by_two_angles_that_together_stay_within_a_turn() {
    let mut document = Document::default();
    let section = add(
        &mut document,
        "Section",
        FeatureKind::from(rectangle(Plane::XZ, (2.0, 0.0), (4.0, 3.0))),
    );
    let revolve = |forward: f64, backward: f64| {
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch: section,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent: RevolveExtent::TwoSides {
                forward: degrees(forward),
                backward: degrees(backward),
            },
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            side: None,
            wall: None,
        }))
    };
    let turned = add(&mut document, "Turned", revolve(90.0, 30.0));
    let too_far = add(&mut document, "Too far", revolve(300.0, 100.0));

    let evaluation = evaluate(&document, &mut Recompute::default());

    let full = PI * (16.0 - 4.0) * 3.0;
    assert!((volume(&evaluation, turned) - full / 3.0).abs() < 1e-3 * full);
    let error = failure(&evaluation, too_far);
    assert_eq!(error.reason, "The two angles add up to more than 360°.");
    assert_eq!(error.remedy, "Enter angles that add up to at most 360°.");
}

#[test]
fn a_hole_drawn_inside_a_chosen_region_cuts_through_the_extrusion() {
    let plate = rectangle(at(0.0), (0.0, 0.0), (10.0, 8.0));
    let chosen = region_at(&plate, 5.0);
    let mut document = Document::default();
    let sketch = add(&mut document, "Plate sketch", FeatureKind::from(plate));
    let block = add(
        &mut document,
        "Block",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::Chosen(vec![chosen]),
            extent: ExtrudeExtent::one_side(millimetres(2.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
        })),
    );
    let mut engine = Recompute::default();
    let before = evaluate(&document, &mut engine);

    let mut transaction = document.transaction("Draw a hole");
    let center = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(4.0, 4.0)));
    transaction.add_sketch_entity(
        sketch,
        Entity::Circle {
            center,
            radius: 2.0,
        },
    );
    document.apply(transaction.finish()).unwrap();
    let after = evaluate(&document, &mut engine);

    assert!((volume(&before, block) - 160.0).abs() < 1e-6);
    assert_eq!(before.feature(block).unwrap().healing, None);
    assert_eq!(after.feature(block).unwrap().state, FeatureState::UpToDate);
    assert!((volume(&after, block) - (160.0 - 8.0 * PI)).abs() < 2e-2);

    let healing = after.feature(block).unwrap().healing.clone().unwrap();
    assert_eq!(
        healing.reason(),
        "After an upstream change, a chosen region of Plate sketch was matched to the most \
         similar geometry."
    );
    document.apply(healing.update(&document).unwrap()).unwrap();
    let updated = evaluate(&document, &mut engine);

    assert_eq!(updated.feature(block).unwrap().healing, None);
    assert!((volume(&updated, block) - volume(&after, block)).abs() < 1e-9);
}

#[test]
fn a_chosen_region_whose_curve_is_deleted_is_left_out_and_said_so() {
    let mut plate = rectangle(at(0.0), (0.0, 0.0), (10.0, 8.0));
    let disc = plate.add_circle(Point2::new(4.0, 4.0), 2.0);
    let regions: Vec<RegionReference> = sketch_regions(&plate)
        .unwrap()
        .iter()
        .map(|region| RegionReference::capture(region, region.anchor()))
        .collect();
    let mut document = Document::default();
    let sketch = add(&mut document, "Plate sketch", FeatureKind::from(plate));
    let block = add(
        &mut document,
        "Block",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::Chosen(regions),
            extent: ExtrudeExtent::one_side(millimetres(2.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
        })),
    );
    let mut engine = Recompute::default();
    evaluate(&document, &mut engine);

    let mut transaction = document.transaction("Delete the disc");
    transaction.remove_sketch_items(sketch, [disc], []);
    document.apply(transaction.finish()).unwrap();
    let after = evaluate(&document, &mut engine);

    let healing = after.feature(block).unwrap().healing.clone().unwrap();
    assert_eq!(
        healing.reason(),
        "After an upstream change, a chosen region of Plate sketch was matched to the most \
         similar geometry, and a chosen region of Plate sketch no longer exists and was left out."
    );
    assert!((volume(&after, block) - 160.0).abs() < 1e-6);
}

#[test]
fn an_offset_stops_an_end_short_of_or_past_the_face_it_reaches() {
    let mut model = plate_at(0.0);
    let top = face_reference(&model.document, model.plate, Vector3::Z, 10.0);
    let pocket = add(
        &mut model.document,
        "Pocket sketch",
        FeatureKind::from(rectangle(at(10.0), (5.0, 5.0), (15.0, 15.0))),
    );
    let short = add(
        &mut model.document,
        "Short pocket",
        extrusion(
            pocket,
            one_side(
                ExtrudeEnd::up_to_next().with_offset(Some(millimetres(-4.0))),
                true,
            ),
            BodyOperation::Remove(model.plate),
        ),
    );
    let tower = add(
        &mut model.document,
        "Tower sketch",
        FeatureKind::from(rectangle(at(30.0), (25.0, 25.0), (35.0, 35.0))),
    );
    let short_tower = add(
        &mut model.document,
        "Tower",
        extrusion(
            tower,
            one_side(
                ExtrudeEnd::up_to_face(top.clone()).with_offset(Some(millimetres(-2.0))),
                true,
            ),
            BodyOperation::NewBody,
        ),
    );
    let past = add(
        &mut model.document,
        "Past",
        extrusion(
            tower,
            one_side(
                ExtrudeEnd::up_to_face(top.clone()).with_offset(Some(millimetres(5.0))),
                true,
            ),
            BodyOperation::NewBody,
        ),
    );
    let behind = add(
        &mut model.document,
        "Behind",
        extrusion(
            tower,
            one_side(
                ExtrudeEnd::up_to_face(top).with_offset(Some(millimetres(-25.0))),
                true,
            ),
            BodyOperation::NewBody,
        ),
    );

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert!((volume(&evaluation, model.plate) - (16_000.0 - 600.0)).abs() < 1e-6);
    assert!(matches!(
        evaluation.feature(short).unwrap().state,
        FeatureState::UpToDate
    ));
    assert!((volume(&evaluation, short_tower) - 1_800.0).abs() < 1e-6);
    assert!((volume(&evaluation, past) - 2_500.0).abs() < 1e-6);
    let refused = failure(&evaluation, behind);
    assert_eq!(
        refused.reason,
        "Stopping 25 mm short of Plate end face would end the extrusion before it starts in \
         places."
    );
    assert_eq!(
        refused.remedy,
        "Enter a smaller end offset, or choose a face or plane farther from the sketch."
    );
    assert_eq!(evaluation.failed_count(), 1);
}
