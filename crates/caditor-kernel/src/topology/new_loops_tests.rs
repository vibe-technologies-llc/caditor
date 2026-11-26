use std::{collections::BTreeSet, f64::consts::TAU};

use caditor_geometry::{Maths, Plane, Point2, Point3, Vector3};

use super::{
    LoopId, Solid,
    validate::{
        ValidationError, faces_given_new_loops, first_validation_tolerance, new_faces_mesh,
    },
};
use crate::{
    curve::Circle,
    fixtures::{Coedges, Fixture},
    interval::Interval,
    sense::Sense,
    surface::PlaneSurface,
    tally,
    test_support::Random,
};

const WIDTH: f64 = 60.0;
const DEPTH: f64 = 50.0;
const PITCH: f64 = 10.0;
const HOLE: f64 = 3.0;
const SWEPT_SIDE: usize = 3;
const SWEPT: f64 = PITCH * SWEPT_SIDE as f64;

#[derive(Clone, Debug)]
enum Shape {
    Round { center: Point2, radius: f64 },
    Polygon(Vec<Point2>),
}

fn round(x: f64, y: f64, radius: f64) -> Shape {
    Shape::Round {
        center: Point2::new(x, y),
        radius,
    }
}

fn polygon(corners: &[(f64, f64)]) -> Shape {
    Shape::Polygon(corners.iter().map(|(x, y)| Point2::new(*x, *y)).collect())
}

fn rectangle(low: (f64, f64), high: (f64, f64)) -> Shape {
    polygon(&[low, (high.0, low.1), high, (low.0, high.1)])
}

fn coedges(fixture: &mut Fixture, shape: &Shape, sense: Sense) -> Coedges {
    match shape {
        Shape::Round { center, radius } => {
            let middle = Point3::new(center.x, center.y, 0.0);
            let start = fixture.vertex(middle + Vector3::new(*radius, 0.0, 0.0));
            let frame = Plane::from_frame(middle, Vector3::Z, Vector3::X).unwrap();

            let edge = fixture.edge(
                Circle::new(frame, *radius).unwrap(),
                Interval::FULL_TURN,
                start,
                start,
            );
            vec![(edge, sense)]
        }
        Shape::Polygon(corners) => {
            let mut corners: Vec<_> = corners
                .iter()
                .map(|corner| fixture.vertex(Point3::new(corner.x, corner.y, 0.0)))
                .collect();

            if !sense.is_same() {
                corners.reverse();
            }
            fixture.polygon_loop(&corners)
        }
    }
}

struct Sheet {
    solid: Solid,
    carried: BTreeSet<LoopId>,
}

fn sheet(outer: &Shape, outer_carried: bool, carried: &[Shape], new: &[Shape]) -> Sheet {
    let mut fixture = Fixture::new();

    let mut loops = vec![coedges(&mut fixture, outer, Sense::Same)];

    for hole in carried.iter().chain(new) {
        loops.push(coedges(&mut fixture, hole, Sense::Reversed));
    }
    fixture.face(PlaneSurface::new(Plane::XY).unwrap(), Sense::Same, &loops);

    let solid = fixture.build_unchecked();
    let face = solid.faces().next().unwrap().1;
    let carried: BTreeSet<LoopId> = face
        .loops()
        .iter()
        .enumerate()
        .filter(|(position, _)| match position {
            0 => outer_carried,
            position => *position <= carried.len(),
        })
        .map(|(_, face_loop)| *face_loop)
        .collect();
    Sheet { solid, carried }
}

fn plate() -> Shape {
    rectangle((0.0, 0.0), (WIDTH, DEPTH))
}

fn grid(side: usize, jitter: impl FnMut() -> (f64, f64, f64)) -> Vec<Shape> {
    let mut jitter = jitter;
    let mut holes = Vec::new();

    for row in 0..side {
        for column in 0..side {
            let (dx, dy, radius) = jitter();

            holes.push(round(
                PITCH * (column as f64 + 0.5) + dx,
                PITCH * (row as f64 + 0.5) + dy,
                radius,
            ));
        }
    }
    holes
}

fn even_grid() -> Vec<Shape> {
    grid(5, || (0.0, 0.0, HOLE))
}

struct Verdicts {
    near: Result<(), ValidationError>,
    whole: Result<(), ValidationError>,
    loops_meshed: usize,
}

fn verdicts(sheet: &Sheet) -> Verdicts {
    let (near, loops_meshed) =
        tally::measure_meshed_loops(|| new_faces_mesh(&sheet.solid, &sheet.carried));
    let every_loop: BTreeSet<LoopId> = faces_given_new_loops(&sheet.solid, &sheet.carried)
        .into_iter()
        .flat_map(|face| sheet.solid.face(face).unwrap().loops().to_vec())
        .collect();
    let whole = sheet
        .solid
        .keeping_loops(&every_loop)
        .unwrap()
        .tessellate(&first_validation_tolerance(&sheet.solid))
        .map(|_| ())
        .map_err(ValidationError::from);
    Verdicts {
        near,
        whole,
        loops_meshed,
    }
}

fn assert_agree(name: &str, sheet: &Sheet) -> Verdicts {
    let found = verdicts(sheet);

    assert_eq!(found.near, found.whole, "{name}");
    found
}

#[test]
fn a_new_loop_far_from_the_carried_ones_meshes_with_the_outer_loop_alone() {
    let found = assert_agree(
        "far",
        &sheet(&plate(), true, &even_grid(), &[round(55.0, 25.0, HOLE)]),
    );

    assert_eq!(found.near, Ok(()));
    assert_eq!(found.loops_meshed, 2);
}

