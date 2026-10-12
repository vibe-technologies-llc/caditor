use std::time::Duration;

use caditor_document::SolidResult;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::MeshQuality;
use caditor_render::{AdapterPreference, GraphicsInfo, Msaa, Shading};
use caditor_sketch::{Constraint, Sketch};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};
use tempfile::TempDir;

use super::{Harness, SCREEN, sample_document};
use crate::{
    app,
    app::Workspace,
    commands::{Command, CommandFrame},
    graphics::{CurveQuality, FrameLimit, Graphics, Hardware},
    model::Action,
    panels::Focus,
    preferences::{PreferenceChange, Preferences, PreferencesCommand, PreferencesTab},
    units::LengthUnit,
    widgets,
};

#[test]
fn the_gear_in_the_menu_bar_opens_the_preferences() {
    let mut harness = Harness::new();

    let gear = CommandFrame::new(&harness.workspace.preferences.keymap, Vec::new())
        .with_keys(Command::Preferences, &Command::Preferences.title());
    harness.click_button(&gear);
    harness.frame();
    harness.show_new_windows();

    assert!(harness.workspace.preferences_open);
    assert!(harness.shows("Preferences"));
}

#[test]
fn preferences_change_units_and_navigation_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Preferences"));
    harness.click("Centimetres");
    assert_eq!(harness.model.length_unit(), LengthUnit::Centimetre);
    harness.click("Navigation");
    harness.click("Scroll up to zoom out");
    assert!(harness.workspace.preferences.navigation.invert_zoom);
    harness.click("Close");
    assert!(!harness.workspace.preferences_open);

    harness.wait_until("the preferences are saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).text("units.length") == Some("cm")
    });
    let stored =
        Preferences::from_settings(caditor_file::Settings::load(&dir.path().join("config")));
    assert_eq!(stored.unit, LengthUnit::Centimetre);
    assert!(stored.navigation.invert_zoom);

    harness.settle();
    assert!(harness.shows("2 cm"));
    let base = harness.document().features().next().unwrap().id();
    let (constraint, _) = harness
        .sketch(base)
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    harness.type_into(
        Focus::Dimension {
            feature: base,
            constraint,
        },
        "3",
    );
    let stored = harness
        .sketch(base)
        .constraint(constraint)
        .and_then(Constraint::dimension)
        .map(|value| harness.document().expression_text(value))
        .unwrap();
    assert_eq!(stored, "3 cm");
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterValue(width), "4");
    assert_eq!(harness.expression_text("width"), "4 cm");

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Defaults(PreferencesTab::General),
    )));
    assert_eq!(harness.model.length_unit(), LengthUnit::Millimetre);
    assert!(harness.shows("20 mm"));
}

#[test]
fn a_preference_that_could_not_be_saved_is_shown_as_a_notice() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("config"), b"not a folder").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    assert!(harness.model.notice().is_none());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.5),
    )));
    harness.wait_until("the failure is shown", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Could not save your preferences: "))
    });

    let notice = harness.model.notice().unwrap();
    assert!(notice.outlasts_edits);
    assert!(
        notice.text.starts_with(
            "Could not save your preferences: a file stands where its folder should be. "
        ),
        "{}",
        notice.text
    );
    assert!(notice.text.ends_with("change one again to retry saving."));
}

#[test]
fn the_side_panel_opens_as_it_was_left_and_follows_changes_to_it() {
    let left = crate::layout::PanelLayout {
        side_width: 420.0,
        right_width: None,
        features_open: true,
        parameters_open: false,
    };
    let mut settings = caditor_file::Settings::default();
    left.write(&mut settings);
    let mut preferences = Preferences::from_settings(settings);
    preferences.onboarding = crate::onboarding::Onboarding::finished();
    let mut harness = Harness::starting(
        None,
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    );

    assert_eq!(harness.workspace.panels.layout(), left);
    assert_eq!(harness.workspace.preferences.panels, left);
    assert!(!harness.shows("width"));

    harness.click(crate::panels::PARAMETERS_TITLE);
    harness.settle();

    assert!(harness.shows("width"));
    assert!(harness.workspace.preferences.panels.parameters_open);
    assert_eq!(harness.workspace.preferences.panels.side_width, 420.0);
}

