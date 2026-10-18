use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch, Text};

use crate::{solid_tests::extrude, *};

const CLOSE: f64 = 1e-6;

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn volume(evaluation: &Evaluation, body: FeatureId) -> f64 {
    evaluation
        .body(body)
        .unwrap()
        .tessellate(&caditor_kernel::SamplingTolerance::new(1e-4, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn lettering(content: &str, anchor: EntityId, height: Expression) -> Text {
    Text {
        anchor,
        content: content.to_owned(),
        height,
        angle: Expression::measure(0.0, Unit::Degree),
        bold: false,
        italic: false,
    }
}

fn millimetres(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

struct Lettered {
    document: Document,
    sketch: FeatureId,
    anchor: EntityId,
    text: EntityId,
}

fn lettered(content: &str, plane: Plane, height: Expression) -> Lettered {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Label", FeatureKind::from(Sketch::new(plane)));
    let anchor = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(2.0, 3.0)));
    transaction.add_sketch_constraint(
        sketch,
        Constraint::Fix {
            point: anchor,
            at: Point2::new(2.0, 3.0),
        },
    );
    let text = transaction.add_sketch_text(sketch, lettering(content, anchor, height));
    document.apply(transaction.finish()).unwrap();
    Lettered {
        document,
        sketch,
        anchor,
        text,
    }
}

fn letter_area(evaluation: &Evaluation, sketch: FeatureId) -> f64 {
    let result = evaluation.feature(sketch).unwrap().result.clone().unwrap();
    let sketch = result.sketch().unwrap();
    let profile = sketch.profile().unwrap();
    profile
        .regions()
        .iter()
        .filter(|region| region.depth() % 2 == 0)
        .filter_map(|region| region.section())
        .map(|section| section.area)
        .sum()
}

fn letter_bounds(evaluation: &Evaluation, sketch: FeatureId) -> (Point2, Point2) {
    let result = evaluation.feature(sketch).unwrap().result.clone().unwrap();
    let sketch = result.sketch().unwrap();
    sketch
        .profile()
        .unwrap()
        .regions()
        .iter()
        .filter_map(|region| region.bounds())
        .fold(
            (
                Point2::splat(f64::INFINITY),
                Point2::splat(f64::NEG_INFINITY),
            ),
            |(low, high), bounds| (low.min(bounds.min()), high.max(bounds.max())),
        )
}

#[test]
fn text_extrudes_into_letters_whose_counters_stay_open() {
    let mut model = lettered("OB", Plane::XY, millimetres(10.0));
    let mut transaction = model.document.transaction("Extrude");
    let body = transaction.add_feature(
        "Letters",
        extrude(model.sketch, "2 mm", false, BodyOperation::NewBody),
    );
    model.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&model.document);
    let area = letter_area(&evaluation, model.sketch);
    let (low, high) = letter_bounds(&evaluation, model.sketch);
    let regions = evaluation
        .feature(model.sketch)
        .unwrap()
        .result
        .as_ref()
        .unwrap()
        .sketch()
        .unwrap()
        .profile()
        .unwrap()
        .regions()
        .len();

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(regions, 5);
    assert!((low.y - 3.0).abs() < 0.2, "{low:?}");
    assert!((high.y - 13.0).abs() < 0.2, "{high:?}");
    assert!(low.x >= 2.0);
    assert!(area > 10.0);
    assert!((volume(&evaluation, body) - 2.0 * area).abs() < 0.01 * area);
}

#[test]
fn text_cut_into_a_plate_takes_out_its_letters() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Plate");
    let outline = transaction.add_feature(
        "Outline",
        FeatureKind::from(crate::solid_tests::rectangle(
            Plane::XY,
            (0.0, 0.0),
            (40.0, 20.0),
        )),
    );
    let plate = transaction.add_feature(
        "Plate",
        extrude(outline, "5 mm", false, BodyOperation::NewBody),
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 5.0), Vector3::Z, Vector3::X).unwrap();
    let sketch = transaction.add_feature("Label", FeatureKind::from(Sketch::new(top)));
    let anchor = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(4.0, 6.0)));
    transaction.add_sketch_text(sketch, lettering("HI", anchor, millimetres(8.0)));
    transaction.add_feature(
        "Engraving",
        extrude(sketch, "1 mm", true, BodyOperation::Remove(plate)),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);
    let area = letter_area(&evaluation, sketch);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(area > 10.0);
    assert!((volume(&evaluation, plate) - (4000.0 - area)).abs() < 0.01 * area);
}

#[test]
fn letters_follow_the_parameter_height_and_the_angle() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let size = transaction.add_parameter("size", transaction.parse("6 mm").unwrap());
    let sketch = transaction.add_feature("Label", FeatureKind::from(Sketch::new(Plane::XY)));
    let anchor = transaction.add_sketch_entity(sketch, Entity::Point(Point2::ZERO));
    transaction.add_sketch_constraint(sketch, Constraint::Coincident(anchor, EntityId::ORIGIN));
    let text = transaction.add_sketch_text(
        sketch,
        Text {
            angle: Expression::measure(90.0, Unit::Degree),
            ..lettering("I", anchor, Expression::Parameter(size))
        },
    );
    document.apply(transaction.finish()).unwrap();

    let upright = letter_bounds(&evaluate(&document), sketch);
    let mut transaction = document.transaction("Taller");
    let taller = transaction.parse("9 mm").unwrap();
    transaction.edit(Edit::SetParameterExpression {
        id: size,
        expression: taller,
    });
    document.apply(transaction.finish()).unwrap();
    let taller = letter_bounds(&evaluate(&document), sketch);

    assert!(upright.1.x < CLOSE && upright.0.x > -6.0 - CLOSE);
    assert!((upright.0.x + 6.0).abs() < 0.2, "{upright:?}");
    assert!((taller.0.x + 9.0).abs() < 0.2, "{taller:?}");
    assert!(document.used_parameters().contains(&size));
    assert_eq!(
        sketch_of(&document, sketch).text(text).unwrap().content,
        "I"
    );
}

