use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Entity, Sketch};

use crate::*;

const EXACT: f64 = 1e-6;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
    ];
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % 4]);
    }
    sketch
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn solved(evaluation: &Evaluation, sketch: FeatureId) -> Sketch {
    evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| result.geometry.clone())
        .unwrap()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
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

fn world_ends(sketch: &Sketch) -> Vec<[Point3; 2]> {
    let plane = sketch.plane();
    sketch
        .entities()
        .filter(|(id, entity)| matches!(entity, Entity::Line { .. }) && sketch.is_projected(*id))
        .filter_map(|(id, _)| sketch.line_endpoints(id))
        .map(|(start, end)| [plane.to_world(start), plane.to_world(end)])
        .collect()
}

struct Block {
    document: Document,
    height: ParameterId,
    base: FeatureId,
    engine: Recompute,
}

fn block() -> Block {
    let mut document = Document::default();
    let mut transaction = document.transaction("Base");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Parameter(height), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    Block {
        document,
        height,
        base,
        engine: Recompute::default(),
    }
}

fn intersect(block: &mut Block, plane: Plane) -> (FeatureId, Vec<SectionCurve>) {
    let solid = evaluate(&block.document, &mut block.engine)
        .body(block.base)
        .unwrap()
        .clone();
    let mut transaction = block.document.transaction("Cut");
    let sketch = transaction.add_feature("Cut", FeatureKind::from(Sketch::new(plane)));
    let curves = section_curves(block.base, &solid, &plane, sketch).unwrap();
    for curve in &curves {
        transaction.add_projection(sketch, curve.source.clone(), &curve.outline);
    }
    block.document.apply(transaction.finish()).unwrap();
    (sketch, curves)
}

#[test]
fn intersecting_a_block_draws_its_cross_section_which_follows_the_body() {
    let mut block = block();
    let across = Plane::new(Point3::new(0.0, 4.0, 0.0), Vector3::Y).unwrap();

    let (sketch, curves) = intersect(&mut block, across);
    let faces: BTreeSet<_> = curves.iter().map(|curve| curve.face).collect();
    let evaluation = evaluate(&block.document, &mut block.engine);
    let ends = world_ends(&solved(&evaluation, sketch));
    let highest = |ends: &[[Point3; 2]]| {
        ends.iter()
            .flatten()
            .map(|point| point.z)
            .fold(f64::NEG_INFINITY, f64::max)
    };

    assert_eq!(curves.len(), 4);
    assert_eq!(faces.len(), 4);
    assert_eq!(ends.len(), 4);
    assert!(
        ends.iter()
            .flatten()
            .all(|point| (point.y - 4.0).abs() < EXACT)
    );
    assert!((highest(&ends) - 4.0).abs() < EXACT);

    set(&mut block.document, block.height, "6 mm");
    let evaluation = evaluate(&block.document, &mut block.engine);
    let ends = world_ends(&solved(&evaluation, sketch));

    assert_eq!(ends.len(), 4);
    assert!((highest(&ends) - 6.0).abs() < EXACT);
}

#[test]
fn a_cross_section_the_sketch_plane_no_longer_cuts_fails_its_sketch_in_words() {
    let mut block = block();
    let level = Plane::new(Point3::new(0.0, 0.0, 3.0), Vector3::Z).unwrap();

    let (sketch, curves) = intersect(&mut block, level);

    assert_eq!(curves.len(), 4);
    assert!(matches!(
        evaluate(&block.document, &mut block.engine)
            .feature(sketch)
            .map(|status| &status.state),
        Some(FeatureState::UpToDate)
    ));

    set(&mut block.document, block.height, "2 mm");
    let error = failure(&evaluate(&block.document, &mut block.engine), sketch);

    assert!(
        error.reason.contains("the cut through Base"),
        "{}",
        error.reason
    );
    assert!(error.reason.contains("no longer cuts"), "{}", error.reason);
    assert!(error.remedy.contains("intersect"), "{}", error.remedy);
}

