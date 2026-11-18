use caditor_document::{FeatureId, FeatureState};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, Wheel, choose, constraints_of_kind, edit_free_sketch, entities_of_kind,
    entity_pickables, near, run_from_palette,
};
use crate::{
    editing::Tool,
    layout,
    patterning::{self, Direction, PatternField, PatternKind, TurnKind},
    sketch_pattern_panel,
};

const CIRCLE_CENTRE: Point2 = Point2::new(5.0, 5.0);
const PIVOT: Point2 = Point2::new(20.0, 0.0);
const FAR_LEFT: Point2 = Point2::new(-20.0, 0.0);
const RECTANGULAR: &str = "repeat sketch geometry in a grid";
const CIRCULAR: &str = "repeat sketch geometry about a point";
const SECOND: Direction = Direction(1);
const THIRD: Direction = Direction(2);

fn row_fixture(harness: &mut Harness) -> (FeatureId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(CIRCLE_CENTRE, 2.0);
    let line = sketch.add_line(Point2::new(0.0, 12.0), Point2::new(10.0, 12.0));
    (edit_free_sketch(harness, sketch), circle, line)
}

fn centres(sketch: &Sketch) -> Vec<Point2> {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(centre, _)| centre)
        .collect()
}

fn has_centre(sketch: &Sketch, expected: Point2) -> bool {
    centres(sketch).iter().any(|centre| near(*centre, expected))
}

fn solves(harness: &Harness, feature: FeatureId) -> bool {
    harness
        .model
        .evaluation()
        .feature(feature)
        .is_some_and(|status| status.state == FeatureState::UpToDate)
}

fn set(harness: &mut Harness, field: PatternField, text: &str) {
    harness.type_into_field(sketch_pattern_panel::field_id(field), text);
}

fn make(harness: &mut Harness) {
    harness.click(sketch_pattern_panel::MAKE);
    harness.settle();
}

fn add_direction(harness: &mut Harness) {
    let panel = harness.position_of(sketch_pattern_panel::SHAPE);
    harness.wheel_until_shown(panel, patterning::ADD_DIRECTION, Wheel::Down);
    harness.hold_still();
    harness.click(patterning::ADD_DIRECTION);
    harness.frame();
}

fn enter(harness: &mut Harness) {
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();
}

#[test]
fn a_rectangular_pattern_repeats_the_selection_at_the_panels_count_and_spacing_in_one_undoable_step()
 {
    let mut harness = Harness::new();
    let (feature, circle, line) = row_fixture(&mut harness);
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, RECTANGULAR);
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(harness.shows(patterning::RECTANGULAR_SELECT_FIRST));
    assert!(harness.shows(sketch_pattern_panel::NOTHING_REPEATED));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("select what to repeat first"))
    );
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));

    harness.select(entity_pickables(feature, &[circle, line]));
    assert!(harness.shows(patterning::RECTANGULAR_PROMPT));
    assert!(harness.shows("2 items"));
    set(&mut harness, PatternField::Count(Direction::FIRST), "3");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "20");
    assert!(harness.shows("Repeat 2 items 2 more times"));
    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 3);
    for expected in [Point2::new(25.0, 5.0), Point2::new(45.0, 5.0)] {
        assert!(has_centre(sketch, expected), "{expected}");
    }
    assert_eq!(constraints_of_kind(sketch, "Horizontal distance").len(), 6);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 2);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(solves(&harness, feature));
    assert!(has_centre(&harness.shown(feature), Point2::new(45.0, 5.0)));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_pattern_that_cannot_be_made_says_why_in_the_panel_and_changes_nothing() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    let before = harness.sketch(feature).clone();
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    set(&mut harness, PatternField::Count(Direction::FIRST), "1");
    assert!(harness.shows_containing("pattern needs a count of at least 2"));
    make(&mut harness);
    set(&mut harness, PatternField::Count(Direction::FIRST), "3");

    set(&mut harness, PatternField::Spacing(Direction::FIRST), "0");
    assert!(harness.shows_containing("spacing cannot be zero"));
    set(&mut harness, PatternField::Spacing(Direction::FIRST), " ");
    assert!(harness.shows("The spacing is missing"));
    assert!(harness.shows("Mend the spacing first"));
    make(&mut harness);
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");

    set(&mut harness, PatternField::Count(Direction::FIRST), "2.5");
    assert!(harness.shows_containing("Use a whole number"));
    set(&mut harness, PatternField::Count(Direction::FIRST), "3");

    harness.click(patterning::ADD_DIRECTION);
    harness.frame();
    set(&mut harness, PatternField::Count(SECOND), "2");
    set(&mut harness, PatternField::Spacing(SECOND), " ");
    assert!(harness.shows("The spacing of direction 2 is missing"));
    let panel = harness.position_of(&SECOND.title());
    harness.wheel_until_shown(panel, "Mend the spacing of direction 2 first", Wheel::Down);
    set(&mut harness, PatternField::Spacing(SECOND), "10");
    set(&mut harness, PatternField::Angle(SECOND), "180");
    assert!(harness.shows_containing("directions run along one line"));
    make(&mut harness);

    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_second_direction_makes_a_grid_square_to_the_first_and_an_angle_slants_a_direction() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);
    assert!(!harness.shows(&SECOND.title()));

    set(&mut harness, PatternField::Count(Direction::FIRST), "2");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    harness.click(patterning::ADD_DIRECTION);
    harness.frame();
    assert!(harness.shows(&SECOND.title()));
    set(&mut harness, PatternField::Count(SECOND), "3");
    set(&mut harness, PatternField::Spacing(SECOND), "12");
    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 6);
    for (x, y) in [
        (15.0, 5.0),
        (5.0, 17.0),
        (15.0, 17.0),
        (5.0, 29.0),
        (15.0, 29.0),
    ] {
        assert!(has_centre(sketch, Point2::new(x, y)), "{x}, {y}");
    }
    assert!(solves(&harness, feature));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.select(entity_pickables(feature, &[circle]));
    harness.click(&SECOND.remove_label());
    harness.frame();
    assert!(!harness.shows(&SECOND.title()));
    set(&mut harness, PatternField::Angle(Direction::FIRST), "90");
    make(&mut harness);
    assert!(has_centre(harness.sketch(feature), Point2::new(5.0, 15.0)));
}

