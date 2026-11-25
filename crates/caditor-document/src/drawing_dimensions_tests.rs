use std::collections::BTreeMap;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector2};
use caditor_kernel::{Curve2, EdgeName, ViewSource};
use caditor_sketch::Sketch;

use crate::{
    combine_tests::{Pair, evaluate, pair},
    *,
};

fn plate_drawing(pair: &Pair) -> Drawing {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    drawing.bodies = vec![pair.plate];
    drawing
}

fn lines_of(document: &Document, drawing: &Drawing, view: StandardView) -> ViewLines {
    let evaluation = evaluate(document, &mut Recompute::default());
    let bodies = drawn_bodies(document, &evaluation, drawing);
    drawing_view(view, &bodies.solids()).unwrap()
}

fn edge_where(lines: &ViewLines, wanted: impl Fn(Point2, Point2) -> bool) -> EdgeName {
    lines
        .pieces
        .iter()
        .find_map(|piece| match (piece.source, &piece.curve) {
            (ViewSource::Edge { name, .. }, Curve2::Line(_)) => {
                let start = piece.curve.point(piece.range.start());
                let end = piece.curve.point(piece.range.end());
                wanted(start, end).then_some(name)
            }
            _ => None,
        })
        .expect("an edge like that is in the view")
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

fn top_edge(lines: &ViewLines) -> EdgeName {
    edge_where(lines, |start, end| near(start.y, 4.0) && near(end.y, 4.0))
}

fn bottom_edge(lines: &ViewLines) -> EdgeName {
    edge_where(lines, |start, end| near(start.y, 0.0) && near(end.y, 0.0))
}

fn left_edge(lines: &ViewLines) -> EdgeName {
    edge_where(lines, |start, end| near(start.x, 0.0) && near(end.x, 0.0))
}

fn length_of(edge: EdgeName) -> DimensionMeasure {
    DimensionMeasure::Linear {
        from: EdgePoint {
            edge,
            spot: EdgeSpot::Start,
        },
        to: EdgePoint {
            edge,
            spot: EdgeSpot::End,
        },
        direction: DimensionDirection::Aligned,
    }
}

fn dimension(id: u64, measure: DimensionMeasure, place: Point2) -> DrawingDimension {
    DrawingDimension {
        id: AnnotationId::from_raw(id),
        view: DrawnView::Standard(StandardView::Front),
        measure,
        place,
        decimals: DEFAULT_DIMENSION_DECIMALS,
        text_offset: 0.0,
    }
}

fn front(view: StandardView) -> DrawnView {
    DrawnView::Standard(view)
}

fn annotated(drawing: &Drawing, lines: &ViewLines) -> SheetPlan {
    let extents = BTreeMap::from([(front(StandardView::Front), lines.bounds.unwrap())]);
    let mut plan = lay_out_sheet(drawing, &TitleFields::default(), &extents, &BTreeMap::new());
    let drawn = DrawnLines::from([(front(StandardView::Front), lines)]);

    plan.annotate(drawing, &drawn);
    plan
}

fn front_only(mut drawing: Drawing) -> Drawing {
    drawing.views = [StandardView::Front].into_iter().collect();
    drawing
}

#[test]
fn a_dimension_is_added_undoably_and_its_number_is_never_reused() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());

    let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();

    editor.apply(transaction).unwrap();

    let before = editor.document().clone();
    let mut first = None;

    let adding = editor
        .document()
        .changing_drawing(id, "Add dimension", |drawing| {
            let number = drawing.new_annotation_id();

            first = Some(number);
            drawing.dimensions.push(DrawingDimension {
                id: number,
                ..dimension(0, length_of(EdgeName::from_digest(7)), Point2::ZERO)
            });
        })
        .unwrap();

    editor.apply(adding).unwrap();
    editor.undo().unwrap();

    let mut second = None;

    let again = editor
        .document()
        .changing_drawing(id, "Add note", |drawing| {
            let number = drawing.new_annotation_id();

            second = Some(number);
            drawing.notes.push(DrawingNote {
                id: number,
                text: "  Break   all edges ".into(),
                at: Point2::new(10.0, -20.0),
                leader: None,
            });
        })
        .unwrap();

    editor.apply(again).unwrap();

    let drawing = editor.document().drawing(id).unwrap();

    assert!(editor.document().drawing(id).is_some());
    assert!(second > first);
    assert_eq!(drawing.notes[0].text, "Break all edges");
    assert!(drawing.dimensions.is_empty());

    editor.undo().unwrap();

    assert!(editor.document().same_content(&before));
}

