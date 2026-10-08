use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, Dimensioned, EntityId, Sketch, Solved, TangentError};

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

fn radius(value: f64) -> Dimensioned {
    Dimensioned {
        expression: Expression::Measure(value, Unit::Millimetre),
        value,
    }
}

fn assert_holds(sketch: &Sketch, circle: EntityId, centre: Point2, size: f64) {
    let solved = solve(sketch);
    assert!(
        solved.solution.redundancies().is_empty(),
        "redundant: {:?}",
        solved.solution.redundancies()
    );
    let (found, found_radius) = solved.geometry.circle(circle).unwrap();
    assert!(found.distance(centre) < EXACT, "{found} is not {centre}");
    assert!((found_radius - size).abs() < EXACT, "{found_radius}");
}

fn triangle(sketch: &mut Sketch) -> [EntityId; 3] {
    [
        sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0)),
        sketch.add_line(Point2::ZERO, Point2::new(0.0, 30.0)),
        sketch.add_line(Point2::new(40.0, 0.0), Point2::new(0.0, 30.0)),
    ]
}

#[test]
fn three_lines_have_an_inscribed_circle_and_circles_outside_chosen_by_the_pointer() {
    let mut sketch = Sketch::new(Plane::XY);
    let lines = triangle(&mut sketch);

    let inscribed = sketch
        .tangent_circle_near(&lines, None, Some(Point2::new(12.0, 8.0)))
        .unwrap();
    let outside = sketch
        .tangent_circle_near(&lines, None, Some(Point2::new(80.0, 80.0)))
        .unwrap();

    assert!(inscribed.center.distance(Point2::new(10.0, 10.0)) < EXACT);
    assert!((inscribed.radius - 10.0).abs() < EXACT);
    assert!(outside.center.x > 40.0 && outside.center.y > 30.0);

    let made = sketch
        .tangent_circle(&lines, None, Some(Point2::new(12.0, 8.0)))
        .unwrap();
    assert_holds(&sketch, made, Point2::new(10.0, 10.0), 10.0);
    assert_eq!(
        sketch
            .constraints()
            .filter(|(_, constraint)| matches!(constraint, Constraint::Tangent(_, circle) if *circle == made))
            .count(),
        3
    );
}

#[test]
fn two_lines_and_a_radius_give_the_circle_in_the_corner_nearest_the_pointer() {
    let mut sketch = Sketch::new(Plane::XY);
    let across = sketch.add_line(Point2::new(-50.0, 0.0), Point2::new(50.0, 0.0));
    let up = sketch.add_line(Point2::new(0.0, -50.0), Point2::new(0.0, 50.0));
    let freedom = solve(&sketch).solution.degrees_of_freedom();

    let made = sketch
        .tangent_circle(
            &[across, up],
            Some(&radius(5.0)),
            Some(Point2::new(-10.0, 10.0)),
        )
        .unwrap();

    assert_holds(&sketch, made, Point2::new(-5.0, 5.0), 5.0);
    assert_eq!(solve(&sketch).solution.degrees_of_freedom(), freedom);
    assert!(sketch.constraints().any(|(_, constraint)| matches!(
        constraint,
        Constraint::Radius { entity, .. } if *entity == made
    )));
}

#[test]
fn a_circle_can_touch_the_axes_and_a_line_at_once() {
    let mut sketch = Sketch::new(Plane::XY);
    let slanted = sketch.add_line(Point2::new(40.0, 0.0), Point2::new(0.0, 40.0));

    let made = sketch
        .tangent_circle(
            &[EntityId::HORIZONTAL_AXIS, EntityId::VERTICAL_AXIS, slanted],
            None,
            Some(Point2::new(10.0, 10.0)),
        )
        .unwrap();

    let expected = 40.0 / (2.0 + std::f64::consts::SQRT_2);
    assert_holds(&sketch, made, Point2::new(expected, expected), expected);
}