#[test]
fn dragging_a_speed_slider_applies_at_once_and_is_saved_when_released() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Navigation");
    let label = harness.position_of("Orbit speed");
    let value = harness.position_of("1.00");
    let start = Pos2::new(value.x - 60.0, label.y);
    let button = |pressed| Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    harness.events.push(Event::PointerMoved(start));
    harness.frame();
    harness.events.push(button(true));
    harness.frame();
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(Pos2::new(
            start.x + 10.0 * step as f32,
            start.y,
        )));
        harness.frame();
    }
    let dragged = harness.workspace.preferences.navigation.orbit_speed;
    assert!(dragged > 1.0, "{dragged}");
    let config = dir.path().join("config");
    assert_eq!(
        caditor_file::Settings::load(&config).number("navigation.orbit_speed"),
        None
    );

    harness.events.push(Event::PointerButton {
        pos: Pos2::new(start.x + 40.0, start.y),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let released = harness.workspace.preferences.navigation.orbit_speed;
    harness.wait_until("the orbit speed is saved", |_| {
        caditor_file::Settings::load(&config).number("navigation.orbit_speed") == Some(released)
    });
}

fn open_preferences(harness: &mut Harness) {
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
}

fn tab_after(harness: &mut Harness, key: Key, modifiers: Modifiers) -> PreferencesTab {
    harness.key(key, modifiers);
    harness.frame();
    harness.frame();
    harness.workspace.preferences_tab
}

fn focused_tab(harness: &Harness) -> Option<usize> {
    let focused = harness.focused()?;
    (0..PreferencesTab::ALL.len()).find(|index| widgets::tab_id("preferences", *index) == focused)
}

#[test]
fn preferences_tabs_switch_by_mouse_and_keyboard_and_the_last_one_stays_open() {
    let mut harness = Harness::new();
    open_preferences(&mut harness);
    assert!(harness.shows("Centimetres"));
    assert!(!harness.shows("Scroll up to zoom out"));

    harness.click("Navigation");
    assert_eq!(
        harness.workspace.preferences_tab,
        PreferencesTab::Navigation
    );
    assert!(harness.shows("Scroll up to zoom out"));
    assert!(!harness.shows("Centimetres"));

    let ctrl = Modifiers::COMMAND;
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl),
        PreferencesTab::Graphics
    );
    assert!(harness.shows("Anti-aliasing"));
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl),
        PreferencesTab::General
    );
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl_shift),
        PreferencesTab::Graphics
    );
    assert_eq!(
        tab_after(&mut harness, Key::PageUp, ctrl),
        PreferencesTab::Navigation
    );
    assert_eq!(
        tab_after(&mut harness, Key::PageDown, ctrl),
        PreferencesTab::Graphics
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.preferences_open);
    open_preferences(&mut harness);
    assert!(harness.shows("Anti-aliasing"));

    for _ in 0..12 {
        if focused_tab(&harness).is_some() {
            break;
        }
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(focused_tab(&harness), Some(0));
    assert_eq!(harness.workspace.preferences_tab, PreferencesTab::Graphics);
    let none = Modifiers::NONE;
    assert_eq!(
        tab_after(&mut harness, Key::ArrowRight, none),
        PreferencesTab::Appearance
    );
    assert_eq!(focused_tab(&harness), Some(1));
    assert_eq!(
        tab_after(&mut harness, Key::ArrowRight, none),
        PreferencesTab::Navigation
    );
    assert_eq!(
        tab_after(&mut harness, Key::ArrowLeft, none),
        PreferencesTab::Appearance
    );
    assert_eq!(
        tab_after(&mut harness, Key::End, none),
        PreferencesTab::Graphics
    );
    assert_eq!(
        tab_after(&mut harness, Key::Home, none),
        PreferencesTab::General
    );
    assert_eq!(
        tab_after(&mut harness, Key::ArrowLeft, none),
        PreferencesTab::Graphics
    );
    assert_eq!(focused_tab(&harness), Some(3));
    assert!(harness.shows("Anti-aliasing"));
}

#[test]
fn every_preferences_tab_fits_the_window_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.workspace.hardware.adapter = Some(test_adapter());
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    open_preferences(&mut harness);
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);

    for tab in PreferencesTab::ALL {
        harness.click(tab.label());
        assert_eq!(harness.workspace.preferences_tab, tab);
        for label in PreferencesTab::ALL
            .map(PreferencesTab::label)
            .into_iter()
            .chain(["Restore defaults", "Close"])
        {
            let rect = harness
                .texts
                .iter()
                .find(|(shown, _)| shown == label)
                .unwrap_or_else(|| panic!("{label} is not on screen in {tab:?}"))
                .1;
            assert!(
                visible.contains_rect(rect),
                "{label} at {rect:?} in {tab:?}"
            );
        }
    }
}

fn test_adapter() -> GraphicsInfo {
    GraphicsInfo {
        adapter: "Test Adapter 9000".to_owned(),
        backend: "Vulkan".to_owned(),
        driver: "test driver 1.2".to_owned(),
        msaa_offered: vec![Msaa::Off, Msaa::X2, Msaa::X4],
        msaa: Msaa::X4,
        vsync_optional: false,
        vsync: true,
    }
}

