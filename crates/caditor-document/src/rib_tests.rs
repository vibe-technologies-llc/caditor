use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{FaceReference, PointClass, SamplingTolerance, Solid, Surface};
use caditor_sketch::Sketch;

use crate::*;

const BOX_VOLUME: f64 = 100.0 * 60.0 * 40.0 - 90.0 * 50.0 * 35.0;

fn rectangle(plane: Plane, min: (f64, f64), max: (f64, f64)) -> Sketch {
    let mut sketch = Sketch::new(plane);
    let corners = [
        Point2::new(min.0, min.1),
        Point2::new(max.0, min.1),
        Point2::new(max.0, max.1),
        Point2::new(min.0, max.1),
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

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn top_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| match face.surface() {
            Surface::Plane(plane) => plane.frame().normal() * face.sense().sign() == Vector3::Z,
            _ => false,
        })
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn open_box() -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (100.0, 60.0))),
    );
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(length("40 mm"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let open = vec![top_face(evaluate(&document).body(base).unwrap())];
    let mut transaction = document.transaction("Shell");
    transaction.add_feature(
        "Shell 1",
        FeatureKind::Shell(Shell {
            body: base,
            open,
            thickness: length("5 mm"),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );
    document.apply(transaction.finish()).unwrap();
    (document, base)
}

fn top_sketch(from: (f64, f64), to: (f64, f64)) -> Sketch {
    let plane = Plane::new(Point3::new(0.0, 0.0, 40.0), Vector3::Z).unwrap();
    let mut sketch = Sketch::new(plane);
    sketch.add_line(Point2::new(from.0, from.1), Point2::new(to.0, to.1));
    sketch
}

fn middle_sketch() -> Sketch {
    let plane =
        Plane::with_x_axis(Point3::new(0.0, 30.0, 0.0), Vector3::NEG_Y, Vector3::X).unwrap();
    let mut sketch = Sketch::new(plane);
    sketch.add_line(Point2::new(12.0, 25.0), Point2::new(33.0, 10.0));
    sketch
}

struct Ribbed {
    document: Document,
    body: FeatureId,
    rib: FeatureId,
}

fn ribbed(sketch: Sketch, change: impl FnOnce(&mut Rib)) -> Ribbed {
    let (mut document, body) = open_box();
    let mut transaction = document.transaction("Rib");
    let sketch = transaction.add_feature("Rib sketch", FeatureKind::from(sketch));
    let mut definition = Rib::new(sketch, body, length("4 mm"));
    change(&mut definition);
    let rib = transaction.add_feature("Rib 1", FeatureKind::Rib(definition));
    document.apply(transaction.finish()).unwrap();
    Ribbed {
        document,
        body,
        rib,
    }
}

fn web(definition: &mut Rib) {
    definition.direction = RibDirection::Square;
    definition.reversed = true;
}

#[test]
fn a_web_inside_an_open_box_runs_on_to_both_walls_and_down_to_the_floor() {
    let model = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let found = volume(&evaluation, model.body);
    let expected = BOX_VOLUME + 4.0 * 50.0 * 35.0;
    assert!(
        (found - expected).abs() < 1e-6 * expected,
        "{found} {expected}"
    );
    let kind = &model.document.feature(model.rib).unwrap().kind;
    assert_eq!(
        model.document.feature(model.rib).unwrap().body(),
        Some(model.body)
    );
    assert!(kind.reference_sketches().len() == 1);
}

#[test]
fn a_web_thickened_to_one_side_lies_left_or_right_of_its_line_walked_from_its_start() {
    let one = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        web(rib);
        rib.side = RibSide::OneSide;
    });
    let other = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        web(rib);
        rib.side = RibSide::OtherSide;
    });

    let one_evaluation = evaluate(&one.document);
    let other_evaluation = evaluate(&other.document);

    let expected = BOX_VOLUME + 4.0 * 50.0 * 35.0;
    for (evaluation, body) in [(&one_evaluation, one.body), (&other_evaluation, other.body)] {
        assert_eq!(evaluation.failed_count(), 0);
        assert!((volume(evaluation, body) - expected).abs() < 1e-6 * expected);
    }
    let holds = |evaluation: &Evaluation, body: FeatureId, x: f64| {
        evaluation
            .body(body)
            .unwrap()
            .classify_point(Point3::new(x, 30.0, 20.0))
            == PointClass::Inside
    };
    assert!(holds(&one_evaluation, one.body, 48.0));
    assert!(!holds(&one_evaluation, one.body, 52.0));
    assert!(holds(&other_evaluation, other.body, 52.0));
    assert!(!holds(&other_evaluation, other.body, 48.0));
}

