use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, ConstraintId, Entity, EntityId, OffsetError, Side, Sketch, Solved};

const EXACT: f64 = 1e-7;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    match sketch.solve(&no_parameters, &|| false) {
        Ok(solved) => solved,
        Err(error) => panic!("the sketch does not solve: {error:?}"),
    }
}

fn mm(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn ends(sketch: &Sketch, curve: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(curve) {
        Some(Entity::Line { start, end } | Entity::Arc { start, end, .. }) => (*start, *end),
        other => panic!("expected a line or an arc, found {other:?}"),
    }
}

fn line_at(sketch: &Sketch, line: EntityId) -> (Point2, Point2) {
    sketch.line_endpoints(line).unwrap()
}

fn assert_corners(found: &[Point2], expected: &[(f64, f64)]) {
    assert_eq!(found.len(), expected.len());
    for (x, y) in expected {
        let expected = Point2::new(*x, *y);
        assert!(
            found.iter().any(|point| point.distance(expected) < EXACT),
            "{expected} is not among {found:?}"
        );
    }
}

fn has(sketch: &Sketch, constraint: &Constraint) -> bool {
    sketch
        .constraints()
        .any(|(_, existing)| existing == constraint)
}

fn count_of(sketch: &Sketch, kind: &str) -> usize {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == kind)
        .count()
}

fn assert_clean(solved: &Solved) {
    assert!(
        solved.solution.redundancies().is_empty(),
        "redundant: {:?}",
        solved.solution.redundancies()
    );
}

fn distance_to_line(sketch: &Sketch, point: Point2, line: EntityId) -> f64 {
    let (start, end) = line_at(sketch, line);
    (end - start).normalize().perp_dot(point - start).abs()
}

struct Rectangle {
    sketch: Sketch,
    sides: [EntityId; 4],
    width: ConstraintId,
}

fn rectangle(width: f64, height: f64) -> Rectangle {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        Point2::ZERO,
        Point2::new(width, 0.0),
        Point2::new(width, height),
        Point2::new(0.0, height),
    ];
    let sides: Vec<EntityId> = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    for index in 0..4 {
        let (_, end) = ends(&sketch, sides[index]);
        let (start, _) = ends(&sketch, sides[(index + 1) % 4]);
        sketch
            .add_constraint(Constraint::Coincident(end, start))
            .unwrap();
        sketch
            .add_constraint(if index % 2 == 0 {
                Constraint::Horizontal(sides[index])
            } else {
                Constraint::Vertical(sides[index])
            })
            .unwrap();
    }
    let (origin_corner, right_corner) = ends(&sketch, sides[0]);
    sketch
        .add_constraint(Constraint::Coincident(origin_corner, EntityId::ORIGIN))
        .unwrap();
    let width = sketch
        .add_constraint(Constraint::Distance {
            from: origin_corner,
            to: right_corner,
            value: mm(width),
        })
        .unwrap();
    let (bottom, top) = ends(&sketch, sides[1]);
    sketch
        .add_constraint(Constraint::Distance {
            from: bottom,
            to: top,
            value: mm(height),
        })
        .unwrap();
    Rectangle {
        sketch,
        sides: [sides[0], sides[1], sides[2], sides[3]],
        width,
    }
}

fn corner_positions(sketch: &Sketch, lines: &[EntityId]) -> Vec<Point2> {
    lines.iter().map(|line| line_at(sketch, *line).0).collect()
}

#[test]
fn a_rectangle_offset_outwards_stays_a_rectangle_its_distance_away_when_it_widens() {
    let Rectangle {
        mut sketch,
        sides,
        width,
    } = rectangle(40.0, 20.0);
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let chain = sketch.offset_chain(&sides).unwrap();
    assert!(chain.is_closed());
    let outside = chain.default_side();
    assert_eq!(chain.side_of(Point2::new(50.0, 10.0)), outside);
    assert_eq!(chain.side_of(Point2::new(20.0, 10.0)), outside.other());

    let copies = sketch.offset(&sides, outside, 5.0, mm(5.0)).unwrap();

    assert_eq!(copies.len(), 4);
    assert_corners(
        &corner_positions(&sketch, &copies),
        &[(-5.0, -5.0), (45.0, -5.0), (45.0, 25.0), (-5.0, 25.0)],
    );
    assert_eq!(count_of(&sketch, "Distance"), 2 + 4);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(width, mm(60.0)).unwrap();
    let wider = solve(&sketch).geometry;
    for (copy, side) in copies.iter().zip(sides) {
        let (start, end) = line_at(&wider, *copy);
        assert!((distance_to_line(&wider, start, side) - 5.0).abs() < EXACT);
        assert!((distance_to_line(&wider, end, side) - 5.0).abs() < EXACT);
    }
    assert_corners(
        &corner_positions(&wider, &copies),
        &[(-5.0, -5.0), (65.0, -5.0), (65.0, 25.0), (-5.0, 25.0)],
    );
}