#[test]
fn a_circular_pattern_turns_the_selection_about_the_clicked_point_and_keeps_it_tied() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(30.0, 0.0), Point2::new(34.0, 0.0));
    let circle = sketch.add_circle(Point2::new(32.0, 3.0), 1.0);
    let pivot = sketch.add_point(PIVOT);
    sketch.add_point(FAR_LEFT);
    let pivot_label = sketch.entity_label(pivot);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.select(entity_pickables(feature, &[line, circle]));
    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    assert!(harness.shows(patterning::CENTRE_PROMPT));
    assert!(harness.shows(sketch_pattern_panel::NO_CENTRE));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert!(harness.model.notice().is_some_and(|notice| {
        notice
            .text
            .contains("click the point to repeat the selection about first")
    }));

    harness.click_at(PIVOT);
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    assert!(harness.shows(&pivot_label));
    set(&mut harness, PatternField::Copies, "4");
    assert!(harness.shows(&format!("Repeat 2 items 3 more times about {pivot_label}")));
    enter(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4 + 4 * 3);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Angle").len(), 3 * 3);
    assert_eq!(harness.model.undo_label(), Some(patterning::TRANSACTION));
    assert!(solves(&harness, feature));
    let shown = harness.shown(feature);
    for expected in [
        Point2::new(17.0, 12.0),
        Point2::new(8.0, -3.0),
        Point2::new(23.0, -12.0),
    ] {
        assert!(has_centre(&shown, expected), "{expected}");
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_circular_pattern_works_from_the_keyboard_alone_over_a_total_angle() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));

    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    assert!(!harness.shows(PatternField::TotalAngle.label()));
    harness.click(TurnKind::Angle.label());
    harness.frame();
    set(&mut harness, PatternField::Copies, "3");
    set(&mut harness, PatternField::TotalAngle, "90");
    enter(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 3);
    assert!(solves(&harness, feature));
    let shown = harness.shown(feature);
    let side = 50.0_f64.sqrt();
    assert!(has_centre(&shown, Point2::new(0.0, side)));
    assert!(has_centre(&shown, Point2::new(-5.0, 5.0)));
}

#[test]
fn both_patterns_are_commands_and_a_selected_point_is_the_centre() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let pivot = sketch.add_point(Point2::new(20.0, 0.0));
    let circle = sketch.add_circle(Point2::new(30.0, 0.0), 1.0);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[pivot, circle]));
    run_from_palette(&mut harness, RECTANGULAR);
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    run_from_palette(&mut harness, CIRCULAR);
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    assert!(harness.shows(patterning::CIRCULAR_PROMPT));
    assert!(harness.shows("1 item"));
    set(&mut harness, PatternField::Copies, "2");
    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 2);
    assert!(has_centre(sketch, Point2::new(10.0, 0.0)));
    assert_eq!(entities_of_kind(sketch, "Point").len(), 3);
    assert!(solves(&harness, feature));
}

