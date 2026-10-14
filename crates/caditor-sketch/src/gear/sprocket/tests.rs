use std::f64::consts::PI;

use caditor_geometry::{Plane, Point2};

use super::{ChainSize, MAX_SPROCKET_TEETH, MIN_SPROCKET_TEETH, Sprocket};
use crate::{
    EntityId, Sketch,
    curve::Faceting,
    gear::{GearCentre, GearError, GearPiece},
};

const TEETH: [u32; 12] = [9, 10, 11, 12, 15, 17, 19, 25, 38, 57, 95, 150];

fn sprocket(designation: &str, teeth: u32) -> Sprocket {
    Sprocket {
        chain: ChainSize::find(designation).unwrap(),
        teeth,
        bore: 0.0,
    }
}

fn polygon(sprocket: &Sprocket) -> Vec<Point2> {
    sprocket
        .outline(Point2::ZERO)
        .unwrap()
        .pieces
        .iter()
        .flat_map(|piece| {
            let mut points = piece.faceted(Faceting::within(1e-3));
            points.pop();
            points
        })
        .collect()
}

fn crosses(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let side = |from: Point2, to: Point2, point: Point2| (to - from).perp_dot(point - from);
    side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
}

#[test]
fn the_pitch_circle_passes_through_the_roller_centres_of_a_chain_of_that_pitch() {
    for size in ChainSize::ALL {
        for teeth in TEETH {
            let sprocket = Sprocket {
                chain: size,
                teeth,
                bore: 0.0,
            };
            let diameter = 2.0 * sprocket.circles().pitch;
            let wanted = size.pitch / (PI / f64::from(teeth)).sin();
            assert!(
                (diameter - wanted).abs() < 1e-9 * wanted,
                "{size:?} {teeth}"
            );
        }
    }
}

#[test]
fn every_size_and_tooth_count_gives_a_closed_outline_inside_the_iso_606_limits() {
    for size in ChainSize::ALL {
        for teeth in TEETH {
            let sprocket = Sprocket {
                chain: size,
                teeth,
                bore: 0.0,
            };
            let outline = sprocket
                .outline(Point2::new(3.0, -2.0))
                .unwrap_or_else(|error| panic!("{} z{teeth}: {error}", size.designation));
            let count = f64::from(teeth);
            let roller = size.roller;
            let pitch_diameter = 2.0 * sprocket.circles().pitch;

            let seat = size.seat_radius();
            assert!(seat >= 0.505 * roller - 1e-12);
            assert!(seat <= 0.505 * roller + 0.069 * roller.cbrt() + 1e-12);

            let angle = sprocket.seating_angle().to_degrees();
            assert!(angle >= 120.0 - 90.0 / count - 1e-9);
            assert!(angle <= 140.0 - 90.0 / count + 1e-9);

            let flank = sprocket.flank_radius();
            assert!(flank >= 0.12 * roller * (count + 2.0) - 1e-9);
            assert!(flank <= 0.008 * roller * (count * count + 180.0) + 1e-9);

            let tip_diameter = 2.0 * sprocket.circles().tip;
            assert!(
                tip_diameter >= pitch_diameter + size.pitch * (1.0 - 1.6 / count) - roller - 1e-9
            );
            assert!(tip_diameter <= pitch_diameter + 1.25 * size.pitch - roller + 1e-9);

            for pair in outline.pieces.windows(2) {
                assert!(pair[0].to().distance(pair[1].from()) < 1e-9);
            }
            let last = outline.pieces.last().unwrap();
            assert!(last.to().distance(outline.pieces[0].from()) < 1e-9);
            assert_eq!(outline.pieces.len(), 4 * teeth as usize);
        }
    }
}

#[test]
fn the_seat_of_each_gap_holds_a_roller_of_the_chain_and_neighbours_are_a_pitch_apart() {
    let sprocket = sprocket("08B", 19);
    let outline = sprocket.outline(Point2::new(10.0, 5.0)).unwrap();
    let seats: Vec<(Point2, Point2, Point2)> = outline
        .pieces
        .iter()
        .filter_map(|piece| match piece {
            GearPiece::Arc {
                center,
                about_gear: false,
                from,
                to,
                ..
            } if (center.distance(*from) - sprocket.chain.seat_radius()).abs() < 1e-9 => {
                Some((*center, *from, *to))
            }
            _ => None,
        })
        .collect();

    assert_eq!(seats.len(), 19);
    for (centre, from, to) in &seats {
        let roller = sprocket.chain.roller / 2.0;
        assert!(centre.distance(*from) > roller);
        assert!(centre.distance(*to) > roller);
        assert!((centre.distance(outline.centre) - sprocket.circles().pitch).abs() < 1e-9);
    }
    for pair in seats.iter().zip(seats.iter().cycle().skip(1)) {
        assert!((pair.0.0.distance(pair.1.0) - sprocket.chain.pitch).abs() < 1e-9);
    }
}

