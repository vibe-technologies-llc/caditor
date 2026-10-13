use std::path::PathBuf;

use caditor_document::Rgb;
use caditor_file::{ExportFormat, Retention, Settings};
use caditor_geometry::Point3;
use egui::{Id, Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, extruded_plate, open_combo,
    preference_groups::{close_preferences, open_tab, reopened, wait_until_stored},
    sample_document, vertex_at,
};
use crate::{
    app::{self, Workspace},
    body_appearance::DEFAULT_COLOUR,
    canvas, colour_selector,
    export::OwnedLook,
    model::Action,
    onboarding::Onboarding,
    preferences::{
        self, BodyColour, Choice, ExportMemory, PreferenceChange, Preferences, PreferencesCommand,
        PreferencesTab,
    },
    units::{Decimals, NumberFormat, TrailingZeros},
    view_cube::{Corner, CubePlacement, CubeSize},
};

#[test]
fn number_preferences_round_the_readouts_drop_trailing_zeros_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (body, _) = extruded_plate(&mut harness);
    let corner = vertex_at(&harness, body, Point3::ZERO);
    let far = vertex_at(&harness, body, Point3::new(40.0, 40.0, 10.0));
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.select([corner, far]);
    harness.wait_until("the distance is measured", |harness| {
        harness.shows("57.446 mm") && harness.shows("40.000 mm")
    });

    open_tab(&mut harness, PreferencesTab::General);
    open_combo(&mut harness, "Length places");
    harness.click(Decimals::Fixed(1).label());
    open_combo(&mut harness, "Angle places");
    harness.click(Decimals::Fixed(0).label());
    harness.click(TrailingZeros::Hidden.label());
    close_preferences(&mut harness);
    harness.frame();

    let numbers = NumberFormat {
        lengths: Decimals::Fixed(1),
        angles: Decimals::Fixed(0),
        trailing_zeros: TrailingZeros::Hidden,
    };
    assert_eq!(harness.workspace.preferences.numbers, numbers);
    assert!(harness.shows("57.4 mm"));
    assert!(harness.shows("40 mm"));
    assert!(!harness.shows("40.000 mm"));
    assert_eq!(harness.model.units().angle_text(44.6), "45°");

    wait_until_stored(&mut harness, dir.path(), |stored| stored.numbers == numbers);
    let mut again = reopened(dir.path());
    app::apply_preferences(&mut again.model, &again.workspace.preferences);
    assert_eq!(again.workspace.preferences.numbers, numbers);
    assert_eq!(again.model.units().measured_length(12.04), "12 mm");
}

fn is_glyph(text: &str) -> bool {
    text.chars()
        .all(|letter| ('\u{e000}'..='\u{f8ff}').contains(&letter))
}

fn place_cube(harness: &mut Harness, placement: CubePlacement) {
    for change in [
        PreferenceChange::ViewCubeSize(placement.size),
        PreferenceChange::ViewCubeCorner(placement.corner),
    ] {
        harness.perform(Action::Preferences(PreferencesCommand::Change(change)));
    }
    harness.frame();
    harness.frame();
}

#[test]
fn the_view_cube_takes_any_size_and_corner_with_the_overlays_clear_of_it_and_is_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.frame();
    let own = ["Top", "Bottom", "Front", "Back", "Right", "Left", "Fit all"];

    for size in CubeSize::ALL.iter().copied() {
        for corner in Corner::ALL {
            let placement = CubePlacement { size, corner };
            place_cube(&mut harness, placement);
            let view = harness.workspace.viewport.rect().unwrap();
            let cube = placement.area(view);

            assert!(view.contains_rect(cube), "{placement:?}");
            assert!(harness.shows("Click the rectangle's first corner"));
            let fit = harness
                .texts
                .iter()
                .find(|(text, _)| text == "Fit all")
                .map(|(_, rect)| *rect)
                .expect("Fit all is shown");
            assert!(cube.contains_rect(fit), "{placement:?}");
            for (text, rect) in &harness.texts {
                let its_own = own.contains(&text.as_str()) || is_glyph(text);
                assert!(
                    its_own || !rect.intersects(cube),
                    "'{text}' runs under the view cube at {placement:?}"
                );
            }
            assert!(harness.shows("Scroll"), "the mouse hints at {placement:?}");
            for axis in ["X", "Y", "Z"] {
                assert!(harness.shows(axis), "the {axis} axis at {placement:?}");
            }
        }
    }

    place_cube(&mut harness, CubePlacement::default());
    open_tab(&mut harness, PreferencesTab::Appearance);
    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "view cube");
    harness.click(CubeSize::Large.label());
    harness.click(Corner::BottomLeft.label());
    close_preferences(&mut harness);
    let chosen = CubePlacement {
        size: CubeSize::Large,
        corner: Corner::BottomLeft,
    };
    assert_eq!(harness.workspace.preferences.view_details.cube, chosen);

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.view_details.cube == chosen
    });
    let again = reopened(dir.path());
    assert_eq!(again.workspace.preferences.view_details.cube, chosen);
}

const SAVES: u32 = 10;