#[test]
fn circles_arcs_and_lines_mix_and_each_circle_found_touches_all_three() {
    let mut sketch = Sketch::new(Plane::XY);
    let small = sketch.add_circle(Point2::new(0.0, 0.0), 10.0);
    let arc = sketch.add_arc(
        Point2::new(60.0, 0.0),
        Point2::new(70.0, 0.0),
        Point2::new(60.0, 10.0),
    );
    let line = sketch.add_line(Point2::new(-20.0, 40.0), Point2::new(90.0, 40.0));
    let curves = [small, arc, line];

    let mut found = Vec::new();
    for near in [
        Point2::new(30.0, 20.0),
        Point2::new(30.0, -30.0),
        Point2::new(30.0, 60.0),
        Point2::new(30.0, 0.0),
        Point2::new(200.0, 0.0),
        Point2::new(-200.0, 0.0),
    ] {
        if let Ok(circle) = sketch.tangent_circle_near(&curves, None, Some(near)) {
            found.push(circle);
        }
    }

    assert!(found.len() >= 2);
    for circle in found {
        let mut working = sketch.clone();
        let made = working
            .tangent_circle(&curves, None, Some(circle.center))
            .unwrap();
        assert_holds(&working, made, circle.center, circle.radius);
    }
}

#[test]
fn a_line_and_a_circle_with_a_radius_touch_it_inside_or_outside_by_the_pointer() {
    let mut sketch = Sketch::new(Plane::XY);
    let ring = sketch.add_circle(Point2::ZERO, 20.0);
    let line = sketch.add_line(Point2::new(-40.0, -15.0), Point2::new(40.0, -15.0));

    let inside = sketch
        .tangent_circle_near(&[ring, line], Some(5.0), Some(Point2::new(12.0, -10.0)))
        .unwrap();
    let outside = sketch
        .tangent_circle_near(&[ring, line], Some(5.0), Some(Point2::new(16.0, -20.0)))
        .unwrap();

    assert!(inside.center.distance(Point2::new(125.0_f64.sqrt(), -10.0)) < EXACT);
    assert!(outside.center.distance(Point2::new(15.0, -20.0)) < EXACT);
}

#[test]
fn choices_that_cannot_make_a_circle_are_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let lines = triangle(&mut sketch);
    let point = sketch.add_point(Point2::new(5.0, 5.0));
    let low = sketch.add_line(Point2::new(0.0, 100.0), Point2::new(40.0, 100.0));
    let high = sketch.add_line(Point2::new(0.0, 110.0), Point2::new(40.0, 110.0));
    let before = sketch.clone();

    assert_eq!(
        sketch.tangent_circle_near(&lines[..1], None, None),
        Err(TangentError::CurveCount { count: 1 })
    );
    assert_eq!(
        sketch.tangent_circle_near(&lines[..2], None, None),
        Err(TangentError::NeedsRadius)
    );
    assert_eq!(
        sketch.tangent_circle_near(&lines, Some(3.0), None),
        Err(TangentError::RadiusWithThree)
    );
    assert!(matches!(
        sketch.tangent_circle_near(&[lines[0], lines[0]], Some(3.0), None),
        Err(TangentError::Repeated { .. })
    ));
    assert!(matches!(
        sketch.tangent_circle_near(&[lines[0], point], Some(3.0), None),
        Err(TangentError::NotCurve { .. })
    ));
    assert_eq!(
        sketch.tangent_circle_near(&lines[..2], Some(0.0), None),
        Err(TangentError::RadiusNotPositive)
    );
    assert!(matches!(
        sketch.tangent_circle_near(&[low, high], Some(3.0), None),
        Err(TangentError::NoCircleOfRadius { .. })
    ));
    assert!(matches!(
        sketch.tangent_circle_near(&[low, high, lines[0]], None, None),
        Err(TangentError::NoCircle { .. })
    ));
    assert!(sketch.same_content(&before));
}