#[test]
fn dimensions_and_notes_are_checked() {
    let mut document = pair().document;

    let (transaction, id) = document.adding_drawing(Vec::new()).unwrap();

    document.apply(transaction).unwrap();

    let edge = EdgeName::from_digest(9);
    let change = |change: &dyn Fn(&mut Drawing)| {
        document
            .changing_drawing(id, "Change", |drawing| change(drawing))
            .err()
    };

    let decimals = change(&|drawing| {
        drawing.dimensions.push(DrawingDimension {
            decimals: MAX_DIMENSION_DECIMALS + 1,
            ..dimension(0, length_of(edge), Point2::ZERO)
        });
    });
    let far = change(&|drawing| {
        drawing
            .dimensions
            .push(dimension(0, length_of(edge), Point2::new(f64::NAN, 0.0)));
    });
    let repeated = change(&|drawing| {
        drawing
            .dimensions
            .push(dimension(3, length_of(edge), Point2::ZERO));
        drawing.notes.push(DrawingNote {
            id: AnnotationId::from_raw(3),
            text: "Note".into(),
            at: Point2::ZERO,
            leader: None,
        });
    });
    let empty = change(&|drawing| {
        drawing.notes.push(DrawingNote {
            id: AnnotationId::from_raw(0),
            text: " \n ".into(),
            at: Point2::ZERO,
            leader: None,
        });
    });
    let long = change(&|drawing| {
        drawing.notes.push(DrawingNote {
            id: AnnotationId::from_raw(0),
            text: "x".repeat(MAX_NOTE_CHARS + 1),
            at: Point2::ZERO,
            leader: None,
        });
    });

    assert_eq!(
        decimals,
        Some(EditError::DimensionDecimals(MAX_DIMENSION_DECIMALS + 1))
    );
    assert_eq!(far, Some(EditError::DimensionPlaceOutOfRange));
    assert_eq!(repeated, Some(EditError::AnnotationIdRepeated(3)));
    assert_eq!(empty, Some(EditError::DrawingNoteEmpty));
    assert_eq!(
        long,
        Some(EditError::DrawingNoteTooLong {
            length: MAX_NOTE_CHARS + 1
        })
    );
}

#[test]
fn dimensions_measure_the_plate_in_its_front_view() {
    let pair = pair();
    let drawing = plate_drawing(&pair);

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);

    let (top, bottom, left) = (top_edge(&lines), bottom_edge(&lines), left_edge(&lines));
    let value = |measure: DimensionMeasure, place: Point2| {
        dimension_geometry(&measure, place, &lines).map(|geometry| geometry.value())
    };

    assert!(near(
        value(length_of(top), Point2::new(10.0, 9.0)).unwrap(),
        20.0
    ));
    assert!(near(
        value(length_of(left), Point2::new(-5.0, 2.0)).unwrap(),
        4.0
    ));
    assert!(near(
        value(
            DimensionMeasure::Parallel {
                first: top,
                second: bottom
            },
            Point2::new(25.0, 2.0)
        )
        .unwrap(),
        4.0
    ));
    assert!(near(
        value(
            DimensionMeasure::Angle {
                first: top,
                second: left
            },
            Point2::new(3.0, 1.0)
        )
        .unwrap(),
        90.0
    ));
    assert_eq!(
        value(
            DimensionMeasure::Angle {
                first: top,
                second: bottom
            },
            Point2::ZERO
        ),
        Err(AnnotationProblem::NoAngle)
    );
    assert_eq!(
        value(DimensionMeasure::Radius(top), Point2::ZERO),
        Err(AnnotationProblem::NotRound)
    );
}

