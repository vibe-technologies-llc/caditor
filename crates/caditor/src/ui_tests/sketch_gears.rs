use caditor_geometry::{Plane, Point2};
use caditor_sketch::{EntityId, Faceting, Sketch, SpurGear};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, edit_free_sketch, entities_of_kind, run_from_palette};
use crate::{
    editing::Tool,
    gear_panel,
    gearing::{self, GearField},
    solid_tools,
};

const SPUR_GEAR: &str = "draw a spur gear";
const CENTRE: Point2 = Point2::new(40.0, 25.0);

fn construction_circles(sketch: &Sketch) -> usize {
    entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter(|circle| sketch.is_construction(*circle))
        .count()
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
    assert_eq!(entities_of_kind(sketch, "Spline").len(), 48);
    assert_eq!(construction_circles(sketch), 4);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 5);
    assert!(sketch.open_ends().is_empty());
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
    let coincident = constraints_of_kind(sketch, "Coincident");
    assert_eq!(coincident.len(), 1);
    assert!(
        entities_of_kind(sketch, "Circle")
            .into_iter()
            .filter_map(|circle| sketch.circle(circle))
            .any(|(centre, radius)| centre == Point2::ZERO && (radius - 15.0).abs() < 1e-9)
    );
    assert!(coincident[0].entities().contains(&EntityId::ORIGIN));
}

#[test]
fn a_drawn_gear_extrudes_into_a_solid_of_its_outline() {
    let mut harness = Harness::new();
    let feature = edit_free_sketch(&mut harness, Sketch::new(Plane::XY));
    let gear = SpurGear {
        module: 2.0,
        teeth: 18,
        pressure_angle: 20.0,
        profile_shift: 0.0,
        root_fillet: 0.75,
        bore: 0.0,
    };
    let outline: Vec<Point2> = gear
        .outline(Point2::ZERO)
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

    run_from_palette(&mut harness, SPUR_GEAR);
    harness.type_into_field(gear_panel::field_id(GearField::Teeth), "18");
    harness.click(&gear_panel::draw_label(None, None));
    harness.settle();
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
    assert_eq!(
        entities_of_kind(harness.sketch(feature), "Spline").len(),
        36
    );
}
