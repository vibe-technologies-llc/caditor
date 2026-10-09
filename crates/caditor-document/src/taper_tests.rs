use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{SamplingTolerance, WallSide};
use caditor_sketch::Sketch;

use crate::*;

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn degrees(value: f64) -> Expression {
    Expression::Measure(value, Unit::Degree)
}

fn evaluate(document: &Document, engine: &mut Recompute) -> Evaluation {
    engine.run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
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

fn frustum(height: f64, bottom: f64, top: f64) -> f64 {
    height / 3.0 * (bottom + top + (bottom * top).sqrt())
}

fn square(side: f64) -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(side, 0.0),
        Point2::new(side, side),
        Point2::new(0.0, side),
    ];
    for index in 0..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    sketch
}

fn shaped(
    sketch: FeatureId,
    distance: f64,
    taper: Option<Expression>,
    wall: Option<Wall>,
) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(millimetres(distance), false),
        operation: BodyOperation::NewBody,
        start: None,
        other_bodies: Vec::new(),
        taper: taper.map(Box::new),
        wall: wall.map(Box::new),
        direction: None,
    }))
}

fn model(sketch: Sketch, kind: impl FnOnce(FeatureId) -> FeatureKind) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Outline", FeatureKind::from(sketch));
    let solid = transaction.add_feature("Spike", kind(outline));
    document.apply(transaction.finish()).unwrap();
    (document, solid)
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-3 * expected,
        "{actual} vs {expected}"
    );
}

#[test]
fn a_tapered_extrusion_follows_a_model_parameter_and_undoes() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let draft = transaction.add_parameter("draft", degrees(5.0));
    let outline = transaction.add_feature("Outline", FeatureKind::from(square(10.0)));
    let block = transaction.add_feature(
        "Block",
        shaped(outline, 4.0, Some(Expression::Parameter(draft)), None),
    );
    document.apply(transaction.finish()).unwrap();
    let mut engine = Recompute::default();
    let at = |angle: f64| {
        let top = 10.0 - 2.0 * 4.0 * angle.to_radians().tan();
        frustum(4.0, 100.0, top * top)
    };

    let first = evaluate(&document, &mut engine);
    let undo = document
        .apply(Transaction::single(
            "Steeper",
            Edit::SetParameterExpression {
                id: draft,
                expression: degrees(12.0),
            },
        ))
        .unwrap();
    let steeper = evaluate(&document, &mut engine);
    document.apply(undo).unwrap();
    let undone = evaluate(&document, &mut engine);

    assert_near(volume(&first, block), at(5.0));
    assert_near(volume(&steeper, block), at(12.0));
    assert_near(volume(&undone, block), at(5.0));
    assert!(
        SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(millimetres(4.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: Some(Box::new(Expression::Parameter(draft))),
            wall: None,
            direction: None,
        })
        .uses_parameter(draft)
    );
}

#[test]
fn an_extrusion_whose_taper_closes_the_profile_fails_naming_it() {
    let (document, spike) = model(square(10.0), |outline| {
        shaped(outline, 10.0, Some(degrees(40.0)), None)
    });

    let evaluation = evaluate(&document, &mut Recompute::default());

    let error = failure(&evaluation, spike);
    assert!(
        error.reason.contains("The taper of Spike"),
        "{}",
        error.reason
    );
    assert!(
        error.remedy.contains("taper angle smaller"),
        "{}",
        error.remedy
    );

    let (steep, spike) = model(square(10.0), |outline| {
        shaped(outline, 1.0, Some(degrees(-89.5)), None)
    });
    let error = failure(&evaluate(&steep, &mut Recompute::default()), spike);
    assert!(error.reason.contains("89"), "{}", error.reason);
}

#[test]
fn a_thin_walled_extrusion_follows_an_open_sketch() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 10.0), Point2::new(0.0, 0.0));
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    let wall = |side| {
        Some(Wall {
            thickness: millimetres(2.0),
            side,
        })
    };

    let (centred, strip) = model(sketch.clone(), |outline| {
        shaped(outline, 5.0, None, wall(WallSide::Centred))
    });
    let (inside, inner) = model(sketch, |outline| {
        shaped(outline, 5.0, None, wall(WallSide::Inside))
    });

    assert_near(
        volume(&evaluate(&centred, &mut Recompute::default()), strip),
        300.0,
    );
    assert_near(
        volume(&evaluate(&inside, &mut Recompute::default()), inner),
        280.0,
    );
}

#[test]
fn a_thin_wall_too_thick_for_its_extrusion_curves_fails_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 1.0);
    let (document, tube) = model(sketch, |outline| {
        shaped(
            outline,
            5.0,
            None,
            Some(Wall {
                thickness: millimetres(3.0),
                side: WallSide::Inside,
            }),
        )
    });

    let error = failure(&evaluate(&document, &mut Recompute::default()), tube);

    assert!(error.reason.contains("too thick"), "{}", error.reason);
    assert!(error.remedy.contains("thinner wall"), "{}", error.remedy);
}