#[test]
fn picked_edges_make_the_dimension_the_user_means() {
    let pair = pair();
    let drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);
    let drawn = DrawnLines::from([(front(StandardView::Front), &lines)]);

    let (top, bottom, left) = (top_edge(&lines), bottom_edge(&lines), left_edge(&lines));
    let pick = |edge, at| EdgePick {
        view: front(StandardView::Front),
        edge,
        at,
    };
    let made = |first, second, place| dimension_from_picks(&drawing, &drawn, first, second, place);

    let along = made(
        pick(top, Point2::new(5.0, 4.0)),
        pick(top, Point2::new(15.0, 4.0)),
        Point2::new(10.0, 12.0),
    );
    let between = made(
        pick(top, Point2::new(5.0, 4.0)),
        pick(bottom, Point2::new(5.0, 0.0)),
        Point2::new(30.0, 2.0),
    );
    let corner = made(
        pick(top, Point2::new(5.0, 4.0)),
        pick(left, Point2::new(0.0, 1.0)),
        Point2::new(4.0, 2.0),
    );
    let elsewhere = made(
        pick(top, Point2::new(5.0, 4.0)),
        EdgePick {
            view: front(StandardView::Top),
            ..pick(bottom, Point2::ZERO)
        },
        Point2::ZERO,
    );
    let isometric = made(
        EdgePick {
            view: front(StandardView::Isometric),
            ..pick(top, Point2::ZERO)
        },
        EdgePick {
            view: front(StandardView::Isometric),
            ..pick(top, Point2::ZERO)
        },
        Point2::ZERO,
    );

    assert_eq!(
        along,
        Ok(DimensionMeasure::Linear {
            from: EdgePoint {
                edge: top,
                spot: EdgeSpot::Start
            },
            to: EdgePoint {
                edge: top,
                spot: EdgeSpot::End
            },
            direction: DimensionDirection::Horizontal,
        })
    );
    assert_eq!(
        between,
        Ok(DimensionMeasure::Parallel {
            first: top,
            second: bottom
        })
    );
    assert_eq!(
        corner,
        Ok(DimensionMeasure::Angle {
            first: top,
            second: left
        })
    );
    assert_eq!(elsewhere, Err(DimensionRefusal::DifferentViews));
    assert_eq!(isometric, Err(DimensionRefusal::Isometric));
}

fn taller(document: &mut Document, plate: FeatureId, height: &str) {
    let mut kind = document.feature(plate).unwrap().kind.clone();

    if let FeatureKind::Solid(SolidFeature::Extrude(extrude)) = &mut kind {
        extrude.extent = ExtrudeExtent::one_side(Expression::parse_stored(height).unwrap(), false);
    }
    document
        .apply(Transaction::single(
            "Taller",
            Edit::SetFeatureKind { id: plate, kind },
        ))
        .unwrap();
}

#[test]
fn a_dimension_follows_its_edges_through_an_upstream_edit() {
    let mut pair = pair();

    let drawing = plate_drawing(&pair);
    let before = lines_of(&pair.document, &drawing, StandardView::Front);

    let measure = DimensionMeasure::Parallel {
        first: top_edge(&before),
        second: bottom_edge(&before),
    };
    let side = length_of(left_edge(&before));

    taller(&mut pair.document, pair.plate, "7 mm");

    let after = lines_of(&pair.document, &drawing, StandardView::Front);
    let value = |measure| {
        dimension_geometry(&measure, Point2::new(25.0, 2.0), &after)
            .unwrap()
            .value()
    };

    assert!(near(value(measure), 7.0));
    assert!(near(value(side), 7.0));
}