#[test]
fn the_tips_lie_on_the_tip_circle_and_the_teeth_keep_a_land() {
    let sprocket = sprocket("12B", 17);
    let outline = sprocket.outline(Point2::ZERO).unwrap();

    let tips: Vec<&GearPiece> = outline
        .pieces
        .iter()
        .filter(|piece| {
            matches!(
                piece,
                GearPiece::Arc {
                    about_gear: true,
                    ..
                }
            )
        })
        .collect();

    assert_eq!(tips.len(), 17);
    for tip in tips {
        let (from, to) = (tip.from(), tip.to());
        assert!((from.length() - outline.circles.tip).abs() < 1e-9);
        assert!((to.length() - outline.circles.tip).abs() < 1e-9);
        assert!(from.distance(to) > 0.02 * sprocket.chain.pitch);
    }
}

#[test]
fn the_outline_never_crosses_itself() {
    for (designation, teeth) in [
        ("08B", 9),
        ("08B", 12),
        ("16A", 19),
        ("48B", 11),
        ("06B", 40),
    ] {
        let points = polygon(&sprocket(designation, teeth));
        let count = points.len();
        for first in 0..count {
            for second in first + 2..count {
                if first == 0 && second == count - 1 {
                    continue;
                }
                assert!(
                    !crosses(
                        points[first],
                        points[(first + 1) % count],
                        points[second],
                        points[(second + 1) % count]
                    ),
                    "{designation} z{teeth}: {first} crosses {second}"
                );
            }
        }
    }
}

#[test]
fn a_drawn_sprocket_has_no_open_end_and_three_construction_circles_and_a_bore() {
    let sprocket = Sprocket {
        bore: 10.0,
        ..sprocket("10B", 21)
    };
    let mut sketch = Sketch::new(Plane::XY);

    let drawn = sketch
        .add_sprocket(&sprocket, GearCentre::Point(EntityId::ORIGIN))
        .unwrap();

    assert!(sketch.open_ends().is_empty());
    assert_eq!(drawn.outline.len(), 4 * 21);
    assert_eq!(drawn.circles.len(), 3);
    assert!(
        drawn
            .circles
            .iter()
            .all(|circle| sketch.is_construction(*circle))
    );
    assert!(drawn.bore.is_some_and(|bore| !sketch.is_construction(bore)));
    assert_eq!(sketch.point(drawn.centre), Some(Point2::ZERO));
}

#[test]
fn values_out_of_range_are_refused_in_words() {
    let few = sprocket("08B", MIN_SPROCKET_TEETH - 1)
        .outline(Point2::ZERO)
        .unwrap_err();
    let many = sprocket("08B", MAX_SPROCKET_TEETH + 1)
        .outline(Point2::ZERO)
        .unwrap_err();
    let bored = Sprocket {
        bore: 500.0,
        ..sprocket("08B", 19)
    }
    .outline(Point2::ZERO)
    .unwrap_err();
    let rollers = Sprocket {
        chain: ChainSize {
            designation: "x",
            pitch: 10.0,
            roller: 12.0,
        },
        ..sprocket("08B", 19)
    }
    .outline(Point2::ZERO)
    .unwrap_err();

    assert_eq!(few, GearError::SprocketTeethOutOfRange { teeth: 8 });
    assert_eq!(few.to_string(), "a sprocket has 9 to 150 teeth, not 8");
    assert_eq!(many, GearError::SprocketTeethOutOfRange { teeth: 151 });
    assert!(matches!(bored, GearError::BoreTooLarge { .. }));
    assert_eq!(rollers, GearError::ChainNotPositive);
}

#[test]
fn a_roller_seated_in_any_gap_touches_no_tooth() {
    for (designation, teeth) in [
        ("08B", 9),
        ("08B", 19),
        ("16A", 12),
        ("48B", 40),
        ("05B", 150),
    ] {
        let sprocket = sprocket(designation, teeth);
        let seated = Point2::new(sprocket.circles().pitch, 0.0);
        let points = polygon(&sprocket);

        let nearest = points
            .iter()
            .map(|point| point.distance(seated))
            .fold(f64::INFINITY, f64::min);

        assert!(
            nearest >= sprocket.chain.roller / 2.0 - 1e-6,
            "{designation} z{teeth}: {nearest}"
        );
        assert!(nearest < sprocket.chain.seat_radius() + 1e-3);
    }
}

#[test]
fn chain_sizes_are_found_by_designation() {
    assert_eq!(ChainSize::find(" 08b ").map(|size| size.pitch), Some(12.7));
    assert_eq!(ChainSize::find("nope"), None);
    assert!(ChainSize::ALL.iter().all(|size| size.roller < size.pitch));
}
