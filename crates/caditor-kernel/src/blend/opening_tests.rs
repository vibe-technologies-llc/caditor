use std::f64::consts::TAU;

use super::{
    tests::{SPANDREL_CENTROID, block_at, edge_through, fillet, run, spandrel, volume},
    *,
};
use crate::test_support::assert_watertight;

const SIDE: f64 = 166.266_667;
const PLATE: f64 = 3.0;
const CORNER: f64 = 12.0;
const ROUND: f64 = 1.5;

fn plate_with_rounded_opening() -> Solid {
    let middle = SIDE / 2.0;
    let plate = block_at((0.0, 0.0, 0.0), (SIDE, SIDE, PLATE), 1);
    let opening = block_at(
        (middle - 55.0, middle - 60.0, -10.0),
        (110.0, 120.0, 20.0),
        2,
    );
    let opened = boolean(&plate, &opening, BooleanOperation::Difference).unwrap();
    let corners: Vec<EdgeId> = [(-55.0, -60.0), (55.0, -60.0), (55.0, 60.0), (-55.0, 60.0)]
        .into_iter()
        .map(|(x, y)| edge_through(&opened, (middle + x, middle + y, PLATE / 2.0)))
        .collect();
    run(&opened, &corners, fillet(CORNER))
}

fn opening_rim_on_the_bottom(solid: &Solid) -> Vec<EdgeId> {
    solid
        .edges()
        .filter(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            middle.z.abs() < 1e-9
                && (middle.x - SIDE / 2.0).abs() < 60.0
                && (middle.y - SIDE / 2.0).abs() < 65.0
        })
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_small_fillet_round_a_rounded_rectangle_opening_in_a_plate_is_rounded() {
    let body = plate_with_rounded_opening();

    let rim = opening_rim_on_the_bottom(&body);
    let before = volume(&body);
    let rounded = run(&body, &rim, fillet(ROUND));
    let removed = before - volume(&rounded);
    let straight = 2.0 * (110.0 - 2.0 * CORNER) + 2.0 * (120.0 - 2.0 * CORNER);
    let round = TAU * (CORNER + SPANDREL_CENTROID * ROUND);
    let expected = (straight + round) * spandrel(ROUND);

    assert_eq!(rim.len(), 8);
    assert_eq!(rounded.validate(), Ok(()));
    assert_watertight(
        "rounded",
        &rounded.tessellate(&rounded.default_tolerance()).unwrap(),
    );
    assert!(
        (removed - expected).abs() <= 0.01 * expected,
        "the fillet removes {removed} instead of {expected}"
    );
}