#[test]
fn crossing_webs_of_one_sketch_each_run_on_to_the_walls() {
    let mut sketch = top_sketch((50.0, 15.0), (50.0, 45.0));
    sketch.add_line(Point2::new(30.0, 30.0), Point2::new(70.0, 30.0));
    let model = ribbed(sketch, web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let expected = BOX_VOLUME + 4.0 * 35.0 * (50.0 + 90.0 - 4.0);
    let found = volume(&evaluation, model.body);
    assert!(
        (found - expected).abs() < 1e-6 * expected,
        "{found} {expected}"
    );
}

#[test]
fn a_rib_parallel_to_its_sketch_fills_the_corner_between_wall_and_floor() {
    let model = ribbed(middle_sketch(), |rib| rib.reversed = true);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let found = volume(&evaluation, model.body);
    let expected = BOX_VOLUME + 0.5 * 35.0 * 25.0 * 4.0;
    assert!(
        (found - expected).abs() < 1e-6 * expected,
        "{found} {expected}"
    );
}

#[test]
fn the_side_a_rib_fills_and_the_way_a_web_runs_start_toward_the_body() {
    let rib = ribbed(middle_sketch(), |_| {});
    let web = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        rib.direction = RibDirection::Square;
    });

    let fills = rib_facing(
        &evaluate(&rib.document),
        rib.document.feature(rib.rib).unwrap().kind.rib().unwrap(),
        Some(rib.rib),
    );
    let runs = rib_facing(
        &evaluate(&web.document),
        web.document.feature(web.rib).unwrap().kind.rib().unwrap(),
        Some(web.rib),
    );

    assert_eq!(fills, Some(true));
    assert_eq!(runs, Some(true));
}

#[test]
fn a_rib_filling_the_open_side_fails_saying_it_runs_past_the_body() {
    let model = ribbed(middle_sketch(), |_| {});

    let error = failure(&evaluate(&model.document), model.rib);

    assert!(error.reason.contains("not closed off"), "{}", error.reason);
    assert!(error.remedy.contains("Reverse"), "{}", error.remedy);
    assert_eq!(error.fix, Some(FixTarget::Feature(model.rib)));
}

#[test]
fn a_web_running_out_of_the_box_meets_no_face_of_the_body() {
    let upward = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        rib.direction = RibDirection::Square;
    });
    let beside = ribbed(top_sketch((150.0, 15.0), (150.0, 45.0)), web);

    let upward = failure(&evaluate(&upward.document), upward.rib);
    let beside = failure(&evaluate(&beside.document), beside.rib);

    assert!(upward.reason.contains("meets no face"), "{}", upward.reason);
    assert!(beside.reason.contains("meets no face"), "{}", beside.reason);
}

fn extruded(sketch: FeatureId, height: &str, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(length(height), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

#[test]
fn a_web_whose_line_runs_out_past_the_body_fails_naming_that_end() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let plate = transaction.add_feature(
        "Plate",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (100.0, 60.0))),
    );
    let base = transaction.add_feature("Base", extruded(plate, "5 mm", BodyOperation::NewBody));
    let upright = transaction.add_feature(
        "Upright",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (5.0, 60.0))),
    );
    transaction.add_feature("Wall", extruded(upright, "40 mm", BodyOperation::Add(base)));
    let sketch = transaction.add_feature(
        "Rib sketch",
        FeatureKind::from(top_sketch((20.0, 30.0), (60.0, 30.0))),
    );
    let mut definition = Rib::new(sketch, base, length("4 mm"));
    web(&mut definition);
    let rib = transaction.add_feature("Rib 1", FeatureKind::Rib(definition));
    document.apply(transaction.finish()).unwrap();

    let error = failure(&evaluate(&document), rib);

    assert!(
        error.reason.contains("The end of Rib 1 at Line") && error.reason.contains("runs on past"),
        "{}",
        error.reason
    );
}

#[test]
fn a_closed_outline_fails_with_the_fix_on_the_sketch() {
    let (mut document, body) = open_box();
    let mut transaction = document.transaction("Rib");
    let top = Plane::new(Point3::new(0.0, 0.0, 40.0), Vector3::Z).unwrap();
    let outline = rectangle(top, (20.0, 20.0), (30.0, 30.0));
    let sketch = transaction.add_feature("Rib sketch", FeatureKind::from(outline));
    let mut definition = Rib::new(sketch, body, length("4 mm"));
    web(&mut definition);
    let rib = transaction.add_feature("Rib 1", FeatureKind::Rib(definition));
    document.apply(transaction.finish()).unwrap();

    let error = failure(&evaluate(&document), rib);

    assert!(
        error.reason.contains("close on themselves"),
        "{}",
        error.reason
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(sketch)));
}

