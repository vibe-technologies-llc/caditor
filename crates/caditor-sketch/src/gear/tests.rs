use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2};

use super::{FLANK_TOLERANCE, GearCentre, GearError, GearOutline, GearPiece, SpurGear};
use crate::{Entity, EntityId, Sketch};

const CHECKS_PER_FLANK: usize = 200;

fn gear(teeth: u32) -> SpurGear {
    SpurGear {
        module: 2.0,
        teeth,
        pressure_angle: 20.0,
        profile_shift: 0.0,
        root_fillet: 0.76,
        bore: 8.0,
    }
}

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn involute_function(radius: f64, base: f64) -> f64 {
    let angle = (base / radius).acos();
    angle.tan() - angle
}

fn worst_flank_error(gear: &SpurGear, outline: &GearOutline) -> f64 {
    let base = outline.circles.base;
    let step = std::f64::consts::TAU / f64::from(gear.teeth);
    let pressure = gear.pressure_angle.to_radians();
    let half = std::f64::consts::PI / (2.0 * f64::from(gear.teeth))
        + 2.0 * gear.profile_shift * pressure.tan() / f64::from(gear.teeth)
        + pressure.tan()
        - pressure;
    let mut worst: f64 = 0.0;
    for piece in &outline.pieces {
        let Some(spline) = piece.spline() else {
            continue;
        };
        let middle = spline.point_at(0.5) - outline.centre;
        let tooth = (middle.y.atan2(middle.x) / step).round() * step;
        let side = if middle.y.atan2(middle.x) > tooth {
            1.0
        } else {
            -1.0
        };
        for index in 0..=CHECKS_PER_FLANK {
            let point = spline.point_at(index as f64 / CHECKS_PER_FLANK as f64) - outline.centre;
            let radius = point.length();
            assert!(radius >= base * (1.0 - 1e-9), "below the base circle");
            let wanted = tooth + side * (half - involute_function(radius, base));
            let turn = point.y.atan2(point.x) - wanted;
            let turn = turn - (turn / std::f64::consts::TAU).round() * std::f64::consts::TAU;
            worst = worst.max(radius * turn.abs());
        }
    }
    worst
}

fn assert_closed(outline: &GearOutline) {
    let pieces = &outline.pieces;
    for pair in pieces.windows(2) {
        assert_eq!(pair[0].to(), pair[1].from());
    }
    assert_eq!(
        pieces.last().map(GearPiece::to),
        pieces.first().map(GearPiece::from)
    );
}

#[test]
fn flanks_follow_the_involute_within_the_tolerance_above_and_below_the_base_circle() {
    for teeth in [20, 60] {
        let gear = gear(teeth);
        let outline = gear.outline(Point2::new(5.0, -3.0)).unwrap();
        let error = worst_flank_error(&gear, &outline);
        assert!(
            error <= 1.5 * FLANK_TOLERANCE * gear.module,
            "{teeth} teeth: {error}"
        );
        assert!(outline.deviation <= FLANK_TOLERANCE * gear.module);
        let flanks = outline
            .pieces
            .iter()
            .filter(|piece| matches!(piece, GearPiece::Flank { .. }))
            .count();
        assert_eq!(flanks, 2 * teeth as usize);
    }
}

#[test]
fn the_circles_follow_from_module_teeth_pressure_angle_and_shift() {
    let shifted = SpurGear {
        profile_shift: 0.5,
        ..gear(20)
    };
    let circles = shifted.circles();
    assert!((circles.pitch - 20.0).abs() < 1e-12);
    assert!((circles.base - 20.0 * 20f64.to_radians().cos()).abs() < 1e-12);
    assert!((circles.tip - 23.0).abs() < 1e-12);
    assert!((circles.root - 18.5).abs() < 1e-12);
}

#[test]
fn the_outline_is_one_closed_loop_and_draws_with_no_open_end() {
    for teeth in [20, 60] {
        let gear = gear(teeth);
        let outline = gear.outline(Point2::ZERO).unwrap();
        assert_closed(&outline);

        let mut sketch = Sketch::new(Plane::XY);
        let drawn = sketch
            .add_gear(&gear, GearCentre::Point(EntityId::ORIGIN))
            .unwrap();

        assert!(sketch.open_ends().is_empty());
        assert_eq!(drawn.outline.len(), outline.pieces.len());
        assert!(
            drawn
                .circles
                .iter()
                .all(|circle| sketch.is_construction(*circle))
        );
        assert!(drawn.bore.is_some_and(|bore| !sketch.is_construction(bore)));
        assert_eq!(sketch.point(drawn.centre), Some(Point2::ZERO));
        assert_eq!(sketch.constraints().len(), 1);
    }
}

