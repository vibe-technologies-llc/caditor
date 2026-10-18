use std::f64::consts::PI;

use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_sketch::{Constraint, Entity, EntityId, Gear, Sketch};

use crate::{solid_tests::extrude, *};

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

fn millimetres(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

fn spur(centre: EntityId, teeth: Expression) -> Gear {
    Gear {
        centre,
        module: millimetres(2.0),
        teeth,
        pressure_angle: Expression::measure(20.0, Unit::Degree),
        profile_shift: Expression::number(0.0),
        root_fillet: millimetres(0.5),
        bore: millimetres(8.0),
        turn: Expression::measure(0.0, Unit::Degree),
    }
}

struct Geared {
    document: Document,
    sketch: FeatureId,
    centre: EntityId,
    gear: EntityId,
}

fn geared(teeth: Expression) -> Geared {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let sketch = transaction.add_feature("Wheel", FeatureKind::from(Sketch::new(Plane::XY)));
    let centre = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(5.0, 5.0)));
    transaction.add_sketch_constraint(
        sketch,
        Constraint::Fix {
            point: centre,
            at: Point2::new(5.0, 5.0),
        },
    );
    let gear = transaction.add_sketch_gear(sketch, spur(centre, teeth));
    document.apply(transaction.finish()).unwrap();
    Geared {
        document,
        sketch,
        centre,
        gear,
    }
}

fn sketch_of(document: &Document, feature: FeatureId) -> &Sketch {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.sketch())
        .unwrap()
}

fn extruded(model: &mut Geared) -> FeatureId {
    let mut transaction = model.document.transaction("Extrude");
    let body = transaction.add_feature(
        "Wheel body",
        extrude(model.sketch, "3 mm", false, BodyOperation::NewBody),
    );
    model.document.apply(transaction.finish()).unwrap();
    body
}

fn region_count(evaluation: &Evaluation, sketch: FeatureId) -> usize {
    let result = evaluation.feature(sketch).unwrap().result.clone().unwrap();
    result.sketch().unwrap().profile().unwrap().regions().len()
}

#[test]
fn a_stored_gear_extrudes_into_a_wheel_with_its_bore_open() {
    let mut model = geared(Expression::number(20.0));
    let body = extruded(&mut model);

    let evaluation = evaluate(&model.document);
    let wheel = volume(&evaluation, body);
    let tip = 2.0 * 22.0 / 2.0;
    let root = 2.0 * 20.0 / 2.0 - 1.25 * 2.0;
    let bore = 4.0;

    assert_eq!(evaluation.failed_count(), 0);
    assert_eq!(region_count(&evaluation, model.sketch), 2);
    assert!(wheel > 3.0 * PI * (root * root - bore * bore));
    assert!(wheel < 3.0 * PI * (tip * tip - bore * bore));
}