#[test]
fn a_thickness_of_zero_fails_in_words() {
    let model = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        web(rib);
        rib.thickness = length("0 mm");
    });

    let error = failure(&evaluate(&model.document), model.rib);

    assert!(
        error
            .reason
            .contains("thickness of the rib must be more than zero"),
        "{}",
        error.reason
    );
}

#[test]
fn an_arched_web_runs_its_arc_on_along_its_end_tangents_and_reads_by_its_curve() {
    let plane = Plane::new(Point3::new(0.0, 0.0, 30.0), Vector3::Z).unwrap();
    let mut arch = Sketch::new(plane);
    arch.add_arc(
        Point2::new(50.0, 30.0),
        Point2::new(60.0, 30.0),
        Point2::new(40.0, 30.0),
    );
    let model = ribbed(arch, web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let expected = BOX_VOLUME + 4.0 * 25.0 * (10.0 * std::f64::consts::PI + 2.0 * 25.0);
    let found = volume(&evaluation, model.body);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} {expected}"
    );
    let solid = evaluation.body(model.body).unwrap();
    let words: Vec<String> = solid
        .faces()
        .filter_map(|(_, face)| face.origin())
        .filter(|origin| origin.feature() == model.rib.raw())
        .map(|origin| describe_origin(&model.document, Some(origin)))
        .collect();
    assert!(
        words.iter().any(|word| word == "Rib 1 top face"),
        "{words:?}"
    );
    assert!(
        words
            .iter()
            .any(|word| word.starts_with("Rib 1 side from Arc")),
        "{words:?}"
    );
    assert!(
        words
            .iter()
            .any(|word| word.starts_with("Rib 1 side carried on from Arc")),
        "{words:?}"
    );
}

fn assert_volume(evaluation: &Evaluation, body: FeatureId, expected: f64) {
    let found = volume(evaluation, body);
    assert!(
        (found - expected).abs() < 1e-6 * expected,
        "{found} {expected}"
    );
}

#[test]
fn a_web_meeting_another_in_a_t_runs_on_only_until_it_meets_that_web() {
    let mut split = top_sketch((50.0, 15.0), (50.0, 30.0));
    split.add_line(Point2::new(50.0, 30.0), Point2::new(50.0, 45.0));
    split.add_line(Point2::new(50.0, 30.0), Point2::new(70.0, 30.0));
    let mut whole = top_sketch((50.0, 15.0), (50.0, 45.0));
    whole.add_line(Point2::new(50.0, 30.0), Point2::new(70.0, 30.0));

    for sketch in [split, whole] {
        let model = ribbed(sketch, web);

        let evaluation = evaluate(&model.document);

        assert_eq!(evaluation.failed_count(), 0);
        assert_volume(
            &evaluation,
            model.body,
            BOX_VOLUME + 4.0 * 35.0 * (50.0 + 43.0),
        );
        let solid = evaluation.body(model.body).unwrap();
        assert_eq!(
            solid.classify_point(Point3::new(30.0, 30.0, 20.0)),
            PointClass::Outside
        );
    }
}

#[test]
fn a_web_thickened_to_one_side_meets_a_t_from_either_side() {
    let toward = |end: f64| {
        let mut sketch = top_sketch((50.0, 15.0), (50.0, 45.0));
        sketch.add_line(Point2::new(50.0, 30.0), Point2::new(end, 30.0));
        ribbed(sketch, |rib| {
            web(rib);
            rib.side = RibSide::OneSide;
        })
    };
    let beside = toward(70.0);
    let behind = toward(20.0);

    let beside_evaluation = evaluate(&beside.document);
    let behind_evaluation = evaluate(&behind.document);

    assert_eq!(beside_evaluation.failed_count(), 0);
    assert_eq!(behind_evaluation.failed_count(), 0);
    assert_volume(
        &beside_evaluation,
        beside.body,
        BOX_VOLUME + 4.0 * 35.0 * (50.0 + 45.0),
    );
    assert_volume(
        &behind_evaluation,
        behind.body,
        BOX_VOLUME + 4.0 * 35.0 * (50.0 + 41.0),
    );
}

