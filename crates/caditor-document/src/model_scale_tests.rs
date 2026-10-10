use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{EdgeReference, FaceName, SamplingTolerance, Solid, WallSide};
use caditor_sketch::{Constraint, ConstraintId, Entity, Sketch};

use crate::*;

struct Model {
    document: Document,
    height: ParameterId,
    outline: FeatureId,
    dimension: ConstraintId,
    base: FeatureId,
    fillet: FeatureId,
    datum: FeatureId,
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-4, 0.02).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn face_names(evaluation: &Evaluation, body: FeatureId) -> BTreeSet<FaceName> {
    evaluation
        .body(body)
        .unwrap()
        .faces()
        .map(|(_, face)| face.name())
        .collect()
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < 1e-6
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

fn outline() -> (Sketch, ConstraintId) {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(10.0, 8.0),
        Point2::new(0.0, 8.0),
    ];
    let lines: Vec<_> = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    let Some(Entity::Line { start, end }) = sketch.entity(lines[0]).cloned() else {
        panic!("the first side is a line");
    };
    let dimension = sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::parse_stored("10 mm").unwrap(),
        })
        .unwrap();
    sketch
        .set_label_offset(dimension, Some(Vector2::new(0.0, -3.0)))
        .unwrap();
    sketch
        .add_constraint(Constraint::Fix {
            point: end,
            at: Point2::new(10.0, 0.0),
        })
        .unwrap();
    (sketch, dimension)
}

