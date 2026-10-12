use caditor_document::{FeatureId, PrimitiveShape};
use caditor_geometry::Point3;
use egui::{Key, Modifiers};

use super::{Harness, drag_value_arrow, length_value, primitive_of, run_from_palette};
use crate::{move_manipulator::Handle, value_gauges::Measured};

fn open_primitive(harness: &mut Harness, name: &str) -> FeatureId {
    harness.select([]);
    run_from_palette(harness, name);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the primitive is open")
}

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .unwrap_or_else(|| panic!("{measured:?} is shown"))
}

fn wedge_sizes(harness: &Harness, wedge: FeatureId) -> [f64; 4] {
    let PrimitiveShape::Wedge {
        length,
        width,
        height,
        top,
    } = &primitive_of(harness, wedge).shape
    else {
        panic!("the wedge stays a wedge");
    };
    [length, width, height, top].map(|size| length_value(harness, size))
}

fn prism_sides(harness: &Harness, prism: FeatureId) -> f64 {
    let PrimitiveShape::Prism { sides, .. } = &primitive_of(harness, prism).shape else {
        panic!("the prism stays a prism");
    };
    length_value(harness, sides)
}

#[test]
fn a_wedge_shows_an_arrow_at_the_end_of_its_top_and_dragging_it_changes_the_top_length() {
    let mut harness = Harness::new();
    let wedge = open_primitive(&mut harness, "Wedge");
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    let before = wedge_sizes(&harness, wedge);
    let foot = foot_of(&harness, Measured::PrimitiveSize(3));

    assert!(
        (foot - Point3::new(before[3] - before[0] / 2.0, 0.0, before[2])).length() < 1e-9,
        "{foot:?} for {before:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::PrimitiveSize(3), 4.0);
    let after = wedge_sizes(&harness, wedge);

    assert_eq!(harness.model.undo_label(), Some("Edit Wedge 1"));
    assert!(
        (after[3] - before[3] - 4.0).abs() <= step,
        "{before:?} to {after:?} with steps of {step}"
    );
    assert_eq!(after[..3], before[..3]);
}

#[test]
fn a_wedge_s_top_length_stops_at_nothing_and_leaves_the_slope_to_its_edge() {
    let mut harness = Harness::new();
    let wedge = open_primitive(&mut harness, "Wedge");
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    let before = wedge_sizes(&harness, wedge);

    drag_value_arrow(&mut harness, Measured::PrimitiveSize(3), 3.0);
    let longer = wedge_sizes(&harness, wedge);
    drag_value_arrow(&mut harness, Measured::PrimitiveSize(3), -30.0);
    let none = wedge_sizes(&harness, wedge);

    assert!(longer[3] > before[3]);
    assert_eq!(none[3], 0.0);
}

#[test]
fn a_prism_shows_an_arrow_on_a_corner_and_dragging_it_changes_the_number_of_sides() {
    let mut harness = Harness::new();
    let prism = open_primitive(&mut harness, "Prism");
    harness.workspace.viewport.advance(super::CAMERA_SETTLE);
    harness.frame();
    let before = prism_sides(&harness, prism);
    let foot = foot_of(&harness, Measured::PrismSides);
    let PrimitiveShape::Prism {
        diameter, height, ..
    } = &primitive_of(&harness, prism).shape
    else {
        panic!("the prism stays a prism");
    };
    let radius = length_value(&harness, diameter) / 2.0;
    let rise = length_value(&harness, height);
    let angle = std::f64::consts::PI / before;
    let corner = Point3::new(radius * angle.sin(), -radius * angle.cos(), rise);

    assert!(
        (foot - corner).length() < 1e-9,
        "{foot:?} instead of {corner:?}"
    );

    drag_value_arrow(&mut harness, Measured::PrismSides, 12.0);
    let more = prism_sides(&harness, prism);

    assert_eq!(harness.model.undo_label(), Some("Edit Prism 1"));
    assert_eq!(before, 6.0);
    assert_eq!(more, 4.0);

    drag_value_arrow(&mut harness, Measured::PrismSides, -12.0);
    let most = prism_sides(&harness, prism);

    assert_eq!(most, 5.0);
}
