use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{FaceName, FaceReference, Solid, Surface};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{Pair, evaluate, pair},
    *,
};

const CLOSE: f64 = 1e-6;

fn shaft() -> (Document, FeatureId) {
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
            extent: ExtrudeExtent::one_side(Expression::parse_stored("20 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, shaft)
}

fn round_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Cylinder(_)))
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn flat_face(solid: &Solid) -> FaceReference {
    let (id, _) = solid
        .faces()
        .find(|(_, face)| matches!(face.surface(), Surface::Plane(_)))
        .unwrap();
    FaceReference::capture(solid, id).unwrap()
}

fn metric(name: &str) -> ThreadSize {
    ThreadSize::from_id(ThreadFamily::MetricCoarse, name).unwrap()
}

fn class(family: ThreadFamily, id: &str) -> ThreadClass {
    family.class_from_id(id).unwrap()
}

fn threaded(
    document: &mut Document,
    body: FeatureId,
    thread: impl FnOnce(&Document) -> Thread,
) -> FeatureId {
    let thread = thread(document);
    let name = format!("Thread {}", document.features().len());
    let mut transaction = document.transaction("Thread");
    let feature = transaction.add_feature(name, FeatureKind::Thread(Thread { body, ..thread }));
    document.apply(transaction.finish()).unwrap();
    feature
}

fn plain_thread(face: FaceReference, body: FeatureId) -> Thread {
    Thread {
        body,
        face,
        size: metric("M8"),
        class: class(ThreadFamily::MetricCoarse, "6g"),
        hand: ThreadHand::Right,
        length: ThreadLength::Full,
        reversed: false,
    }
}

fn result(evaluation: &Evaluation, feature: FeatureId) -> ThreadResult {
    let status = evaluation.feature(feature).unwrap();
    assert_eq!(status.state, FeatureState::UpToDate, "{:?}", status.state);
    status.result.as_deref().unwrap().thread().unwrap().clone()
}

fn failure(evaluation: &Evaluation, feature: FeatureId) -> FeatureError {
    match &evaluation.feature(feature).unwrap().state {
        FeatureState::Failed(error) => error.clone(),
        other => panic!("expected a failure, found {other:?}"),
    }
}

fn near(first: Point3, second: Point3) -> bool {
    (first - second).length() < CLOSE
}

#[test]
fn a_shaft_takes_an_external_thread_drawn_at_its_minor_diameter_from_its_free_end() {
    let (mut document, shaft) = shaft();
    let mut engine = Recompute::default();
    let face = round_face(evaluate(&document, &mut engine).body(shaft).unwrap());
    let thread = threaded(&mut document, shaft, |_| plain_thread(face, shaft));

    let evaluation = evaluate(&document, &mut engine);
    let evaluated = result(&evaluation, thread);

    assert_eq!(evaluated.side, ThreadSide::External);
    assert_eq!(evaluated.designation, "M8-6g");
    assert!((evaluated.placement.length - 20.0).abs() < CLOSE);
    assert!((evaluated.placement.radius - 4.0).abs() < CLOSE);
    assert!((evaluated.placement.thread_radius - (8.0 - 1.226_869 * 1.25) / 2.0).abs() < CLOSE);
    assert!(near(evaluated.placement.start, Point3::new(0.0, 0.0, 20.0)));
    assert!((evaluated.placement.direction - -Vector3::Z).length() < CLOSE);
    assert_eq!(
        document.feature(thread).unwrap().kind.dependencies(),
        [shaft].into()
    );
}

#[test]
fn a_depth_runs_from_the_chosen_end_and_stops_at_the_face_s_end() {
    let (mut document, shaft) = shaft();
    let mut engine = Recompute::default();
    let face = round_face(evaluate(&document, &mut engine).body(shaft).unwrap());
    let thread = threaded(&mut document, shaft, |document| Thread {
        length: ThreadLength::Depth(document.parse("12 mm").unwrap()),
        reversed: true,
        ..plain_thread(face.clone(), shaft)
    });

    let placement = result(&evaluate(&document, &mut engine), thread).placement;
    assert!((placement.length - 12.0).abs() < CLOSE);
    assert!(near(placement.start, Point3::ZERO));
    assert!(near(placement.end(), Point3::new(0.0, 0.0, 12.0)));

    let mut longer = document.feature(thread).unwrap().kind.clone();
    if let FeatureKind::Thread(thread) = &mut longer {
        thread.length = ThreadLength::Depth(document.parse("50 mm").unwrap());
    }
    document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: thread,
                kind: longer,
            },
        ))
        .unwrap();
    let placement = result(&evaluate(&document, &mut engine), thread).placement;
    assert!((placement.length - 20.0).abs() < CLOSE);
}