#[test]
fn a_plane_missing_the_body_has_no_cross_section() {
    let mut block = block();
    let solid = evaluate(&block.document, &mut block.engine)
        .body(block.base)
        .unwrap()
        .clone();
    let above = Plane::new(Point3::new(0.0, 0.0, 9.0), Vector3::Z).unwrap();

    let outcome = section_curves(block.base, &solid, &above, FeatureId::from_raw(99));

    assert!(matches!(outcome, Err(SectionError::Misses)));
}

#[test]
fn a_datum_plane_crosses_the_sketch_in_a_line_that_follows_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Datum");
    let offset = transaction.add_parameter("offset", transaction.parse("3 mm").unwrap());
    let datum = transaction.add_feature(
        "Plane",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Yz),
            rotation: None,
            offset: Expression::Parameter(offset),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let plane = evaluate(&document, &mut engine)
        .feature(datum)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::datum)
        .and_then(DatumResult::plane)
        .unwrap();

    let mut transaction = document.transaction("Cut");
    let sketch = transaction.add_feature("Cut", FeatureKind::from(Sketch::new(Plane::XY)));
    let outline = datum_outline(&plane, &Plane::XY, 5.0).unwrap();
    let line = transaction.add_projection(
        sketch,
        ProjectionSource::DatumPlane { datum, reach: 5.0 },
        &outline,
    );
    document.apply(transaction.finish()).unwrap();
    set(&mut document, offset, "7 mm");
    let evaluation = evaluate(&document, &mut engine);
    let (start, end) = solved(&evaluation, sketch).line_endpoints(line).unwrap();

    assert!(matches!(outline, Outline::Line { .. }));
    assert!((start.x - 7.0).abs() < EXACT && (end.x - 7.0).abs() < EXACT);
    assert!(((start.y - end.y).abs() - 10.0).abs() < EXACT);
    assert!(datum_outline(&Plane::XY, &Plane::XY, 5.0).is_none());
}

#[test]
fn a_datum_section_must_come_from_a_plane() {
    let mut block = block();
    let mut transaction = block.document.transaction("Cut");
    let sketch = transaction.add_feature("Cut", FeatureKind::from(Sketch::new(Plane::XY)));
    transaction.add_projection(
        sketch,
        ProjectionSource::DatumPlane {
            datum: block.base,
            reach: 5.0,
        },
        &Outline::Line {
            start: Point2::ZERO,
            end: Point2::X,
        },
    );

    assert!(matches!(
        block.document.apply(transaction.finish()),
        Err(EditError::NotAPlane(_))
    ));
}

#[test]
fn a_principal_plane_crosses_a_tilted_sketch_in_a_line_that_follows_the_tilt() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Datum");
    let tilt = transaction.add_parameter("tilt", transaction.parse("30 deg").unwrap());
    let datum = transaction.add_feature(
        "Plane",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: Some(PlaneRotation {
                axis: AxisReference::Principal(PrincipalAxis::X),
                angle: Expression::Parameter(tilt),
            }),
            offset: transaction.parse("5 mm").unwrap(),
        })),
    );
    let sketch = transaction.add_feature(
        "Cut",
        FeatureKind::Sketch(SketchFeature::on_datum(Sketch::new(Plane::XY), datum)),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let tilted = solved(&evaluate(&document, &mut engine), sketch).plane();

    let mut transaction = document.transaction("Intersect");
    let source = ProjectionSource::PrincipalPlane {
        plane: PrincipalPlane::Xy,
        reach: 20.0,
    };
    let outline = datum_outline(&Plane::XY, &tilted, 20.0).unwrap();
    let line = transaction.add_projection(sketch, source.clone(), &outline);
    document.apply(transaction.finish()).unwrap();
    set(&mut document, tilt, "60 deg");
    let evaluation = evaluate(&document, &mut engine);
    let steeper = solved(&evaluation, sketch);
    let (start, end) = steeper.line_endpoints(line).unwrap();
    let height = |point: Point2| steeper.plane().to_world(point).z;

    assert_eq!(source.feature(), None);
    assert!(height(start).abs() < EXACT && height(end).abs() < EXACT);
    assert!(((start - end).length() - 40.0).abs() < EXACT);

    set(&mut document, tilt, "0 deg");
    let level = evaluate(&document, &mut engine);

    assert!(failure(&level, sketch).reason.contains("the XY plane"));
}