#[test]
fn an_offset_by_an_expression_keeps_it_in_every_dimension_it_adds() {
    let Rectangle {
        mut sketch, sides, ..
    } = rectangle(40.0, 20.0);
    let wall = Expression::Parameter(ParameterId::from_raw(3));
    let inside = sketch.offset_chain(&sides).unwrap().default_side().other();

    sketch.offset(&sides, inside, 2.0, wall.clone()).unwrap();

    let distances: Vec<&Expression> = sketch
        .constraints()
        .filter_map(|(_, constraint)| match constraint {
            Constraint::Distance { from, value, .. } if sides.contains(from) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(distances, vec![&wall; 4]);
}

fn slot(sketch: &mut Sketch) -> (Vec<EntityId>, ConstraintId) {
    let top = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(40.0, 10.0));
    let bottom = sketch.add_line(Point2::new(40.0, -10.0), Point2::new(0.0, -10.0));
    let right = sketch.add_arc(
        Point2::new(40.0, 0.0),
        Point2::new(40.0, -10.0),
        Point2::new(40.0, 10.0),
    );
    let left = sketch.add_arc(
        Point2::ZERO,
        Point2::new(0.0, 10.0),
        Point2::new(0.0, -10.0),
    );
    let joins = [
        (ends(sketch, top).1, ends(sketch, right).1),
        (ends(sketch, right).0, ends(sketch, bottom).0),
        (ends(sketch, bottom).1, ends(sketch, left).1),
        (ends(sketch, left).0, ends(sketch, top).0),
    ];
    for (a, b) in joins {
        sketch.add_constraint(Constraint::Coincident(a, b)).unwrap();
    }
    for (line, arc) in [(top, right), (bottom, right), (bottom, left), (top, left)] {
        sketch
            .add_constraint(Constraint::Tangent(line, arc))
            .unwrap();
    }
    sketch
        .add_constraint(Constraint::Equal(left, right))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(top)).unwrap();
    let Some(Entity::Arc { center, .. }) = sketch.entity(left).cloned() else {
        panic!("an arc");
    };
    sketch
        .add_constraint(Constraint::Coincident(center, EntityId::ORIGIN))
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: left,
            value: mm(10.0),
        })
        .unwrap();
    let Some(Entity::Arc {
        center: right_center,
        ..
    }) = sketch.entity(right).cloned()
    else {
        panic!("an arc");
    };
    sketch
        .add_constraint(Constraint::Distance {
            from: center,
            to: right_center,
            value: mm(40.0),
        })
        .unwrap();
    (vec![top, bottom, right, left], radius)
}

#[test]
fn a_slot_offset_inside_keeps_round_ends_tangent_to_its_sides_as_its_radius_changes() {
    let mut sketch = Sketch::new(Plane::XY);
    let (curves, radius) = slot(&mut sketch);
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let chain = sketch.offset_chain(&curves).unwrap();
    let inside = chain.side_of(Point2::new(20.0, 0.0));
    assert_eq!(inside, chain.default_side().other());

    let copies = sketch.offset(&curves, inside, 3.0, mm(3.0)).unwrap();

    assert_eq!(copies.len(), 4);
    let arcs: Vec<EntityId> = copies
        .iter()
        .copied()
        .filter(|copy| matches!(sketch.entity(*copy), Some(Entity::Arc { .. })))
        .collect();
    assert_eq!(arcs.len(), 2);
    for arc in &arcs {
        assert!((sketch.circle(*arc).unwrap().1 - 7.0).abs() < EXACT);
    }
    assert_eq!(count_of(&sketch, "Tangent"), 4 + 4);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(15.0)).unwrap();
    let wider = solve(&sketch).geometry;
    for arc in &arcs {
        assert!((wider.circle(*arc).unwrap().1 - 12.0).abs() < EXACT);
    }
    for copy in copies
        .iter()
        .filter(|copy| matches!(wider.entity(**copy), Some(Entity::Line { .. })))
    {
        let (start, end) = line_at(&wider, *copy);
        assert!((start.y.abs() - 12.0).abs() < EXACT && (end.y.abs() - 12.0).abs() < EXACT);
    }
}

