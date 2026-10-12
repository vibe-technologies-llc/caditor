use egui::{
    Event, Key, Modifiers, PointerButton, Pos2, ViewportCommand, viewport::ResizeDirection,
};
use tempfile::TempDir;

use super::{Harness, SCREEN, bar_row, click_window_button, drag_screen, logo_loaded};
use crate::{
    appearance,
    commands::Command,
    icons, menu_bar,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, TitleBar},
    window_frame,
};

fn in_bar(harness: &Harness, label: &str) -> bool {
    let row = bar_row(harness);
    harness
        .texts
        .iter()
        .any(|(shown, rect)| shown == label && (rect.center().y - row).abs() < 12.0)
}

fn rightmost_in_bar(harness: &Harness) -> String {
    let row = bar_row(harness);
    harness
        .texts
        .iter()
        .filter(|(_, rect)| (rect.center().y - row).abs() < 12.0)
        .max_by(|a, b| a.1.center().x.total_cmp(&b.1.center().x))
        .map(|(shown, _)| shown.clone())
        .unwrap()
}

fn empty_bar_spot(harness: &Harness) -> Pos2 {
    Pos2::new(480.0, bar_row(harness))
}

fn asked_window(harness: &Harness, command: &ViewportCommand) -> bool {
    harness.window_commands.contains(command)
}

#[test]
fn the_title_bar_buttons_minimize_maximize_restore_and_close_the_window() {
    let mut harness = Harness::new();

    assert_eq!(rightmost_in_bar(&harness), icons::CLOSE);
    click_window_button(&mut harness, icons::MINIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Minimized(true)));
    click_window_button(&mut harness, icons::MAXIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));

    harness.window.maximized = Some(true);
    harness.frame();
    assert!(in_bar(&harness, icons::RESTORE));
    assert!(!in_bar(&harness, icons::MAXIMIZE));
    click_window_button(&mut harness, icons::RESTORE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(false)));

    click_window_button(&mut harness, icons::CLOSE);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn the_window_buttons_are_wide_targets_a_control_tall_and_close_turns_red() {
    let mut harness = Harness::new();
    harness.frame();

    for name in [
        window_frame::MINIMIZE,
        window_frame::MAXIMIZE,
        window_frame::CLOSE,
    ] {
        let rect = harness.button_rect(name);
        assert_eq!(
            rect.size(),
            egui::vec2(40.0, appearance::CONTROL_HEIGHT),
            "{name}"
        );
    }
    harness.hover_button(window_frame::CLOSE);
    let tokens = appearance::tokens_of(&harness.context);
    assert_eq!(harness.color_of(icons::CLOSE), tokens.text_on_accent);
}

#[test]
fn the_top_right_corner_of_a_maximized_window_closes_it() {
    let mut harness = Harness::new();
    harness.window.maximized = Some(true);
    harness.frame();
    let close = harness.button_rect(window_frame::CLOSE);
    let corner = Pos2::new(SCREEN.max.x - 0.5, SCREEN.min.y + 0.5);

    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    let tokens = appearance::tokens_of(&harness.context);
    let highlighted = harness.color_of(icons::CLOSE) == tokens.text_on_accent;
    harness.click_screen(corner);

    assert!(
        close.max.x < corner.x && close.min.y > corner.y,
        "{close:?}"
    );
    assert!(highlighted);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn dragging_the_title_bar_moves_the_window_and_double_clicking_it_maximizes() {
    let mut harness = Harness::new();
    let spot = empty_bar_spot(&harness);

    harness.click_screen(spot);
    harness.press(spot);
    harness.frame();
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));
    assert!(!asked_window(&harness, &ViewportCommand::StartDrag));

    harness.window_commands.clear();
    harness.let_animations_finish();
    drag_screen(&mut harness, spot, spot + egui::vec2(80.0, 0.0));
    assert!(asked_window(&harness, &ViewportCommand::StartDrag));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn the_window_menu_switches_to_the_system_title_bar_and_back() {
    let mut harness = Harness::new();
    let spot = empty_bar_spot(&harness);

    harness.events.push(Event::PointerMoved(spot));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: spot,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.show_new_windows();
    assert!(harness.shows(window_frame::MINIMIZE));
    harness.click(window_frame::SYSTEM_TITLE_BAR);
    assert_eq!(harness.workspace.preferences.title_bar, TitleBar::System);
    assert!(asked_window(&harness, &ViewportCommand::Decorations(true)));
    assert!(!in_bar(&harness, icons::CLOSE));
    assert!(in_bar(&harness, menu_bar::SEARCH_LABEL));

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::TitleBar(TitleBar::BuiltIn),
    )));
    assert!(asked_window(&harness, &ViewportCommand::Decorations(false)));
    assert_eq!(rightmost_in_bar(&harness), icons::CLOSE);
}

