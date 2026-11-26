use std::collections::BTreeMap;

use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Vector2};
use caditor_kernel::{Curve2, EdgeName, ViewSource};
use caditor_sketch::{
    Sketch,
    annotation::{ARROW_CLEARANCE, ARROW_LENGTH, ARROW_STUB, VALUE_CLEARANCE},
};

use crate::{
    combine_tests::{Pair, block, evaluate, pair},
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
        tolerance: None,
    }
}

fn front(view: StandardView) -> DrawnView {
    DrawnView::Standard(view)
}

fn annotated(drawing: &Drawing, lines: &ViewLines) -> SheetPlan {
    let extents = BTreeMap::from([(front(StandardView::Front), lines.bounds.unwrap())]);
    let mut plan = lay_out_sheet(drawing, &TitleFields::default(), &extents, &BTreeMap::new());
    let drawn = DrawnLines::from([(front(StandardView::Front), lines)]);

    plan.annotate(drawing, &drawn, &CalloutTexts::new());
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
    assert_eq!(lost.texts[0].content, "⌀?");
    assert_eq!(lost.measure, None);
    assert_eq!(kept.problem, None);
    assert_eq!(kept.texts[0].content, "20");
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

    assert_eq!(laid.texts[0].content, "20");
    assert_eq!(laid.texts[0].angle, 0.0);
    assert!(laid.texts[0].at.y > line_at);
    assert!(near(gap_end.y - corner.y, 0.5 * DIMENSION_TEXT_HEIGHT));
    assert!(near(overshoot_end.y - line_at, 0.5 * DIMENSION_TEXT_HEIGHT));
    assert!(matches!(laid.measure, Some(SheetMeasure::Linear { .. })));
    assert_eq!(laid.marks.len(), 2 + 1 + 4);
    assert_eq!(
        plan.annotation_at(laid.texts[0].at, 0.1),
        Some(Annotation::Dimension(AnnotationId::from_raw(0)))
    );
}

struct BarDimension {
    laid: SheetAnnotation,
    plan: SheetPlan,
    start: Point2,
    end: Point2,
}

fn bar_dimension(width: f64, decimals: u8, place: Point2) -> BarDimension {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let bar = block(&mut transaction, "Bar", (0.0, 0.0), (width, 10.0), "4 mm");

    document.apply(transaction.finish()).unwrap();

    let mut drawing = front_only(Drawing::new(DrawingId::from_raw(0), "Drawing 1".into()));

    drawing.bodies = vec![bar];
    drawing.scale = DrawingScale::Ratio(ScaleRatio::new(1, 1));

    let lines = lines_of(&document, &drawing, StandardView::Front);

    drawing.dimensions = vec![DrawingDimension {
        decimals,
        ..dimension(0, length_of(top_edge(&lines)), place)
    }];

    let plan = annotated(&drawing, &lines);
    let placed = *plan.placed(front(StandardView::Front)).unwrap();
    let laid = plan.annotations[0].clone();
    let (first, second) = (Point2::new(0.0, place.y), Point2::new(width, place.y));

    assert_eq!(placed.scale, 1.0);
    BarDimension {
        laid,
        plan,
        start: placed.on_sheet(first),
        end: placed.on_sheet(second),
    }
}

impl BarDimension {
    fn line(&self) -> (Point2, Point2) {
        let SheetMark::Line(from, to) = self.laid.marks[2] else {
            panic!(
                "the third mark is the dimension line: {:?}",
                self.laid.marks
            );
        };
        if from.x <= to.x {
            (from, to)
        } else {
            (to, from)
        }
    }

    fn arrow_backs(&self) -> Vec<Point2> {
        self.laid.marks[3..]
            .iter()
            .map(|mark| match *mark {
                SheetMark::Line(_, back) => back,
                _ => panic!("arrowheads are lines: {mark:?}"),
            })
            .collect()
    }

    fn arrows_point_in(&self) -> bool {
        let backs = self.arrow_backs();
        backs[..2].iter().all(|back| back.x < self.start.x)
            && backs[2..].iter().all(|back| back.x > self.end.x)
    }

    fn arrows_point_out(&self) -> bool {
        self.arrow_backs()
            .iter()
            .all(|back| back.x > self.start.x && back.x < self.end.x)
    }

    fn value_beside(&self) -> f64 {
        let text = &self.laid.texts[0];
        let [low, _, high, _] = text_corners(text);

        let (left, right) = (low.x.min(high.x), low.x.max(high.x));

        if left >= self.end.x {
            left - self.end.x
        } else if right <= self.start.x {
            self.start.x - right
        } else {
            0.0
        }
    }

