use std::collections::BTreeMap;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{Curve2, EdgeName, ViewSource};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{Pair, evaluate, pair},
    thread_tests::{plain_thread, round_face, shaft, threaded},
    *,
};

pub(crate) struct Drilled {
    pub(crate) pair: Pair,
    pub(crate) hole: FeatureId,
}

fn length(document: &Document, text: &str) -> Expression {
    document.parse(text).unwrap()
}

pub(crate) fn drilled_with(style: impl FnOnce(&Document) -> HoleStyle) -> Drilled {
    let mut pair = pair();
    let mut sketch =
        Sketch::new(Plane::from_frame(Point3::new(0.0, 0.0, 4.0), Vector3::Z, Vector3::X).unwrap());

    sketch.add_point(Point2::new(5.0, 5.0));

    let style = style(&pair.document);
    let hole = Hole {
        sketch: FeatureId::from_raw(0),
        body: pair.plate,
        diameter: length(&pair.document, "4 mm"),
        depth: HoleDepth::Blind(length(&pair.document, "3 mm")),
        style,
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
        sizing: HoleSizing::Typed,
        bottom: HoleBottom::Flat,
        thread: TappedThread::default(),
    };
    let mut transaction = pair.document.transaction("Drill");

    let sketch = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    let hole = transaction.add_feature("Hole 1", FeatureKind::Hole(Hole { sketch, ..hole }));

    pair.document.apply(transaction.finish()).unwrap();
    Drilled { pair, hole }
}

pub(crate) fn drilled() -> Drilled {
    drilled_with(|_| HoleStyle::Plain)
}

pub(crate) fn plate_drawing(pair: &Pair) -> Drawing {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    drawing.bodies = vec![pair.plate];
    drawing
}

pub(crate) fn lines_in(document: &Document, drawing: &Drawing, view: StandardView) -> ViewLines {
    let evaluation = evaluate(document, &mut Recompute::default());
    let bodies = drawn_bodies(document, &evaluation, drawing);
    drawing_view(view, &bodies.solids()).unwrap()
}

pub(crate) fn round_edge_at(lines: &ViewLines, centre: Point2) -> EdgeName {
    lines
        .pieces
        .iter()
        .find_map(|piece| match (piece.source, &piece.curve) {
            (ViewSource::Edge { name, .. }, Curve2::Circle(circle))
                if circle.center().distance(centre) < 1e-6 =>
            {
                Some(name)
            }
            _ => None,
        })
        .expect("a round edge there")
}

fn straight_edge(lines: &ViewLines) -> EdgeName {
    lines
        .pieces
        .iter()
        .find_map(|piece| match (piece.source, &piece.curve) {
            (ViewSource::Edge { name, .. }, Curve2::Line(_)) => Some(name),
            _ => None,
        })
        .expect("a straight edge")
}

fn written(lines: &[CalloutLine]) -> Vec<String> {
    lines.iter().map(CalloutLine::text).collect()
}

fn read(drilled: &Drilled) -> Vec<String> {
    let evaluation = evaluate(&drilled.pair.document, &mut Recompute::default());
    written(
        &callout_lines(
            &drilled.pair.document,
            &evaluation.parameters,
            drilled.hole,
            None,
        )
        .unwrap(),
    )
}