#[test]
fn a_wrong_class_size_or_face_fails_the_thread_alone_in_words() {
    let (mut document, shaft) = shaft();
    let mut engine = Recompute::default();
    let solid = evaluate(&document, &mut engine)
        .body(shaft)
        .unwrap()
        .clone();
    let face = round_face(&solid);
    let inside = threaded(&mut document, shaft, |_| Thread {
        class: class(ThreadFamily::MetricCoarse, "6H"),
        ..plain_thread(face.clone(), shaft)
    });
    let large = threaded(&mut document, shaft, |_| Thread {
        size: metric("M12"),
        ..plain_thread(face.clone(), shaft)
    });
    let flat = threaded(&mut document, shaft, |_| {
        plain_thread(flat_face(&solid), shaft)
    });
    let lost = threaded(&mut document, shaft, |_| {
        plain_thread(
            FaceReference::new(FaceName::from_digest(0xdead), None, []),
            shaft,
        )
    });

    let evaluation = evaluate(&document, &mut engine);

    let error = failure(&evaluation, inside);
    assert_eq!(
        error.reason,
        "The threaded face is a shaft, but the class 6H is not one ISO metric coarse offers for \
         an external thread."
    );
    assert_eq!(
        error.remedy,
        "Choose a class for an external thread, such as 6g."
    );
    let error = failure(&evaluation, large);
    assert_eq!(
        error.reason,
        "The shaft is 8 mm across, too narrow for the M12 thread, whose minor diameter is 9.85 mm."
    );
    assert_eq!(
        error.remedy,
        "Choose M8 instead, or change the shaft's diameter."
    );
    assert!(
        failure(&evaluation, flat)
            .reason
            .ends_with("is not cylindrical, so it cannot carry a thread.")
    );
    assert_eq!(
        failure(&evaluation, lost).reason,
        "The threaded face is no longer part of the body of Shaft."
    );
    assert_eq!(evaluation.body(shaft).unwrap(), &solid);
}

fn tapped(pair: &mut Pair) -> FeatureId {
    let mut sketch =
        Sketch::new(Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap());
    sketch.add_point(Point2::new(5.0, 5.0));
    let mut transaction = pair.document.transaction("Drill");
    let sketch = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    let hole = transaction.add_feature(
        "Hole 1",
        FeatureKind::Hole(Hole {
            sketch,
            body: pair.plate,
            diameter: pair.document.parse("5 mm").unwrap(),
            depth: HoleDepth::Blind(pair.document.parse("3 mm").unwrap()),
            style: HoleStyle::Plain,
            reversed: false,
            shape: HoleShape::Round,
            standard: Some(HoleStandard {
                size: MetricSize::M6,
                fit: HoleFit::Tapped,
            }),
            sizing: HoleSizing::Typed,
            bottom: HoleBottom::Flat,
        }),
    );
    pair.document.apply(transaction.finish()).unwrap();
    hole
}

#[test]
fn a_tapped_hole_threads_its_bore_from_the_mouth_and_a_clearance_hole_does_not() {
    let mut pair = pair();
    let hole = tapped(&mut pair);
    let mut engine = Recompute::default();

    let evaluation = evaluate(&pair.document, &mut engine);
    let placed = placed_threads(&pair.document, &evaluation);

    assert_eq!(placed.len(), 1);
    let thread = &placed[0];
    assert_eq!((thread.feature, thread.body), (hole, pair.plate));
    assert_eq!(thread.designation, "M6-6H");
    assert_eq!(thread.side, ThreadSide::Internal);
    assert!(near(thread.placement.start, Point3::new(5.0, 5.0, 4.0)));
    assert!(near(thread.placement.end(), Point3::new(5.0, 5.0, 1.0)));
    assert!((thread.placement.thread_radius - 3.0).abs() < CLOSE);

    let mut kind = pair.document.feature(hole).unwrap().kind.clone();
    if let FeatureKind::Hole(hole) = &mut kind {
        hole.standard = Some(HoleStandard {
            size: MetricSize::M6,
            fit: HoleFit::Normal,
        });
    }
    pair.document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind { id: hole, kind },
        ))
        .unwrap();
    let evaluation = evaluate(&pair.document, &mut engine);
    assert!(placed_threads(&pair.document, &evaluation).is_empty());
}

