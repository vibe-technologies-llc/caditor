use caditor_geometry::Point3;
use egui::{Id, Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, extruded_plate, open_combo,
    preference_groups::{close_preferences, open_tab, reopened, wait_until_stored},
    vertex_at,
};
use crate::{
    app,
    model::Action,
    preferences::{self, Choice, PreferenceChange, PreferencesCommand, PreferencesTab},
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