#[test]
fn the_panels_shape_switches_between_the_two_patterns_and_the_origin_can_be_the_centre() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));

    run_from_palette(&mut harness, RECTANGULAR);
    harness.click(PatternKind::Circular.label());
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::CircularPattern));
    assert!(harness.shows(sketch_pattern_panel::SPREAD));
    assert!(harness.shows(TurnKind::FullTurn.label()));
    assert!(harness.shows(sketch_pattern_panel::NO_CENTRE));

    harness.click(sketch_pattern_panel::USE_ORIGIN);
    harness.frame();
    assert!(!harness.shows(sketch_pattern_panel::NO_CENTRE));
    set(&mut harness, PatternField::Copies, "4");
    make(&mut harness);
    assert_eq!(entities_of_kind(harness.sketch(feature), "Circle").len(), 4);
    assert!(has_centre(&harness.shown(feature), Point2::new(-5.0, 5.0)));

    harness.click(PatternKind::Rectangular.label());
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(harness.shows(PatternField::Spacing(Direction::FIRST).label()));
}

fn slanted_fixture(harness: &mut Harness) -> (FeatureId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(CIRCLE_CENTRE, 1.0);
    let guide = sketch.add_line(Point2::new(-30.0, -30.0), Point2::new(-10.0, -10.0));
    (edit_free_sketch(harness, sketch), circle, guide)
}

#[test]
fn a_line_clicked_in_the_view_sets_the_direction_the_copies_follow() {
    let mut harness = Harness::new();
    let (feature, circle, guide) = slanted_fixture(&mut harness);
    let guide_label = harness.sketch(feature).entity_label(guide);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);
    assert!(harness.shows(sketch_pattern_panel::BY_ANGLE));
    assert!(harness.shows(PatternField::Angle(Direction::FIRST).label()));

    harness.click(&Direction::FIRST.choose_label());
    harness.frame();
    assert!(harness.shows(sketch_pattern_panel::CHOOSING));
    assert!(harness.shows(patterning::DIRECTION_PROMPT));
    harness.click_at(Point2::new(-20.0, -20.0));
    harness.frame();
    assert!(harness.shows(&guide_label));
    assert!(!harness.shows(PatternField::Angle(Direction::FIRST).label()));
    set(&mut harness, PatternField::Count(Direction::FIRST), "3");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    make(&mut harness);

    let sketch = harness.sketch(feature);
    let step = 10.0 / 2.0_f64.sqrt();
    for steps in [1.0, 2.0] {
        let expected = CIRCLE_CENTRE + Point2::new(step * steps, step * steps);
        assert!(has_centre(sketch, expected), "{expected}");
    }
    assert_eq!(constraints_of_kind(sketch, "Parallel through").len(), 2);
    assert!(solves(&harness, feature));

    harness.click(&Direction::FIRST.angle_label());
    harness.frame();
    assert!(harness.shows(sketch_pattern_panel::BY_ANGLE));
    assert!(harness.shows(PatternField::Angle(Direction::FIRST).label()));
}

#[test]
fn a_direction_is_chosen_from_the_keyboard_and_escape_stops_choosing() {
    let mut harness = Harness::new();
    let (feature, circle, _) = slanted_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    harness.click(&Direction::FIRST.choose_label());
    harness.frame();
    assert!(
        harness.shows(sketch_pattern_panel::CHOOSING),
        "choosing first"
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::RectangularPattern));
    assert!(harness.shows(sketch_pattern_panel::BY_ANGLE));

    harness.click(&Direction::FIRST.choose_label());
    harness.frame();
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Vertical axis"));
    set(&mut harness, PatternField::Count(Direction::FIRST), "2");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    make(&mut harness);

    assert!(has_centre(
        harness.sketch(feature),
        CIRCLE_CENTRE + Point2::new(0.0, 10.0)
    ));
    assert!(solves(&harness, feature));
}

#[test]
fn both_sides_and_an_extent_spread_the_copies_either_way_of_the_original() {
    let mut harness = Harness::new();
    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    harness.click("Both sides");
    harness.frame();
    assert!(!harness.shows(&Direction::FIRST.flip_label()));
    harness.click("Extent");
    harness.frame();
    set(&mut harness, PatternField::Count(Direction::FIRST), "3");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "20");
    assert!(harness.shows("Repeat 1 item 4 more times"));
    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 5);
    for x in [-15.0, -5.0, 15.0, 25.0] {
        assert!(has_centre(sketch, Point2::new(x, 5.0)), "{x}");
    }
    assert!(solves(&harness, feature));
}

#[test]
fn each_directions_sides_menu_opens_on_its_own_when_the_panel_is_too_narrow_for_the_segments() {
    let mut harness = Harness::new();
    harness
        .workspace
        .panels
        .set_right_width(Some(layout::MIN_RIGHT_WIDTH));

    let (feature, circle, _) = row_fixture(&mut harness);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);
    set(&mut harness, PatternField::Count(Direction::FIRST), "2");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    add_direction(&mut harness);
    set(&mut harness, PatternField::Count(SECOND), "2");
    set(&mut harness, PatternField::Spacing(SECOND), "12");
    let panel = harness.position_of(&SECOND.title());
    harness.wheel_until_shown(panel, &SECOND.flip_label(), Wheel::Down);

    assert!(!harness.shows("Both sides"));

    choose(&mut harness, "One side", "Both sides");

    assert!(!harness.shows(&SECOND.flip_label()));
    assert!(harness.shows("Repeat 1 item 5 more times"));

    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 6);
    for (x, y) in [(15.0, -7.0), (5.0, 17.0), (15.0, 17.0)] {
        assert!(has_centre(sketch, Point2::new(x, y)), "{x}, {y}");
    }
}

