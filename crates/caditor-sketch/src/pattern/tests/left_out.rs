use super::*;

fn leaving(pattern: RectangularPattern, left_out: &[&[i64]]) -> RectangularPattern {
    RectangularPattern {
        left_out: left_out.iter().map(|steps| steps.to_vec()).collect(),
        ..pattern
    }
}

fn turning(pattern: CircularPattern, left_out: &[i64]) -> CircularPattern {
    CircularPattern {
        left_out: left_out.iter().copied().collect(),
        ..pattern
    }
}

fn square_grid(columns: usize, rows: usize, spacing: f64) -> RectangularPattern {
    grid(row(columns, spacing, 0.0), Some(row(rows, spacing, 90.0)))
}

fn corners_of(sketch: &Sketch, lines: &[EntityId]) -> Vec<Point2> {
    lines
        .iter()
        .map(|line| sketch.point(ends(sketch, *line).0).unwrap())
        .collect()
}

fn assert_holds_its_freedom(sketch: &Sketch, free: usize) -> Solved {
    let solved = solve(sketch);

    assert_clean(&solved);

    assert_eq!(solved.solution.degrees_of_freedom(), free);
    solved
}

#[test]
fn a_copy_left_out_in_the_middle_of_a_row_ties_the_next_to_the_one_before_by_two_steps() {
    let mut sketch = Sketch::new(Plane::XY);

    let original = rectangle(&mut sketch, Point2::ZERO, 10.0, 6.0);
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .rectangular_pattern(&original.lines, &leaving(across(5, 25.0), &[&[2]]))
        .unwrap();

    assert_eq!(copies.len(), 3 * 4);

    let starts = corners_of(&sketch, &[copies[0], copies[4], copies[8]]);

    for (start, expected) in starts.iter().zip([25.0, 75.0, 100.0]) {
        assert_near(*start, Point2::new(expected, 0.0));
    }

    assert!(sketch.constraints().any(|(_, constraint)| matches!(
        constraint,
        Constraint::HorizontalDistance { value, .. }
            if *value == Expression::binary(
                caditor_expression::BinaryOperator::Multiply,
                Expression::Number(2.0),
                mm(25.0),
            )
    )));

    assert_holds_its_freedom(&sketch, free);

    sketch.set_dimension(original.width, mm(14.0)).unwrap();

    let wider = solve(&sketch).geometry;
    let x = |point: EntityId| wider.point(point).unwrap().x;
    let (home, _) = ends(&wider, original.lines[0]);

    for (copy, steps) in [(copies[0], 1.0), (copies[4], 3.0), (copies[8], 4.0)] {
        let (start, end) = ends(&wider, copy);

        assert!((x(end) - x(start) - 14.0).abs() < EXACT);
        assert!((x(start) - x(home) - 25.0 * steps).abs() < EXACT);
    }
}

#[test]
fn a_spacing_naming_a_parameter_still_moves_every_kept_copy() {
    let mut sketch = Sketch::new(Plane::XY);

    let point = sketch.add_point(Point2::ZERO);
    let pattern = RectangularPattern {
        directions: vec![PatternRow::along(
            5,
            Dimensioned {
                expression: Expression::Parameter(SPACING),
                value: 10.0,
            },
            degrees(0.0),
        )],
        distribution: Distribution::Spacing,
        left_out: BTreeSet::from([vec![1], vec![2]]),
    };

    let copies = sketch.rectangular_pattern(&[point], &pattern).unwrap();

    assert_eq!(copies.len(), 2);

    let wider = solve_with(&sketch, 12.5).geometry;
    let home = wider.point(point).unwrap();
    for (copy, steps) in copies.iter().zip([3.0, 4.0]) {
        assert_near(
            wider.point(*copy).unwrap(),
            home + Point2::new(12.5 * steps, 0.0),
        );
    }
}

#[test]
fn the_last_copy_left_out_ends_the_row_one_step_sooner() {
    let mut sketch = Sketch::new(Plane::XY);

    let original = rectangle(&mut sketch, Point2::new(3.0, 4.0), 10.0, 6.0);
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .rectangular_pattern(&original.lines, &leaving(across(4, 20.0), &[&[3]]))
        .unwrap();

    assert_eq!(copies.len(), 2 * 4);

    assert_near(corners_of(&sketch, &[copies[4]])[0], Point2::new(43.0, 4.0));

    assert_holds_its_freedom(&sketch, free);
}