    fn is_picked_by_its_value(&self) -> bool {
        let text = &self.laid.texts[0];
        let [low, _, high, _] = text_corners(text);
        let bounds = self.laid.bounds().unwrap();

        bounds.contains(low)
            && bounds.contains(high)
            && self.plan.annotation_at(text.at, 0.1)
                == Some(Annotation::Dimension(AnnotationId::from_raw(0)))
    }
}

#[test]
fn a_dimension_too_short_for_its_arrows_and_value_puts_both_outside_toward_its_place() {
    let height = DIMENSION_TEXT_HEIGHT;
    let right = bar_dimension(2.0, 1, Point2::new(8.0, 9.0));
    let left = bar_dimension(2.0, 1, Point2::new(-6.0, 9.0));

    let text = &right.laid.texts[0];
    let (from, to) = right.line();

    assert_eq!(text.content, "2");
    assert_eq!(text.angle, 0.0);
    assert!(right.arrows_point_in());
    assert!(from.x <= right.start.x - (ARROW_LENGTH + ARROW_STUB) * height + 1e-9);
    assert!(to.x >= right.end.x + ARROW_LENGTH * height + text_width("2", height));
    assert!(text.at.x > right.end.x + ARROW_LENGTH * height);
    assert!(near(
        right.value_beside(),
        (ARROW_LENGTH + VALUE_CLEARANCE) * height
    ));
    assert!(text.at.y > right.start.y);
    assert!(right.is_picked_by_its_value());

    assert!(left.arrows_point_in());
    assert!(left.laid.texts[0].at.x < left.start.x - ARROW_LENGTH * height);
    assert!(near(
        left.value_beside(),
        (ARROW_LENGTH + VALUE_CLEARANCE) * height
    ));
    assert!(left.is_picked_by_its_value());

    let Some(SheetMeasure::Linear { first, second, .. }) = right.laid.measure else {
        panic!("a linear dimension is measured as one");
    };

    assert!(near(first.distance(second), 2.0));
}

#[test]
fn a_dimension_with_room_for_its_arrows_but_not_its_value_puts_only_the_value_outside() {
    let height = DIMENSION_TEXT_HEIGHT;
    let width = 10.5;

    assert!(width >= (2.0 * (ARROW_LENGTH + ARROW_CLEARANCE)) * height);
    assert!(width < text_width("10.5", height) + 2.0 * VALUE_CLEARANCE * height);

    let laid = bar_dimension(width, 1, Point2::new(10.0, 9.0));
    let (from, to) = laid.line();

    assert_eq!(laid.laid.texts[0].content, "10.5");
    assert!(laid.arrows_point_out());
    assert!(near(from.x, laid.start.x));
    assert!(to.x > laid.end.x + text_width("10.5", height));
    assert!(near(laid.value_beside(), VALUE_CLEARANCE * height));
    assert!(laid.is_picked_by_its_value());
}

