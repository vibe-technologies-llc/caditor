use std::path::Path;

use caditor_file::{RecentFiles, Settings};
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, accesskit::Role, vec2};
use tempfile::TempDir;

use super::{
    CAMERA_SETTLE, Harness, canonical, edit_base_sketch, extruded_plate, sample_document,
    scroll_in_view, viewport_centre,
};
use crate::{
    app::Workspace,
    model::Action,
    onboarding::Onboarding,
    preferences::{
        self, Choice, DoubleClick, FieldOfView, OrbitAround, PreferenceChange, Preferences,
        PreferencesCommand, PreferencesTab, RecentListed, Section, Startup, TooltipDelay,
        Transitions, ZoomToward,
    },
    view_aids::EdgeWeight,
};

pub(super) fn open_tab(harness: &mut Harness, tab: PreferencesTab) {
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click(tab.label());
}

pub(super) fn close_preferences(harness: &mut Harness) {
    harness.click("Close");
    harness.frame();
}

pub(super) fn stored(dir: &Path) -> Preferences {
    Preferences::from_settings(Settings::load(&dir.join("config")))
}

pub(super) fn wait_until_stored(
    harness: &mut Harness,
    dir: &Path,
    done: impl Fn(&Preferences) -> bool,
) {
    harness.wait_until("the preferences are saved", |_| done(&stored(dir)));
}

pub(super) fn reopened(dir: &Path) -> Harness {
    let mut preferences = stored(dir);
    preferences.onboarding = Onboarding::finished();
    Harness::starting(
        Some(dir),
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    )
}

pub(super) fn settle_camera(harness: &mut Harness) {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
}

fn right_drag(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
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
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
}

fn turn_of_a_right_drag(harness: &mut Harness) -> f64 {
    let centre = viewport_centre(harness);
    let before = harness.workspace.viewport.viewpoint().forward();
    right_drag(harness, centre, centre + vec2(60.0, 0.0));
    let after = harness.workspace.viewport.viewpoint().forward();
    before.cross(after).z
}

fn fitted_distance(harness: &mut Harness) -> f64 {
    harness.key(Key::F, Modifiers::NONE);
    harness.frame();
    settle_camera(harness);
    harness.workspace.viewport.viewpoint().distance
}

#[test]
fn navigation_preferences_zoom_at_the_centre_invert_the_orbit_widen_the_view_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    settle_camera(&mut harness);
    let standard_fit = fitted_distance(&mut harness);
    let plain_turn = turn_of_a_right_drag(&mut harness);

    open_tab(&mut harness, PreferencesTab::Navigation);
    harness.click(ZoomToward::Centre.label());
    harness.click(OrbitAround::Centre.label());
    harness.click("Invert left and right");
    harness.click(FieldOfView::Wide.label());
    harness.click(Transitions::Off.label());
    close_preferences(&mut harness);

    let navigation = harness.workspace.preferences.navigation;
    assert_eq!(navigation.zoom_toward, ZoomToward::Centre);
    assert_eq!(navigation.orbit_around, OrbitAround::Centre);
    assert!(navigation.invert_orbit_x && !navigation.invert_orbit_y);
    assert_eq!(navigation.field_of_view, FieldOfView::Wide);
    assert_eq!(navigation.transitions, Transitions::Off);

    let inverted_turn = turn_of_a_right_drag(&mut harness);
    assert!(
        plain_turn * inverted_turn < 0.0,
        "{plain_turn} {inverted_turn}"
    );

    let target = harness.workspace.viewport.viewpoint().target;
    harness.events.push(Event::PointerMoved(
        viewport_centre(&harness) + vec2(150.0, 90.0),
    ));
    harness.frame();
    scroll_in_view(&mut harness, vec2(0.0, 60.0));
    let zoomed = harness.workspace.viewport.viewpoint();
    assert!(zoomed.target.distance(target) < 1e-9 * zoomed.distance);

    harness.key(Key::F, Modifiers::NONE);
    harness.frame();
    assert!(!harness.workspace.viewport.is_animating());
    let wide_fit = harness.workspace.viewport.viewpoint().distance;
    assert!(wide_fit < standard_fit, "{wide_fit} {standard_fit}");

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.navigation == navigation
    });
    let again = reopened(dir.path());
    assert_eq!(again.workspace.preferences.navigation, navigation);
}

