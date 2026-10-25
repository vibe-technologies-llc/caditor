use std::sync::atomic::{AtomicU64, Ordering};

use caditor_geometry::Plane;
use caditor_sketch::{Entity, Sketch};

use crate::{
    seeded::for_each_case,
    sketch::{no_parameters, sketch},
};

const POLL_LIMIT: u64 = 5_000_000;
const SOLVING_CASES: u64 = 3000;
const FEWEST_SOLVED: usize = 300;
const LARGEST_SHIFT: f64 = 1e-3;
const DEGENERATE_GAP: f64 = 1e-3;

fn has_a_degenerate_spline(sketch: &Sketch) -> bool {
    sketch.entities().any(|(_, entity)| match entity {
        Entity::Spline { points, .. } => points.iter().enumerate().any(|(index, point)| {
            points
                .iter()
                .skip(index + 1)
                .any(|other| other == point || near(sketch, *point, *other))
        }),
        _ => false,
    })
}

fn near(
    sketch: &Sketch,
    first: caditor_sketch::EntityId,
    second: caditor_sketch::EntityId,
) -> bool {
    match (sketch.point(first), sketch.point(second)) {
        (Some(first), Some(second)) => first.distance(second) < DEGENERATE_GAP,
        _ => false,
    }
}

pub(crate) fn largest_shift(first: &Sketch, second: &Sketch) -> f64 {
    if first.entities().len() != second.entities().len() {
        return f64::INFINITY;
    }
    first
        .entities()
        .zip(second.entities())
        .map(|((own_id, own), (their_id, theirs))| {
            if own_id != their_id {
                return f64::INFINITY;
            }
            match (own, theirs) {
                (Entity::Point(own), Entity::Point(theirs)) => own.distance(*theirs),
                (Entity::Circle { radius: own, .. }, Entity::Circle { radius: theirs, .. }) => {
                    (own - theirs).abs()
                }
                (
                    Entity::Ellipse {
                        minor_radius: own, ..
                    },
                    Entity::Ellipse {
                        minor_radius: theirs,
                        ..
                    },
                ) => (own - theirs).abs(),
                _ => 0.0,
            }
        })
        .fold(0.0, f64::max)
}

fn solve(sketch: &Sketch) -> Option<caditor_sketch::Solved> {
    let polls = AtomicU64::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= POLL_LIMIT;
    sketch.solve(&no_parameters, &cancelled).ok()
}

#[test]
fn solving_the_same_sketch_twice_gives_the_same_geometry() {
    let mut solved = 0;
    for_each_case("deterministic solving", SOLVING_CASES, |_, input| {
        let Ok(drawn) = sketch(input, Plane::XY) else {
            return;
        };
        let (Some(first), Some(second)) = (solve(&drawn), solve(&drawn)) else {
            return;
        };

        assert!(
            first.geometry.same_geometry(&second.geometry),
            "two solves of one sketch gave different geometry"
        );
        assert_eq!(
            first.solution.degrees_of_freedom(),
            second.solution.degrees_of_freedom()
        );
        solved += 1;
    });
    assert!(
        solved >= FEWEST_SOLVED,
        "only {solved} sketches solved, so the generators no longer reach enough"
    );
}

fn solve_again(
    first: &caditor_sketch::Solved,
) -> Result<caditor_sketch::Solved, caditor_sketch::SketchError> {
    let polls = AtomicU64::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= POLL_LIMIT;
    first
        .geometry
        .solve_from(&no_parameters, &cancelled, &[], Some(&first.memo))
}

#[test]
fn a_solved_sketch_solves_again_from_its_own_result() {
    for_each_case("solving again", SOLVING_CASES, |_, input| {
        let Ok(drawn) = sketch(input, Plane::XY) else {
            return;
        };
        if has_a_degenerate_spline(&drawn) {
            return;
        }
        let Some(first) = solve(&drawn) else {
            return;
        };

        match solve_again(&first) {
            Ok(_) => {}
            Err(error) => panic!("a solved sketch could not be solved again: {error:?}"),
        }
    });
}

#[test]
fn a_solved_sketch_does_not_move_when_solved_again() {
    for_each_case("staying put", SOLVING_CASES, |_, input| {
        let Ok(drawn) = sketch(input, Plane::XY) else {
            return;
        };
        if has_a_degenerate_spline(&drawn) {
            return;
        }
        let Some(first) = solve(&drawn) else {
            return;
        };
        let Ok(again) = solve_again(&first) else {
            return;
        };

        let shift = largest_shift(&first.geometry, &again.geometry);
        assert!(
            shift <= LARGEST_SHIFT,
            "a solved sketch moved by {shift} when solved again"
        );
    });
}