#[test]
fn a_dimension_with_room_for_its_arrows_and_value_keeps_both_between_its_extension_lines() {
    let laid = bar_dimension(40.0, 1, Point2::new(35.0, 9.0));
    let (from, to) = laid.line();

    assert_eq!(laid.laid.texts[0].content, "40");
    assert_eq!(laid.laid.marks.len(), 2 + 1 + 4);
    assert!(laid.arrows_point_out());
    assert!(near(from.x, laid.start.x) && near(to.x, laid.end.x));
    assert!(near(
        laid.laid.texts[0].at.x,
        (laid.start.x + laid.end.x) / 2.0
    ));
    assert_eq!(laid.value_beside(), 0.0);
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
        pointed.texts[0].at,
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

fn written(lines: &ValueLines) -> Vec<String> {
    match lines {
        ValueLines::One(main) => vec![main.clone()],
        ValueLines::Stacked { main, upper, lower } => {
            vec![main.clone(), upper.clone(), lower.clone()]
        }
        ValueLines::Limits { upper, lower } => vec![upper.clone(), lower.clone()],
    }
}

fn toleranced(
    measure: &DimensionMeasure,
    value: f64,
    tolerance: DimensionTolerance,
) -> Vec<String> {
    written(&value_lines(measure, Some(value), 2, Some(tolerance)))
}

#[test]
fn tolerances_write_plus_or_minus_deviations_limits_and_iso_fits() {
    let straight = length_of(EdgeName::from_digest(1));
    let across = DimensionMeasure::Diameter(EdgeName::from_digest(2));
    let turned = DimensionMeasure::Angle {
        first: EdgeName::from_digest(3),
        second: EdgeName::from_digest(4),
    };
    let fit = |code: &str, deviations: bool| DimensionTolerance::Fit {
        fit: IsoFit::from_code(code).unwrap(),
        deviations,
    };

    assert_eq!(
        toleranced(&straight, 20.0, DimensionTolerance::Symmetric(0.1)),
        ["20 \u{b1}0.1"]
    );
    assert_eq!(
        toleranced(&turned, 30.0, DimensionTolerance::Symmetric(0.5)),
        ["30\u{b0} \u{b1}0.5\u{b0}"]
    );
    assert_eq!(
        toleranced(
            &straight,
            20.0,
            DimensionTolerance::Deviations {
                upper: 0.02,
                lower: -0.01
            }
        ),
        ["20", "+0.02", "\u{2212}0.01"]
    );
    assert_eq!(
        toleranced(
            &straight,
            20.0,
            DimensionTolerance::Deviations {
                upper: 0.0,
                lower: -0.05
            }
        ),
        ["20", "0", "\u{2212}0.05"]
    );
    assert_eq!(
        toleranced(
            &straight,
            20.0,
            DimensionTolerance::Limits {
                upper: 0.02,
                lower: -0.01
            }
        ),
        ["20.02", "19.99"]
    );
    assert_eq!(
        toleranced(&across, 10.0, fit("H7", false)),
        ["\u{2300}10 H7"]
    );
    assert_eq!(
        toleranced(&across, 10.0, fit("H7", true)),
        ["\u{2300}10 H7", "+0.015", "0"]
    );
    assert_eq!(
        toleranced(&across, 25.0, fit("g6", true)),
        ["\u{2300}25 g6", "\u{2212}0.007", "\u{2212}0.02"]
    );
    assert_eq!(
        toleranced(&across, 640.0, fit("H7", true)),
        ["\u{2300}640 H7"]
    );
}

#[test]
fn a_dimension_with_deviations_stacks_them_beside_its_value_smaller_and_in_reach() {
    let pair = pair();
    let mut drawing = front_only(plate_drawing(&pair));

    let lines = lines_of(&pair.document, &drawing, StandardView::Front);

    drawing.dimensions = vec![DrawingDimension {
        tolerance: Some(DimensionTolerance::Fit {
            fit: IsoFit::from_code("h6").unwrap(),
            deviations: true,
        }),
        ..dimension(0, length_of(top_edge(&lines)), Point2::new(10.0, 9.0))
    }];

    let plan = annotated(&drawing, &lines);
    let laid = &plan.annotations[0];
    let [main, upper, lower] = laid.texts.as_slice() else {
        panic!("a value and two deviations: {:?}", laid.texts);
    };

    assert_eq!(main.content, "20 h6");
    assert_eq!(upper.content, "0");
    assert_eq!(lower.content, "\u{2212}0.013");
    assert_eq!(upper.height, DEVIATION_TEXT_HEIGHT);
    assert!(upper.at.y > lower.at.y);
    assert!(upper.at.x > main.at.x && lower.at.x > main.at.x);
    assert_eq!(
        plan.annotation_at(lower.at, 0.1),
        Some(Annotation::Dimension(AnnotationId::from_raw(0)))
    );
}

#[test]
fn tolerances_out_of_range_reversed_or_a_fit_on_an_angle_are_refused() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());

    let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();

    editor.apply(transaction).unwrap();

    let with = |tolerance: DimensionTolerance, measure: DimensionMeasure| {
        editor
            .document()
            .changing_drawing(id, "Tolerance", |drawing| {
                drawing.dimensions.push(DrawingDimension {
                    tolerance: Some(tolerance),
                    ..dimension(0, measure, Point2::ZERO)
                });
            })
    };
    let straight = length_of(EdgeName::from_digest(1));
    let turned = DimensionMeasure::Angle {
        first: EdgeName::from_digest(3),
        second: EdgeName::from_digest(4),
    };

    assert!(matches!(
        with(DimensionTolerance::Symmetric(0.0), straight),
        Err(EditError::ToleranceOutOfRange)
    ));
    assert!(matches!(
        with(DimensionTolerance::Symmetric(f64::NAN), straight),
        Err(EditError::ToleranceOutOfRange)
    ));
    assert!(matches!(
        with(
            DimensionTolerance::Deviations {
                upper: -0.1,
                lower: 0.1
            },
            straight
        ),
        Err(EditError::ToleranceReversed)
    ));
    assert!(matches!(
        with(
            DimensionTolerance::Limits {
                upper: 500.0,
                lower: 0.0
            },
            straight
        ),
        Err(EditError::ToleranceOutOfRange)
    ));
    assert!(matches!(
        with(
            DimensionTolerance::Fit {
                fit: IsoFit::from_code("H7").unwrap(),
                deviations: false
            },
            turned
        ),
        Err(EditError::FitOnAngle)
    ));
    assert!(
        with(
            DimensionTolerance::Fit {
                fit: IsoFit::from_code("H7").unwrap(),
                deviations: true
            },
            straight
        )
        .is_ok()
    );
}