#[test]
fn a_whole_row_and_column_left_out_crossing_keeps_the_far_copies_tied() {
    let mut sketch = Sketch::new(Plane::XY);

    let original = rectangle(&mut sketch, Point2::ZERO, 10.0, 6.0);
    let free = solve(&sketch).solution.degrees_of_freedom();
    let pattern = leaving(
        square_grid(3, 3, 20.0),
        &[&[1, 0], &[1, 1], &[1, 2], &[0, 1], &[2, 1]],
    );

    let copies = sketch
        .rectangular_pattern(&original.lines, &pattern)
        .unwrap();

    assert_eq!(copies.len(), 3 * 4);

    let starts = corners_of(&sketch, &[copies[0], copies[4], copies[8]]);

    for (start, expected) in starts.iter().zip([
        Point2::new(40.0, 0.0),
        Point2::new(0.0, 40.0),
        Point2::new(40.0, 40.0),
    ]) {
        assert_near(*start, expected);
    }

    assert_holds_its_freedom(&sketch, free);

    sketch.set_dimension(original.width, mm(12.0)).unwrap();

    let wider = solve(&sketch).geometry;
    let (home, _) = ends(&wider, original.lines[0]);
    let (far, far_end) = ends(&wider, copies[8]);

    let home = wider.point(home).unwrap();

    assert_near(wider.point(far).unwrap(), home + Point2::new(40.0, 40.0));
    assert_near(
        wider.point(far_end).unwrap(),
        home + Point2::new(52.0, 40.0),
    );
}

#[test]
fn a_copy_with_its_row_and_column_toward_the_original_left_out_is_tied_across_both_steps() {
    let mut sketch = Sketch::new(Plane::XY);

    let original = rectangle(&mut sketch, Point2::ZERO, 10.0, 6.0);
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .rectangular_pattern(
            &original.lines,
            &leaving(square_grid(2, 2, 20.0), &[&[1, 0], &[0, 1]]),
        )
        .unwrap();

    assert_eq!(copies.len(), 4);

    assert_near(corners_of(&sketch, &copies)[0], Point2::new(20.0, 20.0));

    assert_holds_its_freedom(&sketch, free);

    let mut slanted = Sketch::new(Plane::XY);

    let point = slanted.add_point(Point2::new(1.0, 2.0));
    let free = solve(&slanted).solution.degrees_of_freedom();
    let pattern = leaving(
        grid(row(3, 10.0, 30.0), Some(row(3, 8.0, 100.0))),
        &[&[1, 0], &[2, 0], &[0, 1], &[0, 2], &[1, 1]],
    );

    let copies = slanted.rectangular_pattern(&[point], &pattern).unwrap();

    assert_eq!(copies.len(), 3);

    let solved = assert_holds_its_freedom(&slanted, free);
    let step = |angle: f64, spacing: f64| Point2::from_angle(angle.to_radians()) * spacing;

    for (copy, (columns, rows)) in copies.iter().zip([(2.0, 1.0), (1.0, 2.0), (2.0, 2.0)]) {
        assert_near(
            solved.geometry.point(*copy).unwrap(),
            Point2::new(1.0, 2.0) + step(30.0, 10.0) * columns + step(100.0, 8.0) * rows,
        );
    }
}

#[test]
fn arcs_with_copies_left_out_repeat_without_a_redundant_tie() {
    let mut sketch = Sketch::new(Plane::XY);

    let arc = quarter_arc(&mut sketch);
    let free = solve(&sketch).solution.degrees_of_freedom();

    sketch
        .rectangular_pattern(
            &[arc],
            &leaving(square_grid(3, 2, 30.0), &[&[1, 0], &[0, 1]]),
        )
        .unwrap();

    assert_holds_its_freedom(&sketch, free);

    let mut corner = Sketch::new(Plane::XY);

    let arc = slanted_arc(&mut corner);
    let free = solve(&corner).solution.degrees_of_freedom();

    corner
        .rectangular_pattern(
            &[arc],
            &leaving(square_grid(2, 2, 30.0), &[&[1, 0], &[0, 1]]),
        )
        .unwrap();

    assert_holds_its_freedom(&corner, free);

    let mut turned = Sketch::new(Plane::XY);

    let arc = slanted_arc(&mut turned);
    let free = solve(&turned).solution.degrees_of_freedom();

    turned
        .circular_pattern(&[arc], EntityId::ORIGIN, &turning(turns(6), &[1, 2]))
        .unwrap();

    assert_holds_its_freedom(&turned, free);
}

