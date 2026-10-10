use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::{EntityId, Sketch};

use crate::*;

fn bar() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-2.0, -1.0),
        Point2::new(2.0, -1.0),
        Point2::new(2.0, 1.0),
        Point2::new(-2.0, 1.0),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

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

fn assert_copies(evaluation: &Evaluation, body: FeatureId, copies: f64) {
    let found = volume(evaluation, body);
    let expected = 16.0 * copies;
    assert!(
        (found - expected).abs() < 0.01 * expected,
        "volume {found} instead of {expected}"
    );
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

struct Model {
    document: Document,
    base: FeatureId,
    path: FeatureId,
    pattern: FeatureId,
}

fn model(path: Sketch, kind: impl FnOnce(FeatureId) -> PatternKind) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(bar()));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse("2 mm", &|_| None).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    let path = transaction.add_feature("Path", FeatureKind::from(path));
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(base, kind(path))),
    );
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        base,
        path,
        pattern,
    }
}

fn quarter_arc() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_arc(
        Point2::new(30.0, 0.0),
        Point2::new(0.0, 0.0),
        Point2::new(30.0, -30.0),
    );
    sketch
}

fn along(path: FeatureId, count: &str, orientation: CopyOrientation) -> PatternKind {
    PatternKind::Curve(CurvePattern {
        sketch: path,
        count: Expression::parse(count, &|_| None).unwrap(),
        spacing: Expression::parse("10 mm", &|_| None).unwrap(),
        measured: CurveSpacing::Spread,
        orientation,
        reversed: false,
    })
}

fn set_kind(model: &mut Model, kind: PatternKind) {
    let pattern = Pattern::new(model.base, kind);
    model
        .document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: model.pattern,
                kind: FeatureKind::from(pattern),
            },
        ))
        .unwrap();
}

fn lowest_y(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .bounding_box()
        .unwrap()
        .min()
        .y
}

#[test]
fn a_curve_pattern_spreads_copies_along_an_arc_kept_or_turned_with_it() {
    let mut model = model(quarter_arc(), |path| {
        along(path, "3", CopyOrientation::Kept)
    });
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 3.0);
    assert!((lowest_y(&evaluation, model.base) + 31.0).abs() < 1e-6);
    assert!(
        model
            .document
            .dependents_of(&[model.path])
            .contains(&model.pattern)
    );

    let path = model.path;
    set_kind(&mut model, along(path, "3", CopyOrientation::Following));
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 3.0);
    assert!((lowest_y(&evaluation, model.base) + 32.0).abs() < 1e-6);

    let solid = evaluation.body(model.base).unwrap();
    let copied = solid
        .faces()
        .filter_map(|(_, face)| face.origin()?.copy())
        .filter(|copy| copy.pattern == model.pattern.raw())
        .map(|copy| copy.index)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(copied.into_iter().collect::<Vec<_>>(), [[1, 0], [2, 0]]);
}

#[test]
fn a_set_distance_along_a_chain_of_lines_stops_at_its_end() {
    let mut chain = Sketch::new(Plane::XY);
    chain.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    chain.add_line(Point2::new(20.0, 20.0), Point2::new(20.0, 0.0));
    let mut model = model(chain, |path| {
        let PatternKind::Curve(curve) = along(path, "3", CopyOrientation::Kept) else {
            panic!("along makes a curve pattern");
        };
        PatternKind::Curve(CurvePattern {
            measured: CurveSpacing::Distance,
            ..curve
        })
    });
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 3.0);
    let bounds = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert!((bounds.max().x - 22.0).abs() < 1e-6);
    assert!((bounds.max().y - 1.0).abs() < 1e-6);

    let path = model.path;
    set_kind(
        &mut model,
        PatternKind::Curve(CurvePattern {
            sketch: path,
            count: Expression::Number(6.0),
            spacing: Expression::parse("10 mm", &|_| None).unwrap(),
            measured: CurveSpacing::Distance,
            orientation: CopyOrientation::Kept,
            reversed: false,
        }),
    );
    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.pattern);
    assert!(
        error
            .reason
            .contains("past the end of the curve, which is 40 mm long")
    );
    assert_eq!(evaluation.failed_count(), 1);
}

