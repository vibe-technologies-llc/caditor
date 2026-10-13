use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector3};
use caditor_kernel::{EdgeId, EdgeReference, Solid};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

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
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
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

    let Outline::Spline {
        points: control, ..
    } = outline
    else {
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

#[test]
fn an_ellipse_and_an_elliptical_arc_project_onto_a_parallel_plane_as_themselves() {
    let mut document = Document::default();
    let mut source = Sketch::new(Plane::XY);
    let ellipse = source.add_ellipse(Point2::new(3.0, 2.0), Point2::new(9.0, 2.0), 2.0);
    let arc = source.add_elliptical_arc(
        Point2::new(-10.0, 0.0),
        Point2::new(-10.0, 5.0),
        3.0,
        Point2::new(-10.0, 5.0),
        Point2::new(-13.0, 0.0),
    );
    let mut transaction = document.transaction("Sketches");
    let below = transaction.add_feature("Oval", FeatureKind::from(source.clone()));
    let flipped = Plane::from_frame(Point3::new(0.0, 0.0, 7.0), -Vector3::Z, Vector3::X).unwrap();
    let above = transaction.add_feature("Lid", FeatureKind::from(Sketch::new(flipped)));
    let [projected_ellipse, projected_arc] = [ellipse, arc].map(|entity| {
        transaction.add_projection(
            above,
            ProjectionSource::SketchEntity {
                sketch: below,
                entity,
            },
            &sketch_outline(&source, entity, &flipped).unwrap(),
        )
    });
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();

    let lid = solved(&evaluate(&document, &mut engine), above);

    let whole = lid.ellipse(projected_ellipse).unwrap();
    assert!(matches!(
        lid.entity(projected_ellipse),
        Some(Entity::Ellipse { .. })
    ));
    assert!(whole.center.distance(Point2::new(3.0, -2.0)) < EXACT);
    assert!((whole.major_radius() - 6.0).abs() < EXACT);
    assert!((whole.minor_radius - 2.0).abs() < EXACT);
    let part = lid.ellipse(projected_arc).unwrap();
    let drawn = source.ellipse(arc).unwrap();
    assert!((part.sweep - drawn.sweep).abs() < 1e-9);
    let middle = part.point_at(part.start + part.sweep / 2.0);
    let expected = drawn.point_at(drawn.start + drawn.sweep / 2.0);
    assert!(middle.distance(Point2::new(expected.x, -expected.y)) < 1e-9);
}

#[test]
fn an_ellipse_seen_at_an_angle_is_still_projected_as_a_spline() {
    let mut source = Sketch::new(Plane::XY);
    let ellipse = source.add_ellipse(Point2::ZERO, Point2::new(6.0, 0.0), 2.0);
    let tilted = Plane::from_frame(Point3::ZERO, Vector3::new(0.0, -1.0, 1.0), Vector3::X).unwrap();

    let outline = sketch_outline(&source, ellipse, &tilted).unwrap();

    assert!(matches!(outline, Outline::Spline { .. }), "{outline:?}");
}

#[test]
fn an_elliptical_edge_of_a_body_projects_onto_a_parallel_plane_as_an_ellipse() {
    let mut document = Document::default();
    let mut oval = Sketch::new(Plane::XY);
    oval.add_ellipse(Point2::new(2.0, 1.0), Point2::new(8.0, 1.0), 3.0);
    let mut transaction = document.transaction("Base");
    let outline = transaction.add_feature("Oval", FeatureKind::from(oval));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(transaction.parse("5 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let solid = evaluate(&document, &mut engine).body(base).unwrap().clone();
    let above = Plane::from_frame(Point3::new(0.0, 0.0, 9.0), Vector3::Z, Vector3::X).unwrap();
    let top = solid
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), caditor_kernel::Curve::Ellipse(_))
                && (edge.curve().point(edge.interval().start()).z - 5.0).abs() < EXACT
        })
        .map(|(id, _)| id)
        .unwrap();

    let outline = edge_outline(&solid, top, &above).unwrap();

    let Outline::Ellipse {
        center,
        major,
        minor_radius,
    } = outline
    else {
        panic!("expected an ellipse, found {outline:?}");
    };
    assert!(center.distance(Point2::new(2.0, 1.0)) < 1e-9);
    assert!((center.distance(major) - 6.0).abs() < 1e-9);
    assert!((minor_radius - 3.0).abs() < 1e-9);
}

#[test]
fn a_crossing_line_reaching_no_length_is_refused() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Sketch", FeatureKind::from(Sketch::new(Plane::XY)));
    document.apply(transaction.finish()).unwrap();
    let point = EntityId::from_raw(0);
    let mut transaction = document.transaction("Point");
    transaction.add_sketch_entity(sketch, Entity::Point(Point2::ZERO));
    document.apply(transaction.finish()).unwrap();
    let project = |reach| {
        Transaction::single(
            "Project",
            Edit::SetSketchProjection {
                feature: sketch,
                id: point,
                source: Some(ProjectionSource::PrincipalPlane {
                    plane: PrincipalPlane::Xz,
                    reach,
                }),
            },
        )
    };

    let refused = [f64::NAN, 0.0, -1.0, f64::INFINITY].map(|reach| document.check(&project(reach)));

    assert!(
        refused
            .iter()
            .all(|refusal| matches!(refusal, Err(EditError::ProjectionReach(_))))
    );
    assert_eq!(document.check(&project(10.0)), Ok(()));
}

