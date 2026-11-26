use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    Drag, EntityId, GearPose, Sketch,
    gear::{ChainSize, Gear, GearCentre, GearPair, SpurGear},
    text::{Text, TextAlign, TextSize},
};

const HEIGHT: ParameterId = ParameterId::from_raw(4);
const CLOSE: f64 = 1e-9;

fn values(id: ParameterId) -> Result<Quantity, EvalError> {
    if id == HEIGHT {
        Ok(Quantity::length(5.0))
    } else {
        Err(EvalError::ParameterMissing)
    }
}

fn text(anchor: EntityId, height: Expression) -> Text {
    Text {
        anchor,
        content: "Vent".to_owned(),
        height,
        angle: Expression::measure(10.0, Unit::Degree),
        bold: false,
        italic: false,
        align: TextAlign::Left,
    }
}

fn spur(centre: EntityId) -> Gear {
    Gear {
        centre,
        module: Expression::measure(2.0, Unit::Millimetre),
        teeth: Expression::number(20.0),
        pressure_angle: Expression::measure(20.0, Unit::Degree),
        profile_shift: Expression::number(0.0),
        root_fillet: Expression::measure(0.5, Unit::Millimetre),
        bore: Expression::measure(6.0, Unit::Millimetre),
        turn: Expression::measure(0.0, Unit::Degree),
        meshing: None,
        chain: None,
    }
}

fn solved(sketch: &Sketch, drags: &[Drag]) -> Sketch {
    let solved = sketch.solve(&values, &|| false).unwrap();

    solved
        .geometry
        .solve_geometry_from(&values, &|| false, drags, None)
        .unwrap()
        .0
}

