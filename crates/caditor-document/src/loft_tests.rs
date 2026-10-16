use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceOrigin, LoftSkin, SamplingTolerance};
use caditor_sketch::Sketch;

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

fn level(height: f64) -> Plane {
    Plane::new(Point3::new(0.0, 0.0, height), Vector3::Z).unwrap()
}

fn square(plane: Plane, centre: (f64, f64), half: f64) -> Sketch {
    let mut sketch = Sketch::new(plane);
    add_square(&mut sketch, centre, half);
    sketch
}

fn add_square(sketch: &mut Sketch, (x, y): (f64, f64), half: f64) {
    let corners = [
        Point2::new(x - half, y - half),
        Point2::new(x + half, y - half),
        Point2::new(x + half, y + half),
        Point2::new(x - half, y + half),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
}

fn frustum(bottom: f64, top: f64, height: f64) -> f64 {
    height / 3.0 * (bottom + top + (bottom * top).sqrt())
}

struct Lofted {
    document: Document,
    sketches: Vec<FeatureId>,
    loft: FeatureId,
}

fn lofted(sketches: Vec<Sketch>, shape: impl FnOnce(&mut Loft)) -> Lofted {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let ids: Vec<FeatureId> = sketches
        .into_iter()
        .enumerate()
        .map(|(index, sketch)| {
            transaction.add_feature(format!("Profile {}", index + 1), FeatureKind::from(sketch))
        })
        .collect();
    let mut made = Loft::new(ids.iter().copied().map(LoftSection::whole).collect());
    shape(&mut made);
    let loft = transaction.add_feature("Loft 1", FeatureKind::Loft(made));
    document.apply(transaction.finish()).unwrap();
    Lofted {
        document,
        sketches: ids,
        loft,
    }
}

#[test]
fn a_loft_from_a_square_to_a_smaller_one_makes_the_frustum() {
    let model = lofted(
        vec![
            square(level(0.0), (0.0, 0.0), 10.0),
            square(level(10.0), (0.0, 0.0), 5.0),
        ],
        |loft| loft.skin = LoftSkin::Ruled,
    );

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let found = volume(&evaluation, model.loft);
    let expected = frustum(400.0, 100.0, 10.0);
    assert!((found - expected).abs() < 1e-6 * expected, "{found}");
}

#[test]
fn a_loft_through_three_profiles_runs_smoothly_and_names_its_sides_from_the_first() {
    let mut middle = Sketch::new(level(6.0));
    middle.add_circle(Point2::ZERO, 3.0);
    let model = lofted(
        vec![
            square(level(0.0), (0.0, 0.0), 5.0),
            middle,
            square(level(12.0), (0.0, 0.0), 5.0),
        ],
        |_| {},
    );

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let solid = evaluation.body(model.loft).unwrap();
    assert_eq!(solid.faces().count(), 6);
    let sides: Vec<String> = solid
        .faces()
        .filter(|(_, face)| matches!(face.origin(), Some(FaceOrigin::Side { .. })))
        .map(|(_, face)| describe_origin(&model.document, face.origin()))
        .collect();
    assert_eq!(sides.len(), 4);
    assert!(
        sides
            .iter()
            .all(|side| side.starts_with("Loft 1 side from Line")),
        "{sides:?}"
    );
}

#[test]
fn a_loft_ending_at_a_point_closes_in_a_tip() {
    let model = lofted(vec![square(level(6.0), (0.0, 0.0), 3.0)], |loft| {
        loft.skin = LoftSkin::Ruled;
        loft.end = Some(PointReference::Origin);
    });

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let found = volume(&evaluation, model.loft);
    assert!((found - 36.0 * 6.0 / 3.0).abs() < 1e-6 * 72.0, "{found}");
}

#[test]
fn a_loft_cuts_from_another_body() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let low = transaction.add_feature(
        "Low",
        FeatureKind::from(square(level(0.0), (0.0, 0.0), 10.0)),
    );
    let high = transaction.add_feature(
        "High",
        FeatureKind::from(square(level(10.0), (0.0, 0.0), 10.0)),
    );
    let mut block = Loft::new(vec![LoftSection::whole(low), LoftSection::whole(high)]);
    block.skin = LoftSkin::Ruled;
    let block = transaction.add_feature("Block", FeatureKind::Loft(block));
    let small_low = transaction.add_feature(
        "Small low",
        FeatureKind::from(square(level(2.0), (0.0, 0.0), 4.0)),
    );
    let small_high = transaction.add_feature(
        "Small high",
        FeatureKind::from(square(level(12.0), (0.0, 0.0), 2.0)),
    );
    let mut pocket = Loft::new(vec![
        LoftSection::whole(small_low),
        LoftSection::whole(small_high),
    ]);
    pocket.skin = LoftSkin::Ruled;
    pocket.operation = BodyOperation::Remove(block);
    transaction.add_feature("Pocket", FeatureKind::Loft(pocket));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(evaluation.failed_count(), 0);
    let top_side = 8.0 - 4.0 * 8.0 / 10.0;
    let expected = 4000.0 - frustum(64.0, top_side * top_side, 8.0);
    let found = volume(&evaluation, block);
    assert!((found - expected).abs() < 1e-6 * expected, "{found}");
}