#[test]
fn a_new_loop_meshes_with_the_carried_loops_its_edges_reach() {
    let found = assert_agree(
        "between four holes",
        &sheet(&plate(), true, &even_grid(), &[round(10.0, 10.0, HOLE)]),
    );

    assert_eq!(found.near, Ok(()));
    assert_eq!(found.loops_meshed, 6);
}

#[test]
fn a_new_loop_crossing_touching_or_nesting_with_a_carried_one_has_the_verdict_of_the_whole_face() {
    let cases = [
        ("crossing a hole", round(9.0, 5.0, HOLE), Some(false)),
        ("touching a hole", round(11.0, 5.0, HOLE), None),
        ("a hair from a hole", round(11.001, 5.0, HOLE), None),
        ("inside a hole", round(5.0, 5.0, 1.0), None),
        ("around a hole", round(5.0, 5.0, 4.0), None),
        ("on a hole's rim", round(5.0, 8.0, 1.0), Some(false)),
        (
            "a square across a hole",
            rectangle((7.0, 3.0), (9.0, 7.0)),
            Some(false),
        ),
        (
            "a square on a hole's edge",
            rectangle((8.0, 3.0), (9.0, 7.0)),
            None,
        ),
    ];

    for (name, new, accepted) in cases {
        let found = assert_agree(name, &sheet(&plate(), true, &even_grid(), &[new]));

        if let Some(accepted) = accepted {
            assert_eq!(found.near.is_ok(), accepted, "{name}: {:?}", found.near);
        }

        assert!(found.loops_meshed < 27, "{name}: {}", found.loops_meshed);
    }
}

#[test]
fn a_new_loop_against_the_outer_boundary_has_the_verdict_of_the_whole_face() {
    let cases = [
        ("tangent to a side", round(WIDTH - HOLE, 25.0, HOLE), None),
        (
            "crossing a side",
            round(WIDTH - 1.0, 25.0, HOLE),
            Some(false),
        ),
        ("in a corner", rectangle((55.0, 45.0), (WIDTH, DEPTH)), None),
        (
            "past a corner",
            rectangle((55.0, 45.0), (61.0, 51.0)),
            Some(false),
        ),
        (
            "crossing itself",
            polygon(&[(52.0, 20.0), (58.0, 30.0), (58.0, 20.0), (52.0, 30.0)]),
            Some(false),
        ),
    ];

    for (name, new, accepted) in cases {
        let found = assert_agree(name, &sheet(&plate(), true, &even_grid(), &[new]));

        if let Some(accepted) = accepted {
            assert_eq!(found.near.is_ok(), accepted, "{name}: {:?}", found.near);
        }
    }
}

#[test]
fn new_loops_crossing_each_other_have_the_verdict_of_the_whole_face() {
    let found = assert_agree(
        "two new holes crossing",
        &sheet(
            &plate(),
            true,
            &even_grid(),
            &[round(55.0, 20.0, HOLE), round(55.0, 24.0, HOLE)],
        ),
    );

    assert!(found.near.is_err());
}

#[test]
fn a_new_outer_loop_meshes_with_the_carried_holes_its_edges_reach() {
    let notched = polygon(&[
        (0.0, 0.0),
        (WIDTH, 0.0),
        (WIDTH, DEPTH),
        (12.0, DEPTH),
        (12.0, 47.0),
        (8.0, 47.0),
        (8.0, DEPTH),
        (0.0, DEPTH),
    ]);

    let clear = assert_agree(
        "notch clear of a hole",
        &sheet(&notched, false, &even_grid(), &[]),
    );

    let deep = polygon(&[
        (0.0, 0.0),
        (WIDTH, 0.0),
        (WIDTH, DEPTH),
        (13.0, DEPTH),
        (13.0, 44.0),
        (7.0, 44.0),
        (7.0, DEPTH),
        (0.0, DEPTH),
    ]);
    let crossing = assert_agree("notch into a hole", &sheet(&deep, false, &even_grid(), &[]));

    assert_eq!(clear.near, Ok(()));
    assert!(
        clear.loops_meshed < 26,
        "only the holes beside the notch: {}",
        clear.loops_meshed
    );
    assert!(crossing.near.is_err());
}

#[test]
fn random_new_loops_have_the_verdict_of_the_whole_face() {
    let mut random = Random::new(0x6e65_775f_6c6f_6f70);
    let mut outcomes = [0usize; 2];

    for case in 0..64 {
        let carried = grid(SWEPT_SIDE, || {
            (
                random.between(-1.0, 1.0),
                random.between(-1.0, 1.0),
                random.between(1.0, HOLE),
            )
        });
        let center = Point2::new(
            random.between(-2.0, SWEPT + 2.0),
            random.between(-2.0, SWEPT + 2.0),
        );
        let size = random.between(0.3, 4.0);

        let new = if case % 3 == 0 {
            let turn = random.between(0.0, 1.0);
            let corners: Vec<(f64, f64)> = (0..3)
                .map(|corner| {
                    let angle = TAU * (turn + corner as f64 / 3.0);
                    (
                        center.x + size * angle.cos_libm(),
                        center.y + size * angle.sin_libm(),
                    )
                })
                .collect();
            polygon(&corners)
        } else {
            Shape::Round {
                center,
                radius: size,
            }
        };

        let found = assert_agree(
            &format!("case {case}: {new:?}"),
            &sheet(
                &rectangle((0.0, 0.0), (SWEPT, SWEPT)),
                true,
                &carried,
                &[new],
            ),
        );

        if let Some(outcome) = outcomes.get_mut(usize::from(found.near.is_ok())) {
            *outcome += 1;
        }
    }

    assert!(
        outcomes.iter().all(|count| *count >= 15),
        "refused and accepted cases: {outcomes:?}"
    );
}