fn principal_projection(feature: FeatureId, id: EntityId) -> Transaction {
    Transaction::single(
        "Project",
        Edit::SetSketchProjection {
            feature,
            id,
            source: Some(ProjectionSource::PrincipalPlane {
                plane: PrincipalPlane::Xz,
                reach: 10.0,
            }),
        },
    )
}

fn projected_flags(document: &Document, feature: FeatureId) -> Vec<EntityId> {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.sketch())
        .map(|sketch| sketch.projected().collect())
        .unwrap()
}

#[test]
fn undoing_a_line_projection_keeps_its_end_point_projected_on_its_own() {
    let mut document = Document::default();

    let mut source = Sketch::new(Plane::XY);
    let line = source.add_line(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0));
    let end = EntityId::from_raw(1);
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Sketch", FeatureKind::from(source));
    document.apply(transaction.finish()).unwrap();
    document.apply(principal_projection(sketch, end)).unwrap();
    let before = document.clone();

    let inverse = document.apply(principal_projection(sketch, line)).unwrap();
    document.apply(inverse).unwrap();

    assert_eq!(projected_flags(&document, sketch), vec![end]);
    assert!(document.same_content(&before));
}

#[test]
fn unprojecting_one_of_two_lines_keeps_their_shared_point_projected() {
    let mut document = Document::default();

    let mut source = Sketch::new(Plane::XY);
    let first = source.add_line(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0));
    let shared = EntityId::from_raw(1);
    let far = source.add_point(Point2::new(4.0, 3.0));
    let second = EntityId::from_raw(4);
    source
        .insert_entity(
            second,
            Entity::Line {
                start: shared,
                end: far,
            },
        )
        .unwrap();
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Sketch", FeatureKind::from(source));
    document.apply(transaction.finish()).unwrap();
    document.apply(principal_projection(sketch, first)).unwrap();
    document
        .apply(principal_projection(sketch, second))
        .unwrap();

    document
        .apply(Transaction::single(
            "Unproject",
            Edit::SetSketchProjection {
                feature: sketch,
                id: first,
                source: None,
            },
        ))
        .unwrap();

    assert_eq!(
        projected_flags(&document, sketch),
        vec![shared, far, second]
    );
}

#[test]
fn undoing_the_removal_of_a_constraint_whose_points_were_projected_since_restores_it() {
    let mut document = Document::default();

    let mut source = Sketch::new(Plane::XY);
    let circle = source.add_circle(Point2::new(4.0, 2.0), 3.0);
    let centre = EntityId::from_raw(0);
    let offset = source
        .add_constraint(Constraint::HorizontalDistance {
            from: centre,
            to: EntityId::VERTICAL_AXIS,
            value: Expression::Measure(4.0, Unit::Millimetre),
        })
        .unwrap();
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Sketch", FeatureKind::from(source));
    document.apply(transaction.finish()).unwrap();
    document
        .apply(principal_projection(sketch, circle))
        .unwrap();
    let before = document.clone();

    let inverse = document
        .apply(Transaction::single(
            "Delete",
            Edit::RemoveSketchConstraint {
                feature: sketch,
                id: offset,
            },
        ))
        .unwrap();
    document.apply(inverse).unwrap();

    assert_eq!(projected_flags(&document, sketch), vec![centre, circle]);
    assert!(document.same_content(&before));
}

struct Datums {
    document: Document,
    point: FeatureId,
    axis: FeatureId,
    plane: FeatureId,
    sketch: FeatureId,
    line: EntityId,
    at_point: EntityId,
    origin: EntityId,
    across: EntityId,
}

fn datum_point_at(document: &Document, x: &str) -> Datum {
    Datum::Point(DatumPoint {
        base: PointReference::Origin,
        offset: [x, "5 mm", "0 mm"].map(|text| document.parse(text).unwrap()),
    })
}

