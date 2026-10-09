use caditor_expression::{EvalError, ParameterId, Quantity};
use caditor_geometry::{Plane, Point2, Vector2};

use crate::{BlendEnd, BlendError, Constraint, Continuity, Entity, EntityId, Sketch, Solved};

const EXACT: f64 = 1e-6;

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Ok(Quantity::plain(0.0))
}

fn solve(sketch: &Sketch) -> Solved {
    match sketch.solve(&no_parameters, &|| false) {
        Ok(solved) => solved,
        Err(error) => panic!("the sketch does not solve: {error:?}"),
    }
}

fn end_of(sketch: &Sketch, curve: EntityId, last: bool) -> BlendEnd {
    let [start, end] = sketch.ends_of_curve(curve).unwrap();
    BlendEnd {
        curve,
        point: if last { end } else { start },
    }
}

fn points_of(sketch: &Sketch, spline: EntityId) -> Vec<Point2> {
    sketch.spline(spline).unwrap().control_points().to_vec()
}

fn curvature(sketch: &Sketch, spline: EntityId, parameter: f64) -> f64 {
    let [tangent, bend] = sketch.spline(spline).unwrap().derivatives(parameter);
    tangent.perp_dot(bend) / tangent.length().powi(3)
}

fn leaving(sketch: &Sketch, spline: EntityId, at_start: bool) -> Vector2 {
    let [tangent, _] = sketch
        .spline(spline)
        .unwrap()
        .derivatives(if at_start { 0.0 } else { 1.0 });
    tangent.normalize()
}

fn parallel(a: Vector2, b: Vector2) -> bool {
    a.normalize().perp_dot(b.normalize()).abs() < EXACT
}

fn count(sketch: &Sketch, kind: &str) -> usize {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == kind)
        .count()
}

#[test]
fn a_tangent_blend_joins_two_lines_and_follows_when_one_turns() {
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_line(Point2::new(-40.0, 0.0), Point2::new(-10.0, 0.0));
    let right = sketch.add_line(Point2::new(10.0, 20.0), Point2::new(10.0, 50.0));
    let (first, second) = (end_of(&sketch, left, true), end_of(&sketch, right, false));

    let preview = sketch
        .blend_curve(first, second, Continuity::Tangent)
        .unwrap();
    let made = sketch.blend(first, second, Continuity::Tangent).unwrap();

    assert_eq!(preview.control_points, points_of(&sketch, made));
    assert_eq!(preview.control_points.len(), 4);
    assert_eq!(count(&sketch, "Coincident"), 2);
    assert_eq!(count(&sketch, "Tangent"), 2);
    assert_eq!(count(&sketch, "Curvature"), 0);
    let solved = solve(&sketch);
    assert!(solved.solution.redundancies().is_empty());
    for (before, after) in preview
        .control_points
        .iter()
        .zip(points_of(&solved.geometry, made))
    {
        assert!(before.distance(after) < EXACT, "{before} moved to {after}");
    }

    let [far, _] = sketch.ends_of_curve(left).unwrap();
    sketch
        .add_constraint(Constraint::Fix {
            point: far,
            at: Point2::new(-35.0, -25.0),
        })
        .unwrap();
    let moved = solve(&sketch).geometry;
    let (start, end) = moved.line_endpoints(left).unwrap();
    let blended = points_of(&moved, made);
    assert!(start.distance(Point2::new(-35.0, -25.0)) < EXACT);
    assert!(blended.first().unwrap().distance(end) < EXACT);
    assert!(parallel(leaving(&moved, made, true), end - start));
    let (low, _) = moved.line_endpoints(right).unwrap();
    assert!(blended.last().unwrap().distance(low) < EXACT);
}

#[test]
fn a_curvature_blend_matches_an_arc_and_a_line_and_keeps_matching_when_they_move() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(-60.0, 0.0), Point2::new(-20.0, 0.0));
    let arc = sketch.add_arc(
        Point2::new(30.0, 10.0),
        Point2::new(40.0, 10.0),
        Point2::new(30.0, 20.0),
    );
    let (first, second) = (end_of(&sketch, line, true), end_of(&sketch, arc, false));

    let made = sketch.blend(first, second, Continuity::Curvature).unwrap();

    assert_eq!(points_of(&sketch, made).len(), 6);
    assert_eq!(count(&sketch, "Curvature"), 2);
    assert!(curvature(&sketch, made, 0.0).abs() < EXACT);
    assert!((curvature(&sketch, made, 1.0) - 0.1).abs() < EXACT);
    let solved = solve(&sketch);
    assert!(solved.solution.redundancies().is_empty());
    assert!((curvature(&solved.geometry, made, 1.0) - 0.1).abs() < EXACT);

    let Some(Entity::Arc { center, .. }) = sketch.entity(arc).cloned() else {
        panic!("the arc is gone");
    };
    sketch
        .add_constraint(Constraint::Fix {
            point: center,
            at: Point2::new(35.0, 25.0),
        })
        .unwrap();
    let moved = solve(&sketch).geometry;
    let radius = moved.arc(arc).unwrap().radius;
    assert!(curvature(&moved, made, 0.0).abs() < EXACT);
    assert!((curvature(&moved, made, 1.0) - 1.0 / radius).abs() < EXACT);
    let (start, end) = moved.line_endpoints(line).unwrap();
    assert!(parallel(leaving(&moved, made, true), end - start));
}