#[test]
fn graphics_options_apply_at_once_are_saved_and_what_the_adapter_lacks_says_why() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.workspace.hardware = Hardware {
        adapter: Some(test_adapter()),
        refresh_rate: Some(60.0),
    };
    harness.workspace.preferences.graphics.msaa = Msaa::X8;
    harness.workspace.preferences.graphics.adapter = AdapterPreference::Performance;
    open_preferences(&mut harness);
    harness.click("Graphics");

    assert!(harness.shows("Test Adapter 9000"));
    assert!(harness.shows("Vulkan"));
    assert!(harness.shows("test driver 1.2"));
    assert!(harness.shows("60 Hz"));
    assert!(harness.shows("This graphics adapter cannot draw 8× anti-aliasing, so 4× is used."));

    harness.click("4×");
    assert_eq!(harness.workspace.preferences.graphics.msaa, Msaa::X4);
    assert!(!harness.shows("This graphics adapter cannot draw 8× anti-aliasing, so 4× is used."));
    harness.click("8×");
    assert_eq!(harness.workspace.preferences.graphics.msaa, Msaa::X4);
    harness.hover("8×");
    assert!(harness.shows("This graphics adapter cannot smooth edges with 8 samples per pixel"));
    harness.click("Wait for the display");
    assert!(harness.workspace.preferences.graphics.vsync);

    harness.click("2×");
    harness.click("Enhanced");
    harness.click("Match the display");
    harness.click("120 fps");
    let graphics = harness.workspace.preferences.graphics;
    assert_eq!(graphics.msaa, Msaa::X2);
    assert_eq!(graphics.shading, Shading::Enhanced);
    assert_eq!(graphics.frame_limit, FrameLimit::Fps120);
    assert_eq!(graphics.render().msaa, Msaa::X2);
    assert_eq!(
        graphics.frame_interval(&harness.workspace.hardware),
        Some(Duration::from_secs_f64(1.0 / 120.0))
    );
    let config = dir.path().join("config");
    harness.wait_until("the graphics settings are saved", |_| {
        let saved = caditor_file::Settings::load(&config);
        saved.number("graphics.msaa") == Some(2.0)
            && saved.text("graphics.shading") == Some("enhanced")
            && saved.text("graphics.frame_limit") == Some("120")
    });

    harness.click("Restore defaults");
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(harness.shows("Restore the defaults of the Graphics tab?"));
    harness.click("Cancel");
    assert!(!harness.shows("Restore the defaults of the Graphics tab?"));
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(harness.workspace.preferences_open);

    harness.click("Restore defaults");
    harness.click("Restore defaults");
    assert_eq!(harness.workspace.preferences.graphics, Graphics::default());
    assert_eq!(harness.workspace.preferences_tab, PreferencesTab::Graphics);
    assert!(harness.shows("This tab is back to its defaults."));
    harness.click_lowest("Undo");
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(!harness.shows("This tab is back to its defaults."));
    harness.wait_until("the undone settings are saved", |_| {
        caditor_file::Settings::load(&config).text("graphics.frame_limit") == Some("120")
    });
}

#[test]
fn the_adapter_preference_starts_on_auto_says_what_it_chose_and_an_explicit_choice_hides_the_line()
{
    let mut harness = Harness::new();
    harness.workspace.hardware = Hardware {
        adapter: Some(test_adapter()),
        refresh_rate: Some(60.0),
    };
    let line = crate::graphics::auto_summary(caditor_render::Machine::detect());
    open_preferences(&mut harness);
    harness.click("Graphics");

    assert_eq!(
        harness.workspace.preferences.graphics.adapter,
        AdapterPreference::Auto
    );
    assert!(harness.shows(&line));

    harness.click("Performance");
    assert_eq!(
        harness.workspace.preferences.graphics.adapter,
        AdapterPreference::Performance
    );
    assert!(!harness.shows(&line));

    harness.click("Auto");
    assert_eq!(
        harness.workspace.preferences.graphics.adapter,
        AdapterPreference::Auto
    );
    assert!(harness.shows(&line));
}

#[test]
fn coarse_curves_mesh_bodies_again_with_fewer_facets_and_apply_from_the_start() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 20.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let body = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let facets = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(body)
            .and_then(|result| result.solid())
            .and_then(SolidResult::mesh)
            .map(|mesh| mesh.triangles().len())
            .expect("the body is meshed")
    };
    let smooth = facets(&harness);
    assert_eq!(harness.model.mesh_quality(), MeshQuality::SMOOTH);

    open_preferences(&mut harness);
    harness.click("Graphics");
    harness.click("Coarse");
    assert_eq!(
        harness.workspace.preferences.graphics.curves,
        CurveQuality::Coarse
    );
    assert_eq!(harness.model.mesh_quality(), MeshQuality::COARSE);
    harness.settle();
    let coarse = facets(&harness);
    assert!(coarse < smooth, "{coarse} facets coarse, {smooth} smooth");

    harness.click("Smooth");
    harness.settle();
    assert_eq!(facets(&harness), smooth);

    let mut stored = caditor_file::Settings::default();
    stored.set_text("graphics.curve_quality", "coarse");
    app::apply_preferences(&mut harness.model, &Preferences::from_settings(stored));
    assert_eq!(harness.model.mesh_quality(), MeshQuality::COARSE);
}
