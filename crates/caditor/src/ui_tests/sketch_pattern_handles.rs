use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers, Pos2};

use super::{
    Harness, Wheel, add_parameters, edit_free_sketch, entities_of_kind, entity_pickables,
    hold_drag, near, release_drag, run_from_palette,
};
use crate::{
    editing::Tool,
    patterning::{self, Direction, PatternField, PatternKind, Spec, TurnKind},
    sketch_pattern_arrows::{self, PatternGrip},
    sketch_pattern_panel,
};

const RECTANGULAR: &str = "repeat sketch geometry in a grid";
const CIRCULAR: &str = "repeat sketch geometry about a point";
const PIVOT: Point2 = Point2::new(20.0, 0.0);
const FIRST: PatternGrip = PatternGrip::Distance(Direction::FIRST);
const SECOND: PatternGrip = PatternGrip::Distance(Direction(1));
const ORIGINAL_MIDDLE: Point2 = Point2::new(5.0, 7.5);

fn row_fixture(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);

    let circle = sketch.add_circle(Point2::new(5.0, 5.0), 2.0);
    let line = sketch.add_line(Point2::new(0.0, 12.0), Point2::new(10.0, 12.0));

    for corner in [Point2::new(-20.0, -30.0), Point2::new(80.0, 60.0)] {
        sketch.add_point(corner);
    }

    let feature = edit_free_sketch(harness, sketch);

    harness.select(entity_pickables(feature, &[circle, line]));
    feature
}

fn open_rectangular(harness: &mut Harness) -> FeatureId {
    let feature = row_fixture(harness);

    run_from_palette(harness, RECTANGULAR);
    harness.settle();
    feature
}

fn text_of(harness: &mut Harness, field: PatternField) -> String {
    let (settings, _) = harness
        .workspace
        .viewport
        .pattern_parts()
        .expect("a sketch pattern is open");
    settings.text(field).unwrap_or_default().to_owned()
}

fn reversed(harness: &mut Harness, direction: Direction) -> bool {
    let (settings, _) = harness
        .workspace
        .viewport
        .pattern_parts()
        .expect("a sketch pattern is open");
    settings.reversed(direction)
}

fn distance_of(harness: &mut Harness, direction: Direction) -> f64 {
    let (settings, patterning) = harness
        .workspace
        .viewport
        .pattern_parts()
        .expect("a sketch pattern is open");
    match settings.spec(patterning.kind()) {
        Some(Spec::Rectangular(pattern)) => pattern
            .directions
            .get(direction.index())
            .map(|row| row.distance.value)
            .expect("the direction is in the pattern"),
        other => panic!("not a rectangular pattern: {other:?}"),
    }
}

fn total_angle(harness: &mut Harness) -> f64 {
    let (settings, patterning) = harness
        .workspace
        .viewport
        .pattern_parts()
        .expect("a sketch pattern is open");
    match settings.spec(patterning.kind()) {
        Some(Spec::Circular(pattern)) => match &pattern.spread {
            caditor_sketch::Spread::Total(angle) => angle.value,
            other => panic!("not spread over an angle: {other:?}"),
        },
        other => panic!("not a circular pattern: {other:?}"),
    }
}

fn arrow_at(harness: &Harness, grip: PatternGrip, along: f64) -> Pos2 {
    harness
        .workspace
        .viewport
        .pattern_arrow_position(grip, along)
        .unwrap_or_else(|| panic!("the {grip:?} arrow is shown"))
}

fn foot_of(harness: &Harness, grip: PatternGrip) -> Point2 {
    harness
        .workspace
        .viewport
        .pattern_arrow_foot(grip)
        .unwrap_or_else(|| panic!("the {grip:?} arrow is shown"))
}

fn grips(harness: &Harness) -> Vec<PatternGrip> {
    harness.workspace.viewport.pattern_arrow_grips()
}