#[test]
fn view_details_thicken_the_edges_hide_outlines_and_the_view_cube_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.frame();
    let standard_width = harness.built().scene.silhouettes[0].width;
    assert!(harness.shows("Fit all"));

    open_tab(&mut harness, PreferencesTab::Appearance);
    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "outlines");
    harness.click(EdgeWeight::Thick.label());
    harness.frame();
    let thick_width = harness.built().scene.silhouettes[0].width;
    harness.click("Outline curved faces");
    harness.click("Show the view cube");
    close_preferences(&mut harness);

    assert!((thick_width - standard_width * EdgeWeight::Thick.factor()).abs() < 1e-6);
    assert!(harness.built().scene.silhouettes.is_empty());
    assert!(!harness.shows("Fit all"));

    let details = harness.workspace.preferences.view_details;
    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.view_details == details
    });
    let mut again = reopened(dir.path());
    extruded_plate(&mut again);
    again.frame();
    assert_eq!(
        again.workspace.preferences.view_details.edges.weight,
        EdgeWeight::Thick
    );
    assert!(again.built().scene.silhouettes.is_empty());
    assert!(!again.shows("Fit all"));
}

#[test]
fn hover_help_waits_as_long_as_preferences_say_and_is_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let standard = harness.context.global_style().interaction.tooltip_delay;

    open_tab(&mut harness, PreferencesTab::Appearance);
    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "hover help");
    harness.click(TooltipDelay::AtOnce.label());
    harness.frame();

    assert_eq!(standard, TooltipDelay::Standard.seconds());
    assert_eq!(
        harness.context.global_style().interaction.tooltip_delay,
        0.0
    );
    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.tooltip_delay == TooltipDelay::AtOnce
    });
    let mut again = reopened(dir.path());
    again.frame();
    assert_eq!(again.context.global_style().interaction.tooltip_delay, 0.0);
}

#[test]
fn sketching_preferences_keep_the_view_keep_the_sketch_on_escape_set_the_toggles_and_are_remembered()
 {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    settle_camera(&mut harness);

    open_tab(&mut harness, PreferencesTab::Sketching);
    harness.click("Turn the view to face it");
    harness.click("Finishes the sketch when nothing is left to cancel");
    harness.click("Show constraint glyphs");
    harness.click("Snap to the grid");
    harness.click("Show dimensions on the model");
    close_preferences(&mut harness);

    let viewport = &harness.workspace.viewport;
    assert!(!viewport.glyphs_shown());
    assert!(viewport.grid_snapping());
    assert!(viewport.dimensions_shown());
    assert!(viewport.snapping() && viewport.typed_dimensions());

    let before = harness.workspace.viewport.destination();
    let base = edit_base_sketch(&mut harness);
    let after = harness.workspace.viewport.destination();
    assert_eq!(harness.editing(), Some(base));
    assert!(before.orientation.angle_between(after.orientation) < 1e-9);
    for _ in 0..4 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.editing(), Some(base));

    let sketching = harness.workspace.preferences.sketching;
    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.sketching == sketching
    });
    let again = reopened(dir.path());
    let viewport = &again.workspace.viewport;
    assert!(!viewport.glyphs_shown());
    assert!(viewport.grid_snapping());
    assert!(viewport.dimensions_shown());
    assert!(!again.workspace.preferences.sketching.escape_finishes);
}

#[test]
fn by_default_entering_a_sketch_faces_it_and_escape_finishes_it() {
    let mut harness = Harness::new();
    settle_camera(&mut harness);

    let before = harness.workspace.viewport.destination();
    edit_base_sketch(&mut harness);
    let after = harness.workspace.viewport.destination();
    for _ in 0..4 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }

    assert!(before.orientation.angle_between(after.orientation) > 1e-3);
    assert_eq!(harness.editing(), None);
}

fn model_files(dir: &TempDir, count: usize) -> Vec<std::path::PathBuf> {
    let folder = canonical(dir).join("models");
    std::fs::create_dir_all(&folder).unwrap();
    let mut recent = RecentFiles::default();
    let paths: Vec<_> = (0..count)
        .map(|index| folder.join(format!("Model {index}.caditor")))
        .collect();
    for path in paths.iter().rev() {
        caditor_file::save(&sample_document().unwrap(), path, false).unwrap();
        recent.add(path.clone());
    }
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    recent.save(&state).unwrap();
    paths
}

