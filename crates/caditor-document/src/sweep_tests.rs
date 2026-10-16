use std::f64::consts::{FRAC_PI_2, PI};

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Curve, EdgeReference, SamplingTolerance, Solid, SweepOrientation};
use caditor_sketch::{EntityId, Sketch};

use crate::*;

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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

fn upright() -> Plane {
    Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap()
}

fn disc(radius: f64) -> Sketch {
    let mut sketch = Sketch::new(upright());
    sketch.add_circle(Point2::ZERO, radius);
    sketch
}

fn bent_path() -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
    let arc = sketch.add_arc(
        Point2::new(5.0, 10.0),
        Point2::new(5.0, 15.0),
        Point2::new(0.0, 10.0),
    );
    (sketch, vec![line, arc])
}

fn swept(sketch: FeatureId, path: SweepPath, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Sweep(Sweep {
        operation,
        ..Sweep::new(sketch, path)
    }))
}

struct Model {
    document: Document,
    path_sketch: FeatureId,
    curves: Vec<EntityId>,
    sweep: FeatureId,
}

fn tube(radius: f64) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature("Profile", FeatureKind::from(disc(radius)));
    let (path, curves) = bent_path();
    let path_sketch = transaction.add_feature("Path", FeatureKind::from(path));
    let sweep = transaction.add_feature(
        "Sweep 1",
        swept(
            profile,
            SweepPath::Curves {
                sketch: path_sketch,
                curves: curves.clone(),
            },
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        path_sketch,
        curves,
        sweep,
    }
}

#[test]
fn a_sweep_along_a_sketch_path_makes_a_tube_of_the_profile_times_the_path_length() {
    let model = tube(1.0);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let length = 10.0 + 5.0 * FRAC_PI_2;
    assert!((volume(&evaluation, model.sweep) - PI * length).abs() < 0.05);
    assert!(
        model
            .document
            .feature(model.sweep)
            .unwrap()
            .kind
            .features()
            .contains(&model.path_sketch)
    );
}

#[test]
fn a_curve_of_the_path_cannot_be_deleted_while_the_sweep_follows_it() {
    let mut model = tube(1.0);
    let mut transaction = model.document.transaction("Delete");
    transaction.remove_sketch_items(model.path_sketch, [model.curves[0]], []);

    let refused = model.document.apply(transaction.finish());

    assert!(
        matches!(refused, Err(EditError::EntityInUse { .. })),
        "{refused:?}"
    );
}

#[test]
fn a_sweep_with_no_path_waits_asking_for_one() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature("Profile", FeatureKind::from(disc(1.0)));
    let sweep = transaction.add_feature(
        "Sweep 1",
        swept(profile, SweepPath::Unchosen, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.feature(sweep).unwrap().state,
        FeatureState::Waiting(MissingInput::Path)
    );
    assert!(
        MissingInput::Path
            .prompt(&document)
            .contains("Choose the curves")
    );
}

#[test]
fn a_profile_too_large_for_the_bend_fails_naming_the_curve() {
    let model = tube(6.0);

    let error = failure(&evaluate(&model.document), model.sweep);

    assert!(
        error.reason().contains("too large for the bend of") && error.reason().contains("of Path"),
        "{}",
        error.reason()
    );
}

#[test]
fn a_sharp_corner_onto_an_arc_fails_saying_how_to_round_it() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature("Profile", FeatureKind::from(disc(0.5)));
    let mut path = Sketch::new(Plane::XY);
    let line = path.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
    let arc = path.add_arc(
        Point2::new(0.0, 15.0),
        Point2::new(0.0, 10.0),
        Point2::new(5.0, 15.0),
    );
    let path_sketch = transaction.add_feature("Path", FeatureKind::from(path));
    let sweep = transaction.add_feature(
        "Sweep 1",
        swept(
            profile,
            SweepPath::Curves {
                sketch: path_sketch,
                curves: vec![line, arc],
            },
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let error = failure(&evaluate(&document), sweep);

    assert!(
        error.reason().contains("sharp corner"),
        "{}",
        error.reason()
    );
    assert!(error.remedy().contains("fillet"), "{}", error.remedy());
    assert_eq!(error.fix, Some(FixTarget::Feature(path_sketch)));
}

fn block(document: &mut Document) -> (FeatureId, FeatureId) {
    let mut transaction = document.transaction("Block");
    let mut outline = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-5.0, -5.0),
        Point2::new(5.0, -5.0),
        Point2::new(5.0, 20.0),
        Point2::new(-5.0, 20.0),
    ];
    for index in 0..4 {
        outline.add_line(corners[index], corners[(index + 1) % 4]);
    }
    let outline = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("10 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (outline, base)
}

