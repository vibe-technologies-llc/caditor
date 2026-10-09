use std::f64::consts::PI;

use caditor_expression::Expression;
use caditor_geometry::{Point2, Point3, Vector3};
use caditor_kernel::{FaceReference, SamplingTolerance, Solid, Surface};

use crate::*;

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.01).unwrap())
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

fn mm(value: f64) -> Expression {
    Expression::measure(value, caditor_expression::Unit::Millimetre)
}

fn cube(size: f64) -> PrimitiveShape {
    PrimitiveShape::Box {
        length: mm(size),
        width: mm(size),
        height: mm(size),
    }
}

fn primitive(shape: PrimitiveShape) -> Primitive {
    Primitive {
        shape,
        plane: PlaneReference::Principal(PrincipalPlane::Xy),
        at: [mm(0.0), mm(0.0)],
        anchor: PrimitiveAnchor::BaseCentre,
        reversed: false,
        operation: BodyOperation::NewBody,
    }
}

fn single(primitive: Primitive) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let feature = transaction.add_feature("Box 1", FeatureKind::Primitive(primitive));
    document.apply(transaction.finish()).unwrap();
    (document, feature)
}

fn bounds(solid: &Solid) -> (Point3, Point3) {
    let bounds = solid.bounding_box().unwrap();
    (bounds.min(), bounds.max())
}

fn close(first: Point3, second: Point3) -> bool {
    first.distance(second) < 1e-6
}

fn face_facing(solid: &Solid, direction: Vector3) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal() * face.sense().sign() == direction,
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

#[test]
fn a_box_makes_a_body_named_by_its_six_sides() {
    let (document, feature) = single(primitive(PrimitiveShape::Box {
        length: mm(10.0),
        width: mm(6.0),
        height: mm(4.0),
    }));
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);

    let solid = evaluation.body(feature).unwrap();
    let mut names: Vec<String> = solid
        .faces()
        .map(|(_, face)| describe_origin(&document, face.origin()))
        .collect();
    names.sort();

    assert!(document.feature(feature).unwrap().makes_body());
    assert!((volume(&evaluation, feature) - 240.0).abs() < 1e-6);
    assert!(close(bounds(solid).0, Point3::new(-5.0, -3.0, 0.0)));
    assert!(close(bounds(solid).1, Point3::new(5.0, 3.0, 4.0)));
    assert_eq!(
        names,
        [
            "Box 1 back face",
            "Box 1 bottom face",
            "Box 1 front face",
            "Box 1 left face",
            "Box 1 right face",
            "Box 1 top face",
        ]
    );
}

#[test]
fn round_primitives_have_their_volumes_and_named_faces() {
    let cases = [
        (
            PrimitiveShape::Cylinder {
                diameter: mm(4.0),
                height: mm(5.0),
            },
            PI * 4.0 * 5.0,
            3,
        ),
        (
            PrimitiveShape::Sphere { diameter: mm(6.0) },
            4.0 / 3.0 * PI * 27.0,
            1,
        ),
        (
            PrimitiveShape::Torus {
                diameter: mm(10.0),
                tube: mm(2.0),
            },
            2.0 * PI * PI * 5.0 * 1.0,
            1,
        ),
    ];
    for (shape, expected, faces) in cases {
        let (document, feature) = single(primitive(shape));
        let mut engine = Recompute::default();
        let evaluation = evaluate(&document, &mut engine);

        let solid = evaluation.body(feature).unwrap();
        let found = volume(&evaluation, feature);

        assert!(
            (found - expected).abs() < expected * 0.01,
            "{found} {expected}"
        );
        assert_eq!(solid.faces().count(), faces);
        assert!(bounds(solid).0.z.abs() < 1e-6);
    }
}

