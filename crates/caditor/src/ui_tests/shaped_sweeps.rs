use caditor_document::Extrude;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::WallSide;
use caditor_sketch::Sketch;
use egui::{Id, Key, Modifiers};

use super::{Harness, choose, extruded, open_extrude, volume_about};

fn frustum(height: f64, bottom: f64, top: f64) -> f64 {
    height / 3.0 * (bottom + top + (bottom * top).sqrt())
}

fn wall_of(extrude: &Extrude) -> Option<(f64, WallSide)> {
    extrude.wall.as_ref().map(|wall| {
        let thickness = match wall.thickness {
            caditor_expression::Expression::Measure(value, _) => value,
            _ => f64::NAN,
        };
        (thickness, wall.side)
    })
}

#[test]
fn an_extrude_panel_takes_a_taper_and_a_thin_wall() {
    let mut harness = Harness::new();
    let plate = extruded(&mut harness, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    assert!(harness.shows("Taper"));
    assert!(volume_about(&harness, plate, 16_000.0));

    harness.type_into_field(Id::new(("solid-field", "taper", plate)), "5 deg");
    harness.settle();
    let taper = open_extrude(&harness, plate).taper;
    assert_eq!(
        taper.map(|angle| harness.document().expression_text(&angle)),
        Some("5 deg".to_owned())
    );
    let top = 40.0 - 20.0 * 5.0_f64.to_radians().tan();
    assert!(volume_about(
        &harness,
        plate,
        frustum(10.0, 1_600.0, top * top)
    ));

    harness.type_into_field(Id::new(("solid-field", "taper", plate)), "0 deg");
    harness.settle();
    assert_eq!(open_extrude(&harness, plate).taper, None);

    harness.click("Thin wall");
    harness.settle();
    assert_eq!(
        wall_of(&open_extrude(&harness, plate)),
        Some((1.0, WallSide::Centred))
    );
    assert!(harness.shows("The wall follows every curve of the sketch"));
    assert!(volume_about(&harness, plate, 1_600.0));

    harness.type_into_field(Id::new(("solid-field", "wall-thickness", plate)), "2 mm");
    harness.settle();
    assert!(volume_about(&harness, plate, 3_200.0));

    choose(&mut harness, "Centred", "Inside");
    assert_eq!(
        wall_of(&open_extrude(&harness, plate)),
        Some((2.0, WallSide::Inside))
    );
    assert!(volume_about(&harness, plate, 3_040.0));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(
        wall_of(&open_extrude(&harness, plate)),
        Some((2.0, WallSide::Centred))
    );

    harness.click_lowest("Solid");
    harness.settle();
    assert_eq!(open_extrude(&harness, plate).wall, None);
    assert!(volume_about(&harness, plate, 16_000.0));
}

#[test]
fn extruding_an_open_sketch_makes_a_thin_wall_along_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 10.0), Point2::new(0.0, 0.0));
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    harness.add_sketch(sketch);
    harness.select([]);

    harness.click("Extrude");
    harness.settle();

    let feature = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    assert_eq!(
        wall_of(&open_extrude(&harness, feature)),
        Some((1.0, WallSide::Centred))
    );
    assert!(volume_about(&harness, feature, 300.0));
}