#[test]
fn a_symmetric_circular_pattern_turns_the_copies_both_ways() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 0.0), 1.0);
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, CIRCULAR);
    harness.click(sketch_pattern_panel::USE_ORIGIN);
    harness.frame();

    harness.click(TurnKind::Symmetric.label());
    harness.frame();
    assert!(harness.shows("Angle each way"));
    set(&mut harness, PatternField::Copies, "2");
    set(&mut harness, PatternField::TotalAngle, "90");
    make(&mut harness);

    let shown = harness.shown(feature);
    assert!(has_centre(&shown, Point2::new(0.0, 10.0)));
    assert!(has_centre(&shown, Point2::new(0.0, -10.0)));
    assert!(solves(&harness, feature));
}

#[test]
fn a_direction_is_chosen_after_gathering_what_to_repeat_in_the_tool() {
    let mut harness = Harness::new();
    let (feature, circle, guide) = slanted_fixture(&mut harness);
    let guide_label = harness.sketch(feature).entity_label(guide);
    run_from_palette(&mut harness, RECTANGULAR);
    harness.select(entity_pickables(feature, &[circle]));
    harness.frame();
    assert!(harness.shows("1 item"));

    harness.click(&Direction::FIRST.choose_label());
    harness.frame();
    harness.click_at(Point2::new(-20.0, -20.0));
    harness.frame();
    harness.frame();

    assert!(harness.shows(&guide_label));
    assert!(harness.shows("1 item"));
    set(&mut harness, PatternField::Count(Direction::FIRST), "2");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    make(&mut harness);
    let step = 10.0 / 2.0_f64.sqrt();
    assert!(has_centre(
        harness.sketch(feature),
        CIRCLE_CENTRE + Point2::new(step, step)
    ));
}

#[test]
fn three_directions_repeat_every_combination_and_removing_one_renumbers_the_rest() {
    let mut harness = Harness::new();
    let (feature, circle, guide) = slanted_fixture(&mut harness);
    let guide_label = harness.sketch(feature).entity_label(guide);
    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, RECTANGULAR);

    set(&mut harness, PatternField::Count(Direction::FIRST), "2");
    set(&mut harness, PatternField::Spacing(Direction::FIRST), "10");
    for _ in [SECOND, THIRD] {
        add_direction(&mut harness);
    }
    assert!(harness.shows(&THIRD.title()));
    assert!(harness.shows(&THIRD.remove_label()));
    assert!(!harness.shows(&Direction::FIRST.remove_label()));
    set(&mut harness, PatternField::Count(SECOND), "2");
    set(&mut harness, PatternField::Spacing(SECOND), "10");
    set(&mut harness, PatternField::Count(THIRD), "2");
    set(&mut harness, PatternField::Spacing(THIRD), "4");
    let panel = harness.position_of(&THIRD.title());
    harness.wheel_until_shown(panel, &THIRD.choose_label(), Wheel::Down);
    harness.hold_still();
    harness.click(&THIRD.choose_label());
    harness.frame();
    harness.click_at(Point2::new(-20.0, -20.0));
    harness.frame();
    assert!(harness.shows(&guide_label));
    make(&mut harness);

    let sketch = harness.sketch(feature);
    let lean = 4.0 / 2.0_f64.sqrt();
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 8);
    for (x, y) in [
        (10.0, 0.0),
        (0.0, 10.0),
        (10.0, 10.0),
        (lean, lean),
        (10.0 + lean, 10.0 + lean),
    ] {
        let expected = CIRCLE_CENTRE + Point2::new(x, y);
        assert!(has_centre(sketch, expected), "{expected}");
    }
    assert!(solves(&harness, feature));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.select(entity_pickables(feature, &[circle]));
    harness.click(&SECOND.remove_label());
    harness.frame();
    assert!(!harness.shows(&THIRD.title()));
    assert!(harness.shows(&SECOND.title()));
    assert!(harness.shows(&guide_label));
    assert!(!harness.shows(&THIRD.choose_label()));
    make(&mut harness);

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 4);
    for (x, y) in [(10.0, 0.0), (lean, lean), (10.0 + lean, lean)] {
        let expected = CIRCLE_CENTRE + Point2::new(x, y);
        assert!(has_centre(sketch, expected), "{expected}");
    }
    assert!(solves(&harness, feature));
}
