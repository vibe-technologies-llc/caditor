use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Faceting, Sketch};
use egui::{Key, Modifiers};

use super::{
    Harness, choose, constraints_of_kind, edit_free_sketch, entities_of_kind, run_from_palette,
};
use crate::{
    editing::Tool,
    gear_panel,
    gearing::{self, GearField},
    selection::Pickable,
    solid_tools,
};

const SPUR_GEAR: &str = "draw a spur gear";
const CENTRE: Point2 = Point2::new(40.0, 25.0);
const PLACED_WITHIN: f64 = 1.0;

fn construction_circles(sketch: &Sketch) -> usize {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter(|circle| sketch.is_construction(*circle))
        .count()
}

fn only_gear(sketch: &Sketch) -> EntityId {
    let gears: Vec<EntityId> = sketch.gears().map(|(id, _)| id).collect();
    assert_eq!(gears.len(), 1, "{gears:?}");
    gears[0]
}

#[test]
fn a_gear_is_drawn_where_clicked_after_too_few_teeth_are_refused_in_words() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, SPUR_GEAR);
    assert_eq!(harness.tool(), Some(Tool::Gear));
    assert!(harness.shows(gear_panel::TITLE));
    assert!(harness.shows(gearing::PROMPT));
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "12");
    assert!(harness.shows_containing("use at least 17 teeth"));
    harness.click_at(CENTRE);
    harness.settle();
    assert!(harness.sketch(feature).same_content(&before));

    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "6 * 4");
    harness.type_into_field(gear_panel::field_id(GearField::Bore), "8 mm");
    assert!(harness.shows("48 mm"));
    harness.point_at(CENTRE);
    assert!(harness.shows_containing("Spur gear of 24 teeth"));
    harness.click_at(CENTRE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let gear = only_gear(sketch);
    let held = sketch.gear(gear).unwrap();
    let shown = harness.shown(feature);
    let outline = shown.gear_outline(gear).unwrap();
    assert_eq!(harness.document().expression_text(&held.teeth), "6 * 4");
    assert_eq!(harness.document().expression_text(&held.bore), "8 mm");
    assert_eq!(sketch.entities().len(), 1);
    assert_eq!(outline.teeth, 24);
    assert!(outline.centre.distance(CENTRE) < PLACED_WITHIN);
    assert_eq!(harness.model.undo_label(), Some(gearing::TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn the_panel_draws_the_gear_at_the_origin_from_its_button() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.type_into_field(gear_panel::field_id(GearField::Module), "3 mm / 2");
    harness.click(&gear_panel::draw_label(None, None));
    harness.settle();

    let sketch = harness.sketch(feature);
    let gear = only_gear(sketch);
    let coincident = constraints_of_kind(sketch, "Coincident");
    let shown = harness.shown(feature);
    let outline = shown.gear_outline(gear).unwrap();
    assert_eq!(coincident.len(), 1);
    assert_eq!(outline.centre, Point2::ZERO);
    assert!((outline.circles.pitch - 15.0).abs() < 1e-9);
    assert!(coincident[0].entities().contains(&EntityId::ORIGIN));
}

#[test]
fn a_drawn_gear_extrudes_into_a_solid_of_its_outline() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "18");
    harness.click(&gear_panel::draw_label(None, None));
    harness.settle();
    let gear = only_gear(harness.sketch(feature));
    let shown = harness.shown(feature);
    let outline: Vec<Point2> = shown
        .gear_outline(gear)
        .unwrap()
        .faceted(Faceting::within(1e-4))
        .into_iter()
        .flat_map(|piece| piece.into_iter().skip(1))
        .collect();
    let area = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        / 2.0;
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");

    let volume = harness.body_volume(extrude);
    assert!(
        (volume - area * solid_tools::DEFAULT_DISTANCE).abs() < 1e-3 * volume,
        "{volume} for {area}"
    );
    assert_eq!(entities_of_kind(harness.sketch(feature), "Spline").len(), 0);
}

