use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use super::GearPair;
use crate::{
    Constraint, EntityId, Sketch, SketchError,
    curve::Faceting,
    gear::{Gear, GearCentre, GearError, GearOutline, GearValueError, SpurGear},
};

const MODULE: ParameterId = ParameterId::from_raw(3);

fn module_of(millimetres: f64) -> impl Fn(ParameterId) -> Result<Quantity, EvalError> {
    move |id| {
        if id == MODULE {
            Ok(Quantity::length(millimetres))
        } else {
            Err(EvalError::ParameterMissing)
        }
    }
}

fn driver(centre: EntityId) -> Gear {
    Gear {
        centre,
        module: Expression::Parameter(MODULE),
        teeth: Expression::number(20.0),
        pressure_angle: Expression::measure(20.0, Unit::Degree),
        profile_shift: Expression::number(0.3),
        root_fillet: Expression::measure(0.5, Unit::Millimetre),
        bore: Expression::measure(0.0, Unit::Millimetre),
        turn: Expression::measure(7.0, Unit::Degree),
        meshing: None,
        chain: None,
    }
}

fn partner(driver: EntityId, teeth: f64, direction: f64) -> Gear {
    Gear::meshed(
        EntityId::ORIGIN,
        driver,
        Expression::number(teeth),
        Expression::measure(5.0, Unit::Millimetre),
        Expression::measure(direction, Unit::Degree),
    )
}

fn placing(teeth: u32, partner_teeth: u32, direction: f64) -> GearPair {
    GearPair {
        driver: SpurGear {
            module: 2.0,
            teeth,
            pressure_angle: 20.0,
            profile_shift: 0.3,
            root_fillet: 0.5,
            bore: 0.0,
        },
        partner_teeth,
        partner_bore: 5.0,
        direction: direction.to_radians(),
    }
}

fn pair_sketch(direction: f64) -> (Sketch, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let (first, second) = sketch
        .add_stored_pair(
            GearCentre::Free(Point2::new(4.0, -3.0)),
            driver(EntityId::ORIGIN),
            partner(EntityId::ORIGIN, 30.0, direction),
            &placing(20, 30, direction),
        )
        .unwrap();
    (sketch, first, second)
}

fn polygon(outline: &GearOutline) -> Vec<Point2> {
    outline
        .pieces
        .iter()
        .flat_map(|piece| {
            let mut points = piece.faceted(Faceting::within(2e-3));
            points.pop();
            points
        })
        .collect()
}

fn inside(polygon: &[Point2], point: Point2) -> bool {
    let mut inside = false;
    for (a, b) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }
    inside
}