#[test]
fn a_drawn_gear_solves_where_it_was_drawn() {
    let gear = gear(24);
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(40.0, 10.0));

    let drawn = sketch.add_gear(&gear, GearCentre::Point(centre)).unwrap();
    let solved = sketch.solve(&no_parameters, &|| false).unwrap();

    assert_eq!(drawn.centre, centre);
    for (id, entity) in sketch.entities() {
        if let Entity::Point(at) = entity {
            let moved = solved.geometry.point(id).unwrap();
            assert!(moved.distance(*at) < 1e-6, "{id} moved");
        }
    }
}

#[test]
fn too_few_teeth_for_the_shift_are_refused_saying_what_would_do() {
    let error = gear(12).outline(Point2::ZERO).unwrap_err();

    assert_eq!(
        error,
        GearError::Undercut {
            teeth: 12,
            pressure_angle: 20.0,
            shift: 0.0,
            least_teeth: 17,
            least_shift: 0.29,
        }
    );
    assert_eq!(
        error.to_string(),
        "12 teeth are undercut at a pressure angle of 20° with no profile shift: use at least 17 \
         teeth, or a profile shift of at least 0.29"
    );
    assert!(gear(17).outline(Point2::ZERO).is_ok());
    let shifted = SpurGear {
        profile_shift: 0.29,
        ..gear(12)
    };
    assert!(shifted.outline(Point2::ZERO).is_ok());
}

#[test]
fn pointed_teeth_a_root_through_the_centre_and_bad_numbers_are_refused() {
    let pointed = SpurGear {
        profile_shift: 1.5,
        ..gear(20)
    };
    let through = SpurGear {
        profile_shift: -9.0,
        ..gear(20)
    };

    assert_eq!(pointed.outline(Point2::ZERO), Err(GearError::PointedTeeth));
    assert_eq!(
        through.outline(Point2::ZERO),
        Err(GearError::RootThroughCentre)
    );
    for (changed, error) in [
        (
            SpurGear {
                module: 0.0,
                ..gear(20)
            },
            GearError::ModuleNotPositive,
        ),
        (gear(2), GearError::TeethOutOfRange { teeth: 2 }),
        (
            SpurGear {
                pressure_angle: 45.0,
                ..gear(20)
            },
            GearError::PressureAngleOutOfRange,
        ),
        (
            SpurGear {
                root_fillet: -1.0,
                ..gear(20)
            },
            GearError::RootFilletNegative,
        ),
    ] {
        assert_eq!(changed.outline(Point2::ZERO), Err(error));
    }
}

#[test]
fn a_root_fillet_or_bore_too_large_is_refused_with_the_largest_that_fits() {
    let filleted = SpurGear {
        root_fillet: 5.0,
        ..gear(20)
    };
    let bored = SpurGear {
        bore: 40.0,
        ..gear(20)
    };

    let Err(GearError::RootFilletTooLarge { largest, .. }) = filleted.outline(Point2::ZERO) else {
        panic!("a 5 mm fillet fits");
    };
    assert!(largest > 0.76 && largest < 5.0);
    let fitting = SpurGear {
        root_fillet: largest * 0.999,
        ..gear(20)
    };
    assert_closed(&fitting.outline(Point2::ZERO).unwrap());
    assert_eq!(
        bored.outline(Point2::ZERO),
        Err(GearError::BoreTooLarge {
            bore: 40.0,
            largest: 33.0,
        })
    );
}

#[test]
fn a_sharp_root_draws_lines_down_to_the_root_circle() {
    let sharp = SpurGear {
        root_fillet: 0.0,
        ..gear(20)
    };

    let outline = sharp.outline(Point2::ZERO).unwrap();

    assert_closed(&outline);
    let root = outline.circles.root;
    let lines: Vec<&GearPiece> = outline
        .pieces
        .iter()
        .filter(|piece| matches!(piece, GearPiece::Line { .. }))
        .collect();
    assert_eq!(lines.len(), 40);
    assert!(lines.iter().all(|line| {
        let ends = [line.from().length(), line.to().length()];
        ends.iter().any(|radius| (radius - root).abs() < 1e-9)
    }));
}