#[test]
fn copies_along_lines_find_a_kept_neighbour_beyond_and_refuse_when_none_is_in_line() {
    let mut sketch = Sketch::new(Plane::XY);

    let guide = sketch.add_line(Point2::new(-20.0, -20.0), Point2::new(-10.0, -15.0));
    let point = sketch.add_point(Point2::new(1.0, 1.0));
    let free = solve(&sketch).solution.degrees_of_freedom();
    let along = grid(
        along_line(3, 10.0, guide, false),
        Some(along_line(3, 10.0, guide, true)),
    );

    let copies = sketch
        .rectangular_pattern(&[point], &leaving(along.clone(), &[&[1, 0], &[0, 1]]))
        .unwrap();

    assert_eq!(copies.len(), 6);

    assert_holds_its_freedom(&sketch, free);

    let mut cut_off = Sketch::new(Plane::XY);

    let guide = cut_off.add_line(Point2::new(-20.0, -20.0), Point2::new(-10.0, -15.0));
    let point = cut_off.add_point(Point2::new(1.0, 1.0));
    let before = cut_off.clone();
    let corner = grid(
        along_line(2, 10.0, guide, false),
        Some(along_line(2, 10.0, guide, true)),
    );

    assert_eq!(
        cut_off.rectangular_pattern(&[point], &leaving(corner, &[&[1, 0], &[0, 1]])),
        Err(PatternError::CopyCutOff {
            copy: PatternCopy::Steps(vec![1, 1])
        })
    );
    assert_eq!(cut_off, before);
}

#[test]
fn a_circular_pattern_leaves_out_turns_and_ties_the_rest_by_whole_steps() {
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(14.0, 0.0));
    let free = solve(&sketch).solution.degrees_of_freedom();

    let copies = sketch
        .circular_pattern(&[line], EntityId::ORIGIN, &turning(turns(6), &[2, 3]))
        .unwrap();

    assert_eq!(copies.len(), 3);

    let solved = assert_holds_its_freedom(&sketch, free);

    for (copy, turn) in copies.iter().zip([1.0_f64, 4.0, 5.0]) {
        let (start, _) = ends(&solved.geometry, *copy);

        assert_near(
            solved.geometry.point(start).unwrap(),
            Point2::from_angle((60.0 * turn).to_radians()) * 10.0,
        );
    }

    let mut both = Sketch::new(Plane::XY);

    let point = both.add_point(Point2::new(10.0, 0.0));
    let free = solve(&both).solution.degrees_of_freedom();
    let pattern = turning(
        CircularPattern {
            count: 3,
            spread: Spread::Symmetric(degrees(60.0)),
            left_out: BTreeSet::new(),
        },
        &[-1],
    );

    let copies = both
        .circular_pattern(&[point], EntityId::ORIGIN, &pattern)
        .unwrap();

    assert_eq!(copies.len(), 3);

    let solved = assert_holds_its_freedom(&both, free);

    assert_near(
        solved.geometry.point(copies[2]).unwrap(),
        Point2::from_angle((-60.0_f64).to_radians()) * 10.0,
    );
}

#[test]
fn the_preview_shows_left_out_copies_apart_and_unknown_copies_are_refused() {
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
    let before = sketch.clone();

    let image = sketch
        .rectangular_image(
            &[line],
            &leaving(across(4, 10.0), &[&[2]]),
            Faceting::within(0.01),
        )
        .unwrap();

    let left_out: Vec<&PatternCopy> = image.left_out().map(|copy| &copy.copy).collect();

    assert_eq!(left_out, [&PatternCopy::Steps(vec![2])]);
    assert_eq!(image.kept().count(), 2);

    assert_near(
        image.left_out().next().unwrap().curves[0][0],
        Point2::new(20.0, 0.0),
    );

    assert_eq!(
        sketch.rectangular_pattern(&[line], &leaving(across(3, 10.0), &[&[3]])),
        Err(PatternError::NoSuchCopy {
            copy: PatternCopy::Steps(vec![3])
        })
    );
    assert_eq!(
        sketch.rectangular_pattern(&[line], &leaving(across(3, 10.0), &[&[0]])),
        Err(PatternError::OriginalStays)
    );
    assert_eq!(
        sketch.rectangular_pattern(&[line], &leaving(across(3, 10.0), &[&[1], &[2]])),
        Err(PatternError::EveryCopyLeftOut)
    );
    assert_eq!(
        sketch.circular_pattern(&[line], EntityId::ORIGIN, &turning(turns(3), &[-1])),
        Err(PatternError::NoSuchCopy {
            copy: PatternCopy::Turn(-1)
        })
    );
    assert_eq!(sketch, before);
    assert_eq!(
        PatternError::NoSuchCopy {
            copy: PatternCopy::Steps(vec![2, 1])
        }
        .to_string(),
        "copy (2, 1) is not one of the pattern's copies, so it cannot be left out"
    );
}
