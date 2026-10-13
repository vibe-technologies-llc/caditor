use caditor_geometry::Point2;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, constraints_of_kind, entities_of_kind, line_ends, near,
    preference_groups::{close_preferences, open_tab, reopened, wait_until_stored},
};
use crate::{
    inference::Inference,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, PreferencesTab},
};

fn draw_nearly_level_line(harness: &mut Harness, from: Point2, to: Point2) {
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
    draw_nearly_level_line(
        &mut harness,
        Point2::new(10.0, 10.0),
        Point2::new(40.0, 10.4),
    );
    assert!(constraints_of_kind(harness.sketch(feature), "Horizontal").is_empty());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Infer(Inference::Horizontal, true),
    )));
    draw_nearly_level_line(
        &mut harness,
        Point2::new(10.0, 30.0),
        Point2::new(40.0, 30.4),
    );

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
}
