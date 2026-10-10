use caditor_document::{CurveSpacing, PatternKind};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;

use super::{Harness, extruded_plate, pattern_of, run_from_palette, volume_about};
use crate::{model::Action, pattern_panel};

#[test]
fn patterns_along_a_sketch_curve_and_at_its_points_come_from_the_palette_and_their_panels() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    line.add_line(Point2::new(0.0, 0.0), Point2::new(200.0, 0.0));
    let path = harness.add_sketch(line);

    harness.select([]);
    run_from_palette(&mut harness, "curve pattern");
    harness.settle();
    harness.let_animations_finish();
    let along = harness
        .workspace
        .editing
        .solid()
        .expect("the curve pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Curve pattern 1"));
    let PatternKind::Curve(curve) = &pattern_of(&harness, along).kind else {
        panic!("a curve pattern follows a curve");
    };
    assert_eq!(curve.sketch, path);
    assert_eq!(pattern_of(&harness, along).body, plate);
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));
    assert!(harness.shows(pattern_panel::CURVE));
    assert!(harness.shows(pattern_panel::CURVE_HINT));

    harness.click(pattern_panel::BY_DISTANCE);
    harness.settle();
    let PatternKind::Curve(curve) = &pattern_of(&harness, along).kind else {
        panic!("the pattern stays along the curve");
    };
    assert_eq!(curve.measured, CurveSpacing::Distance);
    assert!(harness.shows("Spacing"));
    assert!(volume_about(&harness, plate, 28000.0));

    let mut points = Sketch::new(Plane::XY);
    points.add_point(Point2::new(100.0, 100.0));
    points.add_point(Point2::new(-100.0, 100.0));
    let spots = harness.add_sketch(points);
    harness.select([]);
    run_from_palette(&mut harness, "point pattern");
    harness.settle();
    let at_points = harness
        .workspace
        .editing
        .solid()
        .expect("the point pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Point pattern 1"));
    assert_eq!(pattern_of(&harness, at_points).kind.sketch(), Some(spots));
    assert!(harness.shows(pattern_panel::BASE_POINT));
    assert!(volume_about(&harness, plate, 3.0 * 28000.0));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(volume_about(&harness, plate, 28000.0));
}