fn depth_inside(polygon: &[Point2], point: Point2) -> f64 {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| {
            let along = *b - *a;
            let share = ((point - *a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(*a + along * share)
        })
        .fold(f64::INFINITY, f64::min)
}

fn deepest_overlap(first: &[Point2], second: &[Point2]) -> f64 {
    first
        .iter()
        .filter(|point| inside(second, **point))
        .map(|point| depth_inside(second, *point))
        .fold(0.0, f64::max)
}

#[test]
fn a_stored_pair_places_the_partner_at_the_standard_distance_along_its_direction() {
    let (sketch, first, second) = pair_sketch(30.0);
    let solved = sketch.solve(&module_of(2.0), &|| false).unwrap().geometry;

    let driver = solved.gear_outline(first).unwrap();
    let partner = solved.gear_outline(second).unwrap();
    let apart = partner.centre - driver.centre;

    assert_eq!(driver.centre, Point2::new(4.0, -3.0));
    assert!((apart.length() - 50.0).abs() < 1e-9);
    assert!((apart.y.atan2(apart.x).to_degrees() - 30.0).abs() < 1e-9);
    assert_eq!(sketch.partners_of(first), vec![second]);
    assert_eq!(sketch.gear(second).unwrap().driver(), Some(first));
}

#[test]
fn the_partner_follows_the_drivers_module_shift_and_turn_by_the_tooth_ratio() {
    let (sketch, first, second) = pair_sketch(0.0);
    let solved = sketch.solve(&module_of(2.5), &|| false).unwrap().geometry;

    let driver = solved.gear_values(first).unwrap();
    let partner = solved.gear_values(second).unwrap();
    let pair = GearPair {
        driver: *driver.shape.spur().unwrap(),
        partner_teeth: 30,
        partner_bore: 5.0,
        direction: 0.0,
    };

    assert_eq!(partner.shape.spur().unwrap().module, 2.5);
    assert_eq!(partner.shape.spur().unwrap().pressure_angle, 20.0);
    assert_eq!(partner.shape.spur().unwrap().root_fillet, 0.5);
    assert_eq!(partner.shape.spur().unwrap().profile_shift, -0.3);
    assert_eq!(partner.shape.spur().unwrap().teeth, 30);
    assert_eq!(partner.shape.spur().unwrap().bore, 5.0);
    assert!((partner.turn.to_radians() - pair.partner_turn(7.0_f64.to_radians())).abs() < 1e-12);
}

#[test]
fn editing_the_module_moves_the_partner_to_the_new_distance() {
    let (sketch, first, second) = pair_sketch(0.0);

    let small = sketch.solve(&module_of(2.0), &|| false).unwrap().geometry;
    let large = sketch.solve(&module_of(3.0), &|| false).unwrap().geometry;
    let gap = |solved: &Sketch| {
        solved.point(solved.gear(second).unwrap().centre).unwrap()
            - solved.point(solved.gear(first).unwrap().centre).unwrap()
    };

    assert!((gap(&small).x - 50.0).abs() < 1e-9);
    assert!((gap(&large).x - 75.0).abs() < 1e-9);
    assert!(gap(&large).y.abs() < 1e-9);
}

#[test]
fn the_partner_goes_with_a_driver_the_user_constrains() {
    let (mut sketch, first, second) = pair_sketch(90.0);
    let centre = sketch.gear(first).unwrap().centre;
    sketch
        .add_constraint(Constraint::Fix {
            point: centre,
            at: Point2::new(-10.0, 6.0),
        })
        .unwrap();

    let solved = sketch.solve(&module_of(2.0), &|| false).unwrap();
    let partner = solved
        .geometry
        .point(solved.geometry.gear(second).unwrap().centre)
        .unwrap();

    assert!(partner.distance(Point2::new(-10.0, 56.0)) < 1e-9);
    assert_eq!(solved.solution.degrees_of_freedom(), 0);
}

#[test]
fn a_pair_with_a_free_driver_leaves_only_the_drivers_two_freedoms() {
    let (sketch, _, _) = pair_sketch(0.0);

    let solved = sketch.solve(&module_of(2.0), &|| false).unwrap();

    assert_eq!(solved.solution.degrees_of_freedom(), 2);
}

#[test]
fn a_constraint_fighting_the_mesh_is_named_as_the_conflict() {
    let (mut sketch, first, second) = pair_sketch(0.0);
    let driver = sketch.gear(first).unwrap().centre;
    let partner = sketch.gear(second).unwrap().centre;
    sketch
        .add_constraint(Constraint::Fix {
            point: driver,
            at: Point2::ZERO,
        })
        .unwrap();
    let fix = sketch
        .add_constraint(Constraint::Fix {
            point: partner,
            at: Point2::new(10.0, 10.0),
        })
        .unwrap();

    let refused = sketch.solve(&module_of(2.0), &|| false);

    assert!(matches!(
        refused,
        Err(SketchError::Conflict { constraints }) if constraints.contains(&fix)
    ));
}

#[test]
fn the_meshed_teeth_touch_without_overlapping_in_a_direction_off_the_axes() {
    for direction in [0.0, 37.0, 135.0, -80.0] {
        let (sketch, first, second) = pair_sketch(direction);
        let solved = sketch.solve(&module_of(2.0), &|| false).unwrap().geometry;

        let driver = polygon(&solved.gear_outline(first).unwrap());
        let partner = polygon(&solved.gear_outline(second).unwrap());

        assert!(
            deepest_overlap(&driver, &partner) < 5e-3,
            "{direction}: {}",
            deepest_overlap(&driver, &partner)
        );
        assert!(
            deepest_overlap(&partner, &driver) < 5e-3,
            "{direction}: {}",
            deepest_overlap(&partner, &driver)
        );
    }
}

#[test]
fn a_partner_that_cannot_be_cut_fails_the_sketch_naming_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let first = sketch.add_parametric_gear(driver(centre)).unwrap();
    let other = sketch.add_point(Point2::new(60.0, 0.0));
    let second = sketch
        .add_parametric_gear(Gear {
            centre: other,
            ..partner(first, 9.0, 0.0)
        })
        .unwrap();

    let refused = sketch.solve(&module_of(2.0), &|| false);

    let Err(SketchError::Gear { gear, reason, .. }) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(gear, second);
    assert!(matches!(
        reason,
        GearValueError::Build(error) if matches!(*error, GearError::Partner(_))
    ));
}