#[test]
fn a_teeth_parameter_redraws_the_gear_with_more_teeth() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let teeth = transaction.add_parameter("teeth", transaction.parse("20").unwrap());
    let sketch = transaction.add_feature("Wheel", FeatureKind::from(Sketch::new(Plane::XY)));
    let centre = transaction.add_sketch_entity(sketch, Entity::Point(Point2::ZERO));
    transaction.add_sketch_constraint(sketch, Constraint::Coincident(centre, EntityId::ORIGIN));
    transaction.add_sketch_gear(sketch, spur(centre, Expression::Parameter(teeth)));
    let body = transaction.add_feature(
        "Wheel body",
        extrude(sketch, "3 mm", false, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();
    let small = volume(&evaluate(&document), body);

    let mut transaction = document.transaction("More teeth");
    let more = transaction.parse("30").unwrap();
    transaction.edit(Edit::SetParameterExpression {
        id: teeth,
        expression: more,
    });
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    let large = volume(&evaluation, body);

    assert_eq!(evaluation.failed_count(), 0);
    assert!(large > small * 1.8, "{small} then {large}");
    assert!(document.used_parameters().contains(&teeth));
}

#[test]
fn values_that_cannot_make_a_gear_fail_the_sketch_naming_the_gear() {
    let model = geared(Expression::number(20.5));

    let evaluation = evaluate(&model.document);

    let FeatureState::Failed(error) = &evaluation.feature(model.sketch).unwrap().state else {
        panic!("the sketch should fail");
    };
    assert!(
        error
            .reason()
            .contains(&format!("Gear {} cannot be drawn", model.gear)),
        "{}",
        error.reason()
    );
}

#[test]
fn deleting_a_gear_takes_its_lone_centre_and_undo_brings_both_back() {
    let mut model = geared(Expression::number(20.0));
    let before = sketch_of(&model.document, model.sketch).clone();

    let mut transaction = model.document.transaction("Delete gear");
    transaction.remove_sketch_items(model.sketch, [model.gear], []);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let after = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();

    assert_eq!(after.gears().len(), 0);
    assert!(after.entity(model.centre).is_none());
    assert_eq!(after.constraints().len(), 0);
    assert!(sketch_of(&model.document, model.sketch).same_content(&before));
}

#[test]
fn deleting_the_centre_takes_the_gear_and_a_shared_centre_stays_when_only_the_gear_goes() {
    let mut model = geared(Expression::number(20.0));
    let mut transaction = model.document.transaction("Circle");
    transaction.add_sketch_entity(
        model.sketch,
        Entity::Circle {
            center: model.centre,
            radius: 30.0,
        },
    );
    model.document.apply(transaction.finish()).unwrap();

    let mut transaction = model.document.transaction("Delete gear");
    transaction.remove_sketch_items(model.sketch, [model.gear], []);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let kept = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();
    let mut transaction = model.document.transaction("Delete centre");
    transaction.remove_sketch_items(model.sketch, [model.centre], []);
    model.document.apply(transaction.finish()).unwrap();
    let gone = sketch_of(&model.document, model.sketch);

    assert_eq!(kept.gears().len(), 0);
    assert!(kept.entity(model.centre).is_some());
    assert_eq!(gone.gears().len(), 0);
    assert!(gone.entity(model.centre).is_none());
}

#[test]
fn a_reshaped_sketch_adds_changes_and_removes_gears_in_one_undoable_step() {
    let mut model = geared(Expression::number(20.0));
    let before = sketch_of(&model.document, model.sketch).clone();
    let mut after = before.clone();
    after
        .replace_gear(model.gear, spur(model.centre, Expression::number(24.0)))
        .unwrap();
    let point = after.add_point(Point2::new(60.0, 0.0));
    let added = after
        .add_parametric_gear(spur(point, Expression::number(18.0)))
        .unwrap();

    let mut transaction = model.document.transaction("Reshape");
    transaction.reshape_sketch(model.sketch, &before, &after);
    let undo = model.document.apply(transaction.finish()).unwrap();
    let reshaped = sketch_of(&model.document, model.sketch).clone();
    model.document.apply(undo).unwrap();

    assert!(reshaped.same_content(&after));
    assert_eq!(
        reshaped.gear(added).unwrap().teeth,
        Expression::number(18.0)
    );
    assert!(sketch_of(&model.document, model.sketch).same_content(&before));
}

#[test]
fn deleting_a_parameter_writes_its_value_into_the_gear() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sketch");
    let module = transaction.add_parameter("module", transaction.parse("2 mm").unwrap());
    let sketch = transaction.add_feature("Wheel", FeatureKind::from(Sketch::new(Plane::XY)));
    let centre = transaction.add_sketch_entity(sketch, Entity::Point(Point2::ZERO));
    let gear = transaction.add_sketch_gear(
        sketch,
        Gear {
            module: transaction.parse("module * 1.5").unwrap(),
            ..spur(centre, Expression::number(20.0))
        },
    );
    document.apply(transaction.finish()).unwrap();

    let inlining = document.inline_parameter(module).unwrap();
    document.apply(inlining).unwrap();
    let held = sketch_of(&document, sketch).gear(gear).unwrap();

    assert_eq!(document.expression_text(&held.module), "2 mm * 1.5");
}

#[test]
fn scaling_the_model_scales_the_lengths_of_a_gear_and_leaves_its_counts() {
    let mut model = geared(Expression::number(20.0));

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
        .gear(model.gear)
        .unwrap();

    assert_eq!(model.document.expression_text(&held.module), "4 mm");
    assert_eq!(model.document.expression_text(&held.bore), "16 mm");
    assert_eq!(model.document.expression_text(&held.root_fillet), "1 mm");
    assert_eq!(model.document.expression_text(&held.teeth), "20");
}