#[test]
fn a_loft_with_one_profile_waits_for_more() {
    let model = lofted(vec![square(level(0.0), (0.0, 0.0), 5.0)], |_| {});

    let evaluation = evaluate(&model.document);

    assert_eq!(
        evaluation.feature(model.loft).unwrap().state,
        FeatureState::Waiting(MissingInput::Profiles)
    );
}

#[test]
fn a_profile_of_two_shapes_needs_one_chosen() {
    let mut pair = square(level(10.0), (-8.0, 0.0), 2.0);
    add_square(&mut pair, (8.0, 0.0), 2.0);
    let mut model = lofted(vec![square(level(0.0), (8.0, 0.0), 3.0), pair], |loft| {
        loft.skin = LoftSkin::Ruled
    });

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.loft);
    let Some(FeatureResult::Sketch(sketch)) = evaluation
        .feature(model.sketches[1])
        .and_then(|feature| feature.result.as_deref())
    else {
        panic!("no sketch result");
    };
    let right = sketch
        .regions()
        .unwrap()
        .as_ref()
        .unwrap()
        .iter()
        .find(|region| region.region.bounds().unwrap().center().x > 0.0)
        .unwrap()
        .reference();
    let mut chosen = model
        .document
        .feature(model.loft)
        .unwrap()
        .kind
        .loft()
        .unwrap()
        .clone();
    chosen.profiles[1].region = Some(right);
    let mut transaction = model.document.transaction("Choose");
    transaction.edit(Edit::SetFeatureKind {
        id: model.loft,
        kind: FeatureKind::Loft(chosen),
    });
    model.document.apply(transaction.finish()).unwrap();
    let chosen_evaluation = evaluate(&model.document);

    assert!(
        error.reason().contains("2 separate closed shapes"),
        "{}",
        error.reason()
    );
    assert_eq!(chosen_evaluation.failed_count(), 0);
    let found = volume(&chosen_evaluation, model.loft);
    let expected = frustum(36.0, 16.0, 10.0);
    assert!((found - expected).abs() < 1e-6 * expected, "{found}");
}

#[test]
fn profiles_in_one_place_fail_naming_them() {
    let model = lofted(
        vec![
            square(level(0.0), (0.0, 0.0), 5.0),
            square(level(0.0), (0.0, 0.0), 3.0),
        ],
        |_| {},
    );

    let evaluation = evaluate(&model.document);
    let error = failure(&evaluation, model.loft);

    assert!(
        error
            .reason()
            .contains("Profile 1 and Profile 2 lie in the same place"),
        "{}",
        error.reason()
    );
}