#[test]
fn a_driver_cannot_go_while_its_partner_stands_and_the_link_cannot_be_rewired() {
    let (mut sketch, first, second) = pair_sketch(0.0);
    let held = sketch.gear(second).unwrap().clone();

    let removed = sketch.remove_gear(first);
    let rewired = sketch.replace_gear(
        second,
        Gear {
            meshing: None,
            chain: None,
            ..held.clone()
        },
    );
    let spare = sketch.add_point(Point2::new(-60.0, 0.0));
    let chained = sketch.add_parametric_gear(Gear {
        centre: spare,
        ..partner(second, 25.0, 0.0)
    });

    assert!(matches!(removed, Err(SketchError::InUse { entity, .. }) if entity == first));
    assert!(matches!(rewired, Err(SketchError::ChangesStructure { .. })));
    assert!(matches!(chained, Err(SketchError::WrongKind { .. })));
    assert!(sketch.remove_gear(second).is_ok());
    assert!(sketch.remove_gear(first).is_ok());
}

#[test]
fn a_partner_needs_a_driver_that_exists_and_a_centre_of_its_own() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let first = sketch.add_parametric_gear(driver(centre)).unwrap();

    let spare = sketch.add_point(Point2::new(-60.0, 0.0));
    let nobody = sketch.add_parametric_gear(Gear {
        centre: spare,
        ..partner(EntityId::from_raw(99), 25.0, 0.0)
    });
    let same_centre = sketch.add_parametric_gear(Gear {
        centre,
        ..partner(first, 25.0, 0.0)
    });

    assert_eq!(
        nobody,
        Err(SketchError::MissingEntity(EntityId::from_raw(99)))
    );
    assert!(matches!(same_centre, Err(SketchError::SameEntity { .. })));
}

#[test]
fn a_copied_partner_is_left_out_of_the_clipboard() {
    let (sketch, first, second) = pair_sketch(0.0);

    let only_partner = sketch.clip(&[second]);
    let driver_alone = sketch.clip(&[first, second]).unwrap();

    assert!(only_partner.is_err());
    assert_eq!(driver_alone.gear_count(), 1);
}

#[test]
fn a_partner_counts_its_own_expressions_as_the_parameters_it_uses() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let first = sketch.add_parametric_gear(driver(centre)).unwrap();
    let other = sketch.add_point(Point2::new(60.0, 0.0));
    let teeth = ParameterId::from_raw(9);
    sketch
        .add_parametric_gear(Gear {
            centre: other,
            teeth: Expression::Parameter(teeth),
            ..partner(first, 30.0, 0.0)
        })
        .unwrap();

    assert!(sketch.uses_parameter(teeth));
    assert!(sketch.uses_parameter(MODULE));
    assert!(sketch.parameters().contains(&teeth));
}

#[test]
fn a_remembered_solve_is_not_reused_once_the_module_has_changed() {
    let (sketch, first, second) = pair_sketch(0.0);
    let remembered = sketch.solve(&module_of(2.0), &|| false).unwrap().memo;

    let solved = sketch
        .solve_from(&module_of(3.0), &|| false, &[], Some(&remembered))
        .unwrap();
    let fresh = sketch.solve(&module_of(3.0), &|| false).unwrap();
    let at =
        |solved: &Sketch, gear: EntityId| solved.point(solved.gear(gear).unwrap().centre).unwrap();

    assert!(at(&solved.geometry, second).distance(at(&fresh.geometry, second)) < 1e-9);
    assert!(
        (at(&solved.geometry, second).distance(at(&solved.geometry, first)) - 75.0).abs() < 1e-9
    );
}
