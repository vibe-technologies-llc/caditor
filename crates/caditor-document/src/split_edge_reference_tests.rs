use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{EdgeReference, Solid};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

use crate::*;

const CLOSE: f64 = 1e-6;

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn rectangle(min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
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

fn block() -> (Document, FeatureId, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle((0.0, 0.0), (10.0, 8.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(millimetres(4.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, outline, base)
}

fn edge_at(solid: &Solid, point: Point3) -> EdgeReference {
    let (id, _) = solid
        .edges()
        .find(|(_, edge)| {
            let parameter = edge.curve().closest_parameter(point, edge.interval());
            edge.curve().point(parameter).distance(point) < CLOSE
        })
        .unwrap();
    EdgeReference::capture(solid, id).unwrap()
}

fn front_top(document: &Document, base: FeatureId) -> EdgeReference {
    let evaluation = evaluate(document);
    edge_at(evaluation.body(base).unwrap(), Point3::new(5.0, 0.0, 4.0))
}

fn back_top(document: &Document, base: FeatureId) -> EdgeReference {
    let evaluation = evaluate(document);
    edge_at(evaluation.body(base).unwrap(), Point3::new(5.0, 8.0, 4.0))
}

fn sketch_of(document: &Document, outline: FeatureId) -> Sketch {
    document
        .feature(outline)
        .unwrap()
        .kind
        .sketch()
        .unwrap()
        .clone()
}

fn front_line_through(sketch: &Sketch, x: f64) -> EntityId {
    sketch
        .entities()
        .map(|(id, _)| id)
        .find(|id| {
            sketch.line_endpoints(*id).is_some_and(|(start, end)| {
                start.y.abs() < 1e-9
                    && end.y.abs() < 1e-9
                    && (start.x.min(end.x)..=start.x.max(end.x)).contains(&x)
            })
        })
        .unwrap()
}

fn notched(sketch: &Sketch, from: f64, to: f64) -> Sketch {
    let mut after = sketch.clone();
    after.add_line(Point2::new(from, 0.0), Point2::new(from, 2.0));
    after.add_line(Point2::new(from, 2.0), Point2::new(to, 2.0));
    after.add_line(Point2::new(to, 2.0), Point2::new(to, 0.0));
    let front = front_line_through(&after, 0.5 * (from + to));
    after
        .trim(front, Point2::new(0.5 * (from + to), 0.0))
        .unwrap();
    after
}

fn split_front_at(sketch: &Sketch, x: f64) -> Sketch {
    let mut after = sketch.clone();
    let front = front_line_through(&after, x);
    let point = after.add_point(Point2::new(x, 0.0));
    after.split_at(front, point).unwrap();
    after
}

fn moved_off_the_line(sketch: &Sketch) -> Sketch {
    let mut after = notched(sketch, 4.0, 6.0);
    let collinear: Vec<_> = after
        .constraints()
        .filter(|(_, constraint)| matches!(constraint, Constraint::Collinear(..)))
        .map(|(id, _)| id)
        .collect();
    for id in collinear {
        after.remove_constraint(id).unwrap();
    }
    let at_notch: Vec<_> = after
        .entities()
        .filter(|(_, entity)| {
            matches!(entity, Entity::Point(point) if point.distance(Point2::new(6.0, 0.0)) < 1e-9)
        })
        .map(|(id, _)| id)
        .collect();
    for id in at_notch {
        after
            .replace_entity(id, Entity::Point(Point2::new(6.0, -1.0)))
            .unwrap();
    }
    after
}

fn reshape_outline(document: &mut Document, outline: FeatureId, after: &Sketch) {
    let before = sketch_of(document, outline);
    let mut transaction = document.transaction("Reshape");
    transaction.reshape_sketch(outline, &before, after);
    document.apply(transaction.finish()).unwrap();
}

fn notch(document: &mut Document, outline: FeatureId, from: f64, to: f64) {
    let after = notched(&sketch_of(document, outline), from, to);
    reshape_outline(document, outline, &after);
}

fn split_front(document: &mut Document, outline: FeatureId, x: f64) {
    let after = split_front_at(&sketch_of(document, outline), x);
    reshape_outline(document, outline, &after);
}

fn move_piece_off_the_line(document: &mut Document, outline: FeatureId) {
    let after = moved_off_the_line(&sketch_of(document, outline));
    reshape_outline(document, outline, &after);
}

fn add(document: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let mut transaction = document.transaction("Add");
    let feature = transaction.add_feature(name, kind);
    document.apply(transaction.finish()).unwrap();
    feature
}

fn status(evaluation: &Evaluation, feature: FeatureId) -> &FeatureStatus {
    evaluation.feature(feature).unwrap()
}

fn reading(evaluation: &Evaluation, feature: FeatureId) -> f64 {
    let found = status(evaluation, feature);
    assert_eq!(found.state, FeatureState::UpToDate, "{found:?}");
    found
        .result
        .as_deref()
        .unwrap()
        .measurement()
        .unwrap()
        .value
        .value
}

fn point(evaluation: &Evaluation, feature: FeatureId) -> Point3 {
    let found = status(evaluation, feature);
    assert_eq!(found.state, FeatureState::UpToDate, "{found:?}");
    found
        .result
        .as_deref()
        .unwrap()
        .datum()
        .unwrap()
        .point()
        .unwrap()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &status(evaluation, feature).state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn leaves_its_references(evaluation: &Evaluation, feature: FeatureId, document: &Document) {
    if let Some(healing) = &status(evaluation, feature).healing {
        assert_eq!(healing.refreshed, document.feature(feature).unwrap().kind);
    }
}

fn length_of(document: &mut Document, base: FeatureId) -> FeatureId {
    let edge = front_top(document, base);
    add(
        document,
        "Measurement 1",
        FeatureKind::from(Measurement {
            reading: Reading::Of {
                quantity: Of::Length,
                item: MeasuredItem::Edge {
                    body: base,
                    edge: Box::new(edge),
                },
            },
            parameter: None,
        }),
    )
}

fn middle_of(document: &mut Document, base: FeatureId) -> FeatureId {
    let edge = front_top(document, base);
    add(
        document,
        "Middle",
        FeatureKind::Datum(Datum::PointBy(PointBy::EdgeMiddle {
            body: base,
            edge: Box::new(edge),
        })),
    )
}

fn station_on(document: &mut Document, base: FeatureId, name: &str, distance: f64) -> FeatureId {
    let edge = front_top(document, base);
    add(
        document,
        name,
        FeatureKind::Datum(Datum::PointBy(PointBy::Along(Box::new(CurveStation {
            body: base,
            edge: Box::new(edge),
            distance: millimetres(distance),
        })))),
    )
}

#[test]
fn a_measured_edge_reads_the_length_of_every_piece_a_notch_leaves_and_undoes() {
    let (mut document, outline, base) = block();
    let length = length_of(&mut document, base);
    let original = sketch_of(&document, outline);

    let before = reading(&evaluate(&document), length);
    reshape_outline(&mut document, outline, &notched(&original, 4.0, 6.0));
    let notched_once = evaluate(&document);
    let after_one = reading(&notched_once, length);
    notch(&mut document, outline, 7.0, 8.0);
    let after_two = reading(&evaluate(&document), length);
    reshape_outline(&mut document, outline, &original);
    let restored = reading(&evaluate(&document), length);

    assert!((before - 10.0).abs() < CLOSE, "{before}");
    assert!((after_one - 8.0).abs() < CLOSE, "{after_one}");
    assert!((after_two - 7.0).abs() < CLOSE, "{after_two}");
    assert!((restored - 10.0).abs() < CLOSE, "{restored}");
    leaves_its_references(&notched_once, length, &document);
}

#[test]
fn a_distance_from_a_notched_edge_is_read_from_its_nearest_piece() {
    let (mut document, outline, base) = block();
    let first = front_top(&document, base);
    let second = back_top(&document, base);
    let between = add(
        &mut document,
        "Measurement 1",
        FeatureKind::from(Measurement {
            reading: Reading::Between {
                quantity: Between::Distance,
                first: MeasuredItem::Edge {
                    body: base,
                    edge: Box::new(first),
                },
                second: MeasuredItem::Edge {
                    body: base,
                    edge: Box::new(second),
                },
            },
            parameter: None,
        }),
    );

    notch(&mut document, outline, 4.0, 6.0);
    let found = reading(&evaluate(&document), between);

    assert!((found - 8.0).abs() < CLOSE, "{found}");
}

#[test]
fn a_measured_edge_whose_piece_left_its_line_fails_in_words() {
    let (mut document, outline, base) = block();
    let length = length_of(&mut document, base);

    move_piece_off_the_line(&mut document, outline);
    let evaluation = evaluate(&document);
    let error = failure(&evaluation, length);

    assert!(
        error.reason.contains("split into pieces by a sketch edit"),
        "{}",
        error.reason
    );
    assert!(error.remedy.contains("Measure again"), "{}", error.remedy);
    leaves_its_references(&evaluation, length, &document);
}

#[test]
fn an_edge_middle_and_stations_run_on_across_an_edge_split_without_a_gap() {
    let (mut document, outline, base) = block();
    let middle = middle_of(&mut document, base);
    let stations = [
        station_on(&mut document, base, "Two along", 2.0),
        station_on(&mut document, base, "Seven along", 7.0),
        station_on(&mut document, base, "One back", -1.0),
    ];
    let whole = evaluate(&document);
    let before: Vec<Point3> = stations
        .iter()
        .map(|station| point(&whole, *station))
        .collect();

    split_front(&mut document, outline, 3.0);
    let split = evaluate(&document);
    let after: Vec<Point3> = stations
        .iter()
        .map(|station| point(&split, *station))
        .collect();
    let pieces = split
        .body(base)
        .unwrap()
        .edges()
        .filter(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            middle.y.abs() < CLOSE && (middle.z - 4.0).abs() < CLOSE
        })
        .count();

    assert_eq!(pieces, 2);
    assert!(
        point(&whole, middle).distance(Point3::new(5.0, 0.0, 4.0)) < CLOSE,
        "{:?}",
        point(&whole, middle)
    );
    assert!(
        point(&split, middle).distance(Point3::new(5.0, 0.0, 4.0)) < CLOSE,
        "{:?}",
        point(&split, middle)
    );
    for (before, after) in before.iter().zip(&after) {
        assert!(
            before.distance(*after) < CLOSE,
            "{before:?} became {after:?}"
        );
    }
    leaves_its_references(&split, middle, &document);
}

#[test]
fn an_edge_middle_and_a_station_on_a_notched_edge_fail_in_words_rather_than_take_one_piece() {
    let (mut document, outline, base) = block();
    let middle = middle_of(&mut document, base);
    let station = station_on(&mut document, base, "Two along", 2.0);

    notch(&mut document, outline, 4.0, 6.0);
    let evaluation = evaluate(&document);

    for feature in [middle, station] {
        let error = failure(&evaluation, feature);
        assert!(
            error
                .reason
                .contains("several pieces with gaps between them"),
            "{}",
            error.reason
        );
        assert!(
            error.remedy.contains("Choose the piece"),
            "{}",
            error.remedy
        );
        leaves_its_references(&evaluation, feature, &document);
    }
}

#[test]
fn a_station_on_an_edge_whose_piece_left_its_line_fails_in_words() {
    let (mut document, outline, base) = block();
    let station = station_on(&mut document, base, "Two along", 2.0);

    move_piece_off_the_line(&mut document, outline);
    let error = failure(&evaluate(&document), station);

    assert!(
        error.reason.contains("split into pieces by a sketch edit"),
        "{}",
        error.reason
    );
    assert!(
        error.remedy.contains("Choose the edge again"),
        "{}",
        error.remedy
    );
}

fn edge_set(document: &Document, base: FeatureId) -> SelectionSet {
    SelectionSet {
        name: "Front".to_owned(),
        members: vec![SetMember::Edge {
            body: base,
            edge: front_top(document, base),
        }],
    }
}

fn found_lengths(evaluation: &Evaluation, resolved: &SetResolution) -> Vec<f64> {
    let mut lengths: Vec<f64> = resolved
        .found
        .iter()
        .map(|item| match item {
            SetItem::Edge { body, edge } => {
                let edge = evaluation.body(*body).unwrap().edge(*edge).unwrap();
                edge.curve().length(edge.interval())
            }
            SetItem::Face { .. } => panic!("an edge set finds edges"),
        })
        .collect();
    lengths.sort_by(f64::total_cmp);
    lengths
}

#[test]
fn a_selection_set_edge_widens_to_every_piece_of_a_notched_edge() {
    let (mut document, outline, base) = block();
    let set = edge_set(&document, base);

    notch(&mut document, outline, 4.0, 6.0);
    let evaluation = evaluate(&document);
    let resolved = set.resolve(&document, &evaluation);
    let lengths = found_lengths(&evaluation, &resolved);

    assert!(resolved.lost.is_empty(), "{:?}", resolved.lost);
    assert_eq!(lengths.len(), 2);
    assert!(
        (lengths.iter().sum::<f64>() - 8.0).abs() < CLOSE,
        "{lengths:?}"
    );
}

#[test]
fn a_selection_set_edge_partly_traced_keeps_what_it_traced_and_reports_the_rest() {
    let (mut document, outline, base) = block();
    let set = edge_set(&document, base);

    move_piece_off_the_line(&mut document, outline);
    let evaluation = evaluate(&document);
    let resolved = set.resolve(&document, &evaluation);

    assert_eq!(resolved.lost, vec![SetLoss::Edge(base)]);
    assert!(!resolved.found.is_empty());
}

fn project_onto(
    document: &mut Document,
    base: FeatureId,
    reference: EdgeReference,
    plane: Plane,
) -> (FeatureId, EntityId) {
    let evaluation = evaluate(document);
    let solid = evaluation.body(base).unwrap();
    let edge = reference.resolve(solid).unwrap();
    let outline = edge_outline(solid, edge, &plane).unwrap();
    let mut transaction = document.transaction("Project");
    let side = transaction.add_feature("Side", FeatureKind::from(Sketch::new(plane)));
    let projected = transaction.add_projection(
        side,
        ProjectionSource::Edge {
            body: base,
            edge: reference,
        },
        &outline,
    );
    document.apply(transaction.finish()).unwrap();
    (side, projected)
}

fn project_front_top(document: &mut Document, base: FeatureId) -> (FeatureId, EntityId) {
    let reference = front_top(document, base);
    project_onto(document, base, reference, Plane::XZ)
}

fn top_pieces(document: &Document, base: FeatureId, keep: impl Fn(Point3) -> bool) -> usize {
    evaluate(document)
        .body(base)
        .unwrap()
        .edges()
        .filter(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle.z - 4.0).abs() < CLOSE && keep(middle)
        })
        .count()
}

fn solved(evaluation: &Evaluation, sketch: FeatureId) -> Sketch {
    let found = status(evaluation, sketch);
    assert_eq!(found.state, FeatureState::UpToDate, "{found:?}");
    found
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)
        .map(|result| result.geometry.clone())
        .unwrap()
}

fn ends_of(sketch: &Sketch, line: EntityId) -> [Point2; 2] {
    let (start, end) = sketch.line_endpoints(line).unwrap();
    [start, end]
}

fn on_side(x: f64, z: f64) -> Point2 {
    Plane::XZ.to_local(Point3::new(x, 0.0, z))
}

#[test]
fn a_projected_edge_spans_every_piece_of_an_edge_split_without_a_gap() {
    let (mut document, outline, base) = block();
    let (side, line) = project_front_top(&mut document, base);
    let whole = solved(&evaluate(&document), side);

    split_front(&mut document, outline, 3.0);
    let split = solved(&evaluate(&document), side);
    let [start, end] = ends_of(&split, line);

    assert_eq!(
        top_pieces(&document, base, |middle| middle.y.abs() < CLOSE),
        2
    );
    assert!(split.is_projected(line));
    assert_eq!(whole.entity(line), split.entity(line));
    assert!(start.distance(on_side(0.0, 4.0)) < CLOSE, "{start}");
    assert!(end.distance(on_side(10.0, 4.0)) < CLOSE, "{end}");
}

#[test]
fn a_projected_edge_of_a_notched_edge_fails_in_words_rather_than_take_one_piece() {
    let (mut document, outline, base) = block();
    let (side, line) = project_front_top(&mut document, base);

    notch(&mut document, outline, 4.0, 6.0);
    let evaluation = evaluate(&document);
    let error = failure(&evaluation, side);

    assert!(
        error.reason.contains("cut into parts which no longer join"),
        "{}",
        error.reason
    );
    assert!(error.remedy.contains("delete"), "{}", error.remedy);
    assert_eq!(error.fix, Some(FixTarget::Feature(side)));
    let FeatureKind::Sketch(definition) = &document.feature(side).unwrap().kind else {
        panic!("a sketch");
    };
    assert!(definition.projections.contains_key(&line));
    leaves_its_references(&evaluation, side, &document);
}

#[test]
fn a_projected_edge_comes_back_when_its_notch_is_undone() {
    let (mut document, outline, base) = block();
    let (side, line) = project_front_top(&mut document, base);
    let original = sketch_of(&document, outline);

    reshape_outline(&mut document, outline, &notched(&original, 4.0, 6.0));
    let failed = matches!(
        status(&evaluate(&document), side).state,
        FeatureState::Failed(_)
    );
    reshape_outline(&mut document, outline, &original);
    let restored = solved(&evaluate(&document), side);
    let [start, end] = ends_of(&restored, line);

    assert!(failed);
    assert!(start.distance(on_side(0.0, 4.0)) < CLOSE, "{start}");
    assert!(end.distance(on_side(10.0, 4.0)) < CLOSE, "{end}");
}

#[test]
fn a_projected_edge_whose_piece_left_its_line_fails_in_words() {
    let (mut document, outline, base) = block();
    let (side, _) = project_front_top(&mut document, base);

    move_piece_off_the_line(&mut document, outline);
    let error = failure(&evaluate(&document), side);

    assert!(
        error
            .reason
            .contains("only some of the parts can still be traced"),
        "{}",
        error.reason
    );
}

fn half_disc() -> (Document, FeatureId, FeatureId) {
    let mut document = Document::default();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(-5.0, 0.0), Point2::new(5.0, 0.0));
    sketch.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(5.0, 0.0),
        Point2::new(-5.0, 0.0),
    );
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(sketch));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(millimetres(4.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, outline, base)
}

