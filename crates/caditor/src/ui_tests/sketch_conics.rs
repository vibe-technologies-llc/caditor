use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, Sketch, SplineKind};
use egui::{Key, Modifiers};

use super::{
    DRAWN, Harness, edit_free_sketch, entities_of_kind, only_constraint, sketch_entity, type_point,
};
use crate::{annotations, editing::Tool, sketch_toolbar};

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

#[test]
fn the_smart_dimension_gives_a_lone_conic_a_rho_that_drives_it() {
    let mut harness = Harness::new();
    let (start, apex, end) = (
        Point2::new(0.0, 0.0),
        Point2::new(20.0, 20.0),
        Point2::new(40.0, 0.0),
    );
    let mut sketch = Sketch::new(Plane::XY);
    let conic = sketch.add_spline_of(&[start, apex, end], SplineKind::Conic { rho: 0.5 });
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::D);

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 10.0),
        sketch_entity(feature, conic),
    );
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let (constraint, added) = only_constraint(harness.sketch(feature));

    assert_eq!(
        added,
        Constraint::Rho {
            conic,
            value: Expression::Number(0.5),
        }
    );
    assert!(harness.sketch(feature).is_active(constraint));
    harness.frame();
    harness.type_into_field(annotations::field_id(feature, constraint), "0.25");
    harness.settle();
    let shoulder = start.lerp(end, 0.5).lerp(apex, 0.25);
    let shown = harness.shown(feature).spline(conic).unwrap().point_at(0.5);
    assert!(shown.distance(shoulder) < 1e-6, "{shown} is not {shoulder}");
}