#[test]
fn a_height_that_comes_out_at_zero_fails_the_sketch_naming_the_text() {
    let model = lettered("A", Plane::XY, millimetres(0.0));

    let evaluation = evaluate(&model.document);

    let FeatureState::Failed(error) = &evaluation.feature(model.sketch).unwrap().state else {
        panic!("the sketch should fail");
    };
    assert!(
        error
            .reason()
            .contains(&format!("Text {} cannot be drawn", model.text)),
        "{}",
        error.reason()
    );
}

fn sketch_of(document: &Document, feature: FeatureId) -> &Sketch {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.sketch())
        .unwrap()
}

#[test]
fn deleting_a_text_takes_its_lone_point_and_undo_brings_both_back() {
    let mut model = lettered("A", Plane::XY, millimetres(5.0));
    let before = sketch_of(&model.document, model.sketch).clone();

    let mut transaction = model.document.transaction("Delete text");
    transaction.remove_sketch_items(model.sketch, [model.text], []);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let after = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();

    assert_eq!(after.texts().len(), 0);
    assert!(after.entity(model.anchor).is_none());
    assert_eq!(after.constraints().len(), 0);
    assert!(sketch_of(&model.document, model.sketch).same_content(&before));
}

#[test]
fn deleting_the_anchor_takes_the_text_and_a_shared_anchor_stays() {
    let mut model = lettered("A", Plane::XY, millimetres(5.0));
    let mut transaction = model.document.transaction("Line");
    let end = transaction.add_sketch_entity(model.sketch, Entity::Point(Point2::new(9.0, 3.0)));
    transaction.add_sketch_entity(
        model.sketch,
        Entity::Line {
            start: model.anchor,
            end,
        },
    );
    model.document.apply(transaction.finish()).unwrap();

    let mut transaction = model.document.transaction("Delete text");
    transaction.remove_sketch_items(model.sketch, [model.text], []);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let kept = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();
    let mut transaction = model.document.transaction("Delete point");
    transaction.remove_sketch_items(model.sketch, [model.anchor], []);
    model.document.apply(transaction.finish()).unwrap();
    let gone = sketch_of(&model.document, model.sketch);

    assert_eq!(kept.texts().len(), 0);
    assert!(kept.entity(model.anchor).is_some());
    assert_eq!(gone.texts().len(), 0);
    assert!(gone.entity(model.anchor).is_none());
}

#[test]
fn a_reshaped_sketch_adds_changes_and_removes_texts_in_one_undoable_step() {
    let mut model = lettered("A", Plane::XY, millimetres(5.0));
    let before = sketch_of(&model.document, model.sketch).clone();
    let mut after = before.clone();
    after
        .replace_text(
            model.text,
            Text {
                bold: true,
                ..lettering("Bee", model.anchor, millimetres(7.0))
            },
        )
        .unwrap();
    let point = after.add_point(Point2::new(20.0, 0.0));
    let added = after
        .add_text(lettering("C", point, millimetres(3.0)))
        .unwrap();

    let mut transaction = model.document.transaction("Reshape");
    transaction.reshape_sketch(model.sketch, &before, &after);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let reshaped = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();

    assert!(reshaped.same_content(&after));
    assert_eq!(reshaped.text(added).unwrap().content, "C");
    assert!(sketch_of(&model.document, model.sketch).same_content(&before));
}

#[test]
fn deleting_the_height_parameter_writes_its_value_into_the_text() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let size = transaction.add_parameter("size", transaction.parse("6 mm").unwrap());
    let sketch = transaction.add_feature("Label", FeatureKind::from(Sketch::new(Plane::XY)));
    let anchor = transaction.add_sketch_entity(sketch, Entity::Point(Point2::ZERO));
    let text = transaction.add_sketch_text(
        sketch,
        lettering("A", anchor, transaction.parse("size * 2").unwrap()),
    );
    document.apply(transaction.finish()).unwrap();

    let inlining = document.inline_parameter(size).unwrap();
    document.apply(inlining).unwrap();
    let held = sketch_of(&document, sketch).text(text).unwrap();

    assert_eq!(document.expression_text(&held.height), "6 mm * 2");
}

#[test]
fn scaling_the_model_scales_a_written_height() {
    let mut model = lettered("A", Plane::XY, millimetres(5.0));

    let scaled = model
        .document
        .scaled(&ModelScale {
            factor: 2.0,
            centre: Point3::ZERO,
            values: ScaledValues::Plain,
        })
        .unwrap();
    model.document.apply(scaled.transaction).unwrap();
    let held = sketch_of(&model.document, model.sketch)
        .text(model.text)
        .unwrap();

    assert_eq!(model.document.expression_text(&held.height), "10 mm");
}
