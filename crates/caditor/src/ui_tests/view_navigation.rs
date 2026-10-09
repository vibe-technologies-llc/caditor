use std::{f64::consts::PI, time::Duration};

use caditor_geometry::Vector3;
use caditor_render::Viewpoint;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, accesskit::Role};

use super::{CAMERA_SETTLE, Harness, drag_screen, run_from_palette, viewport_centre};
use crate::{
    commands::Command,
    editing::EditingCommand,
    model::Action,
    preferences::{InputMode, PreferenceChange, PreferencesCommand},
    view_cube, view_history,
};

const MID_TURN: Duration = Duration::from_millis(100);

fn settled() -> Harness {
    let mut harness = Harness::new();
    settle_camera(&mut harness);
    harness
}

fn settle_camera(harness: &mut Harness) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
}

fn press(harness: &mut Harness, key: Key, modifiers: Modifiers) {
    harness.key(key, modifiers);
    harness.frame();
}

fn heading(harness: &Harness) -> Viewpoint {
    harness.workspace.viewport.destination()
}

fn looking_from(viewpoint: Viewpoint) -> Vector3 {
    viewpoint.orientation * Vector3::Z
}

fn same_view(a: Viewpoint, b: Viewpoint) -> bool {
    a.target.distance(b.target) < 1e-6 * a.distance
        && (a.distance - b.distance).abs() < 1e-6 * a.distance
        && a.orientation.angle_between(b.orientation) < 1e-6
}

fn change(harness: &mut Harness, change: PreferenceChange) {
    harness.perform(Action::Preferences(PreferencesCommand::Change(change)));
    harness.frame();
}

fn drag_with(harness: &mut Harness, button: PointerButton, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let position = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: to,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
}

fn cube(harness: &mut Harness) -> (Rect, String) {
    if harness.accessible.is_empty() {
        harness.context.enable_accesskit();
        harness.frame();
        harness.frame();
    }
    harness
        .accessible
        .iter()
        .find_map(|(_, node)| {
            let label = node.label()?;
            let bounds = node.bounds()?;
            (node.role() == Role::Button && label.starts_with(view_cube::NAME)).then(|| {
                let rect = Rect::from_min_max(
                    Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                    Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
                );
                (rect, label.to_owned())
            })
        })
        .expect("the view cube is on screen")
}

#[test]
fn a_fit_during_a_turn_to_a_standard_view_keeps_the_view_it_was_turning_to() {
    let mut harness = settled();

    press(&mut harness, Key::Num1, Modifiers::ALT);
    harness.workspace.viewport.advance(MID_TURN);
    let mid_turn = harness.workspace.viewport.viewpoint();
    press(&mut harness, Key::F, Modifiers::NONE);
    settle_camera(&mut harness);
    let fitted = harness.workspace.viewport.viewpoint();

    assert!(mid_turn.forward().distance(Vector3::Y) > 0.1);
    assert!(fitted.forward().distance(Vector3::Y) < 1e-9, "{fitted:?}");
}