#[test]
fn a_removing_sweep_cuts_a_groove_along_its_path() {
    let mut document = Document::default();
    let (_, base) = block(&mut document);
    let mut transaction = document.transaction("Groove");
    let mut profile = Sketch::new(
        Plane::from_frame(Point3::new(0.0, -6.0, 10.0), Vector3::Y, Vector3::X).unwrap(),
    );
    profile.add_circle(Point2::ZERO, 1.0);
    let profile = transaction.add_feature("Groove profile", FeatureKind::from(profile));
    let mut path = Sketch::new(
        Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
    );
    let line = path.add_line(Point2::new(0.0, -6.0), Point2::new(0.0, 30.0));
    let path = transaction.add_feature("Groove path", FeatureKind::from(path));
    let groove = transaction.add_feature(
        "Groove",
        swept(
            profile,
            SweepPath::Curves {
                sketch: path,
                curves: vec![line],
            },
            BodyOperation::Remove(base),
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(groove)
    );
    let expected = 10.0 * 25.0 * 10.0 - 0.5 * PI * 25.0;
    assert!((volume(&evaluation, base) - expected).abs() < 0.05);
}

fn top_long_edge(solid: &Solid) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let curve = edge.curve();
            let middle = curve.point(edge.interval().middle());
            matches!(curve, Curve::Line(_))
                && (middle.z - 10.0).abs() < 1e-9
                && (middle.x - 5.0).abs() < 1e-9
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

#[test]
fn a_sweep_follows_an_edge_of_a_body_by_name() {
    let mut document = Document::default();
    let (_, base) = block(&mut document);
    let evaluation = evaluate(&document);
    let edge = top_long_edge(evaluation.body(base).unwrap());
    let mut transaction = document.transaction("Rail");
    let mut profile = Sketch::new(
        Plane::from_frame(Point3::new(5.0, -5.0, 10.0), Vector3::Y, Vector3::X).unwrap(),
    );
    let corners = [
        Point2::new(-1.0, -1.0),
        Point2::new(1.0, -1.0),
        Point2::new(1.0, 1.0),
        Point2::new(-1.0, 1.0),
    ];
    for index in 0..4 {
        profile.add_line(corners[index], corners[(index + 1) % 4]);
    }
    let profile = transaction.add_feature("Rail profile", FeatureKind::from(profile));
    let rail = transaction.add_feature(
        "Rail",
        swept(
            profile,
            SweepPath::Edges {
                body: base,
                edges: vec![edge],
            },
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(rail)
    );
    assert!((volume(&evaluation, rail) - 4.0 * 25.0).abs() < 1e-6);
    let kind = &document.feature(rail).unwrap().kind;
    assert!(kind.bodies_used().contains(&base));
}

#[test]
fn a_parallel_sweep_keeps_the_profile_upright_along_a_slanted_line() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature("Profile", FeatureKind::from(disc(1.0)));
    let mut path = Sketch::new(Plane::XY);
    let line = path.add_line(Point2::ZERO, Point2::new(6.0, 8.0));
    let path = transaction.add_feature("Path", FeatureKind::from(path));
    let sweep = transaction.add_feature(
        "Sweep 1",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            orientation: SweepOrientation::Parallel,
            ..Sweep::new(
                profile,
                SweepPath::Curves {
                    sketch: path,
                    curves: vec![line],
                },
            )
        })),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, sweep) - PI * 8.0).abs() < 0.05);
}

#[test]
fn inserting_a_sweep_is_one_undoable_change() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketches");
    let profile = transaction.add_feature("Profile", FeatureKind::from(disc(1.0)));
    let (path, curves) = bent_path();
    let path = transaction.add_feature("Path", FeatureKind::from(path));
    document.apply(transaction.finish()).unwrap();
    let mut editor = Editor::new(document);
    let is_sweep = |feature: &Feature| matches!(feature.kind.solid(), Some(SolidFeature::Sweep(_)));

    let mut transaction = editor.document().transaction("Sweep");
    transaction.add_feature(
        "Sweep 1",
        swept(
            profile,
            SweepPath::Curves {
                sketch: path,
                curves,
            },
            BodyOperation::NewBody,
        ),
    );
    let made = transaction.finish();
    editor.apply(made).unwrap();
    let swept_count = editor
        .document()
        .features()
        .filter(|feature| is_sweep(feature))
        .count();
    editor.undo().unwrap();

    assert_eq!(swept_count, 1);
    assert!(!editor.document().features().any(is_sweep));
}

