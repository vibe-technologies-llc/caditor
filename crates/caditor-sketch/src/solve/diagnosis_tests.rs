use std::{collections::BTreeSet, time::Instant};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    Constraint, ConstraintId, Entity, EntityId, Sketch, SketchError, Solved,
    solve::{
        diagnosis::{DIAGNOSIS_WORK, diagnose_failure},
        numeric::{STIFF, Solver},
        system::System,
    },
};

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Result<Solved, SketchError> {
    sketch.solve(&no_parameters, &|| false)
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn add(sketch: &mut Sketch, constraint: Constraint) -> ConstraintId {
    sketch.add_constraint(constraint).unwrap()
}

fn distance(sketch: &mut Sketch, from: EntityId, to: EntityId, length: f64) -> ConstraintId {
    add(
        sketch,
        Constraint::Distance {
            from,
            to,
            value: mm(length),
        },
    )
}

struct Chain {
    sketch: Sketch,
    lines: Vec<EntityId>,
    lengths: Vec<ConstraintId>,
    joins: Vec<ConstraintId>,
    closing: ConstraintId,
}

impl Chain {
    fn along(steps: &[Vector2], level: bool, closing_length: f64) -> Self {
        let mut sketch = Sketch::new(Plane::XY);
        let mut lines = Vec::with_capacity(steps.len());
        let mut lengths = Vec::with_capacity(steps.len());
        let mut joins = Vec::with_capacity(steps.len());
        let mut at = Point2::ZERO;
        let mut previous_end = None;
        for (index, step) in steps.iter().enumerate() {
            let wobble = Vector2::new(0.01, (index % 3) as f64 * 0.01);
            let line = sketch.add_line(at + wobble, at + *step * 1.02);
            let (start, end) = ends(&sketch, line);
            match previous_end {
                Some(previous) => {
                    joins.push(add(&mut sketch, Constraint::Coincident(previous, start)))
                }
                None => {
                    add(&mut sketch, Constraint::Coincident(start, EntityId::ORIGIN));
                }
            }
            if level {
                add(&mut sketch, Constraint::Horizontal(line));
            }
            lengths.push(distance(&mut sketch, start, end, 1.0));
            previous_end = Some(end);
            lines.push(line);
            at += *step;
        }
        let (first, _) = ends(&sketch, lines[0]);
        let (_, last) = ends(&sketch, lines[lines.len() - 1]);
        let closing = distance(&mut sketch, first, last, closing_length);
        Self {
            sketch,
            lines,
            lengths,
            joins,
            closing,
        }
    }

    fn spanning(&self) -> Vec<ConstraintId> {
        let mut spanning: Vec<ConstraintId> = self
            .lengths
            .iter()
            .chain(&self.joins)
            .copied()
            .chain([self.closing])
            .collect();
        spanning.sort_unstable();
        spanning
    }

    fn unsolvable(&self) -> SketchError {
        SketchError::Unsolvable {
            entities: self.lines.clone(),
            newest: Some(self.closing),
        }
    }
}

fn level_steps(count: usize) -> Vec<Vector2> {
    vec![Vector2::X; count]
}

fn out_of_reach(count: usize) -> Chain {
    Chain::along(&level_steps(count), true, count as f64 + 5.0)
}

#[test]
fn a_chain_that_must_fold_a_line_back_is_not_called_a_conflict() {
    let straight = Chain::along(&level_steps(8), true, 6.0);
    let mut one_back = level_steps(8);
    one_back[3] = -Vector2::X;
    let folded = Chain::along(&one_back, true, 6.0);

    let from_straight = solve(&straight.sketch);
    let from_folded = solve(&folded.sketch);

    assert_eq!(from_straight, Err(straight.unsolvable()));
    assert!(from_folded.is_ok(), "{from_folded:?}");
}

#[test]
fn a_chain_that_must_curl_up_far_from_its_drawing_is_not_called_a_conflict() {
    let count = 50;
    let quarter = count as f64 / 4.0;
    let straight = Chain::along(&level_steps(count), false, quarter);
    let slope = (1.0_f64 - 0.25 * 0.25).sqrt();
    let zigzag: Vec<Vector2> = (0..count)
        .map(|index| {
            let rise = if index % 2 == 0 { slope } else { -slope };
            Vector2::new(0.25, rise)
        })
        .collect();
    let curled = Chain::along(&zigzag, false, quarter);

    let from_straight = solve(&straight.sketch);
    let from_curled = solve(&curled.sketch);

    assert_eq!(from_straight, Err(straight.unsolvable()));
    assert!(from_curled.is_ok(), "{from_curled:?}");
}

fn triangle(sides: [f64; 3]) -> (Sketch, Vec<ConstraintId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let corner = sketch.add_point(Point2::ZERO);
    let right = sketch.add_point(Point2::new(4.0, 0.0));
    let top = sketch.add_point(Point2::new(4.0, 2.0));
    add(
        &mut sketch,
        Constraint::Coincident(corner, EntityId::ORIGIN),
    );
    let lengths = [(corner, right), (right, top), (corner, top)]
        .into_iter()
        .zip(sides)
        .map(|((from, to), length)| distance(&mut sketch, from, to, length))
        .collect();
    (sketch, lengths)
}

#[test]
fn sides_that_cannot_close_a_triangle_are_a_conflict_only_their_lengths_show() {
    let (closing, _) = triangle([3.0, 4.0, 5.0]);
    let (open, lengths) = triangle([3.0, 4.0, 10.0]);

    let closed = solve(&closing).unwrap();
    let result = solve(&open);

    assert_eq!(closed.solution.degrees_of_freedom(), 1);
    assert!(closed.solution.redundancies().is_empty());
    assert_eq!(
        result,
        Err(SketchError::Conflict {
            constraints: lengths
        })
    );
}

#[test]
fn an_incircle_given_too_large_a_radius_names_what_sizes_the_triangle() {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [Point2::ZERO, Point2::new(3.2, 0.1), Point2::new(2.9, 3.8)];
    let sides: Vec<EntityId> = (0..3)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 3]))
        .collect();
    let joins: Vec<ConstraintId> = (0..3)
        .map(|index| {
            let (_, end) = ends(&sketch, sides[index]);
            let (next, _) = ends(&sketch, sides[(index + 1) % 3]);
            add(&mut sketch, Constraint::Coincident(end, next))
        })
        .collect();
    let (first, _) = ends(&sketch, sides[0]);
    add(&mut sketch, Constraint::Coincident(first, EntityId::ORIGIN));
    add(&mut sketch, Constraint::Horizontal(sides[0]));
    let lengths: Vec<ConstraintId> = sides
        .iter()
        .zip([3.0, 4.0, 5.0])
        .map(|(side, length)| {
            let (start, end) = ends(&sketch, *side);
            distance(&mut sketch, start, end, length)
        })
        .collect();
    let circle = sketch.add_circle(Point2::new(2.1, 1.2), 0.8);
    let tangents: Vec<ConstraintId> = sides
        .iter()
        .map(|side| add(&mut sketch, Constraint::Tangent(*side, circle)))
        .collect();
    let inscribed = solve(&sketch).unwrap().geometry.circle(circle).unwrap();

    let radius = add(
        &mut sketch,
        Constraint::Radius {
            entity: circle,
            value: mm(1.5),
        },
    );
    let result = solve(&sketch);

    let mut expected: Vec<ConstraintId> = joins
        .into_iter()
        .chain(lengths[1..].iter().copied())
        .chain(tangents)
        .chain([radius])
        .collect();
    expected.sort_unstable();
    assert!(inscribed.0.distance(Point2::new(2.0, 1.0)) < 1e-9);
    assert!((inscribed.1 - 1.0).abs() < 1e-9);
    assert_eq!(
        result,
        Err(SketchError::Conflict {
            constraints: expected
        })
    );
}