fn turned_by(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

fn measure(expression: &Expression) -> f64 {
    expression.evaluate(&values).unwrap().value
}

#[test]
fn a_text_turns_and_scales_where_a_number_sets_it_and_keeps_what_a_parameter_sets() {
    let mut sketch = Sketch::new(Plane::XY);

    let free_anchor = sketch.add_point(Point2::new(0.0, 0.0));
    let driven_anchor = sketch.add_point(Point2::new(0.0, 20.0));
    let free = sketch
        .add_text(text(
            free_anchor,
            Expression::measure(0.5, Unit::Centimetre),
        ))
        .unwrap();
    let driven = sketch
        .add_text(text(driven_anchor, Expression::Parameter(HEIGHT)))
        .unwrap();

    let wanted = TextSize {
        height: 10.0,
        angle: 100.0,
    };
    let posed = solved(
        &sketch,
        &[
            Drag::Text {
                text: free,
                to: wanted,
            },
            Drag::Text {
                text: driven,
                to: wanted,
            },
        ],
    );

    let free_text = posed.text(free).unwrap();
    let driven_text = posed.text(driven).unwrap();

    assert_eq!(free_text.height, Expression::measure(1.0, Unit::Centimetre));
    assert!((measure(&free_text.angle) - 100.0).abs() < CLOSE);
    assert_eq!(posed.text_size(free), Some(wanted));
    assert_eq!(driven_text.height, Expression::Parameter(HEIGHT));
    assert!((measure(&driven_text.angle) - 100.0).abs() < CLOSE);
    assert_eq!(
        posed.text_size(driven),
        Some(TextSize {
            height: 5.0,
            angle: 100.0
        })
    );
}

#[test]
fn a_gear_turns_and_scales_its_module_root_fillet_and_bore_together() {
    let mut sketch = Sketch::new(Plane::XY);

    let centre = sketch.add_point(Point2::new(30.0, 0.0));
    let gear = sketch.add_parametric_gear(spur(centre)).unwrap();

    let posed = solved(
        &sketch,
        &[Drag::Gear {
            gear,
            to: GearPose {
                pitch_radius: 40.0,
                turn: 90.0,
            },
        }],
    );

    let held = posed.gear(gear).unwrap();
    let drawn = posed.gear_values(gear).unwrap();

    assert!((measure(&held.module) - 4.0).abs() < CLOSE);
    assert!((measure(&held.root_fillet) - 1.0).abs() < CLOSE);
    assert!((measure(&held.bore) - 12.0).abs() < CLOSE);
    assert!((measure(&held.turn) - 90.0).abs() < CLOSE);
    assert!((drawn.shape.pitch_radius() - 40.0).abs() < CLOSE);
    assert!((drawn.turn - 90.0).abs() < CLOSE);
}

#[test]
fn a_sprocket_turns_but_keeps_the_size_its_chain_sets() {
    let mut sketch = Sketch::new(Plane::XY);

    let centre = sketch.add_point(Point2::new(0.0, 0.0));
    let sprocket = sketch
        .add_parametric_gear(Gear::sprocket(
            centre,
            ChainSize::DEFAULT,
            Expression::number(18.0),
            Expression::measure(8.0, Unit::Millimetre),
            Expression::measure(0.0, Unit::Degree),
        ))
        .unwrap();
    let before = sketch.gear_values(sprocket).unwrap().shape.pitch_radius();

    let posed = solved(
        &sketch,
        &[Drag::Gear {
            gear: sprocket,
            to: GearPose {
                pitch_radius: before * 2.0,
                turn: -45.0,
            },
        }],
    );

    let drawn = posed.gear_values(sprocket).unwrap();

    assert!((drawn.shape.pitch_radius() - before).abs() < CLOSE);
    assert!((drawn.shape.bore() - 8.0).abs() < CLOSE);
    assert!((drawn.turn + 45.0).abs() < CLOSE);
}

#[test]
fn a_meshing_pair_turned_together_keeps_meshing_and_turns_its_direction() {
    let mut sketch = Sketch::new(Plane::XY);

    let direction = 0.0;
    let pair = GearPair {
        driver: SpurGear {
            module: 2.0,
            teeth: 20,
            pressure_angle: 20.0,
            profile_shift: 0.0,
            root_fillet: 0.5,
            bore: 6.0,
        },
        partner_teeth: 30,
        partner_bore: 5.0,
        direction,
    };

    let (driver, partner) = sketch
        .add_stored_pair(
            GearCentre::Free(Point2::ZERO),
            spur(EntityId::ORIGIN),
            Gear::meshed(
                EntityId::ORIGIN,
                EntityId::ORIGIN,
                Expression::number(30.0),
                Expression::measure(5.0, Unit::Millimetre),
                Expression::measure(direction, Unit::Degree),
            ),
            &pair,
        )
        .unwrap();
    let driver_centre = sketch.gear(driver).unwrap().centre;
    let partner_centre = sketch.gear(partner).unwrap().centre;
    let partner_before = sketch.gear_values(partner).unwrap();
    let distance = pair.centre_distance();

    let pose = |gear: EntityId| {
        let values = sketch.gear_values(gear).unwrap();
        Drag::Gear {
            gear,
            to: GearPose {
                pitch_radius: values.shape.pitch_radius() * 2.0,
                turn: values.turn + 90.0,
            },
        }
    };
    let posed = solved(
        &sketch,
        &[
            Drag::Point {
                point: driver_centre,
                to: Point2::ZERO,
            },
            Drag::Point {
                point: partner_centre,
                to: Point2::new(0.0, distance * 2.0),
            },
            pose(driver),
            pose(partner),
        ],
    );

    let held = posed.gear(partner).unwrap();
    let drawn = posed.gear_values(partner).unwrap();

    assert!((measure(&held.meshing.as_ref().unwrap().direction) - 90.0).abs() < CLOSE);
    assert!((measure(&held.bore) - 10.0).abs() < CLOSE);
    assert!((drawn.shape.pitch_radius() - partner_before.shape.pitch_radius() * 2.0).abs() < CLOSE);
    assert!((turned_by(partner_before.turn, drawn.turn) - 90.0).abs() < 1e-6);
    assert!(
        posed
            .point(partner_centre)
            .unwrap()
            .distance(Point2::new(0.0, distance * 2.0))
            < 1e-6
    );
}
