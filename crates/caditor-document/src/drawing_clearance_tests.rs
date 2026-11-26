use std::collections::BTreeMap;

use caditor_geometry::{Aabb2, Point2};
use caditor_kernel::{Curve2, EdgeName, Interval, Line2, ViewPiece, ViewSource};

use crate::{
    combine_tests::block,
    drawing_clearance::LABEL_CLEARANCE,
    drawing_dimensions_tests::{
        annotated, clearance, dimension, edge_along, edge_where, front, front_only, gap_between,
        laid_of, length_of, lines_of, near, value_of,
    },
    tally, *,
};

struct Block {
    from: f64,
    to: f64,
    depth: (f64, f64),
    height: f64,
}

fn standing(from: f64, to: f64, height: f64) -> Block {
    Block {
        from,
        to,
        depth: (0.0, 10.0),
        height,
    }
}

fn blocks_drawing(blocks: &[Block]) -> (Drawing, ViewLines) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let bodies = blocks
        .iter()
        .enumerate()
        .map(|(index, made)| {
            block(
                &mut transaction,
                &format!("Block {index}"),
                (made.from, made.depth.0),
                (made.to, made.depth.1),
                &format!("{} mm", made.height),
            )
        })
        .collect();

    document.apply(transaction.finish()).unwrap();

    let mut drawing = front_only(Drawing::new(DrawingId::from_raw(0), "Drawing 1".into()));

    drawing.bodies = bodies;
    drawing.scale = DrawingScale::Ratio(ScaleRatio::new(1, 1));

    let lines = lines_of(&document, &drawing, StandardView::Front);
    (drawing, lines)
}

fn upright_edge(lines: &ViewLines, at: f64, (low, high): (f64, f64)) -> EdgeName {
    edge_where(lines, |start, end| {
        near(start.x, at)
            && near(end.x, at)
            && near(start.y.min(end.y), low)
            && near(start.y.max(end.y), high)
    })
}

fn text_box(text: &SheetText) -> Aabb2 {
    Aabb2::from_points(text_corners(text)).unwrap()
}

fn margin() -> f64 {
    LABEL_CLEARANCE * DIMENSION_TEXT_HEIGHT / 2.0
}

fn clear_of_upright(text: &SheetText, x: f64) -> bool {
    let reach = text_box(text);
    x < reach.min().x - margin() || x > reach.max().x + margin()
}

fn note_at(id: u64, text: &str, plan: &SheetPlan, on_sheet: Point2) -> DrawingNote {
    DrawingNote {
        id: AnnotationId::from_raw(id),
        text: text.into(),
        at: note_place(plan, on_sheet),
        leader: None,
    }
}

fn laid_note(plan: &SheetPlan, id: u64) -> &SheetText {
    &plan
        .annotation(Annotation::Note(AnnotationId::from_raw(id)))
        .unwrap()
        .texts[0]
}

#[test]
fn a_value_that_would_sit_on_a_views_visible_line_slides_along_its_span_off_it() {
    let (mut drawing, lines) = blocks_drawing(&[
        standing(0.0, 40.0, 4.0),
        Block {
            depth: (20.0, 30.0),
            ..standing(18.0, 22.0, 20.0)
        },
    ]);

    let bar_top = edge_along(&lines, 4.0, (0.0, 40.0));
    let post = lines
        .pieces
        .iter()
        .find_map(|piece| match piece.source {
            ViewSource::Edge { solid, .. } if piece.curve.point(piece.range.start()).y > 19.0 => {
                Some(solid)
            }
            _ => None,
        })
        .unwrap();
    let mut without_post = lines.clone();

    without_post
        .pieces
        .retain(|piece| !matches!(piece.source, ViewSource::Edge { solid, .. } if solid == post));
    drawing.dimensions = vec![dimension(0, length_of(bar_top), Point2::new(20.0, 9.0))];

    let plan = annotated(&drawing, &lines);
    let unmoved = annotated(&drawing, &without_post);
    let placed = *plan.placed(front(StandardView::Front)).unwrap();

    let value = value_of(&plan, 0);
    let would = value_of(&unmoved, 0);
    let post_sides = [18.0, 22.0].map(|x| placed.on_sheet(Point2::new(x, 0.0)).x);

    assert!(near(would.at.x, placed.on_sheet(Point2::new(20.0, 0.0)).x));
    assert!(
        post_sides
            .iter()
            .all(|side| !clear_of_upright(would, *side))
    );
    assert!(post_sides.iter().all(|side| clear_of_upright(value, *side)));
    assert_eq!(value.at.y, would.at.y);
    assert_eq!(laid_of(&plan, 0).measure, laid_of(&unmoved, 0).measure);
}

