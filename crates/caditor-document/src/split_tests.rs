use std::f64::consts::PI;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{FaceReference, Surface};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{block, evaluate, volume},
    *,
};

const HALF: f64 = 10.0 * 10.0 * 4.0;

struct Model {
    document: Document,
    plate: FeatureId,
    split: FeatureId,
}

fn split_plate(plane: PlaneReference, flipped: bool) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: plate,
            along: SplitAlong::Plane(plane),
            flipped,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        plate,
        split,
    }
}

fn x_range(evaluation: &Evaluation, body: FeatureId) -> (f64, f64) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min().x, bounds.max().x)
}

fn near(found: (f64, f64), expected: (f64, f64)) -> bool {
    (found.0 - expected.0).abs() < 1e-6 && (found.1 - expected.1).abs() < 1e-6
}

#[test]
fn a_split_keeps_the_side_the_plane_faces_and_makes_the_other_a_body_of_its_own() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.plate, model.split), (model.split, model.split)]
    );
    assert!(near(x_range(&evaluation, model.plate), (0.0, 10.0)));
    assert!(near(x_range(&evaluation, model.split), (-10.0, 0.0)));
    assert!((volume(&evaluation, model.plate) - HALF).abs() < 1e-6 * HALF);
    assert!((volume(&evaluation, model.split) - HALF).abs() < 1e-6 * HALF);
    assert_eq!(
        model.document.feature(model.split).unwrap().bodies(),
        vec![model.plate, model.split]
    );
}

#[test]
fn a_flipped_split_keeps_the_other_side() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), true);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert!(near(x_range(&evaluation, model.plate), (-10.0, 0.0)));
    assert!(near(x_range(&evaluation, model.split), (0.0, 10.0)));
}

#[test]
fn a_plane_that_misses_the_body_fails_the_split_alone_in_words() {
    let model = split_plate(PlaneReference::Principal(PrincipalPlane::Xy), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, model.split);
    assert!(
        error
            .reason
            .contains("does not pass through the body of Plate")
    );
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 10.0)));
    assert!(evaluation.body(model.split).is_none());
}

#[test]
fn the_split_off_body_takes_later_features_of_its_own() {
    let mut model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);
    let mut transaction = model.document.transaction("Move");
    let moved = transaction.add_feature(
        "Move 1",
        FeatureKind::Move(Move {
            about: TurnCentre::Origin,
            frame: None,
            body: model.split,
            offset: [
                Expression::parse_stored("-5 mm").unwrap(),
                Expression::parse_stored("0 mm").unwrap(),
                Expression::parse_stored("0 mm").unwrap(),
            ],
            turn: [
                Expression::parse_stored("0 deg").unwrap(),
                Expression::parse_stored("0 deg").unwrap(),
                Expression::parse_stored("0 deg").unwrap(),
            ],
            copy: false,
        }),
    );
    model.document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(x_range(&evaluation, model.split), (-15.0, -5.0)));
    assert!(near(x_range(&evaluation, model.plate), (0.0, 10.0)));
    assert_eq!(
        evaluation.bodies().collect::<Vec<_>>(),
        vec![(model.plate, model.split), (model.split, moved)]
    );
}

#[test]
fn a_failed_split_keeps_its_last_split_off_body_shown_as_stale() {
    let mut model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);
    let mut engine = Recompute::default();
    evaluate(&model.document, &mut engine);
    let mut transaction = model.document.transaction("Move the plane");
    transaction.edit(Edit::SetFeatureKind {
        id: model.split,
        kind: FeatureKind::Split(Split {
            body: model.plate,
            along: SplitAlong::Plane(PlaneReference::Principal(PrincipalPlane::Xy)),
            flipped: false,
        }),
    });
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document, &mut engine);

    assert_eq!(evaluation.failed_count(), 1);
    assert!(evaluation.is_stale(model.split));
    assert!(near(x_range(&evaluation, model.split), (-10.0, 0.0)));
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 10.0)));
}

fn split_plate_along(sketch: Sketch, flipped: bool) -> (Model, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let curve = transaction.add_feature("Curve", FeatureKind::from(sketch));
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: plate,
            along: SplitAlong::Sketch(curve),
            flipped,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (
        Model {
            document,
            plate,
            split,
        },
        curve,
    )
}

fn line_across() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(2.0, -3.0), Point2::new(2.0, 3.0));
    sketch
}

#[test]
fn a_sketch_line_splits_along_itself_carried_on_past_its_ends_keeping_its_left_side() {
    let (model, _) = split_plate_along(line_across(), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 2.0)));
    assert!(near(x_range(&evaluation, model.split), (2.0, 10.0)));
    assert!((volume(&evaluation, model.plate) - 480.0).abs() < 1e-6 * 480.0);
    assert!((volume(&evaluation, model.split) - 320.0).abs() < 1e-6 * 320.0);
}

