use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Curve, EdgeReference, SamplingTolerance, Solid};
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

fn extruded(sketch: FeatureId, depth: &str) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(Expression::parse(depth, &|_| None).unwrap(), false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn model(path: Sketch, kind: impl FnOnce(FeatureId) -> PatternKind) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(bar()));
    let base = transaction.add_feature("Base", extruded(outline, "2 mm"));
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
        path: CurvePatternPath::Sketch(path),
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
            path: CurvePatternPath::Sketch(path),
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
            .reason()
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
    assert!(error.reason().contains("do not join end to end"));
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
    assert!(error.reason().contains("has no lone points"));
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

fn quarter_turn() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    sketch
}

#[test]
fn a_pattern_path_places_a_copy_at_a_distance_along_the_curve_and_reports_its_tangent() {
    let path = PatternPath::of(&quarter_turn()).unwrap();
    let near = path.copy_at(4.0, false, CopyOrientation::Kept).unwrap();
    let far = path
        .copy_at(14.0, false, CopyOrientation::Following)
        .unwrap();
    let back = path.copy_at(4.0, true, CopyOrientation::Kept).unwrap();

    assert_eq!(path.length(), 20.0);
    assert!(!path.is_closed());
    assert!((near.point - Point3::new(4.0, 0.0, 0.0)).length() < 1e-9);
    assert!((near.tangent - Vector3::X).length() < 1e-9);
    assert!((far.point - Point3::new(10.0, 4.0, 0.0)).length() < 1e-9);
    assert!((far.tangent - Vector3::Y).length() < 1e-9);
    assert!(
        (far.placement.apply_point(Point3::new(0.0, 0.0, 0.0)) - Point3::new(10.0, 4.0, 0.0))
            .length()
            < 1e-9
    );
    assert!((back.point - Point3::new(10.0, 6.0, 0.0)).length() < 1e-9);
    assert!((back.tangent + Vector3::Y).length() < 1e-9);
    assert!(PatternPath::of(&Sketch::new(Plane::XY)).is_none());
}

fn rectangle(from: Point2, to: Point2) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        from,
        Point2::new(to.x, from.y),
        to,
        Point2::new(from.x, to.y),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

struct Railed {
    document: Document,
    base: FeatureId,
    rail: FeatureId,
}

fn rail_model() -> Railed {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(bar()));
    let base = transaction.add_feature("Base", extruded(outline, "2 mm"));
    let footprint = transaction.add_feature(
        "Footprint",
        FeatureKind::from(rectangle(Point2::new(0.0, 10.0), Point2::new(30.0, 30.0))),
    );
    let rail = transaction.add_feature("Rail", extruded(footprint, "10 mm"));
    document.apply(transaction.finish()).unwrap();
    Railed {
        document,
        base,
        rail,
    }
}

fn edge_through(solid: &Solid, middle: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let curve = edge.curve();
            matches!(curve, Curve::Line(_))
                && curve.point(edge.interval().middle()).distance(middle) < 1e-9
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

fn rail_edges(railed: &Railed, middles: &[Point3]) -> Vec<EdgeReference> {
    let evaluation = evaluate(&railed.document);
    let solid = evaluation.body(railed.rail).unwrap();
    middles
        .iter()
        .map(|middle| edge_through(solid, *middle))
        .collect()
}

fn along_edges(
    railed: &mut Railed,
    edges: Vec<EdgeReference>,
    orientation: CopyOrientation,
) -> FeatureId {
    let kind = PatternKind::Curve(CurvePattern {
        path: CurvePatternPath::Edges {
            body: railed.rail,
            edges,
        },
        count: Expression::Number(3.0),
        spacing: Expression::parse("10 mm", &|_| None).unwrap(),
        measured: CurveSpacing::Spread,
        orientation,
        reversed: false,
    });
    let mut transaction = railed.document.transaction("Pattern");
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(railed.base, kind)),
    );
    railed.document.apply(transaction.finish()).unwrap();
    pattern
}

const ALONG_X: Point3 = Point3::new(15.0, 10.0, 0.0);
const ALONG_Y: Point3 = Point3::new(30.0, 20.0, 0.0);
const UPRIGHT: Point3 = Point3::new(30.0, 10.0, 5.0);

