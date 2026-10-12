use egui::{Event, Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, drag_screen, laptop_harness, scroll_in_view, viewport_centre};
use crate::{
    onboarding::Hint,
    preferences::{InputMode, Preferences},
};

#[test]
fn the_wheel_zooms_in_caditor_mode_but_two_fingers_orbit_in_laptop_mode() {
    let mut harness = Harness::new();
    harness
        .events
        .push(Event::PointerMoved(viewport_centre(&harness)));
    harness.frame();
    let before = harness.workspace.viewport.viewpoint();

    scroll_in_view(&mut harness, egui::vec2(0.0, 60.0));

    let zoomed = harness.workspace.viewport.viewpoint();
    assert_ne!(zoomed.distance, before.distance);
    assert!(zoomed.forward().dot(before.forward()) > 0.99999);

    let mut harness = laptop_harness();
    let before = harness.workspace.viewport.viewpoint();

    scroll_in_view(&mut harness, egui::vec2(40.0, 0.0));

    let orbited = harness.workspace.viewport.viewpoint();
    assert!((orbited.distance - before.distance).abs() < 1e-6 * before.distance);
    assert!(orbited.forward().dot(before.forward()) < 0.9999);
}

#[test]
fn alt_and_two_fingers_pan_and_a_pinch_zooms_in_laptop_mode() {
    let mut harness = laptop_harness();
    let before = harness.workspace.viewport.viewpoint();

    harness.hold(Modifiers::ALT);
    scroll_in_view(&mut harness, egui::vec2(30.0, 20.0));
    harness.hold(Modifiers::NONE);

    let panned = harness.workspace.viewport.viewpoint();
    assert!(panned.target.distance(before.target) > 1e-6);
    assert!(panned.forward().dot(before.forward()) > 0.99999);

    harness.events.push(Event::Zoom(1.5));
    harness.frame();

    let pinched = harness.workspace.viewport.viewpoint();
    assert!(pinched.distance < panned.distance);
}

#[test]
fn alt_drag_orbits_in_laptop_mode_without_selecting_anything() {
    let mut harness = laptop_harness();
    let centre = viewport_centre(&harness);
    let before = harness.workspace.viewport.viewpoint();

    harness.hold(Modifiers::ALT);
    drag_screen(&mut harness, centre, centre + egui::vec2(60.0, 10.0));
    harness.hold(Modifiers::NONE);

    let orbited = harness.workspace.viewport.viewpoint();
    assert!(orbited.forward().dot(before.forward()) < 0.9999);
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn the_input_mode_is_chosen_in_preferences_and_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Navigation");
    assert!(harness.shows("Input mode"));

    harness.click(InputMode::Laptop.label());

    assert_eq!(
        harness.workspace.preferences.navigation.input_mode,
        InputMode::Laptop
    );
    harness.wait_until("the input mode is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).text("navigation.input_mode")
            == Some("laptop")
    });
    let stored =
        Preferences::from_settings(caditor_file::Settings::load(&dir.path().join("config")));
    assert_eq!(stored.navigation.input_mode, InputMode::Laptop);
    assert_eq!(
        Hint::Navigate.text(&stored.keymap, InputMode::Laptop),
        format!(
            "{} Double-click a face to open the feature that made it, and change any value in \
             the feature tree or in Parameters.",
            InputMode::Laptop.navigation_tip()
        )
    );
}
