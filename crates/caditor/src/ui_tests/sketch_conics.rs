use caditor_geometry::Point2;
use caditor_sketch::{Entity, SplineKind};

use super::{DRAWN, Harness, entities_of_kind, type_point};
use crate::{editing::Tool, sketch_toolbar};

#[test]
fn a_conic_is_drawn_from_its_ends_and_apex_with_a_typed_rho() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();

    harness.click_button(sketch_toolbar::CURVE_WAYS_LABEL);
    harness.click(Tool::Conic.label());
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Conic));
    assert!(harness.shows("Click where the conic starts"));
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows("Click where the conic ends"));
    harness.click_at(Point2::new(50.0, 10.0));
    assert!(harness.shows("Click its apex, where the tangents at its ends meet (rho 0.50)"));
    type_point(&mut harness, "1.5 rho");
    assert!(harness.shows_containing("Rho must lie between 0.01 and 0.99"));
    harness.key(egui::Key::Escape, egui::Modifiers::NONE);
    harness.show_new_windows();
    type_point(&mut harness, "0.3 rho");
    assert!(harness.shows("Click its apex, where the tangents at its ends meet (rho 0.30)"));
    harness.click_at(Point2::new(30.0, 40.0));

    let sketch = harness.sketch(feature);
    let [conic] = entities_of_kind(sketch, "Conic")[..] else {
        panic!("expected one conic");
    };
    let Some(Entity::Spline { points, kind }) = sketch.entity(conic) else {
        panic!("expected a spline");
    };
    assert_eq!(*kind, SplineKind::Conic { rho: 0.3 });
    let expected = [
        Point2::new(10.0, 10.0),
        Point2::new(30.0, 40.0),
        Point2::new(50.0, 10.0),
    ];
    for (point, expected) in points.iter().zip(expected) {
        assert!(sketch.point(*point).unwrap().distance(expected) < DRAWN);
    }
    assert_eq!(harness.model.undo_label(), Some("Draw conic"));
}
