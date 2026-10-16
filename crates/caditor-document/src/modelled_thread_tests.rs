use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceOrigin, Interrupted, Solid, ThreadError};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{evaluate, pair},
    describe::describe_origin,
    thread::{ThreadCause, part_name},
    thread_tests::{
        class, failure, metric, plain_thread, result, round_face, set_hole, shaft, tapped, threaded,
    },
    *,
};

const ALONG: f64 = 1e-3;

fn shaft_of_length(length: &str) -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 4.0);
    let mut transaction = document.transaction("Build");
    let outline = transaction.add_feature("Shaft outline", FeatureKind::from(sketch));
    let shaft = transaction.add_feature(
        "Shaft",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored(length).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, shaft)
}

fn faces_made_by(solid: &Solid, feature: FeatureId) -> Vec<FaceOrigin> {
    solid
        .faces()
        .filter_map(|(_, face)| face.origin())
        .filter(|origin| origin.feature() == feature.raw())
        .collect()
}

#[test]
fn a_modelled_thread_cuts_its_shaft_and_is_placed_on_the_cut_body() {
    let (mut document, shaft) = shaft();
    let mut engine = Recompute::default();
    let face = round_face(evaluate(&document, &mut engine).body(shaft).unwrap());
    let depth = document.parse("3 mm").unwrap();
    let thread = threaded(&mut document, shaft, |_| Thread {
        length: ThreadLength::Depth(depth),
        form: ThreadForm::Modelled,
        ..plain_thread(face, shaft)
    });

    let evaluation = evaluate(&document, &mut engine);
    let status = evaluation.feature(thread).unwrap();
    let cut = faces_made_by(evaluation.body(shaft).unwrap(), thread);
    let placed = placed_threads(&document, &evaluation);

    assert_eq!(status.state, FeatureState::UpToDate, "{:?}", status.state);
    assert!(
        status
            .result
            .as_deref()
            .and_then(FeatureResult::solid)
            .is_some()
    );
    assert!(cut.len() > 4, "{cut:?}");
    assert_eq!(document.feature(thread).unwrap().body(), Some(shaft));
    assert!(document.feature(thread).unwrap().kind.modifies_body());
    assert_eq!(placed.len(), 1);
    assert!(placed[0].modelled);
    assert_eq!(placed[0].side, ThreadSide::External);
    assert_eq!(placed[0].designation, "M8-6g");
    assert!((placed[0].placement.start - Point3::new(0.0, 0.0, 20.0)).length() < ALONG);
    assert!((placed[0].placement.direction - -Vector3::Z).length() < ALONG);
    assert!((placed[0].placement.length - 3.0).abs() < 0.05);
    assert!(
        describe_origin(&document, cut.first().copied()).starts_with("Thread 2 thread"),
        "{}",
        describe_origin(&document, cut.first().copied())
    );
}

#[test]
fn showing_threads_as_cosmetic_evaluates_them_drawn_and_leaves_the_body_whole() {
    let (mut document, shaft) = shaft();
    let mut engine = Recompute::default();
    let face = round_face(evaluate(&document, &mut engine).body(shaft).unwrap());
    let thread = threaded(&mut document, shaft, |_| Thread {
        form: ThreadForm::Modelled,
        ..plain_thread(face, shaft)
    });

    let shown = document.threads_as_cosmetic();
    let evaluation = evaluate(&shown, &mut engine);
    let placed = placed_threads(&document, &evaluation);

    assert!(document.has_modelled_threads());
    assert!(!shown.has_modelled_threads());
    assert_eq!(
        shown.feature(thread).unwrap().kind.thread().unwrap().form,
        ThreadForm::Cosmetic
    );
    assert_eq!(result(&evaluation, thread).designation, "M8-6g");
    assert!(faces_made_by(evaluation.body(shaft).unwrap(), thread).is_empty());
    assert_eq!(placed.len(), 1);
    assert!(!placed[0].modelled);
}

#[test]
fn a_modelled_tapped_hole_cuts_its_thread_into_the_wall_and_places_it() {
    let mut pair = pair();
    let hole = tapped(&mut pair);
    set_hole(&mut pair, hole, |hole| {
        hole.thread.form = ThreadForm::Modelled
    });
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);
    let solid = evaluation.body(pair.plate).unwrap();
    let threaded: Vec<FaceOrigin> = faces_made_by(solid, hole)
        .into_iter()
        .filter(|origin| {
            origin
                .entity()
                .is_some_and(|entity| Hole::thread_part(entity).is_some())
        })
        .collect();
    let lands = faces_made_by(solid, hole)
        .into_iter()
        .filter(|origin| origin.entity().is_some_and(Hole::is_wall))
        .count();
    let placed = placed_threads(&pair.document, &evaluation);
    let described: Vec<String> = threaded
        .iter()
        .map(|origin| describe_origin(&pair.document, Some(*origin)))
        .collect();

    assert_eq!(evaluation.failed_count(), 0);
    assert!(threaded.len() > 4, "{threaded:?}");
    assert_eq!(lands, 1);
    assert!(
        described.iter().any(|text| text == "Hole 1 thread root"),
        "{described:?}"
    );
    assert_eq!(placed.len(), 1);
    assert!(placed[0].modelled);
    assert_eq!(placed[0].side, ThreadSide::Internal);
    assert_eq!(placed[0].designation, "M6-6H");
    assert!((placed[0].placement.start - Point3::new(5.0, 5.0, 4.0)).length() < ALONG);
    assert!((placed[0].placement.length - 3.0).abs() < 0.05);
    assert!((placed[0].placement.direction - -Vector3::Z).length() < ALONG);
}

