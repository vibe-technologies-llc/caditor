use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{EdgeId, EdgeReference, Solid};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::*;

const EXACT: f64 = 1e-9;

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

fn top_front_edge(solid: &Solid, height: f64) -> EdgeId {
    solid
        .edges()
        .find(|(_, edge)| {
            let interval = edge.interval();
            let ends = [
                edge.curve().point(interval.start()),
                edge.curve().point(interval.end()),
            ];
            ends.iter()
                .all(|end| (end.z - height).abs() < EXACT && end.y.abs() < EXACT)
        })
        .map(|(id, _)| id)
        .unwrap()
}

struct Block {
    document: Document,
    height: ParameterId,
    base: FeatureId,
    side: FeatureId,
    line: EntityId,
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
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let solid = evaluate(&document, &mut engine).body(base).unwrap().clone();
    let edge = top_front_edge(&solid, 4.0);
    let mut transaction = document.transaction("Side");
    let side = transaction.add_feature("Side", FeatureKind::from(Sketch::new(Plane::XZ)));
    let line = transaction.add_projection(
        side,
        ProjectionSource::Edge {
            body: base,
            edge: EdgeReference::capture(&solid, edge).unwrap(),
        },
        &edge_outline(&solid, edge, &Plane::XZ).unwrap(),
    );
    document.apply(transaction.finish()).unwrap();
    Block {
        document,
        height,
        base,
        side,
        line,
        engine,
    }
}

fn ends(sketch: &Sketch, line: EntityId) -> [Point2; 2] {
    let (start, end) = sketch.line_endpoints(line).unwrap();
    let mut ends = [start, end];
    ends.sort_by(|a, b| a.x.total_cmp(&b.x));
    ends
}

fn on_side(x: f64, z: f64) -> Point2 {
    Plane::XZ.to_local(Point3::new(x, 0.0, z))
}

#[test]
fn a_projected_edge_follows_its_body_and_adds_no_freedom() {
    let mut block = block();

    let evaluation = evaluate(&block.document, &mut block.engine);
    let first = solved(&evaluation, block.side);
    let solution = evaluation
        .feature(block.side)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| result.solution.clone())
        .unwrap();

    assert!(matches!(
        first.entity(block.line),
        Some(Entity::Line { .. })
    ));
    assert!(first.is_projected(block.line));
    assert_eq!(solution.degrees_of_freedom(), 0);
    for (found, expected) in ends(&first, block.line)
        .iter()
        .zip([on_side(0.0, 4.0), on_side(10.0, 4.0)])
    {
        assert!(
            found.distance(expected) < EXACT,
            "{found} is not {expected}"
        );
    }

    let taller = Transaction::single(
        "Taller",
        Edit::SetParameterExpression {
            id: block.height,
            expression: Expression::parse_stored("6 mm").unwrap(),
        },
    );
    block.document.apply(taller).unwrap();
    let moved = solved(&evaluate(&block.document, &mut block.engine), block.side);

    for (found, expected) in ends(&moved, block.line)
        .iter()
        .zip([on_side(0.0, 6.0), on_side(10.0, 6.0)])
    {
        assert!(
            found.distance(expected) < EXACT,
            "{found} is not {expected}"
        );
    }
}

#[test]
fn deleting_a_projected_line_takes_its_points_and_undo_restores_the_projection() {
    let mut block = block();
    let definition = |document: &Document| -> SketchFeature {
        match &document.feature(block.side).unwrap().kind {
            FeatureKind::Sketch(sketch) => sketch.clone(),
            other => panic!("expected a sketch, found {other:?}"),
        }
    };
    let before = definition(&block.document);

    let mut transaction = block.document.transaction("Delete");
    transaction.remove_sketch_items(block.side, [block.line], []);
    let undo = block.document.apply(transaction.finish()).unwrap();
    let removed = definition(&block.document);

    assert_eq!(removed.sketch.entities().len(), 0);
    assert!(removed.projections.is_empty());

    block.document.apply(undo).unwrap();
    let restored = definition(&block.document);

    assert_eq!(restored, before);
    assert!(
        restored
            .sketch
            .entity(block.line)
            .unwrap()
            .points()
            .iter()
            .all(|point| restored.sketch.is_projected(*point))
    );
}

