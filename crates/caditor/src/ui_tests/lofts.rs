use caditor_document::{FeatureId, FeatureState, Loft, LoftSection, MissingInput};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::LoftSkin;
use caditor_sketch::{EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, choose, run_from_palette};
use crate::{loft_panel, reference_picking::Slot, selection::Pickable};

const LOW_HALF: f64 = 10.0;
const HIGH_HALF: f64 = 5.0;
const HEIGHT: f64 = 10.0;

fn level(height: f64) -> Plane {
    Plane::new(Point3::new(0.0, 0.0, height), Vector3::Z).unwrap()
}

fn add_square(harness: &mut Harness, height: f64, half: f64) -> (FeatureId, EntityId) {
    let mut sketch = Sketch::new(level(height));
    let corners = [
        Point2::new(-half, -half),
        Point2::new(half, -half),
        Point2::new(half, half),
        Point2::new(-half, half),
    ];
    let first = sketch.add_line(corners[0], corners[1]);
    for index in 1..4 {
        sketch.add_line(corners[index], corners[(index + 1) % 4]);
    }
    (harness.add_sketch(sketch), first)
}

fn loft_of(harness: &Harness, feature: FeatureId) -> Loft {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.loft())
        .expect("a loft")
        .clone()
}

fn frustum() -> f64 {
    let (bottom, top) = (4.0 * LOW_HALF * LOW_HALF, 4.0 * HIGH_HALF * HIGH_HALF);
    HEIGHT / 3.0 * (bottom + top + (bottom * top).sqrt())
}

fn near(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-2 * expected
}

fn curve(feature: FeatureId, entity: EntityId) -> Pickable {
    Pickable::SketchEntity { feature, entity }
}

#[test]
fn loft_runs_through_the_selected_sketches_in_order_and_takes_its_sides_from_the_panel() {
    let mut harness = Harness::new();
    let (low, low_line) = add_square(&mut harness, 0.0, LOW_HALF);
    let (high, high_line) = add_square(&mut harness, HEIGHT, HIGH_HALF);
    harness.select([curve(low, low_line), curve(high, high_line)]);

    run_from_palette(&mut harness, "loft");
    harness.settle();

    let loft = harness.workspace.editing.solid().expect("the loft is open");
    assert_eq!(harness.model.undo_label(), Some("Create Loft 1"));
    let made = loft_of(&harness, loft);
    assert_eq!(
        made.profiles,
        vec![LoftSection::whole(low), LoftSection::whole(high)]
    );
    assert_eq!(made.skin, LoftSkin::Smooth);
    assert!(near(harness.body_volume(loft), frustum()));
    assert!(harness.shows(loft_panel::PROFILES));
    let name = |sketch: FeatureId| harness.document().feature(sketch).unwrap().name.clone();
    assert!(harness.shows(&format!("1. {}", name(low))));
    assert!(harness.shows(&format!("2. {}", name(high))));
    assert!(harness.shows(loft_panel::LAST_PROFILE));

    choose(&mut harness, "Smooth", "Ruled");
    assert_eq!(loft_of(&harness, loft).skin, LoftSkin::Ruled);
    assert!(near(harness.body_volume(loft), frustum()));

    harness.click_button("Move this profile later");
    harness.settle();
    assert_eq!(
        loft_of(&harness, loft).profiles,
        vec![LoftSection::whole(high), LoftSection::whole(low)]
    );
    assert!(near(harness.body_volume(loft), frustum()));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(
        loft_of(&harness, loft).profiles,
        vec![LoftSection::whole(low), LoftSection::whole(high)]
    );
}

#[test]
fn loft_with_one_profile_waits_and_takes_the_next_from_a_click_in_the_view() {
    let mut harness = Harness::new();
    let (low, low_line) = add_square(&mut harness, 0.0, LOW_HALF);
    let (high, high_line) = add_square(&mut harness, HEIGHT, HIGH_HALF);
    harness.select([curve(high, high_line)]);

    run_from_palette(&mut harness, "loft");
    harness.settle();

    let loft = harness.workspace.editing.solid().expect("the loft is open");
    assert_eq!(
        loft_of(&harness, loft).profiles,
        vec![LoftSection::whole(high)]
    );
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::LoftProfiles)
    );
    assert_eq!(
        harness
            .model
            .evaluation()
            .feature(loft)
            .map(|status| status.state.clone()),
        Some(FeatureState::Waiting(MissingInput::Profiles))
    );
    assert!(harness.shows("Waiting for profiles"));

    harness.click_pickable(
        level(0.0),
        Point2::new(0.0, -LOW_HALF),
        curve(low, low_line),
    );
    harness.settle();

    assert_eq!(
        loft_of(&harness, loft).profiles,
        vec![LoftSection::whole(high), LoftSection::whole(low)]
    );
    assert!(near(harness.body_volume(loft), frustum()));
    assert_eq!(
        harness
            .workspace
            .editing
            .picking()
            .map(|picking| picking.slot),
        Some(Slot::LoftProfiles)
    );
}
