use caditor_geometry::{Maths, Plane, Point2, Point3, RigidTransform, Vector3};

use super::{
    chord::{ChordError, chord_fillet_radius},
    lofted_tests::cylinder,
    tests::{block_at, check, edge_through, fillet, polygon, run, spandrel, swept},
    *,
};
use crate::{fixtures::cuboid, profile::ProfileCurve};

fn right_prism() -> Solid {
    swept(
        Plane::XY,
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]),
        5.0,
    )
}

#[test]
fn a_chord_across_a_right_angle_is_a_radius_over_the_square_root_of_two() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));
    let edge = edge_through(&block, (10.0, 10.0, 5.0));

    let radius = chord_fillet_radius(&block, &[edge], 2.0).unwrap();

    assert!((radius - 2.0_f64.sqrt()).abs() < 1e-9, "{radius}");
}

#[test]
fn a_filleted_edge_has_the_asked_chord_between_its_tangent_lines() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));
    let edge = edge_through(&block, (10.0, 10.0, 5.0));

    let radius = chord_fillet_radius(&block, &[edge], 3.0).unwrap();
    let rounded = run(&block, &[edge], fillet(radius));

    let chord_squared = 2.0 * radius * radius;
    assert!((chord_squared - 9.0).abs() < 1e-9);
    check("chord fillet", &rounded, 1000.0 - spandrel(radius) * 10.0);
}

#[test]
fn the_rim_of_a_cylinder_turns_by_a_constant_angle() {
    let solid = cylinder();
    let rim = edge_through(&solid, (0.0, 40.0, 60.0));

    let radius = chord_fillet_radius(&solid, &[rim], 4.0).unwrap();

    assert!((radius - 4.0 / 2.0_f64.sqrt()).abs() < 1e-9, "{radius}");
}

#[test]
fn a_sharper_corner_needs_a_smaller_radius_for_the_same_chord() {
    let solid = right_prism();
    let right = edge_through(&solid, (0.0, 0.0, 2.0));
    let acute = edge_through(&solid, (10.0, 0.0, 2.0));

    let right_radius = chord_fillet_radius(&solid, &[right], 1.0).unwrap();
    let acute_radius = chord_fillet_radius(&solid, &[acute], 1.0).unwrap();

    assert!(acute_radius < right_radius);
    let expected = 1.0 / (2.0 * (135.0_f64.to_radians() / 2.0).sin_libm());
    assert!((acute_radius - expected).abs() < 1e-9, "{acute_radius}");
}

#[test]
fn edges_meeting_at_different_angles_are_refused() {
    let solid = right_prism();
    let right = edge_through(&solid, (0.0, 0.0, 2.0));
    let acute = edge_through(&solid, (10.0, 0.0, 2.0));

    let refused = chord_fillet_radius(&solid, &[right, acute], 1.0);

    assert!(
        matches!(refused, Err(ChordError::DifferingTurns { .. })),
        "{refused:?}"
    );
}

#[test]
fn edges_meeting_at_one_angle_share_a_radius() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));
    let first = edge_through(&block, (10.0, 10.0, 5.0));
    let second = edge_through(&block, (0.0, 10.0, 5.0));

    let radius = chord_fillet_radius(&block, &[first, second], 2.0).unwrap();

    assert!((radius - 2.0_f64.sqrt()).abs() < 1e-9);
}

#[test]
fn an_edge_whose_faces_turn_by_a_changing_angle_is_refused() {
    let bore = swept(
        Plane::XY,
        &[ProfileCurve::circle(2, Point2::new(0.0, 0.0), 3.0)],
        30.0,
    );
    let tilt =
        RigidTransform::rotation_about(Point3::new(0.0, 0.0, 15.0), Vector3::X, 0.5).unwrap();
    let bore = bore.transformed(&tilt).unwrap();
    let body = block_at((-20.0, -20.0, 0.0), (40.0, 40.0, 10.0), 1);
    let solid = boolean(&body, &bore, BooleanOperation::Difference).unwrap();

    let refusals: Vec<_> = solid
        .edges()
        .map(|(edge, _)| chord_fillet_radius(&solid, &[edge], 1.0))
        .collect();

    assert!(
        refusals
            .iter()
            .any(|refused| matches!(refused, Err(ChordError::VaryingTurn(_)))),
        "{refusals:?}"
    );
}

#[test]
fn an_empty_choice_is_refused() {
    let block = cuboid(Vector3::new(10.0, 10.0, 10.0));

    assert_eq!(
        chord_fillet_radius(&block, &[], 1.0),
        Err(ChordError::NoEdges)
    );
}