#[test]
fn hole_thread_entities_keep_wall_and_part_apart_from_the_hole_s_own() {
    let wall = 7 * 16 + 1;

    for part in [1, 2, 8 * 40 + 4, 5, caditor_kernel::RING_ENTITY + 2] {
        let entity = Hole::thread_entity(wall, part);

        assert_eq!(Hole::thread_part(entity), Some((wall, part)));
        assert!(!Hole::is_wall(entity));
    }
    assert_eq!(Hole::thread_part(wall), None);
    assert_eq!(part_name(1), "thread root");
    assert_eq!(part_name(8 * 3 + 2), "thread flank");
    assert_eq!(part_name(caditor_kernel::RING_ENTITY), "thread crest");
}

#[test]
fn a_modelled_thread_of_too_many_turns_fails_alone_in_words() {
    let (mut document, shaft) = shaft_of_length("2000 mm");
    let mut engine = Recompute::default();
    let face = round_face(evaluate(&document, &mut engine).body(shaft).unwrap());
    let fine = ThreadSize::from_id(ThreadFamily::MetricFine, "M8x0.75").unwrap();
    let thread = threaded(&mut document, shaft, |_| Thread {
        size: fine,
        form: ThreadForm::Modelled,
        ..plain_thread(face, shaft)
    });

    let evaluation = evaluate(&document, &mut engine);
    let error = failure(&evaluation, thread);

    assert!(
        error
            .reason()
            .starts_with("The modelled M8x0.75 thread on Shaft side from Circle 1"),
        "{}",
        error.reason()
    );
    assert!(
        error
            .reason()
            .contains("more than the 2000 a modelled thread can")
    );
    assert_eq!(
        error.remedy(),
        "Thread a shorter length, or make the thread cosmetic."
    );
    assert_eq!(
        evaluation.feature(shaft).unwrap().state,
        FeatureState::UpToDate
    );
}

#[test]
fn kernel_thread_errors_are_put_in_words_naming_the_face() {
    let size = metric("M8");
    let face = "shaft side from circle 1";
    let cause = |error| ThreadCause::Modelled {
        size,
        side: ThreadSide::External,
        face: face.to_owned(),
        error,
    };

    let short = cause(ThreadError::TooShort {
        length: 0.5,
        needed: 2.25,
    });
    let narrow = cause(ThreadError::RootOutsideFace);

    assert_eq!(
        short.reason(),
        "The modelled M8 thread on shaft side from circle 1 is 0.5 mm long, shorter than the \
         2.25 mm its form needs."
    );
    assert_eq!(
        short.remedy(),
        "Thread a longer length or choose a finer pitch, or make the thread cosmetic."
    );
    assert_eq!(
        narrow.reason(),
        "The shaft at shaft side from circle 1 is too narrow for the root of a modelled M8 \
         thread, so there is nothing to cut."
    );
    assert!(matches!(
        ThreadCause::cutting(
            FeatureId::from_raw(1),
            size,
            ThreadSide::External,
            face,
            ThreadError::Cancelled(Interrupted)
        ),
        Failure::Cancelled
    ));
}

#[test]
fn modelled_profiles_follow_the_basic_forms_with_flat_crests_and_roots() {
    let m8 = metric("M8");
    let external = m8.modelled_profile(ThreadSide::External).unwrap();
    let internal = m8.modelled_profile(ThreadSide::Internal).unwrap();
    let basic_minor = (8.0 - 1.082_532 * 1.25) / 2.0;
    let pipe = ThreadSize::from_id(ThreadFamily::ParallelPipe, "1/2").unwrap();
    let pipe_profile = pipe.modelled_profile(ThreadSide::External).unwrap();
    let trapezoid = ThreadSize::from_id(ThreadFamily::Trapezoidal, "Tr 20x4").unwrap();
    let trapezoid_profile = trapezoid.modelled_profile(ThreadSide::External).unwrap();
    let taper = ThreadSize::from_id(ThreadFamily::TaperPipe, "1/2").unwrap();
    let crest_flat = |profile: &caditor_kernel::ThreadForm| {
        profile.pitch - 2.0 * profile.half_width_at(profile.crest_radius)
    };

    assert!((external.crest_radius - 4.0).abs() < 1e-9);
    assert!((external.root_radius - basic_minor).abs() < 1e-6);
    assert!((external.root_width - 1.25 / 4.0).abs() < 1e-5);
    assert!((internal.root_radius - 4.0).abs() < 1e-9);
    assert!((internal.crest_radius - basic_minor).abs() < 1e-6);
    assert!((internal.root_width - 1.25 / 8.0).abs() < 1e-5);
    assert!((crest_flat(&pipe_profile) - pipe.pitch() / 6.0).abs() < 1e-4);
    assert!((pipe_profile.root_width - pipe.pitch() / 6.0).abs() < 1e-4);
    assert!((crest_flat(&trapezoid_profile) - 4.0 * 0.366_025).abs() < 1e-4);
    assert_eq!(taper.modelled_profile(ThreadSide::External), None);
    assert!(!ThreadFamily::TaperPipe.can_be_modelled());
    assert_eq!(class(ThreadFamily::MetricCoarse, "6H").id(), "6H");
}