#[test]
fn the_anchor_and_the_direction_place_the_shape_from_the_chosen_point() {
    let mut placed = primitive(cube(2.0));
    placed.at = [mm(5.0), mm(7.0)];
    placed.anchor = PrimitiveAnchor::Corner;
    let (mut document, feature) = single(placed.clone());
    let mut engine = Recompute::default();

    let evaluation = evaluate(&document, &mut engine);
    let (low, high) = bounds(evaluation.body(feature).unwrap());
    assert!(close(low, Point3::new(5.0, 7.0, 0.0)));
    assert!(close(high, Point3::new(7.0, 9.0, 2.0)));

    placed.anchor = PrimitiveAnchor::Centre;
    placed.reversed = true;
    let transaction = Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Primitive(placed.clone()),
        },
    );
    document.apply(transaction).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let (low, high) = bounds(evaluation.body(feature).unwrap());
    assert!(close(low, Point3::new(4.0, 6.0, -1.0)));
    assert!(close(high, Point3::new(6.0, 8.0, 1.0)));

    placed.anchor = PrimitiveAnchor::BaseCentre;
    placed.plane = PlaneReference::Principal(PrincipalPlane::Xz);
    placed.shape = PrimitiveShape::Sphere { diameter: mm(2.0) };
    placed.reversed = false;
    let transaction = Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Primitive(placed),
        },
    );
    document.apply(transaction).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(feature).unwrap();
    let (low, high) = bounds(solid);
    let plane = PrincipalPlane::Xz.plane();
    let expected = plane.to_world(Point2::new(5.0, 7.0)) + plane.normal();
    assert!(close((low + high) / 2.0, expected));
}