fn drag(harness: &mut Harness, grip: PatternGrip, along: f64) {
    let from = arrow_at(harness, grip, 0.0);
    let to = arrow_at(harness, grip, along);

    hold_drag(harness, from, to);
    release_drag(harness, to);
}

fn add_direction(harness: &mut Harness) {
    let panel = harness.position_of(sketch_pattern_panel::SHAPE);

    harness.wheel_until_shown(panel, patterning::ADD_DIRECTION, Wheel::Down);
    harness.hold_still();
    harness.click(patterning::ADD_DIRECTION);
    harness.settle();
}

fn type_beside(harness: &mut Harness, field: PatternField, text: &str) {
    harness.type_into_field(sketch_pattern_arrows::field_id(field), text);
    harness.settle();
}

fn centres(sketch: &Sketch) -> Vec<Point2> {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(centre, _)| centre)
        .collect()
}

#[test]
fn each_direction_of_a_rectangular_pattern_has_an_arrow_on_its_last_copy() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);

    assert_eq!(grips(&harness), vec![FIRST]);
    assert!(near(foot_of(&harness, FIRST), Point2::new(25.0, 7.5)));

    add_direction(&mut harness);

    assert_eq!(grips(&harness), vec![FIRST, SECOND]);
    assert!(near(foot_of(&harness, SECOND), Point2::new(5.0, 17.5)));

    type_beside(&mut harness, PatternField::Count(Direction(1)), "3");

    assert!(near(foot_of(&harness, SECOND), Point2::new(5.0, 27.5)));

    type_beside(&mut harness, PatternField::Count(Direction::FIRST), "1");

    assert!(grips(&harness).is_empty());
}

#[test]
fn dragging_an_arrow_changes_its_spacing_live_and_the_copies_made_are_one_undoable_change() {
    let mut harness = Harness::new();

    let feature = open_rectangular(&mut harness);
    let before = harness.sketch(feature).clone();
    let from = arrow_at(&harness, FIRST, 0.0);
    let to = arrow_at(&harness, FIRST, 10.0);

    hold_drag(&mut harness, from, to);
    harness.settle();

    let held = distance_of(&mut harness, Direction::FIRST);

    assert!((held - 15.0).abs() < 1.0, "{held}");
    assert!(harness.shows_containing("Spacing "));
    assert!(foot_of(&harness, FIRST).x > 33.0);
    assert!(harness.model.undo_label() != Some(patterning::TRANSACTION));

    release_drag(&mut harness, to);

    let spacing = distance_of(&mut harness, Direction::FIRST);

    assert!((spacing - held).abs() < 1e-9);
    assert_eq!(
        text_of(&mut harness, PatternField::Spacing(Direction::FIRST)),
        harness
            .model
            .units()
            .show(caditor_expression::Quantity::length(spacing))
    );

    harness.click(sketch_pattern_panel::MAKE);
    harness.settle();

    let made = centres(harness.sketch(feature));

    assert_eq!(made.len(), 3);
    assert!(
        made.iter()
            .any(|centre| near(*centre, Point2::new(5.0 + 2.0 * spacing, 5.0)))
    );
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();

    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn escape_while_dragging_an_arrow_puts_the_spacing_back() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);

    let before = text_of(&mut harness, PatternField::Spacing(Direction::FIRST));
    let from = arrow_at(&harness, FIRST, 0.0);
    let to = arrow_at(&harness, FIRST, 12.0);

    hold_drag(&mut harness, from, to);

    assert_ne!(
        text_of(&mut harness, PatternField::Spacing(Direction::FIRST)),
        before
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    release_drag(&mut harness, to);

    assert_eq!(
        text_of(&mut harness, PatternField::Spacing(Direction::FIRST)),
        before
    );
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(near(foot_of(&harness, FIRST), Point2::new(25.0, 7.5)));
}