fn versions_after_saves(retention: Retention) -> (usize, Retention) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plate.caditor");
    let mut harness = Harness::with_directories(Some(dir.path()));
    open_tab(&mut harness, PreferencesTab::General);
    harness.draft_into_field(Id::new(preferences::SEARCH_FIELD), "old versions");
    harness.click(retention.label());
    close_preferences(&mut harness);

    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for step in 1..=SAVES {
        harness.edit_width(&format!("{} mm", 40 + step));
        harness.key(Key::S, Modifiers::COMMAND);
        harness.wait_until("the change is saved", |harness| {
            !harness.model.is_dirty() && !harness.model.is_saving()
        });
    }
    let versions = caditor_file::history(&path).unwrap().versions.len();

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.retention == retention
    });
    (
        versions,
        reopened(dir.path()).workspace.preferences.retention,
    )
}

#[test]
fn the_version_history_preference_keeps_more_or_fewer_old_versions_and_is_remembered() {
    let (standard, _) = versions_after_saves(Retention::Standard);
    let (fewer, remembered) = versions_after_saves(Retention::Fewer);

    assert_eq!(standard, SAVES as usize);
    assert!((5..standard).contains(&fewer), "{fewer} of {standard}");
    assert_eq!(remembered, Retention::Fewer);
}

fn drawn_body(harness: &mut Harness) -> caditor_render::Color {
    let built = harness.built();
    built
        .scene
        .meshes
        .iter()
        .chain(&built.scene.flat_meshes)
        .find_map(|mesh| mesh.faces.first())
        .map(|face| face.color)
        .expect("the plate is drawn")
}

fn drawn(colour: Rgb) -> caditor_render::Color {
    caditor_render::Color::from_rgb8(colour.red, colour.green, colour.blue)
}

#[test]
fn a_chosen_body_colour_draws_bodies_without_their_own_refuses_an_unseen_one_and_is_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (plate, _) = extruded_plate(&mut harness);
    harness.frame();
    assert_eq!(drawn_body(&mut harness), drawn(DEFAULT_COLOUR));

    harness.perform(Action::Preferences(PreferencesCommand::Tab(
        PreferencesTab::Appearance,
    )));
    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    harness.click_button(&preferences::body_swatch_name("Steel blue"));
    harness.frame();
    harness.frame();

    let steel = Rgb::new(70, 130, 180);
    assert_eq!(
        harness.workspace.preferences.appearance.body,
        BodyColour::Custom(steel)
    );
    assert_eq!(drawn_body(&mut harness), drawn(steel));
    assert_eq!(canvas::body_colour(&harness.context), steel);
    let appearance = &harness.document().feature(plate).unwrap().appearance;
    assert_eq!(appearance.colour, None);
    assert_eq!(OwnedLook::of(appearance), None);

    let hex = colour_selector::hex_id(Id::new(preferences::BODY_SELECTOR));
    harness.click_button(preferences::CUSTOM_BODY);
    harness.frame();
    harness.type_into_field(hex, "#1c1d20");
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.appearance.body,
        BodyColour::Custom(steel)
    );
    assert!(harness.shows_containing("The body colour #1c1d20 shows at only 1.0"));
    assert!(harness.shows_containing("Choose a colour lighter or darker than the 3D view."));

    harness.click_button(preferences::DEFAULT_BODY);
    harness.frame();
    harness.frame();
    assert_eq!(drawn_body(&mut harness), drawn(DEFAULT_COLOUR));
    harness.click_button(&preferences::body_swatch_name("Steel blue"));
    harness.frame();

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.appearance.body == BodyColour::Custom(steel)
    });
    let mut again = reopened(dir.path());
    extruded_plate(&mut again);
    again.frame();
    assert_eq!(
        again.workspace.preferences.appearance.body,
        BodyColour::Custom(steel)
    );
    assert_eq!(drawn_body(&mut again), drawn(steel));
}

fn export_plate(harness: &mut Harness, to: PathBuf, format: Option<&str>) {
    harness.key(Key::E, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    if let Some(format) = format {
        harness.click(format);
    }
    harness.answer_dialog(Some(to));
    harness.click("Export…");
    harness.wait_until("the export is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body"))
    });
}

#[test]
fn export_choices_and_folders_are_remembered_across_restarts_unless_kept_for_the_session() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    export_plate(&mut harness, out.join("plate"), Some("OBJ"));
    assert!(out.join("plate.obj").exists());

    wait_until_stored(&mut harness, dir.path(), |stored| {
        stored.exports.model.format == ExportFormat::Obj
            && stored.exports.folders.model.as_deref() == Some(out.as_path())
    });

    let settings = Settings::load(&dir.path().join("config"));
    let mut preferences = Preferences::from_settings(settings.clone());
    preferences.onboarding = Onboarding::finished();
    let mut again = Harness::starting_with(
        Some(dir.path()),
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
        |files| files.settings_loaded(settings),
    );
    extruded_plate(&mut again);
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    export_plate(&mut again, elsewhere.join("again"), None);

    assert!(elsewhere.join("again.obj").exists());
    assert_eq!(
        again.dialogs.asked_in.lock().last().cloned().flatten(),
        Some(out.clone())
    );

    again.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::ExportMemory(ExportMemory::SessionOnly),
    )));
    wait_until_stored(&mut again, dir.path(), |stored| {
        stored.export_memory == ExportMemory::SessionOnly && stored.exports.is_empty()
    });
    assert_eq!(
        Settings::load(&dir.path().join("config")).keys_under("export.folder"),
        Vec::<String>::new()
    );
}