#[test]
fn a_flipped_sketch_split_keeps_the_right_side() {
    let (model, _) = split_plate_along(line_across(), true);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(x_range(&evaluation, model.plate), (2.0, 10.0)));
    assert!(near(x_range(&evaluation, model.split), (-10.0, 2.0)));
}

#[test]
fn a_chain_of_a_line_and_an_arc_splits_along_the_curve() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_arc(
        Point2::new(0.0, -12.0),
        Point2::new(14.0, -12.0),
        Point2::new(-14.0, -12.0),
    );
    sketch.add_line(Point2::new(-14.0, -12.0), Point2::new(-14.0, -20.0));
    let (model, _) = split_plate_along(sketch, false);
    let reach: f64 = 10.0;
    let radius: f64 = 14.0;
    let below = reach * (radius * radius - reach * reach).sqrt()
        + radius * radius * (reach / radius).asin()
        - 7.0 * 2.0 * reach;

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    let kept = volume(&evaluation, model.plate);
    let apart = volume(&evaluation, model.split);
    assert!((kept - below * 4.0).abs() < 1e-3 * 800.0);
    assert!((kept + apart - 800.0).abs() < 1e-3 * 800.0);
}

#[test]
fn a_sketch_of_two_separate_curves_fails_the_split_in_words_with_the_fix_on_the_sketch() {
    let mut sketch = line_across();
    sketch.add_line(Point2::new(-2.0, -3.0), Point2::new(-2.0, 3.0));
    let (model, curve) = split_plate_along(sketch, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, model.split);
    assert!(
        error
            .reason
            .contains("The curves of Curve do not form one chain")
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(curve)));
    assert!(near(x_range(&evaluation, model.plate), (-10.0, 10.0)));
}

#[test]
fn a_closed_sketch_cannot_split_the_body() {
    let (model, _) = split_plate_along(combine_tests::rectangle((-1.0, -1.0), (1.0, 1.0)), false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (_, error) = evaluation.failures().next().unwrap();
    assert!(error.reason.contains("close on themselves"));
}

#[test]
fn a_curve_beside_the_body_fails_the_split_alone() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(-30.0, 20.0), Point2::new(30.0, 20.0));
    let (model, _) = split_plate_along(sketch, false);

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, model.split);
    assert!(error.reason.contains("does not cross the body of Plate"));
}

#[test]
fn a_body_splits_along_another_body_keeping_what_lies_outside_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let tool = block(&mut transaction, "Tool", (5.0, -8.0), (15.0, 8.0), "6 mm");
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: plate,
            along: SplitAlong::Body(tool),
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(evaluation.failed_count(), 0);
    assert!(near(x_range(&evaluation, plate), (-10.0, 5.0)));
    assert!(near(x_range(&evaluation, split), (5.0, 10.0)));
    assert!(near(x_range(&evaluation, tool), (5.0, 15.0)));
    assert!(
        document
            .feature(split)
            .unwrap()
            .kind
            .bodies_used()
            .contains(&tool)
    );
}

#[test]
fn a_body_cannot_be_split_along_itself() {
    let mut model = split_plate(PlaneReference::Principal(PrincipalPlane::Yz), false);
    let mut transaction = model.document.transaction("Along itself");
    transaction.edit(Edit::SetFeatureKind {
        id: model.split,
        kind: FeatureKind::Split(Split {
            body: model.plate,
            along: SplitAlong::Body(model.plate),
            flipped: false,
        }),
    });
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document, &mut Recompute::default());

    let (_, error) = evaluation.failures().next().unwrap();
    assert!(error.reason.contains("cannot be split along itself"));
}

fn round_tool(shape: PrimitiveShape, anchor: PrimitiveAnchor, reversed: bool) -> Primitive {
    Primitive {
        shape,
        plane: PlaneReference::Principal(PrincipalPlane::Xy),
        at: [
            Expression::parse_stored("0 mm").unwrap(),
            Expression::parse_stored("0 mm").unwrap(),
        ],
        anchor,
        reversed,
        operation: BodyOperation::NewBody,
    }
}