fn half_disc(sketch: &mut Sketch) -> (EntityId, EntityId, ConstraintId) {
    let diameter = sketch.add_line(Point2::new(-10.0, 0.0), Point2::new(10.0, 0.0));
    let arc = sketch.add_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        Point2::new(-10.0, 0.0),
    );
    let (left, right) = ends(sketch, diameter);
    let (arc_start, arc_end) = ends(sketch, arc);
    sketch
        .add_constraint(Constraint::Coincident(right, arc_start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(arc_end, left))
        .unwrap();
    let Some(Entity::Arc { center, .. }) = sketch.entity(arc).cloned() else {
        panic!("an arc");
    };
    sketch
        .add_constraint(Constraint::Midpoint {
            point: center,
            curve: diameter,
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(center, EntityId::ORIGIN))
        .unwrap();
    sketch
        .add_constraint(Constraint::Horizontal(diameter))
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: arc,
            value: mm(10.0),
        })
        .unwrap();
    (diameter, arc, radius)
}

#[test]
fn convex_corners_at_an_arc_are_rounded_about_the_corner_and_follow_it() {
    let mut sketch = Sketch::new(Plane::XY);
    let (diameter, arc, radius) = half_disc(&mut sketch);
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let outside = sketch
        .offset_chain(&[diameter, arc])
        .unwrap()
        .side_of(Point2::new(0.0, -5.0));

    let copies = sketch
        .offset(&[diameter, arc], outside, 2.0, mm(2.0))
        .unwrap();

    assert_eq!(copies.len(), 4);
    let corners: Vec<EntityId> = copies
        .iter()
        .copied()
        .filter(|copy| {
            sketch
                .circle(*copy)
                .is_some_and(|(_, radius)| (radius - 2.0).abs() < EXACT)
        })
        .collect();
    assert_eq!(corners.len(), 2);
    let (left, right) = ends(&sketch, diameter);
    let centres: Vec<EntityId> = corners
        .iter()
        .filter_map(|corner| sketch.center_of(*corner))
        .collect();
    assert!(centres.contains(&left) || centres.contains(&right));
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(20.0)).unwrap();
    let wider = solve(&sketch).geometry;
    let grown = copies
        .iter()
        .filter_map(|copy| wider.circle(*copy))
        .map(|(_, radius)| radius)
        .fold(0.0, f64::max);
    assert!((grown - 22.0).abs() < EXACT, "{grown}");
    for corner in &corners {
        let (centre, radius) = wider.circle(*corner).unwrap();
        assert!((radius - 2.0).abs() < EXACT);
        assert!((centre.x.abs() - 20.0).abs() < EXACT && centre.y.abs() < EXACT);
    }
}

#[test]
fn concave_corners_at_an_arc_are_trimmed_to_where_the_offsets_cross() {
    let mut sketch = Sketch::new(Plane::XY);
    let (diameter, arc, _) = half_disc(&mut sketch);
    let inside = sketch
        .offset_chain(&[diameter, arc])
        .unwrap()
        .side_of(Point2::new(0.0, 5.0));

    let copies = sketch
        .offset(&[diameter, arc], inside, 2.0, mm(2.0))
        .unwrap();

    assert_eq!(copies.len(), 2);
    let reach = 60.0_f64.sqrt();
    let line = copies
        .iter()
        .copied()
        .find(|copy| matches!(sketch.entity(*copy), Some(Entity::Line { .. })))
        .unwrap();
    let (start, end) = line_at(&sketch, line);
    let mut xs = [start.x, end.x];
    xs.sort_by(f64::total_cmp);
    assert!((xs[0] + reach).abs() < EXACT && (xs[1] - reach).abs() < EXACT);
    assert!((start.y - 2.0).abs() < EXACT && (end.y - 2.0).abs() < EXACT);
    let solved = solve(&sketch);
    assert_clean(&solved);
}