#[test]
fn a_curvature_blend_from_a_tight_arc_keeps_its_legs_short() {
    let mut sketch = Sketch::new(Plane::XY);
    let arc = sketch.add_arc(Point2::ZERO, Point2::new(1.0, 0.0), Point2::new(0.0, 1.0));
    let spline = sketch.add_spline(&[
        Point2::new(200.0, 0.0),
        Point2::new(220.0, 30.0),
        Point2::new(260.0, 20.0),
    ]);
    let (first, second) = (end_of(&sketch, arc, true), end_of(&sketch, spline, false));

    let preview = sketch
        .blend_curve(first, second, Continuity::Curvature)
        .unwrap();
    let made = sketch.blend(first, second, Continuity::Curvature).unwrap();

    let reach = preview
        .control_points
        .iter()
        .take(3)
        .map(|point| point.distance(Point2::new(1.0, 0.0)))
        .fold(0.0, f64::max);
    assert!(reach < 10.0, "{reach}");
    assert!((curvature(&sketch, made, 0.0) - 1.0).abs() < EXACT);
    let own = curvature(&sketch, spline, 0.0);
    assert!((curvature(&sketch, made, 1.0) - own).abs() < EXACT);
    let solved = solve(&sketch);
    assert!((curvature(&solved.geometry, made, 0.0) - 1.0).abs() < EXACT);
}

#[test]
fn ends_that_cannot_be_blended_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let circle = sketch.add_circle(Point2::new(30.0, 0.0), 5.0);
    let touching = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    let centre = sketch.center_of(circle).unwrap();
    let tangent = Continuity::Tangent;

    let on_circle = sketch.blend_curve(
        end_of(&sketch, line, true),
        BlendEnd {
            curve: circle,
            point: centre,
        },
        tangent,
    );
    let same = sketch.blend_curve(
        end_of(&sketch, line, false),
        end_of(&sketch, line, true),
        tangent,
    );
    let meeting = sketch.blend_curve(
        end_of(&sketch, line, true),
        end_of(&sketch, touching, false),
        tangent,
    );
    let not_an_end = sketch.blend_curve(
        BlendEnd {
            curve: line,
            point: centre,
        },
        end_of(&sketch, touching, true),
        tangent,
    );

    assert!(matches!(on_circle, Err(BlendError::NoEnds { .. })));
    assert!(matches!(same, Err(BlendError::SameCurve { .. })));
    assert!(matches!(meeting, Err(BlendError::EndsMeet { .. })));
    assert!(matches!(not_an_end, Err(BlendError::NotAnEnd { .. })));
    assert_eq!(
        on_circle.unwrap_err().to_string(),
        format!(
            "{} has no end to blend from; choose the end of a line, arc or spline",
            sketch.entity_label(circle)
        )
    );
}

#[test]
fn ends_of_curves_already_joined_elsewhere_are_refused() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(10.0, 10.0));
    let [_, corner] = sketch.ends_of_curve(first).unwrap();
    let [joined, _] = sketch.ends_of_curve(second).unwrap();
    sketch
        .add_constraint(Constraint::Coincident(corner, joined))
        .unwrap();
    let before = sketch.clone();

    let refused = sketch.blend(
        end_of(&sketch, first, true),
        end_of(&sketch, second, true),
        Continuity::Tangent,
    );

    assert!(matches!(refused, Err(BlendError::AlreadyJoined { .. })));
    assert!(sketch.same_content(&before));
}

#[test]
fn a_curvature_blend_cannot_meet_a_straight_spline() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let straight = sketch.add_spline(&[Point2::new(30.0, 10.0), Point2::new(40.0, 20.0)]);

    let tangent = sketch.clone().blend(
        end_of(&sketch, line, true),
        end_of(&sketch, straight, false),
        Continuity::Tangent,
    );
    let curving = sketch.blend(
        end_of(&sketch, line, true),
        end_of(&sketch, straight, false),
        Continuity::Curvature,
    );

    assert!(tangent.is_ok());
    assert!(matches!(curving, Err(BlendError::Edit(_))));
}
