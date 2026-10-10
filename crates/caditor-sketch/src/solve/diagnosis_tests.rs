use std::{collections::BTreeSet, time::Instant};

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{
    Constraint, ConstraintId, Entity, EntityId, Sketch, SketchError, Solved,
    solve::{
        diagnosis::{DIAGNOSIS_WORK, diagnose_failure},
        numeric::{Failure, STIFF, Solver, components},
        system::System,
        witness::Factored,
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

const HELD: f64 = 1e-6;

fn assert_every_constraint_holds(solved: &Solved) {
    let geometry = &solved.geometry;
    for (id, constraint) in geometry.constraints() {
        match *constraint {
            Constraint::Coincident(a, b) => {
                let (a, b) = (geometry.point(a).unwrap(), geometry.point(b).unwrap());
                assert!(a.distance(b) < HELD, "{id:?} leaves {a} apart from {b}");
            }
            Constraint::Distance { .. } => {
                let measured = geometry.measured(constraint).unwrap();
                let wanted = solved.solution.dimension(id).unwrap();
                assert!(
                    (measured - wanted).abs() < HELD,
                    "{id:?}: {measured} is not {wanted}"
                );
            }
            Constraint::Horizontal(line) => {
                let (start, end) = ends(geometry, line);
                let rise = geometry.point(end).unwrap().y - geometry.point(start).unwrap().y;
                assert!(rise.abs() < HELD, "{id:?} rises by {rise}");
            }
            ref other => panic!("no check for {other:?}"),
        }
    }
}

fn reversed_lines(chain: &Chain, solved: &Solved) -> usize {
    chain
        .lines
        .iter()
        .filter(|line| {
            let (start, end) = ends(&solved.geometry, **line);
            solved.geometry.point(end).unwrap().x < solved.geometry.point(start).unwrap().x
        })
        .count()
}

#[test]
fn a_chain_that_must_fold_a_line_back_solves_from_what_diagnosis_finds() {
    let straight = Chain::along(&level_steps(8), true, 6.0);
    let mut one_back = level_steps(8);
    one_back[3] = -Vector2::X;
    let folded = Chain::along(&one_back, true, 6.0);

    let from_straight = solve(&straight.sketch).unwrap();
    let from_folded = solve(&folded.sketch).unwrap();

    assert_every_constraint_holds(&from_straight);
    assert_every_constraint_holds(&from_folded);
    assert_eq!(reversed_lines(&straight, &from_straight), 1);
    assert_eq!(reversed_lines(&folded, &from_folded), 1);
    assert!(from_straight.solution.is_fully_constrained());
}

#[test]
fn a_part_solved_by_diagnosis_is_recalled_without_diagnosing_it_again() {
    let straight = Chain::along(&level_steps(8), true, 6.0);

    let (first, diagnosed) = super::tally::measure(|| solve(&straight.sketch).unwrap());
    let (again, recalled) = super::tally::measure(|| {
        straight
            .sketch
            .solve_from(&no_parameters, &|| false, &[], Some(&first.memo))
            .unwrap()
    });

    assert_eq!(again.geometry, first.geometry);
    assert_eq!(again.solution, first.solution);
    assert_eq!(again.memo.recalled(), 1);
    assert_eq!(recalled, 0, "diagnosing took {diagnosed}");
}

#[test]
fn a_part_solved_by_diagnosis_leaves_a_conflict_elsewhere_reported_alone() {
    let mut chain = Chain::along(&level_steps(8), true, 6.0);
    let (open, _) = triangle([3.0, 4.0, 10.0]);
    let offset = Vector2::new(0.0, 50.0);
    let corners: Vec<EntityId> = open
        .entities()
        .map(|(_, entity)| match entity {
            Entity::Point(at) => chain.sketch.add_point(*at + offset),
            other => panic!("the triangle has only points, not {other:?}"),
        })
        .collect();
    let lengths: Vec<ConstraintId> = [(0, 1, 3.0), (1, 2, 4.0), (0, 2, 10.0)]
        .into_iter()
        .map(|(from, to, length)| distance(&mut chain.sketch, corners[from], corners[to], length))
        .collect();

    let result = solve(&chain.sketch);

    assert_eq!(
        result,
        Err(SketchError::Conflict {
            constraints: lengths
        })
    );
}

#[test]
fn a_chain_that_must_curl_up_far_from_its_drawing_solves_from_what_diagnosis_finds() {
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

    let from_straight = solve(&straight.sketch).unwrap();
    let from_curled = solve(&curled.sketch).unwrap();

    assert_every_constraint_holds(&from_straight);
    assert_every_constraint_holds(&from_curled);
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

fn with_failure<T>(chain: &Chain, then: impl FnOnce(&Solver<'_>, &[Failure]) -> T) -> T {
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
    then(&solver, &failed)
}

fn diagnosed(chain: &Chain, work: usize) -> SketchError {
    with_failure(chain, |solver, failed| {
        diagnose_failure(&chain.sketch, solver, failed, work).unwrap_err()
    })
}

#[test]
fn a_conflict_spanning_a_whole_part_of_over_a_hundred_entities_is_named_within_the_budget() {
    let chain = out_of_reach(40);

    let tenth = diagnosed(&chain, DIAGNOSIS_WORK / 10);
    let hundredth = diagnosed(&chain, DIAGNOSIS_WORK / 100);

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
fn one_factorisation_steps_to_a_solution_without_each_constraint_the_conflict_needs() {
    let chain = out_of_reach(40);
    let constraints: Vec<ConstraintId> = chain.sketch.constraints().map(|(id, _)| id).collect();

    let stepped = with_failure(&chain, |solver, failed| {
        let failure = &failed[0];
        let mut at = solver.system.values.clone();
        for (variable, value) in failure.component.variables.iter().zip(&failure.settled.end) {
            at[*variable] = *value;
        }
        let factored = Factored::at(solver, &failure.component, &at);
        let mut stepped = Vec::new();
        for constraint in &constraints {
            let mut free = || Ok::<(), ()>(());
            let Some(values) = factored.without(solver, *constraint, &mut free).unwrap() else {
                continue;
            };
            let rest: Vec<usize> = failure
                .component
                .equations
                .iter()
                .copied()
                .filter(|index| solver.system.equations[*index].owner != Some(*constraint))
                .collect();
            let parts = components(solver.system, &rest, &values);
            assert!(parts.iter().all(|part| solver.holds(part, &values)));
            stepped.push(*constraint);
        }
        stepped
    });

    assert_eq!(constraints.len(), 2 * 40 + 41);
    assert_eq!(stepped, chain.spanning());
}

#[test]
#[ignore = "diagnoses a conflict through three hundred lines, best run in release"]
fn a_conflict_through_a_chain_of_three_hundred_lines_is_named_within_the_budget() {
    let chain = out_of_reach(300);

    let named = diagnosed(&chain, DIAGNOSIS_WORK);

    assert_eq!(chain.spanning().len(), 2 * 300);
    assert!(
        named
            == SketchError::Conflict {
                constraints: chain.spanning()
            },
        "{named:?}"
    );
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
        (out_of_reach(6).sketch, false),
        (Chain::along(&level_steps(6), true, 4.0).sketch, true),
        (triangle([3.0, 4.0, 10.0]).0, false),
    ];
    for (sketch, holds) in sketches {
        let first = solve(&sketch);
        let again = solve(&sketch.clone());

        assert_eq!(first.is_ok(), holds, "{first:?}");
        assert_eq!(first, again);
    }
}
