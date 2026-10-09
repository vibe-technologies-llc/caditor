use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Event, Key, Modifiers, PointerButton};

use super::{Harness, edit_free_sketch, entities_of_kind, run_from_palette, sketch_entity};
use crate::{
    drawing::{LINE_CHAIN_PROMPT, Refusal},
    editing::{EditingCommand, Tool},
    model::Action,
    shape_modes::{CircleMode, PolygonMode, RectangleMode, ShapeMode},
    viewport::REFERENCE_STAYS,
};

const TAKE_BACK: &str = "Backspace: take back the last point";

fn hold_key(harness: &mut Harness, key: Key, repeats: usize) {
    for repeat in std::iter::once(false).chain(std::iter::repeat_n(true, repeats)) {
        harness.events.push(Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.events.push(Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
}

fn notice_text(harness: &Harness) -> Option<String> {
    harness.model.notice().map(|notice| notice.text.clone())
}

#[test]
fn smart_dimension_lets_go_of_its_picks_when_another_tool_is_chosen() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 40.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::D);
    harness.click_pickable(
        Plane::XY,
        Point2::new(15.0, 20.0),
        sketch_entity(feature, line),
    );
    harness.frame();

    assert_eq!(harness.tool(), Some(Tool::Dimension));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(sketch_entity(feature, line))
    );

    harness.use_tool(Key::L);

    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn holding_escape_takes_one_step_back_instead_of_finishing_the_sketch() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));

    assert!(harness.shows(LINE_CHAIN_PROMPT));

    hold_key(&mut harness, Key::Escape, 6);

    assert_eq!(harness.editing(), Some(feature));
    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows("Click the start of the line"));

    hold_key(&mut harness, Key::Escape, 6);

    assert_eq!(harness.editing(), Some(feature));
    assert_eq!(harness.tool(), Some(Tool::Select));

    hold_key(&mut harness, Key::Escape, 6);

    assert_eq!(harness.editing(), None);
}

#[test]
fn enter_stops_a_line_chain_as_a_click_on_its_last_point_does() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows(LINE_CHAIN_PROMPT));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("Click the start of the line"));
    assert_eq!(harness.tool(), Some(Tool::Line));

    harness.click_at(Point2::new(40.0, 40.0));

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows(LINE_CHAIN_PROMPT));
}

#[test]
fn enter_on_a_spline_with_too_few_points_says_so_and_keeps_them() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::S);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert_eq!(
        notice_text(&harness).as_deref(),
        Some(Refusal::SplinePoints.reason())
    );
    assert!(harness.shows("Click the next control point"));
    assert!(entities_of_kind(harness.sketch(feature), "Spline").is_empty());

    harness.click_at(Point2::new(40.0, 20.0));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert_eq!(entities_of_kind(harness.sketch(feature), "Spline").len(), 1);
}

#[test]
fn delete_takes_back_the_last_point_and_one_hint_says_so() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::R);

    assert!(!harness.shows_hint(TAKE_BACK));

    harness.click_at(Point2::new(10.0, 10.0));

    assert!(harness.shows("Click the opposite corner"));
    assert!(harness.shows_hint(TAKE_BACK));

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("Click the rectangle's first corner"));
    assert_eq!(harness.tool(), Some(Tool::Rectangle));
    assert!(entities_of_kind(harness.sketch(feature), "Line").is_empty());
}

#[test]
fn a_new_way_keeps_a_first_point_meaning_the_same_and_refuses_otherwise() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.use_tool(Key::R);

    assert_eq!(
        notice_text(&harness).as_deref(),
        Some("Finish or cancel the rectangle first, then change how it is drawn")
    );
    assert!(harness.shows("Click the opposite corner"));

    harness.perform(Action::Editing(EditingCommand::SetMode(
        ShapeMode::Rectangle(RectangleMode::ThreePoints),
    )));
    harness.frame();

    assert!(harness.shows("Click where its first side ends"));

    harness.click_at(Point2::new(40.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 4);

    harness.perform(Action::Editing(EditingCommand::SetMode(ShapeMode::Circle(
        CircleMode::Center,
    ))));
    harness.frame();
    harness.click_at(Point2::new(80.0, 20.0));
    harness.perform(Action::Editing(EditingCommand::SetMode(
        ShapeMode::Polygon(PolygonMode::Corner),
    )));
    harness.frame();

    assert_eq!(harness.tool(), Some(Tool::Polygon));
    assert!(harness.shows_containing("Click a corner of the"));
}

#[test]
fn with_snapping_off_the_prompt_says_so_and_names_alt() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    assert!(harness.shows_hint("Ctrl: place freely"));

    run_from_palette(&mut harness, "turn snapping on or off");
    harness.frame();

    assert!(harness.shows("Snapping off"));
    assert!(!harness.shows_hint("Ctrl: place freely"));
    assert!(harness.shows_hint("Alt: snap to the grid or nearby geometry"));
}

#[test]
fn dragging_the_origin_says_it_stays_put_and_draws_no_box() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(-40.0, -30.0), Point2::new(40.0, -30.0));
    sketch.add_point(Point2::new(15.0, 10.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    let from = harness.hover_pickable(
        Plane::XY,
        Point2::ZERO,
        sketch_entity(feature, EntityId::ORIGIN),
    );
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(Point2::new(30.0, 20.0));
    for step in 1..=4 {
        let position = from + (target - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }

    assert!(harness.shows(REFERENCE_STAYS));

    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();

    assert!(!harness.shows(REFERENCE_STAYS));
    assert!(harness.workspace.viewport.selection().is_empty());
}
