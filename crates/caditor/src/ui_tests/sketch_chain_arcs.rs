use caditor_document::FeatureId;
use caditor_geometry::Point2;
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};

use super::{Harness, constraints_of_kind, drag_in_sketch, entities_of_kind, line_ends, near};
use crate::{
    drawing::{CHAIN_ARC_PROMPT, LINE_CHAIN_PROMPT, NOTHING_TO_ARC_FROM},
    editing::Tool,
};

fn line_then_corner(harness: &mut Harness) -> FeatureId {
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    feature
}

fn arc_centre(sketch: &Sketch, arc: EntityId) -> Point2 {
    sketch.arc(arc).expect("an arc").center
}

fn press_and_move_through(harness: &mut Harness, from: Pos2, through: &[Point2]) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let mut at = from;
    for point in through {
        let target = harness.on_screen(*point);
        for step in 1..=4 {
            harness.events.push(Event::PointerMoved(
                at + (target - at) * (step as f32 / 4.0),
            ));
            harness.frame();
        }
        at = target;
    }
    harness.events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();
}

#[test]
fn dragging_from_a_line_chains_last_point_draws_a_tangent_arc_and_lines_go_on() {
    let mut harness = Harness::new();
    let feature = line_then_corner(&mut harness);

    let from = harness.on_screen(Point2::new(40.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(60.0, 30.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("the first line stays alone");
    };
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("the drag draws one arc");
    };
    let centre = arc_centre(sketch, arc);

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert_eq!(
        constraints_of_kind(sketch, "Tangent"),
        vec![Constraint::Tangent(line, arc)]
    );
    assert!(near(centre, Point2::new(40.0, 30.0)), "{centre}");
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    harness.click_at(Point2::new(60.0, 60.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let (start, _) = line_ends(sketch, lines[1]);
    let start = sketch.point(start).expect("the line's start");

    assert_eq!(lines.len(), 2);
    assert_eq!(entities_of_kind(sketch, "Arc").len(), 1);
    assert!(near(start, Point2::new(60.0, 30.0)), "{start}");
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 2);
}

#[test]
fn a_chain_arc_bends_to_the_side_of_the_line_the_drag_goes() {
    let mut harness = Harness::new();
    let feature = line_then_corner(&mut harness);

    let from = harness.on_screen(Point2::new(40.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(60.0, -10.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("the drag draws one arc");
    };
    let centre = arc_centre(sketch, arc);

    assert!(near(centre, Point2::new(40.0, -10.0)), "{centre}");
}

#[test]
fn dragging_back_onto_the_last_point_draws_no_arc_and_keeps_the_chain() {
    let mut harness = Harness::new();
    let feature = line_then_corner(&mut harness);

    let from = harness.on_screen(Point2::new(40.0, 10.0));
    press_and_move_through(
        &mut harness,
        from,
        &[Point2::new(60.0, 30.0), Point2::new(40.0, 10.0)],
    );

    assert!(entities_of_kind(harness.sketch(feature), "Arc").is_empty());
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    harness.click_at(Point2::new(40.0, 40.0));

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);
    assert!(entities_of_kind(harness.sketch(feature), "Arc").is_empty());
}

#[test]
fn the_tangent_arc_key_turns_the_next_segment_of_a_line_chain_into_an_arc_and_back() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.use_tool(Key::T);

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.ends_with(NOTHING_TO_ARC_FROM))
    );

    harness.click_at(Point2::new(40.0, 10.0));
    harness.use_tool(Key::T);

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows(CHAIN_ARC_PROMPT));
    assert!(harness.shows_hint("T: a line instead"));

    harness.use_tool(Key::T);

    assert!(harness.shows(LINE_CHAIN_PROMPT));
    assert!(harness.shows_hint("T: a tangent arc next"));

    harness.use_tool(Key::T);
    harness.click_at(Point2::new(60.0, 30.0));

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("the first line stays alone");
    };
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("the key draws one arc");
    };

    assert_eq!(
        constraints_of_kind(sketch, "Tangent"),
        vec![Constraint::Tangent(line, arc)]
    );
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    harness.click_at(Point2::new(60.0, 60.0));

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);
    assert_eq!(entities_of_kind(harness.sketch(feature), "Arc").len(), 1);
}