#[test]
fn an_arrow_dragged_back_past_the_original_flips_its_direction() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);
    drag(&mut harness, FIRST, -40.0);

    assert!(reversed(&mut harness, Direction::FIRST));
    assert!(distance_of(&mut harness, Direction::FIRST) > 0.0);
    assert!(foot_of(&harness, FIRST).x < ORIGINAL_MIDDLE.x);
}

#[test]
fn the_fields_beside_an_arrow_edit_its_count_and_spacing_with_units_and_expressions() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);

    let count = PatternField::Count(Direction::FIRST);
    let spacing = PatternField::Spacing(Direction::FIRST);

    type_beside(&mut harness, count, "4");
    type_beside(&mut harness, spacing, "1 cm + 2 mm");

    assert_eq!(text_of(&mut harness, count), "4");
    assert_eq!(text_of(&mut harness, spacing), "1 cm + 2 mm");
    assert!((distance_of(&mut harness, Direction::FIRST) - 12.0).abs() < 1e-9);
    assert!(near(foot_of(&harness, FIRST), Point2::new(41.0, 7.5)));
}

#[test]
fn clicking_an_arrow_puts_the_focus_in_its_spacing_field() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);

    let at = arrow_at(&harness, FIRST, 0.0);

    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    harness.press(at);
    harness.settle();

    assert_eq!(
        harness.focused(),
        Some(sketch_pattern_arrows::field_id(PatternField::Spacing(
            Direction::FIRST
        )))
    );
}

#[test]
fn a_spacing_following_a_parameter_is_not_dragged_and_says_what_drives_it() {
    let mut harness = Harness::new();

    add_parameters(&mut harness, &[("pitch", "10 mm")]);
    open_rectangular(&mut harness);
    type_beside(
        &mut harness,
        PatternField::Spacing(Direction::FIRST),
        "pitch",
    );

    let at = arrow_at(&harness, FIRST, 0.0);

    harness.events.push(Event::PointerMoved(at));
    harness.hold_still();

    assert!(harness.shows_containing("Spacing follows pitch"));

    drag(&mut harness, FIRST, 10.0);

    assert_eq!(
        text_of(&mut harness, PatternField::Spacing(Direction::FIRST)),
        "pitch"
    );
}

#[test]
fn a_circular_pattern_over_an_angle_has_an_arrow_turning_its_last_copy() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let circle = sketch.add_circle(Point2::new(32.0, 0.0), 1.0);

    sketch.add_point(PIVOT);
    sketch.add_point(Point2::new(-20.0, -20.0));

    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, CIRCULAR);
    harness.click_at(PIVOT);
    harness.settle();

    assert!(grips(&harness).is_empty());

    harness.click(TurnKind::Angle.label());
    harness.settle();
    type_beside(&mut harness, PatternField::TotalAngle, "90");

    assert_eq!(grips(&harness), vec![PatternGrip::Angle]);
    assert!(near(
        foot_of(&harness, PatternGrip::Angle),
        Point2::new(20.0, 12.0)
    ));

    drag(&mut harness, PatternGrip::Angle, 30.0);

    let turned = total_angle(&mut harness);

    assert!((turned - 120.0).abs() <= 5.0, "{turned}");
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
}

#[test]
fn the_arrows_go_when_the_pattern_closes() {
    let mut harness = Harness::new();

    open_rectangular(&mut harness);

    assert!(!grips(&harness).is_empty());

    harness.click(PatternKind::Circular.label());
    harness.settle();

    assert!(grips(&harness).is_empty());

    harness.click(PatternKind::Rectangular.label());
    harness.settle();

    assert!(!grips(&harness).is_empty());

    harness.click_button(&sketch_pattern_panel::close_label(PatternKind::Rectangular));
    harness.settle();

    assert_eq!(harness.tool(), Some(Tool::Select));
    assert!(grips(&harness).is_empty());
    assert!(!harness.shows_containing("Drag to change the spacing"));
}