#[test]
fn full_screen_toggles_from_the_keyboard_and_its_button_leaves_it() {
    let mut harness = Harness::new();

    harness.key(Key::F11, Modifiers::NONE);
    harness.frame();
    assert!(asked_window(&harness, &ViewportCommand::Fullscreen(true)));

    harness.window.fullscreen = Some(true);
    harness.window_commands.clear();
    harness.frame();
    assert!(!in_bar(&harness, icons::MAXIMIZE));
    assert!(!in_bar(&harness, icons::MINIMIZE));
    let maximize = harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == Command::MaximizeWindow)
        .unwrap();
    assert!(maximize.availability.is_err());
    click_window_button(&mut harness, icons::LEAVE_FULL_SCREEN);
    assert!(asked_window(&harness, &ViewportCommand::Fullscreen(false)));
}

#[test]
fn the_window_border_resizes_the_window_unless_it_is_maximized() {
    let mut harness = Harness::new();
    let right = Pos2::new(SCREEN.max.x - 1.0, 500.0);
    let corner = Pos2::new(SCREEN.max.x - 1.0, SCREEN.max.y - 1.0);

    harness.click_screen(right);
    harness.click_screen(corner);
    assert!(asked_window(
        &harness,
        &ViewportCommand::BeginResize(ResizeDirection::East)
    ));
    assert!(asked_window(
        &harness,
        &ViewportCommand::BeginResize(ResizeDirection::SouthEast)
    ));

    harness.window.maximized = Some(true);
    harness.window_commands.clear();
    harness.frame();
    harness.click_screen(right);
    assert!(
        !harness
            .window_commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::BeginResize(_)))
    );
}

#[test]
fn the_window_buttons_and_title_bar_still_work_while_a_dialog_is_open() {
    let mut harness = Harness::new();

    harness.perform(Action::Preferences(PreferencesCommand::Show));
    assert!(harness.workspace.preferences_open);
    click_window_button(&mut harness, icons::MAXIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));
    let spot = empty_bar_spot(&harness);
    drag_screen(&mut harness, spot, spot + egui::vec2(80.0, 0.0));
    assert!(asked_window(&harness, &ViewportCommand::StartDrag));
    assert!(harness.workspace.preferences_open);
}

#[test]
fn the_model_title_shows_the_edited_sketch_and_saves_from_its_details() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.edit_width("45 mm");
    harness.frame();
    assert!(in_bar(&harness, "Untitled"));
    assert!(in_bar(&harness, "Unsaved"));
    harness.click("Untitled");
    assert!(harness.shows("Unsaved changes"));
    assert!(!harness.shows(menu_bar::COPY_PATH));
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click(&Command::Save.title());
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert!(in_bar(&harness, "plate.caditor"));
    assert!(!in_bar(&harness, "Unsaved"));

    harness.click("plate.caditor");
    assert!(harness.shows("All changes saved"));
    assert!(harness.shows(menu_bar::COPY_PATH));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let feature = harness.draw_on_new_sketch();
    let name = harness.document().feature(feature).unwrap().name.clone();
    assert!(in_bar(&harness, &name));
}

fn file_menu_after_title_bar(harness: &mut Harness, title_bar: TitleBar) -> f32 {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::TitleBar(title_bar),
    )));
    harness.frame();
    harness.position_of("File").x
}

#[test]
fn the_built_in_title_bar_leads_with_the_logo_and_the_system_one_leaves_it_out() {
    let mut harness = Harness::new();

    let without_logo = file_menu_after_title_bar(&mut harness, TitleBar::System);
    let with_logo = file_menu_after_title_bar(&mut harness, TitleBar::BuiltIn);

    assert!(with_logo > without_logo + 10.0);
    assert!(logo_loaded(&harness, 32));
}