#[test]
fn inserting_a_loft_is_one_undoable_change() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketches");
    let low = transaction.add_feature(
        "Low",
        FeatureKind::from(square(level(0.0), (0.0, 0.0), 5.0)),
    );
    let high = transaction.add_feature(
        "High",
        FeatureKind::from(square(level(5.0), (0.0, 0.0), 2.0)),
    );
    document.apply(transaction.finish()).unwrap();
    let mut editor = Editor::new(document);
    let is_loft = |feature: &Feature| feature.kind.loft().is_some();

    let mut transaction = editor.document().transaction("Loft");
    transaction.add_feature(
        "Loft 1",
        FeatureKind::Loft(Loft::new(vec![
            LoftSection::whole(low),
            LoftSection::whole(high),
        ])),
    );
    editor.apply(transaction.finish()).unwrap();
    let lofts = editor
        .document()
        .features()
        .filter(|feature| is_loft(feature))
        .count();
    let uses = editor.document().dependents_of(&[low]);
    editor.undo().unwrap();

    assert_eq!(lofts, 1);
    assert_eq!(uses.len(), 1);
    assert!(!editor.document().features().any(is_loft));
}

fn ring_sections(count: usize) -> Vec<Sketch> {
    (0..count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            let radial = Vector3::new(angle.cos(), angle.sin(), 0.0);
            let plane = Plane::from_frame(
                Point3::ZERO + radial * 10.0,
                Vector3::Z.cross(radial),
                radial,
            )
            .unwrap();
            square(plane, (0.0, 0.0), 1.0)
        })
        .collect()
}

#[test]
fn a_loft_closing_back_to_its_first_profile_makes_a_ring() {
    let ring = lofted(ring_sections(12), |loft| loft.closed = true);
    let two = lofted(ring_sections(12).into_iter().take(2).collect(), |loft| {
        loft.closed = true;
    });

    let evaluation = evaluate(&ring.document);
    let refused = failure(&evaluate(&two.document), two.loft);

    assert_eq!(evaluation.failed_count(), 0);
    let expected = 4.0 * std::f64::consts::TAU * 10.0;
    assert!((volume(&evaluation, ring.loft) - expected).abs() < 5e-3 * expected);
    assert!(
        refused.reason().contains("at least three"),
        "{}",
        refused.reason()
    );
}

#[test]
fn square_ends_follow_their_weight_parameter_and_refuse_a_weight_of_zero() {
    let shifted = || {
        vec![
            square(level(0.0), (0.0, 0.0), 1.0),
            square(level(10.0), (5.0, 0.0), 1.0),
        ]
    };
    let mut model = lofted(shifted(), |_| {});
    let mut transaction = model.document.transaction("Square");
    let weight = transaction.add_parameter("bulge", Expression::parse_stored("1.5").unwrap());
    let mut squared = model.document.feature(model.loft).unwrap().kind.clone();
    if let FeatureKind::Loft(loft) = &mut squared {
        loft.square_start = Some(Expression::Parameter(weight));
        loft.square_end = Some(Expression::number(1.0));
    }
    transaction.edit(Edit::SetFeatureKind {
        id: model.loft,
        kind: squared,
    });
    model.document.apply(transaction.finish()).unwrap();
    let flat = lofted(shifted(), |loft| {
        loft.square_end = Some(Expression::number(0.0));
    });
    let ruled = lofted(shifted(), |loft| {
        loft.skin = LoftSkin::Ruled;
        loft.square_start = Some(Expression::number(1.0));
    });

    let evaluation = evaluate(&model.document);
    let weightless = failure(&evaluate(&flat.document), flat.loft);
    let straight = failure(&evaluate(&ruled.document), ruled.loft);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.loft) - 40.0).abs() < 1e-2);
    assert!(
        model
            .document
            .feature(model.loft)
            .unwrap()
            .kind
            .uses_parameter(weight)
    );
    assert!(
        weightless.reason().contains("weight"),
        "{}",
        weightless.reason()
    );
    assert!(
        straight.remedy().contains("Smooth"),
        "{}",
        straight.remedy()
    );
    assert!(!model.sketches.is_empty());
}
