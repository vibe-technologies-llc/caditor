use caditor_geometry::Point3;
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, extruded_plate, open_combo,
    preference_groups::{close_preferences, open_tab, reopened, wait_until_stored},
    vertex_at,
};
use crate::{
    app,
    preferences::{Choice, PreferencesTab},
    units::{Decimals, NumberFormat, TrailingZeros},
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