#[test]
fn a_value_that_would_sit_on_another_dimensions_line_moves_off_it() {
    let (mut drawing, lines) =
        blocks_drawing(&[standing(0.0, 4.0, 30.0), standing(40.0, 80.0, 4.0)]);

    let post_side = upright_edge(&lines, 4.0, (0.0, 30.0));
    let bar_top = edge_along(&lines, 4.0, (40.0, 80.0));

    drawing.dimensions = vec![
        dimension(0, length_of(post_side), Point2::new(60.0, 15.0)),
        dimension(1, length_of(bar_top), Point2::new(60.0, 5.0)),
    ];

    let mut alone = drawing.clone();

    alone.dimensions.remove(0);

    let plan = annotated(&drawing, &lines);
    let unmoved = annotated(&alone, &lines);
    let placed = *plan.placed(front(StandardView::Front)).unwrap();

    let line_x = placed.on_sheet(Point2::new(60.0, 0.0)).x;
    let value = value_of(&plan, 1);
    let would = value_of(&unmoved, 1);

    assert!(laid_of(&plan, 0).marks.iter().any(|mark| matches!(
        mark,
        SheetMark::Line(from, to) if near(from.x, line_x) && near(to.x, line_x)
    )));
    assert!(gap_between(value_of(&plan, 0), would) >= clearance() - 1e-9);
    assert!(!clear_of_upright(would, line_x));
    assert!(clear_of_upright(value, line_x));
    assert_eq!(value.at.y, would.at.y);
}

fn stacked_on_one_bar(levels: (f64, f64)) -> (Drawing, ViewLines) {
    let (mut drawing, lines) = blocks_drawing(&[standing(0.0, 40.0, 4.0)]);

    let top = edge_along(&lines, 4.0, (0.0, 40.0));
    let bottom = edge_along(&lines, 0.0, (0.0, 40.0));

    drawing.dimensions = vec![
        dimension(0, length_of(top), Point2::new(20.0, levels.0)),
        dimension(1, length_of(bottom), Point2::new(20.0, levels.1)),
    ];
    (drawing, lines)
}

#[test]
fn a_value_with_no_clear_place_keeps_the_layout_the_user_gave_it() {
    let (mut drawing, lines) = stacked_on_one_bar((9.0, 10.0));

    let mut alone = drawing.clone();

    alone.dimensions.remove(0);

    let unmoved = annotated(&alone, &lines);

    drawing.notes = vec![note_at(
        2,
        "Every value along this span is crowded by this note",
        &unmoved,
        value_of(&unmoved, 1).at,
    )];

    let plan = annotated(&drawing, &lines);

    assert!(gap_between(laid_note(&plan, 2), value_of(&plan, 1)) < clearance());
    assert!(gap_between(value_of(&plan, 0), value_of(&plan, 1)) < clearance());
    assert_eq!(laid_of(&plan, 1), laid_of(&unmoved, 1));
}

#[test]
fn a_value_on_a_text_moves_onto_a_line_rather_than_stay_when_no_place_is_clear_of_both() {
    let (mut drawing, lines) = blocks_drawing(&[standing(0.0, 40.0, 4.0)]);
    let top = edge_along(&lines, 4.0, (0.0, 40.0));

    drawing.dimensions = vec![dimension(0, length_of(top), Point2::new(20.0, 2.0))];

    let unmoved = annotated(&drawing, &lines);
    let would = value_of(&unmoved, 0).clone();
    let top_on_sheet = unmoved
        .placed(front(StandardView::Front))
        .unwrap()
        .on_sheet(Point2::new(0.0, 4.0))
        .y;

    drawing.notes = vec![note_at(1, "Dry", &unmoved, would.at)];

    let plan = annotated(&drawing, &lines);
    let value = value_of(&plan, 0);
    let reach = text_box(value);

    assert!(reach.min().y < top_on_sheet && reach.max().y > top_on_sheet);
    assert!(gap_between(laid_note(&plan, 1), &would) < clearance());
    assert!(gap_between(laid_note(&plan, 1), value) >= clearance() - 1e-9);
    assert_eq!(value.at.y, would.at.y);
}

