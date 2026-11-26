use caditor_document::FeatureId;
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, constraints_of_kind, entities_of_kind, line_ends, near,
    preference_groups::{close_preferences, open_tab, reopened, wait_until_stored},
    run_from_palette,
};
use crate::{
    inference::Inference,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, PreferencesTab},
};

fn draw_line(harness: &mut Harness, from: Point2, to: Point2) {
    harness.use_tool(Key::L);
    harness.click_at(from);
    harness.point_at(to);
    harness.click_at(to);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
}

#[test]
fn a_nearly_level_line_gets_no_horizontal_constraint_once_horizontal_is_turned_off() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    let feature = harness.draw_on_new_sketch();

    run_from_palette(&mut harness, "Draw in round sizes");

    open_tab(&mut harness, PreferencesTab::Sketching);

    assert!(harness.shows(Inference::Horizontal.label()));

    harness.click(Inference::Horizontal.label());
    close_preferences(&mut harness);

    assert!(
        !harness
            .workspace
            .preferences
            .sketching
            .inferred
            .allows(Inference::Horizontal)
    );

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 30.0));
    harness.point_at(Point2::new(40.0, 30.4));

    assert!(!harness.shows("Horizontal"));

    harness.click_at(Point2::new(40.0, 30.4));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };

    let (start, end) = line_ends(sketch, line);

    assert!(near(sketch.point(start).unwrap(), Point2::new(10.0, 30.0)));
    assert!(near(sketch.point(end).unwrap(), Point2::new(40.0, 30.4)));
    assert!(constraints_of_kind(sketch, "Horizontal").is_empty());

    let inferred = harness.workspace.preferences.sketching.inferred;

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.sketching.inferred == inferred
    });

    let again = reopened(dir.path());
    let remembered = again.workspace.preferences.sketching.inferred;

    assert!(!remembered.allows(Inference::Horizontal));
    assert!(remembered.allows(Inference::Vertical));
}

#[test]
fn turning_a_kind_back_on_infers_it_again() {
    let mut harness = Harness::new();

    let feature = harness.draw_on_new_sketch();

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Infer(Inference::Horizontal, false),
    )));
    draw_line(
        &mut harness,
        Point2::new(10.0, 10.0),
        Point2::new(40.0, 10.4),
    );

    assert!(constraints_of_kind(harness.sketch(feature), "Horizontal").is_empty());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Infer(Inference::Horizontal, true),
    )));
    draw_line(
        &mut harness,
        Point2::new(10.0, 30.0),
        Point2::new(40.0, 30.4),
    );

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
}

const REFERENCE_LENGTH: f64 = 20.0;

fn draw_reference_and_aim_near_its_length(
    harness: &mut Harness,
    feature: FeatureId,
) -> (EntityId, Point2) {
    draw_line(
        harness,
        Point2::new(10.0, 10.0),
        Point2::new(10.0 + REFERENCE_LENGTH, 10.0),
    );

    let [reference] = entities_of_kind(harness.sketch(feature), "Line")[..] else {
        panic!("one line should be drawn");
    };
    let start = Point2::new(12.0, 18.0);
    let nearly = start + Vector2::ONE.normalize() * (REFERENCE_LENGTH - 0.1);

    harness.click_at(start);
    harness.point_at(nearly);

    (reference, nearly)
}

fn finish_line_at(harness: &mut Harness, end: Point2) {
    harness.click_at(end);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
}

fn length_of(sketch: &Sketch, line: EntityId) -> f64 {
    let (start, end) = line_ends(sketch, line);
    sketch
        .point(start)
        .unwrap()
        .distance(sketch.point(end).unwrap())
}

fn drawn_after(harness: &Harness, feature: FeatureId, reference: EntityId) -> EntityId {
    let sketch = harness.sketch(feature);
    let [drawn] = entities_of_kind(sketch, "Line")
        .into_iter()
        .filter(|line| *line != reference)
        .collect::<Vec<_>>()[..]
    else {
        panic!("one more line should be drawn");
    };
    drawn
}

#[test]
fn a_line_drawn_nearly_as_long_as_a_nearby_one_takes_its_length_and_is_held_equal_to_it() {
    let mut harness = Harness::new();

    let feature = harness.draw_on_new_sketch();
    let (reference, nearly) = draw_reference_and_aim_near_its_length(&mut harness, feature);

    let label = harness.sketch(feature).entity_label(reference);

    assert!(harness.shows(&format!("Equal length to {label}")));

    finish_line_at(&mut harness, nearly);

    let drawn = drawn_after(&harness, feature, reference);

    let sketch = harness.sketch(feature);
    let (length, wanted) = (length_of(sketch, drawn), length_of(sketch, reference));

    assert!((length - wanted).abs() < 1e-6, "{length} against {wanted}");
    assert_eq!(
        constraints_of_kind(sketch, "Equal"),
        [Constraint::Equal(drawn, reference)]
    );
}

#[test]
fn with_equal_length_turned_off_a_line_keeps_the_length_drawn_and_gets_no_equal_constraint() {
    let mut harness = Harness::new();

    let feature = harness.draw_on_new_sketch();

    run_from_palette(&mut harness, "Draw in round sizes");

    open_tab(&mut harness, PreferencesTab::Sketching);

    assert!(harness.shows(Inference::EqualLength.label()));

    harness.click(Inference::EqualLength.label());
    close_preferences(&mut harness);

    assert!(
        !harness
            .workspace
            .preferences
            .sketching
            .inferred
            .allows(Inference::EqualLength)
    );

    let (reference, nearly) = draw_reference_and_aim_near_its_length(&mut harness, feature);

    let label = harness.sketch(feature).entity_label(reference);

    assert!(!harness.shows(&format!("Equal length to {label}")));

    finish_line_at(&mut harness, nearly);

    let drawn = drawn_after(&harness, feature, reference);

    let sketch = harness.sketch(feature);
    let (length, unchanged) = (length_of(sketch, drawn), length_of(sketch, reference));

    assert!((length - (REFERENCE_LENGTH - 0.1)).abs() < 0.01, "{length}");
    assert!(
        (length - unchanged).abs() > 0.05,
        "{length} against {unchanged}"
    );
    assert!(constraints_of_kind(sketch, "Equal").is_empty());
}