#[test]
fn a_projected_entity_is_never_removed_while_it_still_follows_its_source() {
    let mut block = block();

    let refused = block.document.apply(Transaction::single(
        "Remove",
        Edit::RemoveSketchEntity {
            feature: block.side,
            id: block.line,
        },
    ));

    assert!(matches!(refused, Err(EditError::StillProjected { .. })));
}

#[test]
fn a_body_deleted_from_under_a_projection_fails_its_sketch() {
    let mut block = block();

    block
        .document
        .apply(Transaction::single(
            "Delete base",
            Edit::RemoveFeature { id: block.base },
        ))
        .unwrap();
    let evaluation = evaluate(&block.document, &mut block.engine);

    assert!(matches!(
        evaluation.feature(block.side).unwrap().state,
        FeatureState::Failed(_)
    ));
}

#[test]
fn a_projection_must_come_from_a_feature_above_its_sketch() {
    let mut block = block();
    let below = {
        let mut transaction = block.document.transaction("Below");
        let below = transaction.add_feature("Below", FeatureKind::from(Sketch::new(Plane::XY)));
        block.document.apply(transaction.finish()).unwrap();
        below
    };

    let refused = block.document.apply(Transaction::single(
        "Project",
        Edit::SetSketchProjection {
            feature: block.side,
            id: block.line,
            source: Some(ProjectionSource::SketchEntity {
                sketch: below,
                entity: EntityId::from_raw(0),
            }),
        },
    ));

    assert_eq!(refused, Err(EditError::MissingFeature));
}

#[test]
fn a_circle_projected_from_another_sketch_follows_its_radius() {
    let mut document = Document::default();
    let mut source = Sketch::new(Plane::XY);
    let circle = source.add_circle(Point2::new(3.0, 2.0), 5.0);
    let mut transaction = document.transaction("Sketches");
    let below = transaction.add_feature("Ring", FeatureKind::from(source.clone()));
    let lifted = Plane::from_frame(Point3::new(0.0, 0.0, 7.0), Vector3::Z, Vector3::X).unwrap();
    let above = transaction.add_feature("Lid", FeatureKind::from(Sketch::new(lifted)));
    let projected = transaction.add_projection(
        above,
        ProjectionSource::SketchEntity {
            sketch: below,
            entity: circle,
        },
        &sketch_outline(&source, circle, &lifted).unwrap(),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let first = solved(&evaluate(&document, &mut engine), above);

    assert_eq!(first.circle(projected), Some((Point2::new(3.0, 2.0), 5.0)));

    let Some(Entity::Circle { center, .. }) = source.entity(circle).cloned() else {
        panic!("expected a circle");
    };
    document
        .apply(Transaction::single(
            "Grow",
            Edit::SetSketchEntity {
                feature: below,
                id: circle,
                entity: Entity::Circle {
                    center,
                    radius: 9.0,
                },
            },
        ))
        .unwrap();
    let grown = solved(&evaluate(&document, &mut engine), above);

    assert_eq!(grown.circle(projected), Some((Point2::new(3.0, 2.0), 9.0)));

    let mut transaction = document.transaction("Delete ring");
    transaction.remove_sketch_items(below, [circle], []);
    document.apply(transaction.finish()).unwrap();
    let error = failure(&evaluate(&document, &mut engine), above);

    assert_eq!(
        error.reason,
        format!("Circle {projected} is projected from geometry of Ring that no longer exists.")
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(above)));
}

#[test]
fn a_circle_seen_at_an_angle_is_projected_as_a_spline_through_its_outline() {
    let mut source = Sketch::new(Plane::XY);
    let circle = source.add_circle(Point2::ZERO, 4.0);
    let tilted = Plane::from_frame(Point3::ZERO, Vector3::new(0.0, -1.0, 1.0), Vector3::X).unwrap();

    let outline = sketch_outline(&source, circle, &tilted).unwrap();

    let Outline::Spline(control) = outline else {
        panic!("expected a spline, found {outline:?}");
    };
    assert_eq!(control.len(), PROJECTED_SPLINE_POINTS);
    let spline = caditor_sketch::BSpline::clamped(control).unwrap();
    let minor = 4.0 * std::f64::consts::FRAC_1_SQRT_2;
    for step in 0..=20 {
        let point = spline.point_at(f64::from(step) / 20.0);
        let on_ellipse = (point.x / 4.0).powi(2) + (point.y / minor).powi(2);
        assert!(
            (on_ellipse - 1.0).abs() < 1e-3,
            "{point} is off the ellipse"
        );
    }
}