fn reshaped(drilled: &mut Drilled, change: impl FnOnce(&Document, &mut Hole)) {
    let document = &mut drilled.pair.document;
    let mut hole = document
        .feature(drilled.hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone();

    change(document, &mut hole);
    document
        .apply(Transaction::single(
            "Change hole",
            Edit::SetFeatureKind {
                id: drilled.hole,
                kind: FeatureKind::Hole(hole),
            },
        ))
        .unwrap();
}

fn callout(drilled: &Drilled, edge: EdgeName) -> DrawingCallout {
    DrawingCallout {
        id: AnnotationId::from_raw(0),
        view: DrawnView::Standard(StandardView::Top),
        feature: drilled.hole,
        edge,
        at: Point2::new(-60.0, 40.0),
    }
}

fn laid_out(document: &Document, drawing: &Drawing, view: StandardView) -> SheetPlan {
    let evaluation = evaluate(document, &mut Recompute::default());
    let lines = lines_in(document, drawing, view);
    let drawn = DrawnView::Standard(view);

    let extents = BTreeMap::from([(drawn, lines.bounds.unwrap())]);
    let mut plan = lay_out_sheet(drawing, &TitleFields::default(), &extents, &BTreeMap::new());
    let texts = callout_texts(document, &evaluation, drawing);

    plan.annotate(drawing, &DrawnLines::from([(drawn, &lines)]), &texts);
    plan
}

#[test]
fn a_hole_s_edge_names_the_hole_and_another_edge_is_refused() {
    let drilled = drilled();
    let drawing = plate_drawing(&drilled.pair);

    let lines = lines_in(&drilled.pair.document, &drawing, StandardView::Top);
    let evaluation = evaluate(&drilled.pair.document, &mut Recompute::default());

    let mouth = round_edge_at(&lines, Point2::new(5.0, 5.0));
    let outline = straight_edge(&lines);

    assert_eq!(
        callout_feature(&drilled.pair.document, &evaluation, &drawing, mouth),
        Ok(drilled.hole)
    );
    assert_eq!(
        callout_feature(&drilled.pair.document, &evaluation, &drawing, outline),
        Err(CalloutRefusal::NotAHole)
    );
}

#[test]
fn a_plain_hole_reads_its_diameter_and_depth_and_follows_an_edit() {
    let mut drilled = drilled();

    assert_eq!(read(&drilled), ["\u{2300}4 \u{21a7}3"]);

    reshaped(&mut drilled, |document, hole| {
        hole.diameter = length(document, "6.6 mm");
        hole.depth = HoleDepth::ThroughAll;
    });

    assert_eq!(read(&drilled), ["\u{2300}6.6"]);
}

#[test]
fn counterbores_countersinks_and_steps_add_a_line_each_with_their_symbol() {
    let bored = drilled_with(|document| HoleStyle::Counterbore {
        diameter: length(document, "6 mm"),
        depth: length(document, "1 mm"),
    });
    let sunk = drilled_with(|document| HoleStyle::Countersink {
        diameter: length(document, "6 mm"),
        angle: document.parse("90 deg").unwrap(),
    });
    let stepped = drilled_with(|document| {
        HoleStyle::Stepped(vec![
            HoleStep {
                diameter: length(document, "8 mm"),
                depth: length(document, "0.5 mm"),
            },
            HoleStep {
                diameter: length(document, "6 mm"),
                depth: length(document, "1 mm"),
            },
        ])
    });

    assert_eq!(
        read(&bored),
        ["\u{2300}4 \u{21a7}3", "\u{2334}\u{2300}6 \u{21a7}1"]
    );
    assert_eq!(
        read(&sunk),
        ["\u{2300}4 \u{21a7}3", "\u{2335}\u{2300}6 \u{d7} 90\u{b0}"]
    );
    assert_eq!(
        read(&stepped),
        [
            "\u{2300}4 \u{21a7}3",
            "\u{2334}\u{2300}8 \u{21a7}0.5",
            "\u{2334}\u{2300}6 \u{21a7}1"
        ]
    );
}

#[test]
fn a_tapped_hole_reads_its_thread_above_the_drill() {
    let mut drilled = drilled();

    reshaped(&mut drilled, |document, hole| {
        hole.standard = Some(HoleStandard {
            size: MetricSize::M3,
            fit: HoleFit::Tapped,
        });
        hole.diameter = length(document, "2.5 mm");
        hole.thread.depth = Some(length(document, "2 mm"));
    });

    assert_eq!(read(&drilled), ["M3-6H \u{21a7}2", "\u{2300}2.5 \u{21a7}3"]);
}

#[test]
fn a_thread_feature_is_found_from_its_face_s_edge_and_reads_its_designation() {
    let (mut document, body) = shaft();
    let mut engine = Recompute::default();

    let face = round_face(evaluate(&document, &mut engine).body(body).unwrap());
    let thread = threaded(&mut document, body, |document| Thread {
        length: ThreadLength::Depth(document.parse("12 mm").unwrap()),
        ..plain_thread(face, body)
    });

    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    drawing.bodies = vec![body];

    let lines = lines_in(&document, &drawing, StandardView::Top);
    let evaluation = evaluate(&document, &mut engine);
    let rim = round_edge_at(&lines, Point2::ZERO);

    assert_eq!(
        callout_feature(&document, &evaluation, &drawing, rim),
        Ok(thread)
    );
    assert_eq!(
        written(&callout_lines(&document, &evaluation.parameters, thread, None).unwrap()),
        ["M8-6g \u{21a7}12"]
    );
}

#[test]
fn a_callout_points_its_leader_at_the_circle_and_says_what_it_lost() {
    let drilled = drilled();
    let document = &drilled.pair.document;

    let lines = lines_in(document, &plate_drawing(&drilled.pair), StandardView::Top);
    let mouth = round_edge_at(&lines, Point2::new(5.0, 5.0));

    let mut drawing = plate_drawing(&drilled.pair);

    drawing.views = [StandardView::Top].into_iter().collect();
    drawing.callouts.push(callout(&drilled, mouth));

    let plan = laid_out(document, &drawing, StandardView::Top);
    let laid = plan
        .annotation(Annotation::Callout(AnnotationId::from_raw(0)))
        .unwrap();
    let placed = plan.placed(DrawnView::Standard(StandardView::Top)).unwrap();
    let centre = placed.on_sheet(Point2::new(5.0, 5.0));
    let radius = 2.0 * placed.scale;

    assert_eq!(laid.problem, None);
    assert!(laid.texts.iter().any(|text| text.content == "4"));
    assert!(laid.marks.iter().any(|mark| match *mark {
        SheetMark::Line(_, end) => (end.distance(centre) - radius).abs() < 1e-6,
        _ => false,
    }));

    let mut lost = drawing.clone();

    lost.callouts[0].edge = EdgeName::from_digest(0xdead);

    let plan = laid_out(document, &lost, StandardView::Top);
    let laid = plan
        .annotation(Annotation::Callout(AnnotationId::from_raw(0)))
        .unwrap();

    assert_eq!(laid.problem, Some(AnnotationProblem::EdgeGone));
    assert_eq!(laid.content(), "?");

    let mut gone = drawing;

    gone.callouts[0].feature = FeatureId::from_raw(4096);

    let plan = laid_out(document, &gone, StandardView::Top);
    let laid = plan
        .annotation(Annotation::Callout(AnnotationId::from_raw(0)))
        .unwrap();

    assert_eq!(laid.problem, Some(AnnotationProblem::FeatureGone));
    assert_eq!(laid.content(), "?");
}

#[test]
fn a_callout_is_added_undoably_keeps_its_feature_and_is_refused_off_the_sheet() {
    let drilled = drilled();
    let mut editor = Editor::new(drilled.pair.document.clone());

    let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();

    editor.apply(transaction).unwrap();

    let adding = editor
        .document()
        .changing_drawing(id, "Add callout", |drawing| {
            let number = drawing.new_annotation_id();

            drawing.callouts.push(DrawingCallout {
                id: number,
                ..callout(&drilled, EdgeName::from_digest(9))
            });
        })
        .unwrap();

    editor.apply(adding).unwrap();

    let stored = editor.document().drawing(id).unwrap().callouts.clone();

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].feature, drilled.hole);

    editor.undo().unwrap();

    assert!(editor.document().drawing(id).unwrap().callouts.is_empty());

    let refused = editor
        .document()
        .changing_drawing(id, "Add callout", |drawing| {
            drawing.callouts.push(DrawingCallout {
                at: Point2::new(5000.0, 0.0),
                ..callout(&drilled, EdgeName::from_digest(9))
            });
        });

    assert!(matches!(refused, Err(EditError::CalloutPlaceOutOfRange)));
}