#[test]
fn a_curve_pattern_follows_a_chain_of_model_edges_kept_or_turned_with_it() {
    let mut railed = rail_model();
    let edges = rail_edges(&railed, &[ALONG_X, ALONG_Y]);
    let kept = along_edges(&mut railed, edges.clone(), CopyOrientation::Kept);

    let evaluation = evaluate(&railed.document);
    let bounds = evaluation
        .body(railed.base)
        .unwrap()
        .bounding_box()
        .unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, railed.base, 3.0);
    assert!((bounds.max().x - 32.0).abs() < 1e-6, "{bounds:?}");
    assert!((bounds.max().y - 21.0).abs() < 1e-6, "{bounds:?}");
    assert!(
        railed
            .document
            .dependents_of(&[railed.rail])
            .contains(&kept)
    );

    let mut turning = rail_model();
    let edges = rail_edges(&turning, &[ALONG_X, ALONG_Y]);
    along_edges(&mut turning, edges, CopyOrientation::Following);

    let evaluation = evaluate(&turning.document);
    let bounds = evaluation
        .body(turning.base)
        .unwrap()
        .bounding_box()
        .unwrap();

    assert_eq!(evaluation.failed_count(), 0);
    assert_copies(&evaluation, turning.base, 3.0);
    assert!((bounds.max().x - 41.0).abs() < 1e-6, "{bounds:?}");
    assert!((bounds.max().y - 32.0).abs() < 1e-6, "{bounds:?}");
}

#[test]
fn copies_along_edges_that_leave_a_plane_turn_with_the_tangent_into_space() {
    let railed = rail_model();
    let evaluation = evaluate(&railed.document);
    let solid = evaluation.body(railed.rail).unwrap();
    let edges = [ALONG_X, UPRIGHT].map(|middle| edge_through(solid, middle));
    let path = PatternPath::along_edges(solid, &edges).unwrap();

    let top = path
        .copy_at(40.0, false, CopyOrientation::Following)
        .unwrap();
    let back = path
        .copy_at(15.0, true, CopyOrientation::Following)
        .unwrap();

    assert!((path.length() - 40.0).abs() < 1e-9);
    assert!(!path.is_closed());
    assert!((top.point - Point3::new(30.0, 10.0, 10.0)).length() < 1e-9);
    assert!((top.tangent - Vector3::Z).length() < 1e-9);
    assert!((top.placement.apply_vector(Vector3::X) - Vector3::Z).length() < 1e-9);
    assert!((top.placement.apply_vector(Vector3::Y) - Vector3::Y).length() < 1e-9);
    assert!((top.placement.apply_point(Point3::new(0.0, 10.0, 0.0)) - top.point).length() < 1e-9);
    assert!((back.point - Point3::new(25.0, 10.0, 0.0)).length() < 1e-9);
    assert!((back.tangent + Vector3::X).length() < 1e-9);
    assert!((back.placement.apply_vector(Vector3::Z) - Vector3::X).length() < 1e-9);
}

#[test]
fn edges_that_do_not_chain_fail_the_pattern_with_the_fix_on_it() {
    let mut railed = rail_model();
    let edges = rail_edges(&railed, &[ALONG_X, Point3::new(15.0, 30.0, 0.0)]);
    let pattern = along_edges(&mut railed, edges, CopyOrientation::Kept);

    let evaluation = evaluate(&railed.document);
    let error = failure(&evaluation, pattern);

    assert!(
        error.reason().contains("do not join end to end"),
        "{}",
        error.reason()
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(pattern)));
    assert!(evaluation.body(railed.base).is_some());
}

#[test]
fn a_curve_pattern_along_no_edges_waits_for_its_path() {
    let mut railed = rail_model();
    let pattern = along_edges(&mut railed, Vec::new(), CopyOrientation::Kept);

    let evaluation = evaluate(&railed.document);

    assert!(matches!(
        evaluation.feature(pattern).unwrap().state,
        FeatureState::Waiting(MissingInput::Path)
    ));
}

#[test]
fn copies_round_a_closed_edge_share_its_length() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(bar()));
    let base = transaction.add_feature("Base", extruded(outline, "2 mm"));
    let mut round = Sketch::new(Plane::XY);
    round.add_circle(Point2::new(60.0, 0.0), 20.0);
    let disc = transaction.add_feature("Round", FeatureKind::from(round));
    let drum = transaction.add_feature("Drum", extruded(disc, "5 mm"));
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let solid = evaluation.body(drum).unwrap();
    let (rim, _) = solid
        .edges()
        .find(|(_, edge)| {
            matches!(edge.curve(), Curve::Circle(_))
                && edge.curve().point(edge.interval().start()).z.abs() < 1e-9
        })
        .unwrap();
    let edges = vec![EdgeReference::capture(solid, rim).unwrap()];
    let path = PatternPath::along_edges(solid, &edges).unwrap();
    assert!(path.is_closed());
    assert!((path.length() - 40.0 * std::f64::consts::PI).abs() < 1e-6);

    let mut transaction = document.transaction("Pattern");
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(
            base,
            PatternKind::Curve(CurvePattern {
                path: CurvePatternPath::Edges { body: drum, edges },
                count: Expression::Number(4.0),
                spacing: Expression::parse("10 mm", &|_| None).unwrap(),
                measured: CurveSpacing::Spread,
                orientation: CopyOrientation::Following,
                reversed: true,
            }),
        )),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(pattern)
    );
    assert_copies(&evaluation, base, 4.0);
}