#[test]
fn four_webs_meeting_at_one_point_run_on_as_two_crossing_webs() {
    let mut sketch = top_sketch((50.0, 15.0), (50.0, 30.0));
    sketch.add_line(Point2::new(50.0, 30.0), Point2::new(50.0, 45.0));
    sketch.add_line(Point2::new(50.0, 30.0), Point2::new(30.0, 30.0));
    sketch.add_line(Point2::new(50.0, 30.0), Point2::new(70.0, 30.0));
    let model = ribbed(sketch, web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.body,
        BOX_VOLUME + 4.0 * 35.0 * (50.0 + 90.0 - 4.0),
    );
}

#[test]
fn curves_coming_back_to_a_point_they_pass_through_fail_with_the_fix_on_the_sketch() {
    let mut sketch = top_sketch((20.0, 30.0), (40.0, 30.0));
    sketch.add_line(Point2::new(40.0, 30.0), Point2::new(60.0, 20.0));
    sketch.add_line(Point2::new(60.0, 20.0), Point2::new(60.0, 40.0));
    sketch.add_line(Point2::new(60.0, 40.0), Point2::new(40.0, 30.0));
    let model = ribbed(sketch, web);

    let error = failure(&evaluate(&model.document), model.rib);

    assert!(
        error.reason.contains("come back to a point"),
        "{}",
        error.reason
    );
    assert!(matches!(error.fix, Some(FixTarget::Feature(sketch)) if sketch != model.rib));
}

#[test]
fn a_drafted_web_widens_toward_the_floor_it_meets() {
    let model = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
        web(rib);
        rib.draft = length("2 deg");
    });

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let widening = 35.0 * 35.0 * 2.0_f64.to_radians().tan();
    let found = volume(&evaluation, model.body);
    let expected = BOX_VOLUME + 50.0 * (4.0 * 35.0 + widening);
    assert!(
        (found - expected).abs() < 1e-4 * expected,
        "{found} {expected}"
    );
    let solid = evaluation.body(model.body).unwrap();
    assert_eq!(
        solid.classify_point(Point3::new(52.3, 30.0, 6.0)),
        PointClass::Inside
    );
    assert_eq!(
        solid.classify_point(Point3::new(52.3, 30.0, 39.0)),
        PointClass::Outside
    );
}

#[test]
fn a_draft_below_zero_or_too_steep_fails_in_words() {
    for (draft, words) in [("-1 deg", "cannot be negative"), ("30 deg", "or steeper")] {
        let model = ribbed(top_sketch((50.0, 15.0), (50.0, 45.0)), |rib| {
            web(rib);
            rib.draft = length(draft);
        });

        let error = failure(&evaluate(&model.document), model.rib);

        assert!(error.reason.contains(words), "{}", error.reason);
        assert_eq!(error.fix, Some(FixTarget::Feature(model.rib)));
    }
}

fn arc_at(radius: f64, from_degrees: f64, to_degrees: f64) -> Sketch {
    let plane = Plane::new(Point3::new(0.0, 0.0, 30.0), Vector3::Z).unwrap();
    let mut sketch = Sketch::new(plane);
    let centre = Point2::new(50.0, 30.0);
    let on = |degrees: f64| {
        let angle = f64::to_radians(degrees);
        Point2::new(
            centre.x + radius * angle.cos(),
            centre.y + radius * angle.sin(),
        )
    };
    sketch.add_arc(centre, on(from_degrees), on(to_degrees));
    sketch
}

fn round_ends(rib: &mut Rib) {
    web(rib);
    rib.ends = RibEnds::RoundArcs;
}

#[test]
fn a_web_s_arc_carried_on_round_its_circle_stops_at_the_walls_it_meets() {
    let model = ribbed(arc_at(28.0, -30.0, 30.0), round_ends);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let band = |radius: f64| {
        25.0 * (radius * radius - 625.0).sqrt() + radius * radius * (25.0 / radius).asin()
    };
    let expected = BOX_VOLUME + 25.0 * (band(30.0) - band(26.0));
    let found = volume(&evaluation, model.body);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} {expected}"
    );
    let solid = evaluation.body(model.body).unwrap();
    assert_eq!(
        solid.classify_point(Point3::new(50.0 + 28.0 * 0.6, 30.0 + 28.0 * 0.8, 20.0)),
        PointClass::Inside
    );
    assert_eq!(
        solid.classify_point(Point3::new(22.0, 30.0, 20.0)),
        PointClass::Outside
    );
}

#[test]
fn a_web_s_arc_whose_circle_never_meets_the_body_fails_naming_it() {
    let model = ribbed(arc_at(10.0, 0.0, 180.0), round_ends);

    let error = failure(&evaluate(&model.document), model.rib);

    assert!(
        error.reason.contains("The end of Rib 1 at Arc") && error.reason.contains("runs on past"),
        "{}",
        error.reason
    );
    assert!(error.remedy.contains("straight"), "{}", error.remedy);
}

