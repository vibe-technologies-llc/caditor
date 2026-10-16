use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use crate::*;

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn state(evaluation: &Evaluation, feature: FeatureId) -> FeatureState {
    evaluation.feature(feature).unwrap().state.clone()
}

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn square() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 8.0),
        Point2::new(10.0, 8.0),
    ];
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % corners.len()]);
    }
    sketch
}

fn extrusion(sketch: FeatureId, regions: RegionChoice, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions,
        extent: ExtrudeExtent::one_side(length("4 mm"), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn turning(axis: Option<AxisReference>) -> PatternKind {
    PatternKind::Circular(CircularPattern {
        axis,
        count: Expression::number(3.0),
        angle: length("360 deg"),
        reversed: false,
    })
}

struct Patterned {
    document: Document,
    engine: Recompute,
    base: FeatureId,
    pattern: FeatureId,
    after: FeatureId,
}

fn patterned(axis: Option<AxisReference>) -> Patterned {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(square()));
    let base = transaction.add_feature(
        "Base",
        extrusion(outline, RegionChoice::All, BodyOperation::NewBody),
    );
    let pattern = transaction.add_feature(
        "Pattern 1",
        FeatureKind::from(Pattern::new(base, turning(axis))),
    );
    let after = transaction.add_feature(
        "Raise",
        extrusion(outline, RegionChoice::All, BodyOperation::Add(base)),
    );
    document.apply(transaction.finish()).unwrap();
    Patterned {
        document,
        engine: Recompute::default(),
        base,
        pattern,
        after,
    }
}

fn set_axis(model: &mut Patterned, axis: Option<AxisReference>) {
    let transaction = Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: model.pattern,
            kind: FeatureKind::from(Pattern::new(model.base, turning(axis))),
        },
    );
    model.document.apply(transaction).unwrap();
}

#[test]
fn a_circular_pattern_without_an_axis_waits_for_one_and_is_not_counted_as_failed() {
    let mut model = patterned(None);

    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(
        state(&evaluation, model.pattern),
        FeatureState::Waiting(MissingInput::Axis)
    );
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(evaluation.failures().count(), 0);
    assert_eq!(state(&evaluation, model.after), FeatureState::UpToDate);
    assert!(evaluation.feature(model.pattern).unwrap().result.is_none());
}

#[test]
fn a_pattern_whose_axis_is_cleared_waits_and_keeps_its_last_result() {
    let mut model = patterned(Some(AxisReference::Principal(PrincipalAxis::Z)));
    let computed = evaluate(&model.document, &mut model.engine);
    let last_good = computed.feature(model.pattern).unwrap().result.clone();

    set_axis(&mut model, None);
    let evaluation = evaluate(&model.document, &mut model.engine);

    assert_eq!(state(&computed, model.pattern), FeatureState::UpToDate);
    assert!(last_good.is_some());
    assert_eq!(
        state(&evaluation, model.pattern),
        FeatureState::Waiting(MissingInput::Axis)
    );
    assert_eq!(evaluation.feature(model.pattern).unwrap().result, last_good);

    set_axis(&mut model, Some(AxisReference::Principal(PrincipalAxis::Z)));
    let chosen = evaluate(&model.document, &mut model.engine);

    assert_eq!(state(&chosen, model.pattern), FeatureState::UpToDate);
}

fn upright_disc() -> Sketch {
    let plane = Plane::from_frame(Point3::ZERO, Vector3::Y, Vector3::X).unwrap();
    let mut sketch = Sketch::new(plane);
    sketch.add_circle(Point2::ZERO, 1.0);
    sketch
}

#[test]
fn a_sweep_without_a_path_waits_and_what_uses_its_body_fails_as_without_a_result() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let profile = transaction.add_feature("Profile", FeatureKind::from(upright_disc()));
    let sweep = transaction.add_feature(
        "Sweep 1",
        FeatureKind::Solid(SolidFeature::Sweep(Sweep::new(
            profile,
            SweepPath::Unchosen,
        ))),
    );
    let moved = transaction.add_feature(
        "Move 1",
        FeatureKind::Move(Move {
            body: sweep,
            offset: [length("1 mm"), length("0 mm"), length("0 mm")],
            turn: [length("0 deg"), length("0 deg"), length("0 deg")],
            copy: false,
            about: TurnCentre::Origin,
            frame: None,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        state(&evaluation, sweep),
        FeatureState::Waiting(MissingInput::Path)
    );
    let FeatureState::Failed(error) = state(&evaluation, moved) else {
        panic!("the move should fail without the swept body");
    };
    assert_eq!(error.fix, Some(FixTarget::Feature(sweep)));
    assert!(error.reason().contains("Sweep 1"), "{}", error.reason());
}

#[test]
fn an_extrusion_with_no_region_chosen_waits_for_regions_of_its_sketch() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(square()));
    let base = transaction.add_feature(
        "Base",
        extrusion(
            outline,
            RegionChoice::Chosen(Vec::new()),
            BodyOperation::NewBody,
        ),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());

    let waiting = MissingInput::Regions { sketch: outline };
    assert_eq!(state(&evaluation, base), FeatureState::Waiting(waiting));
    assert_eq!(waiting.waiting_words(), "Waiting for regions");
    assert_eq!(
        waiting.prompt(&document),
        "Choose at least one region of Outline to sweep."
    );
}

#[test]
fn a_blend_and_an_offset_face_with_nothing_chosen_wait_for_their_body() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(square()));
    let base = transaction.add_feature(
        "Base",
        extrusion(outline, RegionChoice::All, BodyOperation::NewBody),
    );
    let fillet = transaction.add_feature(
        "Fillet 1",
        FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body: base,
            edges: Vec::new(),
            groups: Vec::new(),
            size: length("1 mm"),
            form: ChamferForm::Equal,
            flipped: false,
        }),
    );
    let offset = transaction.add_feature(
        "Offset face 1",
        FeatureKind::OffsetFace(OffsetFace {
            body: base,
            faces: Vec::new(),
            distance: length("1 mm"),
            tangent: false,
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document, &mut Recompute::default());

    assert_eq!(
        state(&evaluation, fillet),
        FeatureState::Waiting(MissingInput::Edges { body: base })
    );
    assert_eq!(
        state(&evaluation, offset),
        FeatureState::Waiting(MissingInput::FacesToMove { body: base })
    );
    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(
        MissingInput::Edges { body: base }.prompt(&document),
        "Choose at least one edge of the body of Base."
    );
}