#[test]
fn a_selected_gear_is_edited_in_place_keeping_its_centre_and_follows_the_new_teeth() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click_at(CENTRE);
    harness.settle();
    let gear = only_gear(harness.sketch(feature));
    let centre = harness.sketch(feature).gear(gear).unwrap().centre;
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: gear,
    }]);
    run_from_palette(&mut harness, SPUR_GEAR);
    let update = gear_panel::update_label(Some(harness.sketch(feature)), gear);
    assert!(harness.shows(&update));
    assert!(harness.shows(gearing::EDIT_PROMPT));
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "30");
    harness.type_into_field(gear_panel::field_id(GearField::Turn), "45 deg");
    harness.click(&update);
    harness.settle();

    let sketch = harness.sketch(feature);
    let held = sketch.gear(only_gear(sketch)).unwrap();
    let shown = harness.shown(feature);
    let outline = shown.gear_outline(gear).unwrap();
    assert_eq!(held.centre, centre);
    assert_eq!(harness.document().expression_text(&held.teeth), "30");
    assert_eq!(harness.document().expression_text(&held.turn), "45 deg");
    assert_eq!(outline.teeth, 30);
    assert_eq!(harness.model.undo_label(), Some(gearing::EDIT_TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    let held = sketch.gear(only_gear(sketch)).unwrap();
    assert_eq!(harness.document().expression_text(&held.teeth), "20");
}

#[test]
fn deleting_a_selected_gear_takes_its_centre() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click_at(CENTRE);
    harness.settle();
    let gear = only_gear(harness.sketch(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: gear,
    }]);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();

    let sketch = harness.sketch(feature);
    assert_eq!(sketch.gears().len(), 0);
    assert_eq!(sketch.entities().len(), 0);
}

fn circle_centres(sketch: &Sketch, construction: bool) -> Vec<(Point2, f64)> {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter(|circle| sketch.is_construction(*circle) == construction)
        .filter_map(|circle| sketch.circle(circle))
        .collect()
}

fn pair_ids(sketch: &Sketch) -> (EntityId, EntityId) {
    let gears: Vec<(EntityId, bool)> = sketch
        .gears()
        .map(|(id, gear)| (id, gear.meshing.is_some()))
        .collect();
    assert_eq!(gears.len(), 2, "{gears:?}");
    let driver = gears.iter().find(|(_, meshing)| !meshing).unwrap().0;
    let partner = gears.iter().find(|(_, meshing)| *meshing).unwrap().0;
    (driver, partner)
}

fn centres_apart(harness: &Harness, feature: caditor_document::FeatureId) -> Point2 {
    let shown = harness.shown(feature);
    let (driver, partner) = pair_ids(harness.sketch(feature));
    let at = |gear: EntityId| shown.gear_outline(gear).unwrap().centre;
    Point2::ZERO + (at(partner) - at(driver))
}

#[test]
fn a_gear_pair_is_stored_as_two_gears_at_the_centre_distance_and_a_meshing_gear_that_cannot_be_cut_is_refused()
 {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click("Gear pair");
    assert!(harness.shows(GearField::PairTeeth.label()));
    assert!(harness.shows(GearField::Direction.label()));
    harness.type_into_field(gear_panel::field_id(GearField::PairTeeth), "8");
    assert!(harness.shows_containing("The meshing gear: 8 teeth are undercut"));
    harness.click_at(CENTRE);
    harness.settle();
    assert!(harness.sketch(feature).same_content(&before));

    harness.type_into_field(gear_panel::field_id(GearField::PairTeeth), "15 * 2");
    harness.type_into_field(gear_panel::field_id(GearField::PairBore), "6 mm");
    harness.type_into_field(gear_panel::field_id(GearField::Direction), "90 deg");
    harness.point_at(CENTRE);
    assert!(harness.shows_containing("Gear pair of 20 and 30 teeth"));
    assert!(harness.shows_containing("centres 50 mm apart"));
    harness.click_at(CENTRE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let (driver, partner) = pair_ids(sketch);
    let held = sketch.gear(partner).unwrap();
    let shown = harness.shown(feature);
    let first = shown.gear_outline(driver).unwrap();
    let second = shown.gear_outline(partner).unwrap();
    assert_eq!(held.driver(), Some(driver));
    assert_eq!(harness.document().expression_text(&held.teeth), "15 * 2");
    assert_eq!(harness.document().expression_text(&held.bore), "6 mm");
    assert_eq!(sketch.entities().len(), 2);
    assert_eq!(entities_of_kind(sketch, "Spline").len(), 0);
    assert!(first.centre.distance(CENTRE) < PLACED_WITHIN);
    assert!((second.centre - first.centre).length() - 50.0 < 1e-6);
    assert!((second.centre.x - first.centre.x).abs() < 1e-6);
    assert!((second.centre.y - first.centre.y - 50.0).abs() < 1e-6);
    assert_eq!(second.teeth, 30);
    assert_eq!(harness.model.undo_label(), Some(gearing::PAIR_TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn editing_the_first_gear_of_a_pair_moves_and_reshapes_the_one_that_follows() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click("Gear pair");
    harness.click_at(CENTRE);
    harness.settle();
    let (driver, partner) = pair_ids(harness.sketch(feature));
    assert!((centres_apart(&harness, feature).x - 50.0).abs() < 1e-6);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: driver,
    }]);
    run_from_palette(&mut harness, SPUR_GEAR);
    assert!(harness.shows_containing("meshes with it and follows its values"));
    harness.type_into_field(gear_panel::field_id(GearField::Module), "3 mm");
    harness.type_into_field(gear_panel::field_id(GearField::ProfileShift), "0.25");
    harness.click(&gear_panel::update_label(
        Some(harness.sketch(feature)),
        driver,
    ));
    harness.settle();

    let shown = harness.shown(feature);
    let values = shown.gear_values(partner).unwrap();
    assert!((centres_apart(&harness, feature).x - 75.0).abs() < 1e-6);
    assert_eq!(values.spur.module, 3.0);
    assert_eq!(values.spur.profile_shift, -0.25);
    assert_eq!(harness.model.undo_label(), Some(gearing::EDIT_TRANSACTION));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!((centres_apart(&harness, feature).x - 50.0).abs() < 1e-6);
}