#[test]
fn a_key_nudge_during_a_turn_glides_on_from_the_view_it_was_turning_to() {
    let mut harness = settled();

    press(&mut harness, Key::Num2, Modifiers::ALT);
    harness.workspace.viewport.advance(MID_TURN);
    press(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    let gliding = harness.workspace.viewport.is_animating();
    settle_camera(&mut harness);
    let turned = harness.workspace.viewport.viewpoint();

    assert!(gliding);
    assert!(
        turned.forward().distance(Vector3::NEG_Z) < 1e-9,
        "{turned:?}"
    );
    assert!((turned.up().angle_between(Vector3::Y) - PI / 12.0).abs() < 1e-9);
}

#[test]
fn a_key_orbit_turns_further_at_a_higher_orbit_speed() {
    let mut harness = settled();
    change(&mut harness, PreferenceChange::OrbitSpeed(2.0));
    press(&mut harness, Key::Num1, Modifiers::ALT);
    settle_camera(&mut harness);

    press(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    settle_camera(&mut harness);
    let turned = harness.workspace.viewport.viewpoint().forward();

    assert!((turned.angle_between(Vector3::Y) - PI / 6.0).abs() < 1e-9);
}

#[test]
fn the_navigation_hint_names_the_buttons_of_the_input_mode() {
    let mut harness = settled();
    let caditor = harness.shows_hint(InputMode::Caditor.navigation_hint());

    change(
        &mut harness,
        PreferenceChange::InputMode(InputMode::Fusion360),
    );
    harness.frame();

    assert!(caditor);
    assert!(harness.shows_hint(InputMode::Fusion360.navigation_hint()));
    assert!(!harness.shows("Right-drag"));
}

#[test]
fn previous_view_steps_back_through_chosen_views_and_turns() {
    let mut harness = settled();
    let opened = heading(&harness);

    press(&mut harness, Key::Num2, Modifiers::ALT);
    settle_camera(&mut harness);
    let top = heading(&harness);
    press(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    settle_camera(&mut harness);
    let nudged = heading(&harness);
    let centre = viewport_centre(&harness);
    drag_with(
        &mut harness,
        PointerButton::Secondary,
        centre,
        centre + egui::vec2(80.0, 30.0),
    );
    let dragged = heading(&harness);

    let mut stepped = Vec::new();
    for _ in 0..3 {
        press(&mut harness, Key::ArrowLeft, Modifiers::ALT);
        stepped.push(heading(&harness));
        settle_camera(&mut harness);
    }
    press(&mut harness, Key::ArrowLeft, Modifiers::ALT);
    let after_all = heading(&harness);

    assert!(!same_view(dragged, nudged));
    assert!(same_view(stepped[0], nudged));
    assert!(same_view(stepped[1], top));
    assert!(same_view(stepped[2], opened));
    assert!(same_view(after_all, opened));
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.ends_with(view_history::NO_EARLIER_VIEW))
    );
}

#[test]
fn finishing_a_sketch_goes_back_to_the_view_before_it_with_one_key() {
    let mut harness = settled();
    let before = heading(&harness);

    harness.draw_on_new_sketch();
    let facing = heading(&harness);
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    run_from_palette(&mut harness, "previous view");
    harness.frame();
    settle_camera(&mut harness);

    assert!(!same_view(facing, before));
    assert!(same_view(heading(&harness), before));
}

#[test]
fn dragging_the_view_cube_orbits_and_home_goes_to_the_isometric_view() {
    let mut harness = settled();
    press(&mut harness, Key::Num1, Modifiers::ALT);
    settle_camera(&mut harness);
    let front = heading(&harness);
    let (rect, _) = cube(&mut harness);

    drag_screen(
        &mut harness,
        rect.center(),
        rect.center() + egui::vec2(30.0, 0.0),
    );
    let orbited = heading(&harness);
    let selection_kept_empty = harness.workspace.viewport.selection().is_empty();
    harness.click_button(view_cube::HOME_NAME);
    settle_camera(&mut harness);
    let home = looking_from(heading(&harness));

    assert!(orbited.forward().z.abs() < 1e-9);
    assert!(orbited.forward().angle_between(front.forward()) > 0.05);
    assert!(selection_kept_empty);
    assert!(home.distance(Vector3::new(1.0, -1.0, 1.0).normalize()) < 1e-9);
}

#[test]
fn the_view_cube_hover_names_the_keys_of_a_standard_view() {
    let mut harness = settled();
    press(&mut harness, Key::Num1, Modifiers::ALT);
    settle_camera(&mut harness);

    let (_, name) = cube(&mut harness);
    harness.hover_button(&name);

    assert!(harness.shows("View from front (Alt+1)"));
}

#[test]
fn a_middle_double_click_fits_the_view_in_every_input_mode() {
    for mode in InputMode::ALL {
        let mut harness = settled();
        change(&mut harness, PreferenceChange::InputMode(mode));
        press(&mut harness, Key::F, Modifiers::NONE);
        settle_camera(&mut harness);
        let fitted = heading(&harness);
        let centre = viewport_centre(&harness);
        harness.events.push(Event::PointerMoved(centre));
        harness.frame();
        press(&mut harness, Key::PageDown, Modifiers::NONE);
        press(&mut harness, Key::PageDown, Modifiers::NONE);
        settle_camera(&mut harness);
        let zoomed = heading(&harness);

        for _ in 0..2 {
            for pressed in [true, false] {
                harness.events.push(Event::PointerButton {
                    pos: centre,
                    button: PointerButton::Middle,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
                harness.frame();
            }
        }
        settle_camera(&mut harness);

        assert!(!same_view(zoomed, fitted), "{mode:?}");
        assert!(same_view(heading(&harness), fitted), "{mode:?}");
    }
}

#[test]
fn the_view_menu_offers_previous_view_and_looking_at_a_face() {
    let mut harness = settled();

    harness.click("View");

    assert!(harness.shows(&Command::PreviousView.title()));
    assert!(harness.shows(&Command::LookAtFace.title()));
    assert!(harness.shows(&Command::LookAtSketch.title()));
}