#[test]
fn a_circle_offset_is_a_circle_on_its_centre_that_keeps_its_distance() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(5.0, 5.0), 10.0);
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: circle,
            value: mm(10.0),
        })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let outside = sketch
        .offset_chain(&[circle])
        .unwrap()
        .side_of(Point2::new(30.0, 5.0));

    let copies = sketch.offset(&[circle], outside, 3.0, mm(3.0)).unwrap();

    let [copy] = copies.as_slice() else {
        panic!("one circle");
    };
    assert_eq!(sketch.center_of(*copy), sketch.center_of(circle));
    assert!((sketch.circle(*copy).unwrap().1 - 13.0).abs() < EXACT);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(20.0)).unwrap();
    assert!((solve(&sketch).geometry.circle(*copy).unwrap().1 - 23.0).abs() < EXACT);
}

#[test]
fn an_open_chain_is_offset_to_the_side_asked_for_with_its_ends_left_free() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
    let second = sketch.add_line(Point2::new(20.0, 0.0), Point2::new(20.0, 20.0));
    let (_, corner) = ends(&sketch, first);
    let (start, _) = ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(corner, start))
        .unwrap();
    let chain = sketch.offset_chain(&[second, first]).unwrap();
    assert!(!chain.is_closed());
    let below = chain.side_of(Point2::new(10.0, -3.0));
    assert_eq!(chain.side_of(Point2::new(10.0, 3.0)), below.other());
    assert!((chain.distance_to(Point2::new(10.0, -3.0)) - 3.0).abs() < EXACT);

    let copies = sketch
        .offset(&[first, second], below, 4.0, mm(4.0))
        .unwrap();

    let points: Vec<Point2> = copies
        .iter()
        .flat_map(|copy| {
            let (a, b) = line_at(&sketch, *copy);
            [a, b]
        })
        .collect();
    assert!(
        points
            .iter()
            .any(|point| point.distance(Point2::new(24.0, -4.0)) < EXACT)
    );
    assert!(has(
        &sketch,
        &Constraint::Distance {
            from: first,
            to: copies[0],
            value: mm(4.0)
        }
    ));
    assert_clean(&solve(&sketch));
}

#[test]
fn an_offset_that_shrinks_an_arc_to_nothing_or_crosses_itself_is_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let label = sketch.entity_label(circle);
    let inside = sketch
        .offset_chain(&[circle])
        .unwrap()
        .side_of(Point2::ZERO);
    assert_eq!(
        sketch.offset(&[circle], inside, 5.0, mm(5.0)),
        Err(OffsetError::Collapses {
            entity: circle,
            label: label.clone(),
        })
    );
    assert_eq!(
        sketch
            .offset(&[circle], inside, 6.0, mm(6.0))
            .unwrap_err()
            .to_string(),
        format!("offsetting {label} this far would shrink it to nothing")
    );
    assert_eq!(
        sketch.offset(&[circle], inside, 0.0, mm(0.0)),
        Err(OffsetError::NotPositive)
    );

    let Rectangle { sketch, sides, .. } = rectangle(40.0, 4.0);
    let inside = sketch.offset_chain(&sides).unwrap().default_side().other();
    let refused = sketch.clone().offset(&sides, inside, 3.0, mm(3.0));
    assert!(
        matches!(
            refused,
            Err(OffsetError::UsedUp { .. } | OffsetError::CrossesItself)
        ),
        "{refused:?}"
    );
}

#[test]
fn only_one_chain_of_lines_arcs_or_a_circle_can_be_offset() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let other = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 5.0));
    let circle = sketch.add_circle(Point2::new(30.0, 0.0), 3.0);
    let spline = sketch.add_spline(&[Point2::new(0.0, 20.0), Point2::new(5.0, 25.0)]);
    let branch = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    let fork = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(20.0, -10.0));

    assert_eq!(
        sketch.offset_chain(&[line, other]),
        Err(OffsetError::SeveralChains { count: 2 })
    );
    assert_eq!(
        sketch.offset_chain(&[line, circle]),
        Err(OffsetError::SeveralChains { count: 2 })
    );
    assert_eq!(
        sketch.offset_chain(&[spline]).unwrap_err().to_string(),
        format!(
            "{} cannot be offset; only lines, arcs and circles can",
            sketch.entity_label(spline)
        )
    );
    assert_eq!(
        sketch.offset_chain(&[line, branch, fork]),
        Err(OffsetError::Branches)
    );
    assert_eq!(
        sketch.offset_chain(&[EntityId::HORIZONTAL_AXIS]),
        Err(OffsetError::NothingSelected)
    );
    assert!(matches!(
        sketch.offset_chain(&[line, branch]),
        Ok(chain) if chain.curves().len() == 2
    ));
}