#[test]
fn a_meshing_gear_is_edited_for_its_teeth_and_direction_and_refuses_what_it_follows() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click("Gear pair");
    harness.click_at(CENTRE);
    harness.settle();
    let (_, partner) = pair_ids(harness.sketch(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: partner,
    }]);
    run_from_palette(&mut harness, SPUR_GEAR);
    assert!(harness.shows(gear_panel::EDITING_MESHING));
    assert!(!harness.shows(GearField::Module.label()));
    harness.type_into_field(gear_panel::field_id(GearField::PairTeeth), "8");
    assert!(harness.shows_containing("The meshing gear: 8 teeth are undercut"));
    harness.type_into_field(gear_panel::field_id(GearField::PairTeeth), "40");
    harness.type_into_field(gear_panel::field_id(GearField::Direction), "180 deg");
    harness.click(&gear_panel::update_label(
        Some(harness.sketch(feature)),
        partner,
    ));
    harness.settle();

    let apart = centres_apart(&harness, feature);
    assert!((apart.x + 60.0).abs() < 1e-6, "{apart:?}");
    assert!(apart.y.abs() < 1e-6);
}

#[test]
fn deleting_the_first_gear_of_a_pair_takes_the_other_and_both_centres() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click("Gear pair");
    harness.click_at(CENTRE);
    harness.settle();
    let (driver, _) = pair_ids(harness.sketch(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: driver,
    }]);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();
    let sketch = harness.sketch(feature);
    assert_eq!(sketch.gears().len(), 0);
    assert_eq!(sketch.entities().len(), 0);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.sketch(feature).gears().len(), 2);
}

#[test]
fn a_sprocket_for_the_chosen_chain_is_drawn_with_its_pitch_circle() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    let before = harness.sketch(feature).clone();

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.click("Sprocket");
    assert!(!harness.shows(GearField::Module.label()));
    assert!(harness.shows(gear_panel::CHAIN));
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "5");
    assert!(harness.shows_containing("Use a whole number from 9 to 150"));

    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "18");
    choose(&mut harness, "08B · 12.7 mm pitch", "10B · 15.875 mm pitch");
    assert!(harness.shows_containing("91.4"));
    harness.point_at(CENTRE);
    assert!(harness.shows_containing("Sprocket of 18 teeth for 10B chain"));
    harness.click_at(CENTRE);
    harness.settle();

    let sketch = harness.sketch(feature);
    let pitch_radius = 15.875 / (2.0 * (std::f64::consts::PI / 18.0).sin());
    assert_eq!(entities_of_kind(sketch, "Arc").len(), 4 * 18);
    assert!(entities_of_kind(sketch, "Spline").is_empty());
    assert_eq!(construction_circles(sketch), 3);
    assert!(
        circle_centres(sketch, true)
            .into_iter()
            .any(|(centre, radius)| {
                centre.distance(CENTRE) < PLACED_WITHIN && (radius - pitch_radius).abs() < 1e-9
            })
    );
    assert!(sketch.open_ends().is_empty());
    assert_eq!(
        harness.model.undo_label(),
        Some(gearing::SPROCKET_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn the_kind_and_the_values_chosen_stay_when_the_panel_switches_between_them() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "24");
    harness.click("Sprocket");
    assert!(harness.shows_containing("24"));
    harness.click("Single gear");
    harness.click(&gear_panel::draw_label(None, None));
    harness.settle();

    let sketch = harness.sketch(feature);
    let gear = only_gear(sketch);
    assert_eq!(
        harness
            .document()
            .expression_text(&sketch.gear(gear).unwrap().teeth),
        "24"
    );
    assert_eq!(harness.model.undo_label(), Some(gearing::TRANSACTION));
}
