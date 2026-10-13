use std::f64::consts::{FRAC_PI_2, PI};

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{PipeShape, SamplingTolerance};
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

fn length(text: &str) -> Expression {
    Expression::parse_stored(text).unwrap()
}

fn elbow() -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 30.0));
    (sketch, vec![first, second])
}

struct Model {
    document: Document,
    path_sketch: FeatureId,
    curves: Vec<EntityId>,
    pipe: FeatureId,
}

fn piped(pipe: impl FnOnce(SweepPath) -> Pipe) -> Model {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let (path, curves) = elbow();
    let path_sketch = transaction.add_feature("Path", FeatureKind::from(path));
    let made = pipe(SweepPath::Curves {
        sketch: path_sketch,
        curves: curves.clone(),
    });
    let pipe = transaction.add_feature("Pipe 1", FeatureKind::Pipe(made));
    document.apply(transaction.finish()).unwrap();
    Model {
        document,
        path_sketch,
        curves,
        pipe,
    }
}

fn round(path: SweepPath) -> Pipe {
    Pipe::new(path, length("4 mm"), length("6 mm"))
}

#[test]
fn a_pipe_follows_its_path_rounding_the_corner_to_the_bend_radius() {
    let model = piped(round);

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    let centreline = 20.0 - 6.0 + 30.0 - 6.0 + 6.0 * FRAC_PI_2;
    let found = volume(&evaluation, model.pipe);
    assert!(
        (found - PI * 4.0 * centreline).abs() < 1e-3 * found,
        "{found} {}",
        PI * 4.0 * centreline
    );
    let kind = &model.document.feature(model.pipe).unwrap().kind;
    assert!(kind.features().contains(&model.path_sketch));
    assert!(kind.reference_sketches().contains(&model.path_sketch));
    assert_eq!(kind.parameters().len(), 0);
}

#[test]
fn a_hollow_square_pipe_with_sharp_corners_is_its_wall_times_the_path() {
    let model = piped(|path| Pipe {
        shape: PipeShape::Square,
        wall: Some(length("1 mm")),
        bend: length("0 mm"),
        ..round(path)
    });

    let evaluation = evaluate(&model.document);

    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume(&evaluation, model.pipe) - (16.0 - 4.0) * 50.0).abs() < 1e-6);
}

#[test]
fn a_wall_too_thick_fails_saying_how_thin_it_must_be() {
    let model = piped(|path| Pipe {
        wall: Some(length("2 mm")),
        ..round(path)
    });

    let error = failure(&evaluate(&model.document), model.pipe);

    assert!(
        error.reason.contains("half its diameter"),
        "{}",
        error.reason
    );
    assert_eq!(error.fix, Some(FixTarget::Feature(model.pipe)));
}

#[test]
fn a_bend_too_tight_or_too_long_fails_naming_the_lines() {
    let tight = piped(|path| Pipe {
        bend: length("1 mm"),
        ..round(path)
    });
    let long = piped(|path| Pipe {
        bend: length("25 mm"),
        ..round(path)
    });

    let tight = failure(&evaluate(&tight.document), tight.pipe);
    let long = failure(&evaluate(&long.document), long.pipe);

    assert!(
        tight.reason.contains("tighter than Pipe 1") && tight.reason.contains("of Path"),
        "{}",
        tight.reason
    );
    assert!(
        tight.remedy.contains("bend radius larger"),
        "{}",
        tight.remedy
    );
    assert!(
        long.reason.contains("Rounding the corner between") && long.reason.contains("of Path"),
        "{}",
        long.reason
    );
}

#[test]
fn a_size_that_is_not_a_length_above_zero_fails_in_words() {
    let model = piped(|path| Pipe {
        size: length("0 mm"),
        ..round(path)
    });

    let error = failure(&evaluate(&model.document), model.pipe);

    assert_eq!(
        error.reason,
        "The diameter of the pipe must be more than zero."
    );
}