fn split_plate_along_surface(
    tool: Primitive,
    curved: fn(&Surface) -> bool,
    flipped: bool,
) -> (Document, FeatureId, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let round = transaction.add_feature("Round", FeatureKind::Primitive(tool));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let solid = evaluation.body(round).unwrap();
    let (id, _) = solid
        .faces()
        .find(|(_, face)| curved(face.surface()))
        .unwrap();
    let face = FaceAttachment {
        body: round,
        face: FaceReference::capture(solid, id).unwrap(),
    };
    let mut transaction = document.transaction("Split");
    let split = transaction.add_feature(
        "Split 1",
        FeatureKind::Split(Split {
            body: plate,
            along: SplitAlong::Surface(face),
            flipped,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, plate, split)
}

fn near_volume(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 2e-3 * expected
}

fn rod() -> Primitive {
    round_tool(
        PrimitiveShape::Cylinder {
            diameter: Expression::parse_stored("6 mm").unwrap(),
            height: Expression::parse_stored("2 mm").unwrap(),
        },
        PrimitiveAnchor::BaseCentre,
        true,
    )
}

#[test]
fn a_cylindrical_face_of_another_body_splits_along_its_whole_cylinder_keeping_the_side_it_faces() {
    let (document, plate, split) = split_plate_along_surface(
        rod(),
        |surface| matches!(surface, Surface::Cylinder(_)),
        false,
    );

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    let inside = PI * 9.0 * 4.0;
    assert!(near_volume(volume(&evaluation, plate), 800.0 - inside));
    assert!(near_volume(volume(&evaluation, split), inside));
    let kind = &document.feature(split).unwrap().kind;
    assert!(kind.bodies_used().len() == 2);

    let (document, plate, split) = split_plate_along_surface(
        rod(),
        |surface| matches!(surface, Surface::Cylinder(_)),
        true,
    );

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert!(near_volume(volume(&evaluation, plate), inside));
    assert!(near_volume(volume(&evaluation, split), 800.0 - inside));
}

#[test]
fn a_spherical_and_a_conical_face_split_along_their_whole_surfaces() {
    let ball = round_tool(
        PrimitiveShape::Sphere {
            diameter: Expression::parse_stored("8 mm").unwrap(),
        },
        PrimitiveAnchor::Centre,
        false,
    );
    let (document, plate, split) =
        split_plate_along_surface(ball, |surface| matches!(surface, Surface::Sphere(_)), true);

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    let cap = 2.0 / 3.0 * PI * 64.0;
    assert!(near_volume(volume(&evaluation, plate), cap));
    assert!(near_volume(volume(&evaluation, split), 800.0 - cap));

    let cone = round_tool(
        PrimitiveShape::Cone {
            bottom: Expression::parse_stored("4 mm").unwrap(),
            top: Expression::parse_stored("0 mm").unwrap(),
            height: Expression::parse_stored("4 mm").unwrap(),
        },
        PrimitiveAnchor::BaseCentre,
        true,
    );
    let (document, plate, split) =
        split_plate_along_surface(cone, |surface| matches!(surface, Surface::Cone(_)), false);

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    let frustum = PI * 4.0 / 3.0 * (16.0 + 8.0 + 4.0);
    assert!(near_volume(volume(&evaluation, plate), 800.0 - frustum));
    assert!(near_volume(volume(&evaluation, split), frustum));
}

#[test]
fn a_surface_that_misses_the_body_fails_the_split_alone_in_words() {
    let far = Primitive {
        at: [
            Expression::parse_stored("40 mm").unwrap(),
            Expression::parse_stored("0 mm").unwrap(),
        ],
        ..rod()
    };
    let (document, plate, split) = split_plate_along_surface(
        far,
        |surface| matches!(surface, Surface::Cylinder(_)),
        false,
    );

    let evaluation = evaluate(&document, &mut Recompute::default());

    let (failed, error) = evaluation.failures().next().unwrap();
    assert_eq!(failed, split);
    assert!(
        error.reason.starts_with("The surface of Round wall"),
        "{}",
        error.reason
    );
    assert!(evaluation.body(plate).is_some());
}

#[test]
fn a_split_face_divides_faces_along_the_surface_of_a_curved_face() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = block(
        &mut transaction,
        "Plate",
        (-10.0, -5.0),
        (10.0, 5.0),
        "4 mm",
    );
    let round = transaction.add_feature("Round", FeatureKind::Primitive(rod()));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let capture = |body: FeatureId, wanted: &dyn Fn(&Surface, f64) -> bool| {
        let solid = evaluation.body(body).unwrap();
        let (id, _) = solid
            .faces()
            .find(|(_, face)| wanted(face.surface(), face.sense().sign()))
            .unwrap();
        FaceReference::capture(solid, id).unwrap()
    };
    let wall = capture(round, &|surface, _| matches!(surface, Surface::Cylinder(_)));
    let top = capture(plate, &|surface, sign| match surface {
        Surface::Plane(plane) => plane.frame().normal().z * sign > 0.5,
        _ => false,
    });
    let faces_before = evaluation.body(plate).unwrap().faces().count();
    let mut transaction = document.transaction("Split face");
    let split = transaction.add_feature(
        "Split face 1",
        FeatureKind::SplitFace(SplitFace {
            body: plate,
            faces: vec![top],
            along: SplitAlong::Surface(FaceAttachment {
                body: round,
                face: wall,
            }),
            carry: SplitCarry::Square,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.failures().next()
    );
    assert_eq!(
        evaluation.body(plate).unwrap().faces().count(),
        faces_before + 1
    );
    assert!(near_volume(volume(&evaluation, plate), 800.0));
    assert!(
        document
            .feature(split)
            .unwrap()
            .kind
            .bodies_used()
            .contains(&round)
    );
}