fn with_far_lines(lines: &ViewLines, count: usize) -> ViewLines {
    let mut crowded = lines.clone();
    let template = lines.pieces[0].clone();
    let columns = 50;

    crowded.pieces.extend((0..count).map(|index| {
        let column = (index % columns) as f64;
        let row = (index / columns) as f64;

        let from = Point2::new(column * 0.8, -20.0 - row * 0.4);

        ViewPiece {
            curve: Curve2::Line(Line2::through(from, from + Point2::new(0.5, 0.2)).unwrap()),
            range: Interval::new(0.0, 0.5).unwrap(),
            source: match template.source {
                ViewSource::Edge {
                    solid, edge, kind, ..
                } => ViewSource::Edge {
                    solid,
                    edge,
                    name: EdgeName::from_digest(1_000_000 + index as u128),
                    kind,
                },
                other => other,
            },
            ..template.clone()
        }
    }));
    crowded
}

#[test]
fn laying_out_beside_a_view_of_thousands_of_lines_tests_only_the_lines_near_each_value() {
    let (drawing, lines) = stacked_on_one_bar((9.0, 10.0));

    let some = with_far_lines(&lines, 2000);
    let more = with_far_lines(&lines, 4000);

    let (plan, tested) = tally::measure(|| annotated(&drawing, &some));
    let (again, tested_again) = tally::measure(|| annotated(&drawing, &more));

    let placed = *plan.placed(front(StandardView::Front)).unwrap();
    let farthest = more.pieces.last().unwrap();

    let far_end = placed.on_sheet(farthest.curve.point(farthest.range.end()));

    assert!(plan.frame.contains(far_end));
    assert_eq!(plan.annotations, again.annotations);
    assert_eq!(tested, tested_again);
    assert!(tested <= 32, "{tested} lines tested");
}

fn dragged_values(drawing: &Drawing, lines: &ViewLines, holding: bool) -> Vec<f64> {
    let extents = BTreeMap::from([(front(StandardView::Front), lines.bounds.unwrap())]);
    let drawn = DrawnLines::from([(front(StandardView::Front), lines)]);
    let mut held = ValuePlaces::default();
    let mut values = Vec::new();

    for step in 0..=72 {
        let mut moved = drawing.clone();

        moved.dimensions[1].place.y = 4.0 + f64::from(step) * 0.25;

        let mut plan = lay_out_sheet(&moved, &TitleFields::default(), &extents, &BTreeMap::new());

        let chosen = plan.annotate_holding(&moved, &drawn, &CalloutTexts::new(), &held);

        let value = value_of(&plan, 1);

        for other in [value_of(&plan, 0), laid_note(&plan, 2)] {
            assert!(gap_between(other, value) >= clearance() - 1e-9);
        }
        if holding {
            held = chosen;
        }
        values.push(value.at.x);
    }
    values
}

fn positions_taken(values: &[f64]) -> Vec<f64> {
    let mut taken: Vec<f64> = Vec::new();

    for value in values {
        if taken.last().is_none_or(|last| !near(*last, *value)) {
            taken.push(*value);
        }
    }
    taken
}

fn returns_to_a_dodge(taken: &[f64], placed: f64) -> bool {
    taken.iter().enumerate().any(|(index, value)| {
        !near(*value, placed)
            && taken
                .get(index + 2..)
                .is_some_and(|later| later.iter().any(|again| near(*again, *value)))
    })
}

#[test]
fn a_dimension_dragged_across_a_neighbour_keeps_its_dodge_rather_than_flip_back_and_forth() {
    let (mut drawing, lines) = blocks_drawing(&[standing(0.0, 80.0, 4.0)]);

    let top = edge_along(&lines, 4.0, (0.0, 80.0));
    let bottom = edge_along(&lines, 0.0, (0.0, 80.0));

    drawing.dimensions = vec![
        DrawingDimension {
            tolerance: Some(DimensionTolerance::Limits {
                upper: 0.1,
                lower: -0.1,
            }),
            ..dimension(0, length_of(top), Point2::new(40.0, 12.0))
        },
        dimension(1, length_of(bottom), Point2::new(40.0, 14.0)),
    ];

    let before_note = annotated(&drawing, &lines);
    let slid = value_of(&before_note, 1).at;

    drawing.notes = vec![note_at(2, "Dry", &before_note, slid)];

    let placed = before_note
        .placed(front(StandardView::Front))
        .unwrap()
        .on_sheet(Point2::new(40.0, 0.0))
        .x;
    let held = positions_taken(&dragged_values(&drawing, &lines, true));
    let afresh = positions_taken(&dragged_values(&drawing, &lines, false));

    assert!(slid.x > placed);
    assert!(returns_to_a_dodge(&afresh, placed), "{afresh:?}");
    assert!(!returns_to_a_dodge(&held, placed), "{held:?}");
    assert!(held.first().is_some_and(|first| near(*first, placed)));
    assert!(held.last().is_some_and(|last| near(*last, placed)));
    assert!(held.len() >= 3, "{held:?}");
}
