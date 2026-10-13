use std::collections::BTreeSet;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{
    Constraint, Drag, Entity, EntityId, Sketch,
    solve::{
        numeric::{ReachWork, STIFF, Solver, components},
        system::System,
        tally,
    },
};

const SAG: f64 = 0.1;
const ON_THE_RAY: f64 = 1e-6;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

struct Arm {
    sketch: Sketch,
    hand: EntityId,
}

fn zigzag_arm(count: usize) -> Arm {
    let mut sketch = Sketch::new(Plane::XY);
    let run = (1.0 - SAG * SAG).sqrt();
    let corner = |index: usize| Point2::new(index as f64 * run, (index % 2) as f64 * SAG);

    let mut hand = EntityId::ORIGIN;
    for index in 0..count {
        let line = sketch.add_line(corner(index), corner(index + 1));
        let (start, end) = ends(&sketch, line);
        sketch
            .add_constraint(Constraint::Coincident(hand, start))
            .unwrap();
        sketch
            .add_constraint(Constraint::Distance {
                from: start,
                to: end,
                value: Expression::Measure(1.0, Unit::Millimetre),
            })
            .unwrap();
        hand = end;
    }
    Arm { sketch, hand }
}

fn drag_past_reach(count: usize) -> (Point2, Point2, usize) {
    let arm = zigzag_arm(count);
    let pointer = Point2::new(1.2, 0.5) * count as f64;

    let (followed, work) = tally::measure(|| {
        arm.sketch
            .solve_geometry_from(
                &no_parameters,
                &|| false,
                &[Drag::Point {
                    point: arm.hand,
                    to: pointer,
                }],
                None,
            )
            .unwrap()
            .0
    });
    let stretched = pointer.normalize() * count as f64;

    (followed.point(arm.hand).unwrap(), stretched, work)
}

#[test]
fn a_chain_of_500_lines_dragged_past_its_reach_follows_the_pointer() {
    const FRAME_WORK: usize = 45_000_000;

    let (hand, stretched, work) = drag_past_reach(500);

    assert!(
        hand.distance(stretched) < ON_THE_RAY * 500.0,
        "{hand} is not {stretched}"
    );
    assert!(work < FRAME_WORK, "{work}");
}

#[test]
fn a_reach_out_of_work_stops_closer_at_a_pose_that_holds() {
    const ALLOWED: usize = 2_000_000;
    let arm = zigzag_arm(500);
    let dimensions = arm.sketch.evaluate(&no_parameters).unwrap();
    let system = System::build(&arm.sketch, &dimensions).unwrap();
    let stiff = BTreeSet::new();
    let solver = Solver {
        system: &system,
        cancelled: &|| false,
        stiff: &stiff,
        stiffness: STIFF,
    };

    let every: Vec<usize> = (0..system.equations.len()).collect();
    let parts = components(&system, &every, &system.values);
    let [part] = parts.as_slice() else {
        panic!("{} parts", parts.len());
    };
    let x = system.points[&arm.hand];
    let pointer = Point2::new(600.0, 250.0);
    let targets = [(x, pointer.x), (x + 1, pointer.y)];
    let mut values = system.values.clone();
    let mut work = ReachWork::new(ALLOWED);
    let ((), spent) = tally::measure(|| {
        solver
            .reach(part, &targets, &system.values, &mut values, &mut work)
            .unwrap();
    });
    let hand = |values: &[f64]| Point2::new(values[x], values[x + 1]);

    assert!(solver.holds(part, &values));
    assert!(hand(&values).distance(pointer) < hand(&system.values).distance(pointer));
    assert!(hand(&values).distance(pointer) > 150.0 + 1.0);
    assert!(spent < 2 * ALLOWED, "{spent}");
}

#[test]
#[ignore = "drags a chain of thousands of points through a whole frame, best run in release"]
fn a_chain_of_2000_lines_dragged_past_its_reach_follows_the_pointer() {
    const FRAME_WORK: usize = 700_000_000;

    let (hand, stretched, work) = drag_past_reach(2_000);

    assert!(
        hand.distance(stretched) < ON_THE_RAY * 2_000.0,
        "{hand} is not {stretched}"
    );
    assert!(work < FRAME_WORK, "{work}");
}