#[test]
fn clicking_one_curve_takes_the_chain_it_belongs_to_up_to_a_branch() {
    let Rectangle { sketch, sides, .. } = rectangle(40.0, 20.0);
    let mut chain = sketch.offset_chain_through(sides[2]);
    chain.sort();
    assert_eq!(chain, sides.to_vec());

    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    let third = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let spur = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(10.0, 20.0));
    let mut chain = sketch.offset_chain_through(first);
    chain.sort();
    assert_eq!(chain, vec![first, second]);
    assert_eq!(sketch.offset_chain_through(third), vec![third]);
    assert_eq!(sketch.offset_chain_through(spur), vec![spur]);
}

#[test]
fn a_failed_offset_leaves_the_sketch_as_it_was() {
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 5.0);
    let before = sketch.clone();

    let refused = sketch.offset(&[circle], Side::Left, 10.0, mm(10.0));

    assert!(matches!(refused, Err(OffsetError::Collapses { .. })));
    assert_eq!(sketch, before);
}

#[test]
fn a_lens_of_two_arcs_offset_outwards_closes_without_a_redundant_constraint() {
    let mut sketch = Sketch::new(Plane::XY);
    let left = Point2::new(-10.0, 0.0);
    let right = Point2::new(10.0, 0.0);
    let upper = sketch.add_arc(Point2::new(0.0, -5.0), right, left);
    let lower = sketch.add_arc(Point2::new(0.0, 5.0), left, right);
    let (upper_start, upper_end) = ends(&sketch, upper);
    let (lower_start, lower_end) = ends(&sketch, lower);
    sketch
        .add_constraint(Constraint::Coincident(upper_end, lower_start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Coincident(lower_end, upper_start))
        .unwrap();
    let radius = sketch
        .add_constraint(Constraint::Radius {
            entity: upper,
            value: mm(125.0_f64.sqrt()),
        })
        .unwrap();
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let outside = sketch.offset_chain(&[upper, lower]).unwrap().default_side();

    let copies = sketch
        .offset(&[upper, lower], outside, 2.0, mm(2.0))
        .unwrap();

    assert_eq!(copies.len(), 4);
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);

    sketch.set_dimension(radius, mm(12.0)).unwrap();
    let changed = solve(&sketch);
    assert_clean(&changed);
    let original = changed.geometry.circle(upper).unwrap().1;
    let grown = copies
        .iter()
        .filter_map(|copy| changed.geometry.circle(*copy))
        .map(|(_, radius)| radius)
        .fold(0.0, f64::max);
    assert!((original - 12.0).abs() < EXACT);
    assert!(grown > 12.0, "{grown}");
}

#[test]
fn a_rounded_rectangle_offset_inwards_keeps_every_corner_concentric() {
    let mut sketch = Sketch::new(Plane::XY);
    let sides = [
        sketch.add_line(Point2::new(5.0, 0.0), Point2::new(35.0, 0.0)),
        sketch.add_line(Point2::new(40.0, 5.0), Point2::new(40.0, 15.0)),
        sketch.add_line(Point2::new(35.0, 20.0), Point2::new(5.0, 20.0)),
        sketch.add_line(Point2::new(0.0, 15.0), Point2::new(0.0, 5.0)),
    ];
    let corners = [
        sketch.add_arc(
            Point2::new(35.0, 5.0),
            Point2::new(35.0, 0.0),
            Point2::new(40.0, 5.0),
        ),
        sketch.add_arc(
            Point2::new(35.0, 15.0),
            Point2::new(40.0, 15.0),
            Point2::new(35.0, 20.0),
        ),
        sketch.add_arc(
            Point2::new(5.0, 15.0),
            Point2::new(5.0, 20.0),
            Point2::new(0.0, 15.0),
        ),
        sketch.add_arc(
            Point2::new(5.0, 5.0),
            Point2::new(0.0, 5.0),
            Point2::new(5.0, 0.0),
        ),
    ];
    for index in 0..4 {
        let (_, side_end) = ends(&sketch, sides[index]);
        let (arc_start, arc_end) = ends(&sketch, corners[index]);
        let (next_start, _) = ends(&sketch, sides[(index + 1) % 4]);
        sketch
            .add_constraint(Constraint::Coincident(side_end, arc_start))
            .unwrap();
        sketch
            .add_constraint(Constraint::Coincident(arc_end, next_start))
            .unwrap();
        sketch
            .add_constraint(Constraint::Tangent(sides[index], corners[index]))
            .unwrap();
        sketch
            .add_constraint(Constraint::Tangent(corners[index], sides[(index + 1) % 4]))
            .unwrap();
    }
    let freedom = solve(&sketch).solution.degrees_of_freedom();
    let curves: Vec<EntityId> = sides.iter().chain(&corners).copied().collect();
    let inside = sketch
        .offset_chain(&curves)
        .unwrap()
        .side_of(Point2::new(20.0, 10.0));

    let copies = sketch.offset(&curves, inside, 2.0, mm(2.0)).unwrap();

    assert_eq!(copies.len(), 8);
    for copy in &copies {
        if let Some((_, radius)) = sketch.circle(*copy) {
            assert!((radius - 3.0).abs() < EXACT);
        }
    }
    let solved = solve(&sketch);
    assert_clean(&solved);
    assert_eq!(solved.solution.degrees_of_freedom(), freedom);
    assert_eq!(count_of(&sketch, "Distance"), 1);
}