#[test]
fn a_rib_parallel_to_its_sketch_leaves_its_draft_aside() {
    let model = ribbed(middle_sketch(), |rib| {
        rib.reversed = true;
        rib.draft = length("5 deg");
    });

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert_volume(
        &evaluation,
        model.body,
        BOX_VOLUME + 0.5 * 35.0 * 25.0 * 4.0,
    );
}

fn run_to_walls(from: Point2, outward: Vector2) -> f64 {
    let reach = |start: f64, step: f64, low: f64, high: f64| {
        if step > 0.0 {
            (high - start) / step
        } else if step < 0.0 {
            (low - start) / step
        } else {
            f64::INFINITY
        }
    };
    reach(from.x, outward.x, 5.0, 95.0).min(reach(from.y, outward.y, 5.0, 55.0))
}

fn path_inside_box(sketch: &Sketch) -> f64 {
    let curves = crate::solid::profile_curves(sketch);
    let (path, range) = curves.first().unwrap().curve().unwrap();
    let ahead = |parameter: f64| path.evaluate(parameter).first.normalize();
    let start = path.point(range.start());
    let end = path.point(range.end());
    path.length(range)
        + run_to_walls(start, -ahead(range.start()))
        + run_to_walls(end, ahead(range.end()))
}

fn top_plane() -> Plane {
    Plane::new(Point3::new(0.0, 0.0, 30.0), Vector3::Z).unwrap()
}

#[test]
fn a_web_along_an_elliptical_arc_runs_its_ends_on_to_the_walls() {
    let mut sketch = Sketch::new(top_plane());
    sketch.add_elliptical_arc(
        Point2::new(50.0, 30.0),
        Point2::new(70.0, 30.0),
        10.0,
        Point2::new(70.0, 30.0),
        Point2::new(30.0, 30.0),
    );
    let model = ribbed(sketch, web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let half_perimeter = 0.5 * std::f64::consts::PI * (3.0 * 30.0 - (70.0_f64 * 50.0).sqrt());
    let expected = BOX_VOLUME + 4.0 * 25.0 * (half_perimeter + 50.0);
    let found = volume(&evaluation, model.body);
    assert!(
        (found - expected).abs() < 1e-3 * expected,
        "{found} {expected}"
    );
    let solid = evaluation.body(model.body).unwrap();
    assert_eq!(
        solid.classify_point(Point3::new(50.0, 40.0, 20.0)),
        PointClass::Inside
    );
    assert_eq!(
        solid.classify_point(Point3::new(50.0, 30.0, 20.0)),
        PointClass::Outside
    );
}

#[test]
fn a_web_along_a_spline_runs_its_ends_on_and_reads_by_its_curve() {
    let mut sketch = Sketch::new(top_plane());
    sketch.add_spline(&[
        Point2::new(30.0, 20.0),
        Point2::new(45.0, 40.0),
        Point2::new(60.0, 20.0),
        Point2::new(75.0, 40.0),
    ]);
    let length = path_inside_box(&sketch);
    let model = ribbed(sketch, web);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let expected = BOX_VOLUME + 4.0 * 25.0 * length;
    let found = volume(&evaluation, model.body);
    assert!(
        (found - expected).abs() < 2e-3 * (expected - BOX_VOLUME),
        "{found} {expected}"
    );
    let words: Vec<String> = evaluation
        .body(model.body)
        .unwrap()
        .faces()
        .filter_map(|(_, face)| face.origin())
        .filter(|origin| origin.feature() == model.rib.raw())
        .map(|origin| describe_origin(&model.document, Some(origin)))
        .collect();
    assert!(
        words
            .iter()
            .any(|word| word.starts_with("Rib 1 side from Spline")),
        "{words:?}"
    );
}

#[test]
fn a_web_along_a_spline_with_a_draft_fails_saying_only_lines_and_arcs_take_one() {
    let mut sketch = Sketch::new(top_plane());
    sketch.add_spline(&[
        Point2::new(30.0, 20.0),
        Point2::new(45.0, 40.0),
        Point2::new(60.0, 20.0),
        Point2::new(75.0, 40.0),
    ]);
    let model = ribbed(sketch, |rib| {
        web(rib);
        rib.draft = length("3 deg");
    });

    let error = failure(&evaluate(&model.document), model.rib);

    assert!(
        error.reason.contains("only lines and arcs"),
        "{}",
        error.reason
    );
    assert!(error.remedy.contains("draft"), "{}", error.remedy);
}