#[test]
fn a_projected_arc_spans_every_piece_of_an_arc_split_in_two() {
    let (mut document, outline, base) = half_disc();
    let evaluation = evaluate(&document);
    let reference = edge_at(evaluation.body(base).unwrap(), Point3::new(0.0, 5.0, 4.0));
    let (side, arc) = project_onto(&mut document, base, reference, Plane::XY);
    let whole = solved(&evaluate(&document), side);
    let wanted = whole.arc(arc).unwrap();

    let mut after = sketch_of(&document, outline);
    let curve = after
        .entities()
        .map(|(id, _)| id)
        .find(|id| after.arc(*id).is_some())
        .unwrap();
    let middle = after.add_point(Point2::new(0.0, 5.0));
    after.split_at(curve, middle).unwrap();
    reshape_outline(&mut document, outline, &after);
    let split = solved(&evaluate(&document), side);
    let found = split.arc(arc).unwrap();

    assert_eq!(top_pieces(&document, base, |middle| middle.y > CLOSE), 2);
    assert!(split.is_projected(arc));
    assert!(found.center.distance(wanted.center) < CLOSE);
    assert!(
        found
            .point_at(found.start_angle)
            .distance(wanted.point_at(wanted.start_angle))
            < CLOSE
    );
    assert!((found.sweep - wanted.sweep).abs() < CLOSE);
}

fn traced(document: &Document, base: FeatureId, reference: &EdgeReference) -> TracedEdge {
    let evaluation = evaluate(document);
    trace_edge(document, evaluation.body(base).unwrap(), reference)
}

#[test]
fn a_traced_edge_is_every_piece_a_notch_leaves_and_is_partly_traced_when_one_left_its_line() {
    let (mut document, outline, base) = block();
    let reference = front_top(&document, base);

    let whole = traced(&document, base, &reference);
    notch(&mut document, outline, 4.0, 6.0);
    let notched = traced(&document, base, &reference);
    let (mut other, other_outline, other_base) = block();
    move_piece_off_the_line(&mut other, other_outline);
    let moved = traced(&other, other_base, &reference);

    assert!(matches!(whole, TracedEdge::Found(pieces) if pieces.len() == 1));
    assert!(matches!(notched, TracedEdge::Found(pieces) if pieces.len() == 2));
    assert_eq!(moved, TracedEdge::PartlyTraced);
}