fn gap_to(sketch: &Sketch, ellipse: EntityId, point: Point2) -> f64 {
    sketch
        .closest_on_ellipse(ellipse, point)
        .unwrap()
        .distance(point)
}

fn assert_spline_at_distance(sketch: &Sketch, spline: EntityId, ellipse: EntityId, distance: f64) {
    let curve = sketch.spline(spline).unwrap();
    for index in 0..=200 {
        let point = curve.point_at(f64::from(index) / 200.0);
        let gap = gap_to(sketch, ellipse, point);
        assert!((gap - distance).abs() < 1e-3, "{gap} at {point}");
    }
}

#[test]
fn an_ellipse_is_offset_either_way_as_a_closed_fit_point_spline_within_tolerance() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::ZERO, Point2::new(10.0, 0.0), 4.0);
    let chain = sketch.offset_chain(&[ellipse]).unwrap();

    assert!(!chain.follows());
    assert!(chain.is_closed());
    assert_eq!(chain.side_of(Point2::new(0.0, 6.0)), Side::Right);
    assert_eq!(chain.side_of(Point2::new(0.0, 3.0)), Side::Left);
    assert_eq!(chain.default_side(), Side::Right);
    assert_eq!(chain.outline(Side::Right, 2.0).unwrap().curve_count(), 1);
    assert!(matches!(
        chain.outline(Side::Left, 1.7),
        Err(OffsetError::Collapses { .. })
    ));

    for (side, distance) in [(Side::Right, 2.0), (Side::Left, 1.0)] {
        let mut copy = sketch.clone();
        let made = copy
            .offset(&[ellipse], side, distance, mm(distance))
            .unwrap();
        let [spline] = made.as_slice() else {
            panic!("expected one spline, found {made:?}");
        };
        assert!(matches!(
            copy.entity(*spline),
            Some(Entity::Spline { kind, .. }) if kind.is_closed()
        ));
        assert_spline_at_distance(&copy, *spline, ellipse, distance);
        let solved = solve(&copy);
        assert!(solved.solution.redundancies().is_empty());
    }
}

#[test]
fn an_elliptical_arc_is_offset_as_an_open_spline_and_an_ellipse_in_a_chain_is_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_elliptical_arc(
        Point2::ZERO,
        Point2::new(10.0, 0.0),
        4.0,
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 4.0),
    );
    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(20.0, 0.0));

    assert_eq!(sketch.offset_chain_through(arc), vec![arc]);
    let refused = sketch.offset_chain(&[arc, line]).unwrap_err();
    assert!(matches!(refused, OffsetError::EllipseNotOffsettable { .. }));
    assert!(refused.to_string().contains("on its own"), "{refused}");

    let made = sketch.offset(&[arc], Side::Right, 1.5, mm(1.5)).unwrap();
    let [spline] = made.as_slice() else {
        panic!("expected one spline, found {made:?}");
    };
    let Some(Entity::Spline { points, kind }) = sketch.entity(*spline) else {
        panic!("expected a spline");
    };
    assert!(!kind.is_closed());
    let first = sketch.point(points[0]).unwrap();
    assert!(first.distance(Point2::new(11.5, 0.0)) < EXACT, "{first}");
    assert_spline_at_distance(&sketch, *spline, arc, 1.5);
}
