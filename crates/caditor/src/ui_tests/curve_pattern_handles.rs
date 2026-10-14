use caditor_document::{CurvePattern, CurveSpacing, FeatureId, PatternKind};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;

use super::{
    CAMERA_SETTLE, Harness, drag_value_arrow, extruded_plate, hold_drag, length_value, pattern_of,
    release_drag, run_from_palette,
};
use crate::{move_manipulator::Handle, pattern_panel, value_gauges::Measured};

const ARC_RADIUS: f64 = 30.0;
const PLATE_MIDDLE: Point3 = Point3::new(20.0, 20.0, 5.0);

fn open_curve_pattern(harness: &mut Harness, end: f64) -> FeatureId {
    let mut line = Sketch::new(Plane::XY);
    line.add_line(Point2::new(0.0, 0.0), Point2::new(end, 0.0));
    open_curve_pattern_along(harness, line)
}

fn open_curve_pattern_along(harness: &mut Harness, path: Sketch) -> FeatureId {
    extruded_plate(harness);
    harness.add_sketch(path);
    harness.select([]);
    run_from_palette(harness, "curve pattern");
    harness.settle();
    harness.let_animations_finish();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness
        .workspace
        .editing
        .solid()
        .expect("the curve pattern is open")
}

fn curve_of(harness: &Harness, pattern: FeatureId) -> CurvePattern {
    let PatternKind::Curve(curve) = &pattern_of(harness, pattern).kind else {
        panic!("the pattern follows a curve");
    };
    curve.clone()
}

fn count_of(harness: &Harness, pattern: FeatureId) -> f64 {
    length_value(harness, &curve_of(harness, pattern).count)
}

fn foot_of(harness: &Harness, measured: Measured) -> Point3 {
    harness
        .workspace
        .viewport
        .handle_foot(Handle::Value(measured))
        .unwrap_or_else(|| panic!("{measured:?} is shown"))
}

#[test]
fn a_curve_pattern_spread_over_its_curve_has_a_count_arrow_on_its_second_copy() {
    let mut harness = Harness::new();
    let pattern = open_curve_pattern(&mut harness, 60.0);
    let foot = foot_of(&harness, Measured::CurveCount);

    assert_eq!(count_of(&harness, pattern), 4.0);
    assert!(
        (foot - Point3::new(40.0, 20.0, 5.0)).length() < 1e-9,
        "{foot:?}"
    );

    drag_value_arrow(&mut harness, Measured::CurveCount, -5.0);

    assert_eq!(harness.model.undo_label(), Some("Edit Curve pattern 1"));
    assert_eq!(count_of(&harness, pattern), 5.0);

    drag_value_arrow(&mut harness, Measured::CurveCount, 5.0);

    assert!(count_of(&harness, pattern) < 5.0);
}

#[test]
fn a_curve_pattern_spaced_by_distance_has_a_spacing_arrow_on_its_last_copy() {
    let mut harness = Harness::new();
    let pattern = open_curve_pattern(&mut harness, -60.0);
    harness.click(pattern_panel::BY_DISTANCE);
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let curve = curve_of(&harness, pattern);
    let spacing = length_value(&harness, &curve.spacing);
    let count = count_of(&harness, pattern);
    let foot = foot_of(&harness, Measured::CurveSpacing);

    assert_eq!(curve.measured, CurveSpacing::Distance);
    assert!(
        (foot - Point3::new(20.0 - spacing * (count - 1.0), 20.0, 5.0)).length() < 1e-9,
        "{foot:?}"
    );

    let step = drag_value_arrow(&mut harness, Measured::CurveSpacing, 12.0);
    let wider = length_value(&harness, &curve_of(&harness, pattern).spacing);

    assert_eq!(harness.model.undo_label(), Some("Edit Curve pattern 1"));
    assert!(
        (wider - spacing - 12.0 / (count - 1.0)).abs() <= step,
        "{spacing} to {wider} with steps of {step}"
    );
    assert_eq!(count_of(&harness, pattern), count);
}

fn off_the_arc(foot: Point3) -> f64 {
    let on_path = foot - PLATE_MIDDLE + Vector3::new(ARC_RADIUS, 0.0, 0.0);
    (Point2::new(on_path.x, on_path.y).length() - ARC_RADIUS).abs() + on_path.z.abs()
}

#[test]
fn a_curve_pattern_along_an_arc_keeps_its_count_arrow_on_the_arc_while_it_is_dragged() {
    let mut harness = Harness::new();
    let mut arc = Sketch::new(Plane::XY);
    arc.add_arc(
        Point2::ZERO,
        Point2::new(ARC_RADIUS, 0.0),
        Point2::new(-ARC_RADIUS, 0.0),
    );
    let pattern = open_curve_pattern_along(&mut harness, arc);
    let to_second_copy = Vector3::new(
        ARC_RADIUS * (std::f64::consts::FRAC_PI_3.cos() - 1.0),
        ARC_RADIUS * std::f64::consts::FRAC_PI_3.sin(),
        0.0,
    );
    let at_rest = foot_of(&harness, Measured::CurveCount);

    assert_eq!(count_of(&harness, pattern), 4.0);
    assert!(
        at_rest.distance(PLATE_MIDDLE + to_second_copy) < 1e-6,
        "{at_rest:?}"
    );

    let viewport = &harness.workspace.viewport;
    let from = viewport
        .handle_position(Handle::Value(Measured::CurveCount), 0.0)
        .expect("the count arrow is shown");
    let to = viewport
        .handle_position(Handle::Value(Measured::CurveCount), -15.0)
        .expect("the count arrow is shown");
    hold_drag(&mut harness, from, to);
    let dragged = foot_of(&harness, Measured::CurveCount);

    assert!(off_the_arc(dragged) < 0.05, "{dragged:?}");
    assert!(dragged.distance(at_rest) > 5.0, "{dragged:?}");

    release_drag(&mut harness, to);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let placed = foot_of(&harness, Measured::CurveCount);

    assert_eq!(harness.model.undo_label(), Some("Edit Curve pattern 1"));
    assert!(count_of(&harness, pattern) > 5.0);
    assert!(off_the_arc(placed) < 1e-6, "{placed:?}");
}