#[test]
fn a_dimension_whose_edge_is_gone_is_kept_drawn_dashed_and_says_why() {
    let pair = pair();
    let mut drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);

    drawing.dimensions = vec![
        dimension(0, length_of(top_edge(&lines)), Point2::new(10.0, 9.0)),
        dimension(
            1,
            DimensionMeasure::Diameter(EdgeName::from_digest(0xdead)),
            Point2::new(5.0, 5.0),
        ),
    ];

    let plan = annotated(&drawing, &lines);
    let drawn = DrawnLines::from([(front(StandardView::Front), &lines)]);
    let lost = plan
        .annotation(Annotation::Dimension(AnnotationId::from_raw(1)))
        .unwrap();
    let kept = plan
        .annotation(Annotation::Dimension(AnnotationId::from_raw(0)))
        .unwrap();

    assert_eq!(plan.annotations.len(), 2);
    assert_eq!(lost.problem, Some(AnnotationProblem::EdgeGone));
    assert_eq!(lost.text.content, "⌀?");
    assert_eq!(lost.measure, None);
    assert_eq!(kept.problem, None);
    assert_eq!(kept.text.content, "20");
    assert_eq!(
        dimension_problem(&drawing, &drawing.dimensions[1], &drawn),
        Some(Err(AnnotationProblem::EdgeGone))
    );

    drawing.views.clear();

    assert_eq!(
        dimension_problem(&drawing, &drawing.dimensions[0], &drawn),
        Some(Err(AnnotationProblem::ViewGone))
    );
}

#[test]
fn a_linear_dimension_is_laid_out_with_gapped_extension_lines_arrows_and_its_value_above() {
    let pair = pair();
    let mut drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);

    drawing.dimensions = vec![DrawingDimension {
        decimals: 1,
        ..dimension(0, length_of(top_edge(&lines)), Point2::new(10.0, 9.0))
    }];

    let plan = annotated(&drawing, &lines);
    let placed = *plan.placed(front(StandardView::Front)).unwrap();
    let laid = &plan.annotations[0];
    let line_at = placed.on_sheet(Point2::new(0.0, 9.0)).y;
    let corner = placed.on_sheet(Point2::new(0.0, 4.0));

    let SheetMark::Line(gap_end, overshoot_end) = laid.marks[0] else {
        panic!("the first mark is an extension line");
    };

    assert_eq!(laid.text.content, "20");
    assert_eq!(laid.text.angle, 0.0);
    assert!(laid.text.at.y > line_at);
    assert!(near(gap_end.y - corner.y, 0.5 * DIMENSION_TEXT_HEIGHT));
    assert!(near(overshoot_end.y - line_at, 0.5 * DIMENSION_TEXT_HEIGHT));
    assert!(matches!(laid.measure, Some(SheetMeasure::Linear { .. })));
    assert_eq!(laid.marks.len(), 2 + 1 + 4);
    assert_eq!(
        plan.annotation_at(laid.text.at, 0.1),
        Some(Annotation::Dimension(AnnotationId::from_raw(0)))
    );
}

