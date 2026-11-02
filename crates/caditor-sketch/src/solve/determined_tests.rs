use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Entity, EntityId, EntityState, Sketch, Solved, SplineKind};

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
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

struct Floating {
    sketch: Sketch,
    lines: [EntityId; 4],
    corners: [EntityId; 4],
}

fn floating_rectangle() -> Floating {
    let mut sketch = Sketch::new(Plane::XY);
    let lines = [
        sketch.add_line(Point2::new(3.0, 2.0), Point2::new(43.0, 2.0)),
        sketch.add_line(Point2::new(43.0, 2.0), Point2::new(43.0, 22.0)),
        sketch.add_line(Point2::new(43.0, 22.0), Point2::new(3.0, 22.0)),
        sketch.add_line(Point2::new(3.0, 22.0), Point2::new(3.0, 2.0)),
    ];
    let ends: Vec<(EntityId, EntityId)> = lines.iter().map(|line| ends(&sketch, *line)).collect();
    for (index, (_, end)) in ends.iter().enumerate() {
        let next = ends.get((index + 1) % 4).unwrap().0;
        sketch
            .add_constraint(Constraint::Coincident(*end, next))
            .unwrap();
    }
    let [bottom, right, top, left] = lines;
    for constraint in [
        Constraint::Horizontal(bottom),
        Constraint::Vertical(right),
        Constraint::Horizontal(top),
        Constraint::Vertical(left),
        Constraint::Distance {
            from: ends.first().unwrap().0,
            to: ends.first().unwrap().1,
            value: mm(40.0),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let corners = [0, 1, 2, 3].map(|index| ends.get(index).unwrap().0);
    Floating {
        sketch,
        lines,
        corners,
    }
}

fn solved(sketch: &Sketch) -> Solved {
    sketch.solve(&no_parameters, &|| false).unwrap()
}

fn determines(sketch: &Sketch, dimension: &Constraint) -> bool {
    let solved = solved(sketch);
    solved
        .geometry
        .determines(solved.solution.dimensions(), dimension)
        .unwrap()
}

fn length_of(sketch: &Sketch, line: EntityId, value: f64) -> Constraint {
    let (from, to) = ends(sketch, line);
    Constraint::Distance {
        from,
        to,
        value: mm(value),
    }
}

#[test]
fn the_width_of_a_floating_rectangle_is_determined_across_from_its_dimension() {
    let Floating { sketch, lines, .. } = floating_rectangle();
    let [_, _, top, _] = lines;

    let solution = solved(&sketch).solution;

    assert_eq!(
        solution.entity_state(top),
        Some(EntityState::UnderConstrained)
    );
    assert!(determines(&sketch, &length_of(&sketch, top, 40.0)));
}

#[test]
fn the_free_height_of_a_floating_rectangle_is_not_determined() {
    let Floating { sketch, lines, .. } = floating_rectangle();
    let [_, right, _, left] = lines;

    assert!(!determines(&sketch, &length_of(&sketch, right, 20.0)));
    assert!(!determines(&sketch, &length_of(&sketch, left, 20.0)));
}

#[test]
fn an_equal_length_carries_the_dimension_round_a_corner() {
    let Floating {
        mut sketch, lines, ..
    } = floating_rectangle();
    let [bottom, right, _, left] = lines;
    sketch
        .add_constraint(Constraint::Equal(bottom, right))
        .unwrap();

    assert!(determines(&sketch, &length_of(&sketch, left, 40.0)));
}

#[test]
fn a_diagonal_still_free_to_stretch_is_not_determined() {
    let Floating {
        sketch, corners, ..
    } = floating_rectangle();
    let [first, _, third, _] = corners;

    let solved = solved(&sketch);
    let measuring = Constraint::Distance {
        from: first,
        to: third,
        value: mm(1.0),
    };
    let diagonal = Constraint::Distance {
        from: first,
        to: third,
        value: mm(solved.geometry.measured(&measuring).unwrap()),
    };

    assert!(
        !solved
            .geometry
            .determines(solved.solution.dimensions(), &diagonal)
            .unwrap()
    );
}

#[test]
fn a_horizontal_distance_along_the_dimensioned_side_is_determined() {
    let Floating {
        sketch, corners, ..
    } = floating_rectangle();
    let [first, _, third, _] = corners;

    let across = Constraint::HorizontalDistance {
        from: first,
        to: third,
        value: mm(40.0),
    };

    assert!(determines(&sketch, &across));
}

fn pinned_spline() -> (Sketch, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline_of(
        &[
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 6.0),
            Point2::new(25.0, -3.0),
            Point2::new(32.0, 8.0),
        ],
        SplineKind::Control { closed: false },
    );
    let Some(Entity::Spline { points, .. }) = sketch.entity(spline).cloned() else {
        panic!("expected a spline");
    };
    for point in points {
        let at = sketch.point(point).unwrap();
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }
    (sketch, spline)
}

#[test]
fn a_distance_from_a_pinned_spline_to_a_pinned_point_is_determined_though_it_brings_a_parameter() {
    let (mut sketch, spline) = pinned_spline();
    let away = sketch.add_point(Point2::new(15.0, 12.0));
    sketch
        .add_constraint(Constraint::Fix {
            point: away,
            at: Point2::new(15.0, 12.0),
        })
        .unwrap();
    let measuring = Constraint::Distance {
        from: away,
        to: spline,
        value: mm(1.0),
    };
    let wanted = solved(&sketch).geometry.measured(&measuring).unwrap();

    assert!(determines(
        &sketch,
        &Constraint::Distance {
            from: away,
            to: spline,
            value: mm(wanted),
        }
    ));
}

#[test]
fn a_distance_from_a_pinned_spline_to_a_free_point_is_not_determined() {
    let (mut sketch, spline) = pinned_spline();
    let away = sketch.add_point(Point2::new(15.0, 12.0));
    let measuring = Constraint::Distance {
        from: away,
        to: spline,
        value: mm(1.0),
    };
    let wanted = solved(&sketch).geometry.measured(&measuring).unwrap();

    assert!(!determines(
        &sketch,
        &Constraint::Distance {
            from: away,
            to: spline,
            value: mm(wanted),
        }
    ));
}