#[test]
fn a_conflict_spanning_a_whole_part_of_over_a_hundred_entities_is_named_within_the_budget() {
    let chain = out_of_reach(40);
    let dimensions = chain.sketch.evaluate(&no_parameters).unwrap();
    let system = System::build(&chain.sketch, &dimensions).unwrap();
    let stiff = BTreeSet::new();
    let solver = Solver {
        system: &system,
        cancelled: &|| false,
        stiff: &stiff,
        stiffness: STIFF,
    };
    let every: Vec<usize> = (0..system.equations.len()).collect();
    let mut values = system.values.clone();
    let failed = solver.solve(&every, &mut values).unwrap();

    let tenth = diagnose_failure(&chain.sketch, &solver, &failed, DIAGNOSIS_WORK / 10).unwrap();
    let hundredth =
        diagnose_failure(&chain.sketch, &solver, &failed, DIAGNOSIS_WORK / 100).unwrap();

    assert_eq!(chain.sketch.entities().len(), 3 * 40);
    assert_eq!(chain.spanning().len(), 2 * 40);
    assert_eq!(
        tenth,
        SketchError::Conflict {
            constraints: chain.spanning()
        }
    );
    assert_eq!(hundredth, chain.unsolvable());
}

#[test]
#[ignore = "measures a conflict across hundreds of entities, best run in release"]
fn a_conflict_across_hundreds_of_entities_is_named_within_seconds() {
    for count in [100, 150] {
        let chain = out_of_reach(count);

        let started = Instant::now();
        let result = solve(&chain.sketch);
        let elapsed = started.elapsed();

        println!(
            "{} entities, {} constraints: {elapsed:?}",
            chain.sketch.entities().len(),
            chain.sketch.constraints().len()
        );
        assert_eq!(
            result,
            Err(SketchError::Conflict {
                constraints: chain.spanning()
            })
        );
        assert!(elapsed.as_secs() < 5, "{elapsed:?}");
    }
}

#[test]
fn diagnosis_gives_the_same_answer_every_time() {
    let sketches = [
        out_of_reach(6).sketch,
        Chain::along(&level_steps(6), true, 4.0).sketch,
        triangle([3.0, 4.0, 10.0]).0,
    ];
    for sketch in sketches {
        let first = solve(&sketch);
        let again = solve(&sketch.clone());

        assert!(first.is_err());
        assert_eq!(first, again);
    }
}