fn disc_document() -> (Document, FeatureId) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");
    let mut sketch = Sketch::new(Plane::XY);

    sketch.add_circle(Point2::new(10.0, 10.0), 6.0);

    let outline = transaction.add_feature("Disc outline", FeatureKind::from(sketch));
    let disc = transaction.add_feature(
        "Disc",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: outline,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("3 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );

    document.apply(transaction.finish()).unwrap();
    (document, disc)
}

#[test]
fn a_circle_seen_round_takes_a_diameter_and_one_seen_edge_on_refuses() {
    let (document, disc) = disc_document();
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    drawing.bodies = vec![disc];

    let top = lines_of(&document, &drawing, StandardView::Top);
    let drawn = DrawnLines::from([(front(StandardView::Top), &top)]);
    let circle = top
        .pieces
        .iter()
        .find_map(|piece| match (piece.source, &piece.curve) {
            (ViewSource::Edge { name, .. }, Curve2::Circle(_)) => Some(name),
            _ => None,
        })
        .unwrap();
    let pick = EdgePick {
        view: front(StandardView::Top),
        edge: circle,
        at: Point2::new(16.0, 10.0),
    };

    let made = dimension_from_picks(&drawing, &drawn, pick, pick, Point2::new(20.0, 20.0));
    let diameter = dimension_geometry(&DimensionMeasure::Diameter(circle), Point2::ZERO, &top)
        .unwrap()
        .value();
    let radius = dimension_geometry(&DimensionMeasure::Radius(circle), Point2::ZERO, &top)
        .unwrap()
        .value();

    let side = lines_of(&document, &drawing, StandardView::Front);
    let edge_on = dimension_geometry(&DimensionMeasure::Radius(circle), Point2::ZERO, &side);

    assert_eq!(made, Ok(DimensionMeasure::Diameter(circle)));
    assert!(near(diameter, 12.0));
    assert!(near(radius, 6.0));
    assert!(matches!(
        edge_on,
        Err(AnnotationProblem::NotRound | AnnotationProblem::EdgeGone)
    ));
}

#[test]
fn a_note_points_its_leader_at_its_edge_and_says_when_the_edge_is_gone() {
    let pair = pair();
    let mut drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);
    let leader = |edge| {
        Some(NoteLeader {
            view: front(StandardView::Front),
            edge,
        })
    };

    drawing.notes = vec![
        DrawingNote {
            id: AnnotationId::from_raw(0),
            text: "Break all edges".into(),
            at: Point2::new(60.0, 40.0),
            leader: leader(top_edge(&lines)),
        },
        DrawingNote {
            id: AnnotationId::from_raw(1),
            text: "Lost".into(),
            at: Point2::new(-60.0, 40.0),
            leader: leader(EdgeName::from_digest(0xbad)),
        },
    ];

    let plan = annotated(&drawing, &lines);
    let placed = *plan.placed(front(StandardView::Front)).unwrap();
    let pointed = &plan.annotations[0];
    let lost = &plan.annotations[1];
    let tip = placed.on_sheet(Point2::new(10.0, 4.0));

    assert_eq!(pointed.problem, None);
    assert!(pointed.marks.iter().any(|mark| matches!(
        mark,
        SheetMark::Line(_, to) if (*to - tip).length() < 1e-6
    )));
    assert_eq!(
        pointed.text.at,
        note_on_sheet(&plan, Point2::new(60.0, 40.0))
    );
    assert_eq!(lost.problem, Some(AnnotationProblem::EdgeGone));
    assert!(lost.marks.is_empty());
}

#[test]
fn moving_a_dimension_or_note_keeps_it_by_its_view_and_scaling_the_model_carries_it() {
    let pair = pair();
    let mut drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);

    drawing.dimensions = vec![dimension(
        0,
        length_of(top_edge(&lines)),
        Point2::new(10.0, 9.0),
    )];
    drawing.notes = vec![DrawingNote {
        id: AnnotationId::from_raw(1),
        text: "Note".into(),
        at: Point2::ZERO,
        leader: None,
    }];

    let plan = annotated(&drawing, &lines);
    let scale = plan.placed(front(StandardView::Front)).unwrap().scale;
    let mut moved = drawing.clone();

    plan.moved_annotation(
        &mut moved,
        Annotation::Dimension(AnnotationId::from_raw(0)),
        Vector2::new(0.0, 10.0),
    );
    plan.moved_annotation(
        &mut moved,
        Annotation::Note(AnnotationId::from_raw(1)),
        Vector2::new(5.0, 0.0),
    );

    assert!(near(moved.dimensions[0].place.y, 9.0 + 10.0 / scale));
    assert_eq!(moved.notes[0].at, Point2::new(5.0, 0.0));

    let mut document = pair.document.clone();

    let mut drawings = document.drawings().clone();

    drawings.drawings.push(drawing);
    document
        .apply(Transaction::single(
            "Drawings",
            Edit::SetDrawings {
                drawings: Box::new(drawings),
            },
        ))
        .unwrap();

    let scaled = document
        .scaled(&ModelScale {
            factor: 2.0,
            centre: Point3::ZERO,
            values: ScaledValues::Plain,
        })
        .unwrap();

    document.apply(scaled.transaction).unwrap();

    let kept = &document.drawings().drawings[0];

    assert!(near(kept.dimensions[0].place.x, 20.0));
    assert!(near(kept.dimensions[0].place.y, 18.0));
    assert_eq!(kept.notes[0].at, Point2::ZERO);
}