#[test]
fn a_closed_curve_shares_its_length_among_all_the_copies() {
    let mut circle = Sketch::new(Plane::XY);
    circle.add_circle(Point2::new(30.0, 0.0), 30.0);
    let model = model(circle, |path| along(path, "4", CopyOrientation::Kept));
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 4.0);
    let bounds = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert!((bounds.min().x + 62.0).abs() < 1e-6);
}

#[test]
fn a_path_that_branches_fails_the_pattern_with_the_fix_on_the_sketch() {
    let mut branching = quarter_arc();
    branching.add_line(Point2::new(0.0, 0.0), Point2::new(-10.0, 0.0));
    branching.add_line(Point2::new(0.0, 0.0), Point2::new(0.0, 10.0));
    let model = model(branching, |path| along(path, "3", CopyOrientation::Kept));
    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.pattern);
    assert!(error.reason.contains("do not join end to end"));
    assert_eq!(error.fix, Some(FixTarget::Feature(model.path)));
    assert!(evaluation.body(model.base).is_some());
}

fn points() -> (Sketch, [EntityId; 3]) {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(10.0, 10.0));
    let second = sketch.add_point(Point2::new(-10.0, 10.0));
    let at_base = sketch.add_point(Point2::new(0.0, 0.0));
    sketch.add_line(Point2::new(0.0, 20.0), Point2::new(20.0, 20.0));
    (sketch, [first, second, at_base])
}

#[test]
fn a_point_pattern_places_a_copy_at_each_lone_point_named_after_it() {
    let (sketch, [first, second, _]) = points();
    let mut model = model(sketch, |path| {
        PatternKind::Points(PointsPattern {
            sketch: path,
            base: PointReference::Origin,
        })
    });
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 3.0);
    let solid = evaluation.body(model.base).unwrap();
    let copied = solid
        .faces()
        .find_map(|(_, face)| face.origin().filter(|origin| origin.copy().is_some()));
    let words = describe_origin(&model.document, copied);
    assert!(words.starts_with("Pattern 1 copy at point "), "{words}");
    let index = point_instance(first).unwrap();
    assert_eq!(instance_point(index), Some(first));

    let mut skipping = model
        .document
        .feature(model.pattern)
        .unwrap()
        .kind
        .pattern()
        .unwrap()
        .clone();
    skipping.skipped.insert(point_instance(second).unwrap());
    model
        .document
        .apply(Transaction::single(
            "Leave one out",
            Edit::SetFeatureKind {
                id: model.pattern,
                kind: FeatureKind::from(skipping),
            },
        ))
        .unwrap();
    let evaluation = evaluate(&model.document);
    assert_copies(&evaluation, model.base, 2.0);
    let solid = evaluation.body(model.base).unwrap();
    assert!(
        solid
            .faces()
            .filter_map(|(_, face)| face.origin()?.copy())
            .all(|copy| copy.index == index)
    );
}

#[test]
fn a_point_pattern_needs_lone_points_and_refuses_a_feature_that_is_no_sketch() {
    let mut model = model(quarter_arc(), |path| {
        PatternKind::Points(PointsPattern {
            sketch: path,
            base: PointReference::Origin,
        })
    });
    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.pattern);
    assert!(error.reason.contains("has no lone points"));
    assert_eq!(error.fix, Some(FixTarget::Feature(model.path)));

    let base = model.base;
    let refused = model.document.apply(Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: model.pattern,
            kind: FeatureKind::from(Pattern::new(
                base,
                PatternKind::Points(PointsPattern {
                    sketch: base,
                    base: PointReference::Origin,
                }),
            )),
        },
    ));
    assert!(refused.is_err());
}

#[test]
fn copies_along_a_spline_are_spaced_by_its_length() {
    let mut arch = Sketch::new(Plane::XY);
    arch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 20.0),
        Point2::new(20.0, 0.0),
    ]);
    let model = model(arch, |path| along(path, "3", CopyOrientation::Kept));
    let evaluation = evaluate(&model.document);
    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, model.base, 3.0);
    let bounds = evaluation.body(model.base).unwrap().bounding_box().unwrap();
    assert!((bounds.max().y - 11.0).abs() < 1e-6, "{bounds:?}");
    assert!((bounds.max().x - 22.0).abs() < 1e-6, "{bounds:?}");
}