fn datums() -> Datums {
    let mut document = Document::default();
    let side = PrincipalPlane::Xz.plane();
    let mut transaction = document.transaction("Datums");
    let point = transaction.add_feature(
        "Spot",
        FeatureKind::Datum(datum_point_at(&document, "10 mm")),
    );
    let axis = transaction.add_feature(
        "Post",
        FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(
            PlaneReference::Principal(PrincipalPlane::Xy),
            PointReference::Datum(point),
        ))),
    );
    let plane = transaction.add_feature(
        "Shelf",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: document.parse("4 mm").unwrap(),
        })),
    );
    let sketch = transaction.add_feature("Side", FeatureKind::from(Sketch::new(side)));
    let post = Ray::new(Point3::new(10.0, 5.0, 0.0), Vector3::Z).unwrap();
    let line = transaction.add_projection(
        sketch,
        ProjectionSource::DatumAxis {
            datum: axis,
            reach: 20.0,
        },
        &axis_outline(&post, &side, 20.0).unwrap(),
    );
    let at_point = transaction.add_projection(
        sketch,
        ProjectionSource::DatumPoint { datum: point },
        &point_outline(Point3::new(10.0, 5.0, 0.0), &side),
    );
    let origin = transaction.add_projection(
        sketch,
        ProjectionSource::Origin,
        &point_outline(Point3::ZERO, &side),
    );
    let across = transaction.add_projection(
        sketch,
        ProjectionSource::PrincipalAxis {
            axis: PrincipalAxis::Y,
            reach: 20.0,
        },
        &axis_outline(&PrincipalAxis::Y.ray().unwrap(), &side, 20.0).unwrap(),
    );
    document.apply(transaction.finish()).unwrap();
    Datums {
        document,
        point,
        axis,
        plane,
        sketch,
        line,
        at_point,
        origin,
        across,
    }
}

fn world(sketch: &Sketch, point: Point2) -> Point3 {
    sketch.plane().to_world(point)
}

#[test]
fn datum_axes_points_principal_axes_and_the_origin_project_and_follow_their_datums() {
    let mut model = datums();
    let mut engine = Recompute::default();

    let first = solved(&evaluate(&model.document, &mut engine), model.sketch);
    let [start, end] = ends(&first, model.line);

    assert!((world(&first, start).x - 10.0).abs() < EXACT);
    assert!((world(&first, end).x - 10.0).abs() < EXACT);
    assert!((world(&first, start).z - world(&first, end).z).abs() > 39.0);
    assert!(
        world(&first, first.point(model.at_point).unwrap()).distance(Point3::new(10.0, 0.0, 0.0))
            < EXACT
    );
    assert!(world(&first, first.point(model.origin).unwrap()).length() < EXACT);
    assert!(
        world(&first, first.point(model.across).unwrap()).length() < EXACT,
        "the Y axis is seen end-on from the XZ plane"
    );
    assert!(first.is_projected(model.line));
    assert_eq!(
        model.document.dependents_of(&[model.point]),
        vec![model.axis, model.sketch]
    );
    assert_eq!(
        model.document.dependents_of(&[model.axis]),
        vec![model.sketch]
    );

    model
        .document
        .apply(Transaction::single(
            "Move the spot",
            Edit::SetFeatureKind {
                id: model.point,
                kind: FeatureKind::Datum(datum_point_at(&model.document, "25 mm")),
            },
        ))
        .unwrap();
    let moved = solved(&evaluate(&model.document, &mut engine), model.sketch);
    let [start, end] = ends(&moved, model.line);

    assert!((world(&moved, start).x - 25.0).abs() < EXACT);
    assert!((world(&moved, end).x - 25.0).abs() < EXACT);
    assert!(
        world(&moved, moved.point(model.at_point).unwrap()).distance(Point3::new(25.0, 0.0, 0.0))
            < EXACT
    );
}

#[test]
fn a_projected_datum_must_be_of_its_kind_and_one_deleted_fails_the_sketch_in_words() {
    let mut model = datums();
    let mut engine = Recompute::default();
    let wrong = |source: ProjectionSource| {
        model.document.check(&Transaction::single(
            "Project",
            Edit::SetSketchProjection {
                feature: model.sketch,
                id: model.line,
                source: Some(source),
            },
        ))
    };

    assert_eq!(
        wrong(ProjectionSource::DatumAxis {
            datum: model.plane,
            reach: 20.0,
        }),
        Err(EditError::NotAnAxis("Shelf".to_owned()))
    );
    assert_eq!(
        wrong(ProjectionSource::DatumPoint { datum: model.axis }),
        Err(EditError::NotAPoint("Post".to_owned()))
    );
    assert_eq!(
        wrong(ProjectionSource::PrincipalAxis {
            axis: PrincipalAxis::X,
            reach: 0.0,
        }),
        Err(EditError::ProjectionReach(0.0))
    );

    model
        .document
        .apply(Transaction::single(
            "Delete the post",
            Edit::RemoveFeature { id: model.axis },
        ))
        .unwrap();
    let error = failure(&evaluate(&model.document, &mut engine), model.sketch);

    assert_eq!(error.reason, "It uses a feature that no longer exists.");
}

#[test]
fn a_projected_axis_turned_end_on_fails_its_sketch_naming_the_axis() {
    let mut model = datums();
    let mut engine = Recompute::default();

    model
        .document
        .apply(Transaction::single(
            "Lay the post down",
            Edit::SetFeatureKind {
                id: model.axis,
                kind: FeatureKind::Datum(Datum::Axis(DatumAxis::NormalTo(
                    PlaneReference::Principal(PrincipalPlane::Xz),
                    PointReference::Datum(model.point),
                ))),
            },
        ))
        .unwrap();
    let error = failure(&evaluate(&model.document, &mut engine), model.sketch);

    assert_eq!(
        error.reason,
        format!(
            "Line {} is projected from Post whose shape in this sketch changed kind, for \
             example a line now seen end-on.",
            model.line
        )
    );
}