#[test]
fn a_thread_follows_its_face_when_a_later_feature_moves_the_body() {
    let mut pair = pair();
    let hole = tapped(&mut pair);
    let mut engine = Recompute::default();
    let solid = evaluate(&pair.document, &mut engine)
        .body(pair.plate)
        .unwrap()
        .clone();
    let wall = solid
        .faces()
        .find(|(_, face)| {
            matches!(face.origin(), Some(caditor_kernel::FaceOrigin::Side { feature, .. }) if feature == hole.raw())
                && matches!(face.surface(), Surface::Cylinder(_))
        })
        .map(|(id, _)| FaceReference::capture(&solid, id).unwrap())
        .unwrap();
    let plate = pair.plate;
    let thread = threaded(&mut pair.document, plate, |_| Thread {
        size: metric("M6"),
        class: class(ThreadFamily::MetricCoarse, "6H"),
        hand: ThreadHand::Left,
        ..plain_thread(wall, plate)
    });
    let mut transaction = pair.document.transaction("Move");
    let length = |text: &str| Expression::parse_stored(text).unwrap();
    let movement = Move {
        body: plate,
        offset: [length("10 mm"), length("0 mm"), length("0 mm")],
        turn: [length("0 deg"), length("0 deg"), length("0 deg")],
        copy: false,
        about: TurnCentre::Origin,
        frame: None,
    };
    transaction.add_feature("Move 1", FeatureKind::Move(movement));
    pair.document.apply(transaction.finish()).unwrap();

    let evaluation = evaluate(&pair.document, &mut engine);
    let placed: Vec<PlacedThread> = placed_threads(&pair.document, &evaluation)
        .into_iter()
        .filter(|placed| placed.feature == thread)
        .collect();

    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].designation, "M6-6H-LH");
    assert!(near(placed[0].placement.start, Point3::new(15.0, 5.0, 4.0)));
}

#[test]
fn designations_follow_each_standard_s_notation() {
    let text = |family: ThreadFamily, size: &str, class_id: &str, hand: ThreadHand| {
        ThreadDesignation {
            size: ThreadSize::from_id(family, size).unwrap(),
            class: class(family, class_id),
            hand,
        }
        .text()
    };

    assert_eq!(
        text(ThreadFamily::MetricCoarse, "M20", "6g", ThreadHand::Right),
        "M20-6g"
    );
    assert_eq!(
        text(ThreadFamily::MetricFine, "M8x1", "6H", ThreadHand::Left),
        "M8x1-6H-LH"
    );
    assert_eq!(
        text(ThreadFamily::ParallelPipe, "1/2", "A", ThreadHand::Right),
        "G 1/2 A"
    );
    assert_eq!(
        text(ThreadFamily::ParallelPipe, "1 1/4", "", ThreadHand::Right),
        "G 1 1/4"
    );
    assert_eq!(
        text(ThreadFamily::TaperPipe, "3/4", "Rc", ThreadHand::Right),
        "Rc 3/4"
    );
    assert_eq!(
        text(ThreadFamily::TaperPipe, "1", "R", ThreadHand::Left),
        "R 1 LH"
    );
    assert_eq!(
        text(ThreadFamily::Trapezoidal, "Tr 20x4", "7e", ThreadHand::Left),
        "Tr 20x4 LH-7e"
    );
    assert_eq!(ThreadFamily::MetricCoarse.sizes().len(), 28);
    assert!(
        ThreadFamily::MetricFine
            .sizes()
            .iter()
            .any(|size| size.id() == "M64x4")
    );
    assert_eq!(
        ThreadSize::of_hole(HoleStandard {
            size: MetricSize::M12,
            fit: HoleFit::TappedFine(FinePitch::First),
        })
        .map(ThreadSize::id),
        Some("M12x1.25".to_owned())
    );
    assert_eq!(
        ThreadFamily::MetricCoarse
            .nearest(10.2, ThreadSide::Internal)
            .id(),
        "M12"
    );
}