#[test]
fn a_pipe_with_no_path_waits_for_one() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let pipe = transaction.add_feature("Pipe 1", FeatureKind::Pipe(round(SweepPath::Unchosen)));
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.feature(pipe).unwrap().state,
        FeatureState::Waiting(MissingInput::Path)
    );
}

#[test]
fn a_curve_of_the_path_cannot_be_deleted_while_a_pipe_follows_it() {
    let mut model = piped(round);
    let mut transaction = model.document.transaction("Delete");
    transaction.remove_sketch_items(model.path_sketch, [model.curves[0]], []);

    let refused = model.document.apply(transaction.finish());

    assert!(
        matches!(refused, Err(EditError::EntityInUse { .. })),
        "{refused:?}"
    );
}

#[test]
fn a_removing_pipe_cuts_a_channel_from_its_body_and_keeps_the_tool() {
    let mut document = Document::default();
    let mut transaction = document.transaction("Block");
    let mut outline = Sketch::new(Plane::XY);
    let corners = [
        Point2::new(-5.0, -5.0),
        Point2::new(30.0, -5.0),
        Point2::new(30.0, 40.0),
        Point2::new(-5.0, 40.0),
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
            extent: ExtrudeExtent::one_side(length("10 mm"), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    let mut path = Sketch::new(Plane::XY);
    let line = path.add_line(Point2::new(0.0, -10.0), Point2::new(0.0, 50.0));
    let path = transaction.add_feature("Channel path", FeatureKind::from(path));
    let channel = transaction.add_feature(
        "Channel",
        FeatureKind::Pipe(Pipe {
            operation: BodyOperation::Remove(base),
            ..round(SweepPath::Curves {
                sketch: path,
                curves: vec![line],
            })
        }),
    );
    document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&document);

    assert_eq!(
        evaluation.failed_count(),
        0,
        "{:?}",
        evaluation.feature(channel)
    );
    let expected = 35.0 * 45.0 * 10.0 - 0.5 * PI * 4.0 * 45.0;
    let found = volume(&evaluation, base);
    assert!(
        (found - expected).abs() < 1e-4 * expected,
        "{found} {expected}"
    );
    assert!(!document.feature(channel).unwrap().makes_body());
    assert_eq!(
        repeatable_on(&document.feature(channel).unwrap().kind),
        Some(base)
    );
}

#[test]
fn a_pipe_s_faces_are_worded_as_its_outside_and_inside() {
    let model = piped(|path| Pipe {
        wall: Some(length("1 mm")),
        ..round(path)
    });
    let evaluation = evaluate(&model.document);
    let solid = evaluation.body(model.pipe).unwrap();

    let words: Vec<String> = solid
        .faces()
        .map(|(_, face)| describe_origin(&model.document, face.origin()))
        .collect();

    assert!(
        words.iter().any(|word| word == "Pipe 1 outside"),
        "{words:?}"
    );
    assert!(
        words.iter().any(|word| word == "Pipe 1 inside"),
        "{words:?}"
    );
    assert!(
        words.iter().any(|word| word == "Pipe 1 start face"),
        "{words:?}"
    );
}

#[test]
fn a_pipe_s_size_may_be_a_parameter() {
    let mut model = piped(round);
    let mut transaction = model.document.transaction("Name");
    let diameter = transaction.add_parameter("diameter", length("5 mm"));
    let mut pipe = round(SweepPath::Curves {
        sketch: model.path_sketch,
        curves: model.curves.clone(),
    });
    pipe.size = Expression::Parameter(diameter);
    transaction.edit(Edit::SetFeatureKind {
        id: model.pipe,
        kind: FeatureKind::Pipe(pipe),
    });
    model.document.apply(transaction.finish()).unwrap();

    let kind = &model.document.feature(model.pipe).unwrap().kind;

    assert!(kind.uses_parameter(diameter));
    assert!(evaluate(&model.document).failed_count() == 0);
}