fn square_profile() -> Sketch {
    let mut sketch = Sketch::new(upright());
    let corners = [
        Point2::new(-1.0, -1.0),
        Point2::new(1.0, -1.0),
        Point2::new(1.0, 1.0),
        Point2::new(-1.0, 1.0),
    ];
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % corners.len()]);
    }
    sketch
}

fn straight_path() -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(0.0, 10.0));
    (sketch, vec![line])
}

fn circle_path() -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(-5.0, 0.0), 5.0);
    (sketch, vec![circle])
}

fn stored(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

struct Shaped {
    document: Document,
    sweep: FeatureId,
    twist: ParameterId,
}

fn shaped(path: (Sketch, Vec<EntityId>), twist: &str, scale: &str) -> Shaped {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let twist = transaction.add_parameter("spin", stored(twist));
    let profile = transaction.add_feature("Profile", FeatureKind::from(square_profile()));
    let (path, curves) = path;
    let path_sketch = transaction.add_feature("Path", FeatureKind::from(path));
    let sweep = transaction.add_feature(
        "Sweep 1",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            twist: Expression::Parameter(twist),
            scale: stored(scale),
            ..Sweep::new(
                profile,
                SweepPath::Curves {
                    sketch: path_sketch,
                    curves,
                },
            )
        })),
    );
    document.apply(transaction.finish()).unwrap();
    Shaped {
        document,
        sweep,
        twist,
    }
}

#[test]
fn a_twisted_sweep_keeps_the_area_times_the_length_as_its_twist_parameter_changes() {
    let mut model = shaped(straight_path(), "90 deg", "1");

    let quarter = evaluate(&model.document);
    let mut transaction = model.document.transaction("Spin");
    transaction.edit(Edit::SetParameterExpression {
        id: model.twist,
        expression: stored("2 * 360 deg"),
    });
    model.document.apply(transaction.finish()).unwrap();
    let doubled = evaluate(&model.document);

    assert_eq!(quarter.failed_count(), 0);
    assert_eq!(doubled.failed_count(), 0);
    assert!((volume(&quarter, model.sweep) - 40.0).abs() < 0.01);
    assert!((volume(&doubled, model.sweep) - 40.0).abs() < 0.01);
    assert!(
        model
            .document
            .feature(model.sweep)
            .unwrap()
            .kind
            .uses_parameter(model.twist)
    );
}

#[test]
fn a_scaled_sweep_along_a_line_is_a_frustum() {
    let model = shaped(straight_path(), "0 deg", "0.5");

    let evaluation = evaluate(&model.document);

    let frustum = 10.0 / 3.0 * (4.0 + 1.0 + 2.0);
    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.sweep) - frustum).abs() < 0.01);
}

#[test]
fn a_closed_path_brought_back_turned_names_the_twist_that_closes_it() {
    let model = shaped(circle_path(), "90 deg", "1");
    let scaled = shaped(circle_path(), "0 deg", "2");

    let error = failure(&evaluate(&model.document), model.sweep);
    let refused = failure(&evaluate(&scaled.document), scaled.sweep);

    assert!(
        error.reason().contains("turned by 90°"),
        "{}",
        error.reason()
    );
    assert!(
        error.remedy().contains("Set the twist to 0°"),
        "{}",
        error.remedy()
    );
    assert!(
        refused.remedy().contains("end scale to 1"),
        "{}",
        refused.remedy()
    );
}

#[test]
fn an_end_scale_of_zero_or_a_twist_given_as_a_length_fails_in_words() {
    let flat = shaped(straight_path(), "0 deg", "0");
    let long = shaped(straight_path(), "5 mm", "1");

    let flattened = failure(&evaluate(&flat.document), flat.sweep);
    let lengthened = failure(&evaluate(&long.document), long.sweep);

    assert!(
        flattened.reason().contains("must be more than zero"),
        "{}",
        flattened.reason()
    );
    assert!(
        lengthened.remedy().contains("gives an angle"),
        "{}",
        lengthened.remedy()
    );
}