fn started_with(
    dir: &TempDir,
    document: caditor_document::Document,
    preferences: &Preferences,
) -> Harness {
    let settings = preferences.settings();
    let mut preferences = preferences.clone();
    preferences.onboarding = Onboarding::finished();
    let mut harness = Harness::starting_with(
        Some(dir.path()),
        document,
        Workspace::with_preferences(preferences),
        |files| files.settings_loaded(settings),
    );
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    harness
}

#[test]
fn startup_recent_files_and_double_click_preferences_do_what_they_say_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let paths = model_files(&dir, 5);
    let mut harness = started_with(&dir, sample_document().unwrap(), &Preferences::default());
    assert_eq!(harness.files.recent().len(), 5);
    assert_eq!(harness.model.path(), None);

    open_tab(&mut harness, PreferencesTab::General);
    harness.click(Startup::LastModel.label());
    harness.click(RecentListed::Three.label());
    harness.click(DoubleClick::Rename.label());
    close_preferences(&mut harness);
    assert_eq!(harness.files.recent().len(), 3);

    harness.settle();
    harness.double_click("Side sketch");
    assert!(harness.workspace.panels.renaming.is_some());
    assert_eq!(harness.editing(), None);

    let preferences = harness.workspace.preferences.clone();
    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.startup == Startup::LastModel
            && stored.recent_listed == RecentListed::Three
            && stored.double_click == DoubleClick::Rename
    });
    drop(harness);

    let mut again = started_with(
        &dir,
        caditor_document::Document::default(),
        &stored(dir.path()),
    );
    again.wait_until("the last model opens", |harness| {
        harness.model.path() == paths.first().map(std::path::PathBuf::as_path)
    });
    assert_eq!(again.files.recent().len(), 3);
    assert_eq!(
        again.workspace.preferences.double_click,
        preferences.double_click
    );
}

#[test]
fn the_search_field_filters_the_settings_by_name_across_every_tab() {
    let mut harness = Harness::new();
    open_tab(&mut harness, PreferencesTab::General);
    assert!(harness.shows("Centimetres"));

    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "field of view");

    assert!(harness.shows("Field of view"));
    assert!(harness.shows(PreferencesTab::Navigation.label()));
    assert!(!harness.shows("Centimetres"));
    assert!(!harness.shows("Orbit speed"));

    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "grid");
    assert!(harness.shows(Section::Colours.title()));
    assert!(harness.shows("Snap to the grid"));
    assert!(!harness.shows("Field of view"));

    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "no such setting");
    assert!(harness.shows_containing("No setting matches"));
    harness.click(preferences::CLEAR_SEARCH);
    assert!(harness.workspace.preferences_search.is_empty());
    assert!(harness.shows("Centimetres"));

    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "zoom");
    harness.key(Key::Tab, Modifiers::NONE);
    harness.frame();
    assert_ne!(harness.focused(), Some(Id::new(preferences::SEARCH_FIELD)));
    harness.click(PreferencesTab::Graphics.label());
    assert!(harness.workspace.preferences_search.is_empty());
    assert!(harness.shows("Anti-aliasing"));
}

#[test]
fn a_section_resets_alone_and_its_reset_can_be_undone() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::FieldOfView(FieldOfView::Narrow),
    )));
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::ZoomToward(ZoomToward::Centre),
    )));
    open_tab(&mut harness, PreferencesTab::Navigation);
    harness.read_screen();
    let reset_view = preferences::reset_name(Section::NavigationView);
    assert!(harness.accessible_named(Role::Button, &reset_view));
    assert!(!harness.accessible_named(Role::Button, &preferences::reset_name(Section::Input)));

    harness.click_button(&reset_view);

    let navigation = harness.workspace.preferences.navigation;
    assert_eq!(navigation.field_of_view, FieldOfView::Standard);
    assert_eq!(navigation.zoom_toward, ZoomToward::Centre);
    assert!(harness.shows("View is back to its defaults."));

    harness.click_lowest("Undo");
    assert_eq!(
        harness.workspace.preferences.navigation.field_of_view,
        FieldOfView::Narrow
    );
}