fn on_a_base(
    operation: impl Fn(FeatureId) -> BodyOperation,
    reversed: bool,
) -> (Evaluation, Document, FeatureId, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let base = transaction.add_feature("Base", FeatureKind::Primitive(primitive(cube(10.0))));
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(base).unwrap();
    let top = face_facing(solid, Vector3::Z);
    let plane = face_plane(solid, top.resolve(solid).unwrap()).unwrap();
    let middle = plane.to_local(Point3::new(0.0, 0.0, 10.0));

    let mut transaction = document.transaction("Boss");
    let boss = transaction.add_feature(
        "Boss",
        FeatureKind::Primitive(Primitive {
            plane: PlaneReference::Face(FaceAttachment {
                body: base,
                face: top,
            }),
            at: [mm(middle.x), mm(middle.y)],
            reversed,
            operation: operation(base),
            ..primitive(PrimitiveShape::Cylinder {
                diameter: mm(2.0),
                height: mm(3.0),
            })
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    (evaluation, document, base, boss)
}

#[test]
fn a_primitive_on_a_face_joins_or_cuts_its_body() {
    let (evaluation, document, base, boss) = on_a_base(BodyOperation::Add, false);
    let boss_feature = document.feature(boss).unwrap();
    assert!(!boss_feature.makes_body());
    assert_eq!(boss_feature.body(), Some(base));
    assert!(boss_feature.kind.bodies_used().contains(&base));
    let added = volume(&evaluation, base);
    assert!((added - (1000.0 + PI * 3.0)).abs() < 0.1, "{added}");
    assert!(evaluation.cuts(boss).is_empty());

    let (evaluation, _, base, boss) = on_a_base(BodyOperation::Remove, true);
    let removed = volume(&evaluation, base);
    assert!((removed - (1000.0 - PI * 3.0)).abs() < 0.1, "{removed}");
    assert_eq!(evaluation.cuts(boss).len(), 1);
}

#[test]
fn a_cut_that_takes_the_whole_body_fails_alone_in_words() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let base = transaction.add_feature("Base", FeatureKind::Primitive(primitive(cube(2.0))));
    let cut = transaction.add_feature(
        "Cut",
        FeatureKind::Primitive(Primitive {
            operation: BodyOperation::Remove(base),
            anchor: PrimitiveAnchor::Centre,
            ..primitive(cube(10.0))
        }),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);

    let error = failure(&evaluation, cut);
    assert_eq!(error.reason, "The box removes all of the body of Base.");
    assert!(evaluation.body_result(base).is_some());
}

#[test]
fn sizes_take_parameters_and_bad_sizes_fail_in_words() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let side = transaction.add_parameter("side", transaction.parse("3 mm").unwrap());
    let feature = transaction.add_feature(
        "Box 1",
        FeatureKind::Primitive(primitive(PrimitiveShape::Box {
            length: Expression::Parameter(side),
            width: mm(1.0),
            height: mm(1.0),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    assert!(document.feature(feature).unwrap().uses_parameter(side));
    let evaluation = evaluate(&document, &mut engine);
    assert!((volume(&evaluation, feature) - 3.0).abs() < 1e-6);

    let mut transaction = document.transaction("Zero");
    let zero = transaction.parse("0 mm").unwrap();
    transaction.edit(Edit::SetParameterExpression {
        id: side,
        expression: zero,
    });
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let error = failure(&evaluation, feature);
    assert_eq!(
        error.reason,
        "The length of the box must be more than zero."
    );
    assert_eq!(error.remedy, "Enter a length above zero.");

    let (document, torus) = single(primitive(PrimitiveShape::Torus {
        diameter: mm(4.0),
        tube: mm(4.0),
    }));
    let evaluation = evaluate(&document, &mut Recompute::default());
    assert!(
        failure(&evaluation, torus)
            .reason
            .starts_with("The tube diameter of the torus must be less than its diameter")
    );
}

#[test]
fn a_reference_to_a_side_survives_resizing() {
    let (mut document, feature) = single(primitive(cube(4.0)));
    let mut engine = Recompute::default();
    let evaluation = evaluate(&document, &mut engine);
    let front = face_facing(evaluation.body(feature).unwrap(), Vector3::NEG_Y);

    let transaction = Transaction::single(
        "Resize",
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Primitive(primitive(PrimitiveShape::Box {
                length: mm(9.0),
                width: mm(2.0),
                height: mm(5.0),
            })),
        },
    );
    document.apply(transaction).unwrap();
    let evaluation = evaluate(&document, &mut engine);
    let solid = evaluation.body(feature).unwrap();
    let found = front.resolve(solid).unwrap();
    let face = solid.face(found).unwrap();

    assert_eq!(
        describe_origin(&document, face.origin()),
        "Box 1 front face"
    );
}

#[test]
fn cones_wedges_and_prisms_have_their_volumes_bounds_and_named_faces() {
    let count = |sides: f64| Expression::Number(sides);
    let cases = [
        (
            PrimitiveShape::Cone {
                bottom: mm(4.0),
                top: mm(0.0),
                height: mm(3.0),
            },
            PI * 4.0,
            Point3::new(2.0, 2.0, 3.0),
            vec!["Box 1 bottom face", "Box 1 wall"],
        ),
        (
            PrimitiveShape::Cone {
                bottom: mm(2.0),
                top: mm(4.0),
                height: mm(3.0),
            },
            7.0 * PI,
            Point3::new(2.0, 2.0, 3.0),
            vec!["Box 1 bottom face", "Box 1 top face", "Box 1 wall"],
        ),
        (
            PrimitiveShape::Wedge {
                length: mm(10.0),
                width: mm(6.0),
                height: mm(4.0),
                top: mm(4.0),
            },
            (10.0 + 4.0) / 2.0 * 4.0 * 6.0,
            Point3::new(5.0, 3.0, 4.0),
            vec![
                "Box 1 back face",
                "Box 1 bottom face",
                "Box 1 front face",
                "Box 1 left face",
                "Box 1 sloped face",
                "Box 1 top face",
            ],
        ),
        (
            PrimitiveShape::Wedge {
                length: mm(10.0),
                width: mm(6.0),
                height: mm(4.0),
                top: mm(0.0),
            },
            10.0 * 4.0 / 2.0 * 6.0,
            Point3::new(5.0, 3.0, 4.0),
            vec![
                "Box 1 back face",
                "Box 1 bottom face",
                "Box 1 front face",
                "Box 1 left face",
                "Box 1 sloped face",
            ],
        ),
        (
            PrimitiveShape::Prism {
                sides: count(6.0),
                diameter: mm(4.0),
                height: mm(5.0),
            },
            3.0 * 3.0_f64.sqrt() / 2.0 * 4.0 * 5.0,
            Point3::new(2.0, 3.0_f64.sqrt(), 5.0),
            vec![
                "Box 1 bottom face",
                "Box 1 side face",
                "Box 1 side face",
                "Box 1 side face",
                "Box 1 side face",
                "Box 1 side face",
                "Box 1 side face",
                "Box 1 top face",
            ],
        ),
    ];
    for (shape, expected, high, faces) in cases {
        let (document, feature) = single(primitive(shape.clone()));
        let mut engine = Recompute::default();
        let evaluation = evaluate(&document, &mut engine);

        let solid = evaluation.body(feature).unwrap();
        let found = volume(&evaluation, feature);
        let mut names: Vec<String> = solid
            .faces()
            .map(|(_, face)| describe_origin(&document, face.origin()))
            .collect();
        names.sort();

        assert!(
            (found - expected).abs() < expected * 0.01,
            "{shape:?}: {found} {expected}"
        );
        assert!(bounds(solid).0.z.abs() < 1e-6, "{shape:?}");
        assert!(
            (bounds(solid).1 - high).abs().max_element() < 1e-2,
            "{shape:?}: {:?}",
            bounds(solid).1
        );
        assert_eq!(names, faces, "{shape:?}");
    }
}

#[test]
fn a_cone_a_wedge_or_a_prism_out_of_shape_fails_alone_in_words() {
    let cases = [
        (
            PrimitiveShape::Cone {
                bottom: mm(0.0),
                top: mm(0.0),
                height: mm(3.0),
            },
            "cannot both be zero",
        ),
        (
            PrimitiveShape::Cone {
                bottom: mm(-1.0),
                top: mm(2.0),
                height: mm(3.0),
            },
            "cannot be negative",
        ),
        (
            PrimitiveShape::Wedge {
                length: mm(4.0),
                width: mm(6.0),
                height: mm(4.0),
                top: mm(5.0),
            },
            "cannot be more than its length",
        ),
        (
            PrimitiveShape::Prism {
                sides: Expression::Number(2.5),
                diameter: mm(4.0),
                height: mm(5.0),
            },
            "whole number from 3 to 64",
        ),
        (
            PrimitiveShape::Prism {
                sides: Expression::Number(65.0),
                diameter: mm(4.0),
                height: mm(5.0),
            },
            "whole number from 3 to 64",
        ),
    ];
    for (shape, reason) in cases {
        let (document, feature) = single(primitive(shape.clone()));
        let mut engine = Recompute::default();
        let evaluation = evaluate(&document, &mut engine);

        assert!(
            failure(&evaluation, feature).reason.contains(reason),
            "{shape:?}: {}",
            failure(&evaluation, feature).reason
        );
    }
}

#[test]
fn a_reversed_cone_or_wedge_grows_below_the_plane_with_its_base_on_it() {
    for shape in [
        PrimitiveShape::Cone {
            bottom: mm(4.0),
            top: mm(0.0),
            height: mm(3.0),
        },
        PrimitiveShape::Wedge {
            length: mm(10.0),
            width: mm(6.0),
            height: mm(4.0),
            top: mm(0.0),
        },
    ] {
        let mut placed = primitive(shape.clone());
        placed.reversed = true;
        let (document, feature) = single(placed);
        let mut engine = Recompute::default();
        let evaluation = evaluate(&document, &mut engine);

        let solid = evaluation.body(feature).unwrap();
        let (low, high) = bounds(solid);
        let base = solid
            .faces()
            .find(|(_, face)| match face.surface() {
                Surface::Plane(plane) => plane.frame().normal() * face.sense().sign() == Vector3::Z,
                _ => false,
            })
            .map(|(_, face)| describe_origin(&document, face.origin()))
            .unwrap();

        assert!(
            high.z.abs() < 1e-6 && low.z < -2.9,
            "{shape:?}: {low} {high}"
        );
        assert!(base.ends_with("bottom face"), "{shape:?}: {base}");
    }
}