fn model() -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let height = transaction.add_parameter("height", transaction.parse("4 mm").unwrap());
    let (sketch, dimension) = outline();
    let outline = transaction.add_feature("Outline", FeatureKind::from(sketch));
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
    let datum = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(Datum::Plane(DatumPlane {
            base: PlaneReference::Principal(PrincipalPlane::Xy),
            rotation: None,
            offset: transaction.parse("5 mm").unwrap(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(base).unwrap();
    let edges = vec![
        edge_at(solid, Point3::new(5.0, 0.0, 4.0)),
        edge_at(solid, Point3::new(0.0, 4.0, 4.0)),
    ];
    let mut transaction = document.transaction("Fillet");
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges,
            size: transaction.parse("1 mm").unwrap(),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        height,
        outline,
        dimension,
        base,
        fillet,
        datum,
    }
}

fn scale(factor: f64, centre: Point3, values: ScaledValues) -> ModelScale {
    ModelScale {
        factor,
        centre,
        values,
    }
}

fn text(document: &Document, expression: &Expression) -> String {
    document.expression_text(expression)
}

fn datum_offset(document: &Document, datum: FeatureId) -> String {
    match document
        .feature(datum)
        .and_then(|feature| feature.kind.datum())
    {
        Some(Datum::Plane(plane)) => text(document, &plane.offset),
        other => panic!("a datum plane was expected, found {other:?}"),
    }
}

fn bounds(evaluation: &Evaluation, body: FeatureId) -> (Point3, Point3) {
    let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
    (bounds.min(), bounds.max())
}

fn near(found: Point3, expected: [f64; 3]) -> bool {
    (found - Point3::from_array(expected)).length() < 1e-6
}

#[test]
fn scaling_the_model_by_two_multiplies_the_volume_by_eight_keeping_ids_and_names() {
    let mut model = model();
    let original = model.document.clone();
    let before = evaluate(&model.document);
    let ids: Vec<FeatureId> = model.document.features().map(Feature::id).collect();
    let names = face_names(&before, model.base);
    let volume_before = volume(&before, model.base);

    let scaled = model
        .document
        .scaled(&scale(2.0, Point3::ZERO, ScaledValues::AndParameters))
        .unwrap();
    let undo = model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let sketch = model
        .document
        .feature(model.outline)
        .and_then(|feature| feature.kind.sketch())
        .unwrap();
    let height = model.document.parameter(model.height).unwrap();
    let blend = model.document.feature(model.fillet).unwrap();

    assert_eq!(after.failed_count(), 0);
    assert!(
        (volume(&after, model.base) - 8.0 * volume_before).abs() < 1e-3 * volume_before,
        "{} against {}",
        volume(&after, model.base),
        volume_before
    );
    assert_eq!(
        model
            .document
            .features()
            .map(Feature::id)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(face_names(&after, model.base), names);
    assert_eq!(text(&model.document, &height.expression), "8 mm");
    assert_eq!(
        text(
            &model.document,
            sketch
                .constraint(model.dimension)
                .unwrap()
                .dimension()
                .unwrap()
        ),
        "20 mm"
    );
    assert_eq!(
        sketch.label_offset(model.dimension),
        Some(Vector2::new(0.0, -6.0))
    );
    assert!(sketch.constraints().any(
        |(_, held)| matches!(held, Constraint::Fix { at, .. } if *at == Point2::new(20.0, 0.0))
    ));
    assert_eq!(
        text(&model.document, &blend.kind.blend().unwrap().size),
        "2 mm"
    );
    assert_eq!(datum_offset(&model.document, model.datum), "10 mm");
    assert_eq!(scaled.summary.parameters, 1);
    assert_eq!(scaled.summary.sketches, 1);
    assert_eq!(scaled.summary.features, 2);
    assert!(scaled.summary.kept.is_empty());

    model.document.apply(undo).unwrap();
    let back = evaluate(&model.document);

    assert!(model.document.same_content(&original));
    assert!((volume(&back, model.base) - volume_before).abs() < 1e-6 * volume_before);
}

#[test]
fn scaling_plain_values_leaves_parameters_and_says_which_features_follow_them() {
    let mut model = model();

    let scaled = model
        .document
        .scaled(&scale(2.0, Point3::ZERO, ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let (low, high) = bounds(&after, model.base);

    assert_eq!(
        text(
            &model.document,
            &model.document.parameter(model.height).unwrap().expression
        ),
        "4 mm"
    );
    assert_eq!(scaled.summary.kept, ["Base"]);
    assert_eq!(scaled.summary.parameters, 0);
    assert!(near(low, [0.0, 0.0, 0.0]), "{low:?}");
    assert!(near(high, [20.0, 16.0, 4.0]), "{high:?}");
}

#[test]
fn a_named_value_scales_with_the_plain_values() {
    let mut model = model();
    let mut transaction = model.document.transaction("Name");
    let named = transaction.add_owned_parameter(
        "radius",
        transaction.parse("1 mm").unwrap(),
        ParameterOwner::Feature {
            feature: model.fillet,
            value: "Radius".to_owned(),
        },
    );
    let mut blend = model
        .document
        .feature(model.fillet)
        .and_then(|feature| feature.kind.blend())
        .unwrap()
        .clone();
    blend.size = Expression::Parameter(named);
    transaction.edit(Edit::SetFeatureKind {
        id: model.fillet,
        kind: FeatureKind::Blend(blend),
    });
    model.document.apply(transaction.finish()).unwrap();

    let scaled = model
        .document
        .scaled(&scale(3.0, Point3::ZERO, ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();

    assert_eq!(
        text(
            &model.document,
            &model.document.parameter(named).unwrap().expression
        ),
        "3 mm"
    );
    assert!(!scaled.summary.kept.contains(&"Fillet 1".to_owned()));
}

#[test]
fn a_thin_wall_thickens_with_the_model() {
    let mut model = model();
    let mut extrude = match model
        .document
        .feature(model.base)
        .and_then(|feature| feature.kind.solid())
    {
        Some(SolidFeature::Extrude(extrude)) => extrude.clone(),
        other => panic!("an extrusion was expected, found {other:?}"),
    };
    let mut transaction = model.document.transaction("Wall");
    extrude.wall = Some(Box::new(Wall {
        thickness: transaction.parse("0.5 mm").unwrap(),
        side: WallSide::Inside,
    }));
    transaction.edit(Edit::SetFeatureKind {
        id: model.base,
        kind: FeatureKind::Solid(SolidFeature::Extrude(extrude)),
    });
    model.document.apply(transaction.finish()).unwrap();

    let scaled = model
        .document
        .scaled(&scale(2.0, Point3::ZERO, ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();

    let thickness = match model
        .document
        .feature(model.base)
        .and_then(|feature| feature.kind.solid())
    {
        Some(SolidFeature::Extrude(extrude)) => extrude.wall.as_ref().unwrap().thickness.clone(),
        other => panic!("an extrusion was expected, found {other:?}"),
    };
    assert_eq!(text(&model.document, &thickness), "1 mm");
}

#[test]
fn ellipses_keep_their_shape_as_the_model_grows() {
    let mut document = Document::default();
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(1.0, 2.0), Point2::new(5.0, 2.0), 2.0);
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(3.0, 0.0),
        1.0,
        Point2::new(3.0, 0.0),
        Point2::new(0.0, 1.0),
    );
    let mut transaction = document.transaction("Ovals");
    let feature = transaction.add_feature("Ovals", FeatureKind::from(sketch));
    document.apply(transaction.finish()).unwrap();

    let scaled = document
        .scaled(&scale(2.0, Point3::ZERO, ScaledValues::Plain))
        .unwrap();
    document.apply(scaled.transaction).unwrap();

    let sketch = document
        .feature(feature)
        .and_then(|feature| feature.kind.sketch())
        .unwrap();
    let Some(Entity::Ellipse {
        center,
        major,
        minor_radius,
    }) = sketch.entity(ellipse).cloned()
    else {
        panic!("the ellipse is kept");
    };
    let Some(Entity::EllipticalArc {
        minor_radius: arc_minor,
        ..
    }) = sketch.entity(arc).cloned()
    else {
        panic!("the elliptical arc is kept");
    };
    assert_eq!(minor_radius, 4.0);
    assert_eq!(arc_minor, 2.0);
    assert_eq!(sketch.point(center), Some(Point2::new(2.0, 4.0)));
    assert_eq!(sketch.point(major), Some(Point2::new(10.0, 4.0)));
}

#[test]
fn scaling_about_a_point_keeps_that_point_in_place() {
    let mut model = model();

    let scaled = model
        .document
        .scaled(&scale(
            2.0,
            Point3::new(10.0, 8.0, 0.0),
            ScaledValues::AndParameters,
        ))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let (low, high) = bounds(&after, model.base);

    assert_eq!(after.failed_count(), 0);
    assert!(near(low, [-10.0, -8.0, 0.0]), "{low:?}");
    assert!(near(high, [10.0, 8.0, 8.0]), "{high:?}");
    assert_eq!(datum_offset(&model.document, model.datum), "10 mm");
}

fn image(centre: Point3, point: Point3) -> Point3 {
    centre + (point - centre) * 2.0
}

#[test]
fn a_centre_off_principal_geometry_moves_its_users_onto_datums_where_it_lands() {
    let mut model = model();
    let mut transaction = model.document.transaction("Users");
    let block = transaction.add_feature(
        "Block",
        FeatureKind::Primitive(Primitive {
            shape: PrimitiveShape::Box {
                length: transaction.parse("2 mm").unwrap(),
                width: transaction.parse("3 mm").unwrap(),
                height: transaction.parse("4 mm").unwrap(),
            },
            plane: PlaneReference::Principal(PrincipalPlane::Xz),
            at: [
                transaction.parse("20 mm").unwrap(),
                transaction.parse("1 mm").unwrap(),
            ],
            anchor: PrimitiveAnchor::Corner,
            reversed: false,
            operation: BodyOperation::NewBody,
        }),
    );
    transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: model.base,
            plane: PlaneReference::Principal(PrincipalPlane::Yz),
            keep_original: true,
            mirrored: Vec::new(),
            faces: Vec::new(),
        }),
    );
    let axis = transaction.add_feature(
        "Axis 1",
        FeatureKind::Datum(Datum::Axis(DatumAxis::Along(AxisReference::Principal(
            PrincipalAxis::Z,
        )))),
    );
    let frame = transaction.add_feature(
        "Frame 1",
        FeatureKind::Datum(Datum::Frame(Box::new(DatumFrame::world()))),
    );
    model.document.apply(transaction.finish()).unwrap();
    let original = model.document.clone();
    let before = evaluate(&model.document);
    let centre = Point3::new(5.0, 3.0, 2.0);

    let scaled = model
        .document
        .scaled(&scale(2.0, centre, ScaledValues::AndParameters))
        .unwrap();
    let moved: Vec<(&str, Vec<PrincipalGeometry>)> = scaled
        .summary
        .moved
        .iter()
        .map(|moved| (moved.name.as_str(), moved.geometry.clone()))
        .collect();

    assert_eq!(
        moved,
        vec![
            ("Block", vec![PrincipalGeometry::Plane(PrincipalPlane::Xz)]),
            (
                "Mirror 1",
                vec![PrincipalGeometry::Plane(PrincipalPlane::Yz)]
            ),
            ("Axis 1", vec![PrincipalGeometry::Axis(PrincipalAxis::Z)]),
            ("Frame 1", vec![PrincipalGeometry::Origin]),
        ]
    );
    assert_eq!(
        scaled.summary.datums,
        vec![
            "Origin, scaled".to_owned(),
            "Axes and planes, scaled".to_owned()
        ]
    );

    let inverse = model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let names: Vec<&str> = model
        .document
        .features()
        .take(2)
        .map(|feature| feature.name.as_str())
        .collect();
    let landed = image(centre, Point3::ZERO);
    let axis_ray = after
        .feature(axis)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::datum)
        .and_then(DatumResult::axis)
        .unwrap();
    let frame_plane = after
        .feature(frame)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::datum)
        .and_then(DatumResult::frame)
        .unwrap();

    assert_eq!(after.failed_count(), 0);
    assert_eq!(names, ["Origin, scaled", "Axes and planes, scaled"]);
    for body in [model.base, block] {
        let (low, high) = bounds(&before, body);
        let (scaled_low, scaled_high) = bounds(&after, body);
        let expected = (image(centre, low), image(centre, high));
        assert!(
            near(scaled_low, expected.0.to_array()),
            "{scaled_low:?} {expected:?}"
        );
        assert!(
            near(scaled_high, expected.1.to_array()),
            "{scaled_high:?} {expected:?}"
        );
    }
    assert!((axis_ray.direction() - Vector3::Z).length() < 1e-9);
    assert!((axis_ray.origin() - landed).cross(Vector3::Z).length() < 1e-9);
    assert!(near(frame_plane.origin(), landed.to_array()));

    model.document.apply(inverse).unwrap();

    assert!(model.document.same_content(&original));
}

#[test]
fn a_centre_on_the_principal_geometry_adds_no_datum() {
    let mut model = model();
    let mut transaction = model.document.transaction("Mirror");
    transaction.add_feature(
        "Mirror 1",
        FeatureKind::Mirror(Mirror {
            body: model.base,
            plane: PlaneReference::Principal(PrincipalPlane::Yz),
            keep_original: true,
            mirrored: Vec::new(),
            faces: Vec::new(),
        }),
    );
    model.document.apply(transaction.finish()).unwrap();

    let along_it = model
        .document
        .scaled(&scale(2.0, Point3::new(0.0, 5.0, 0.0), ScaledValues::Plain))
        .unwrap();

    assert!(along_it.summary.moved.is_empty());
    assert!(along_it.summary.datums.is_empty());
    assert!(
        model
            .document
            .displaced_by_scale(Point3::new(0.0, 5.0, 0.0))
            .is_empty()
    );
}

#[test]
fn a_projected_principal_plane_moves_onto_a_datum_plane_where_it_lands() {
    let mut model = model();
    let mut transaction = model.document.transaction("Sketch");
    let sketch = transaction.add_feature("Side", FeatureKind::from(Sketch::new(Plane::XZ)));
    let source = ProjectionSource::PrincipalPlane {
        plane: PrincipalPlane::Xy,
        reach: 20.0,
    };
    let outline = datum_outline(&Plane::XY, &Plane::XZ, 20.0).unwrap();
    let line = transaction.add_projection(sketch, source, &outline);
    model.document.apply(transaction.finish()).unwrap();

    let scaled = model
        .document
        .scaled(&scale(2.0, Point3::new(0.0, 0.0, 5.0), ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let solved = after
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| result.geometry.clone())
        .unwrap();
    let (start, end) = solved.line_endpoints(line).unwrap();
    let height = |point: Point2| solved.plane().to_world(point).z;

    assert_eq!(scaled.summary.datums, vec!["XY plane, scaled".to_owned()]);
    assert_eq!(after.failed_count(), 0);
    assert!((height(start) + 5.0).abs() < 1e-9, "{}", height(start));
    assert!((height(end) + 5.0).abs() < 1e-9, "{}", height(end));
}

#[test]
fn a_factor_that_is_not_above_zero_or_is_one_is_refused() {
    let model = model();
    let refused = |factor: f64| {
        model
            .document
            .scaled(&scale(factor, Point3::ZERO, ScaledValues::Plain))
    };

    assert_eq!(
        refused(0.0).err(),
        Some(ModelScaleError::FactorNotPositive(0.0))
    );
    assert_eq!(
        refused(-2.0).err(),
        Some(ModelScaleError::FactorNotPositive(-2.0))
    );
    assert_eq!(refused(1.0).err(), Some(ModelScaleError::FactorIsOne));
    assert_eq!(
        refused(1e9).err(),
        Some(ModelScaleError::FactorOutOfRange(1e9))
    );
}

#[test]
fn an_import_is_scaled_and_moved_with_the_model() {
    let mut model = model();
    let solid = evaluate(&model.document).body(model.base).unwrap().clone();
    let mut placement = BodyPlacement::default();
    placement.offset[0] = model.document.parse("30 mm").unwrap();
    let mut transaction = model.document.transaction("Import");
    let imported = transaction.add_feature(
        "Bracket",
        FeatureKind::Import(Import::new("bracket.step", solid, "text").placed(placement)),
    );
    model.document.apply(transaction.finish()).unwrap();

    let scaled = model
        .document
        .scaled(&scale(0.5, Point3::new(0.0, 0.0, 4.0), ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let after = evaluate(&model.document);
    let (low, high) = bounds(&after, imported);
    let import = model
        .document
        .feature(imported)
        .and_then(|feature| feature.kind.import())
        .unwrap();

    assert_eq!(after.failed_count(), 0);
    assert_eq!(text(&model.document, &import.placement.scale), "0.5");
    assert!(near(low, [15.0, 0.0, 2.0]), "{low:?}");
    assert!(near(high, [20.0, 4.0, 4.0]), "{high:?}");
}

#[test]
fn saved_views_follow_the_model() {
    let mut model = model();
    let view = SavedView {
        target: Point3::new(5.0, 4.0, 2.0),
        orientation: caditor_geometry::Rotation3::IDENTITY,
        distance: 50.0,
    };
    let views = SavedViews {
        named: vec![NamedView {
            name: "Front".to_owned(),
            view,
        }],
        home: None,
    };
    model
        .document
        .apply(Transaction::single(
            "Views",
            Edit::SetSavedViews {
                views: Box::new(views),
            },
        ))
        .unwrap();

    let scaled = model
        .document
        .scaled(&scale(2.0, Point3::ZERO, ScaledValues::Plain))
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let kept = &model.document.saved_views().named[0].view;

    assert!(near(kept.target, [10.0, 8.0, 4.0]));
    assert!((kept.distance - 100.0).abs() < 1e-9);
}
