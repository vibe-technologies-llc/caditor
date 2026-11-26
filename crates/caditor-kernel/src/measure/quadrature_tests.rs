use caditor_geometry::{Maths, Vector3};

use super::quadrature::{QUARTIC_RULE, gauss_legendre, mean_length};
use crate::numeric::GAUSS_LEGENDRE;

const EXACT: f64 = 1e-14;

fn monomial_integral(power: usize) -> f64 {
    if power.is_multiple_of(2) {
        2.0 / (power as f64 + 1.0)
    } else {
        0.0
    }
}

#[test]
fn gauss_rules_integrate_every_polynomial_of_their_degree_exactly() {
    for count in 1..15 {
        let rule = gauss_legendre(count);

        assert_eq!(rule.len(), count);
        assert!(rule.windows(2).all(|pair| pair[0].0 < pair[1].0));

        for power in 0..2 * count {
            let found: f64 = rule
                .iter()
                .map(|(node, weight)| weight * node.powi(power as i32))
                .sum();

            assert!(
                (found - monomial_integral(power)).abs() <= EXACT,
                "{count} points on x^{power}: {found}"
            );
        }
    }
}

#[test]
fn gauss_rules_match_the_tabulated_ones() {
    let tables: [&[(f64, f64)]; 2] = [&GAUSS_LEGENDRE, &QUARTIC_RULE];

    for table in tables {
        let found = gauss_legendre(table.len());

        for ((node, weight), (expected_node, expected_weight)) in found.iter().zip(table) {
            assert!((node - expected_node).abs() <= 4.0 * f64::EPSILON);
            assert!((weight - expected_weight).abs() <= 4.0 * f64::EPSILON);
        }
    }
}

fn textbook_mean_length(start: Vector3, end: Vector3) -> f64 {
    let change = end - start;
    let square = change.length_squared();

    let shift = start.dot(change) / square;
    let height = start.cross(change).length() / square;
    let antiderivative = |along: f64| {
        let root = (along * along + height * height).sqrt();
        0.5 * (along * root + height * height * (along / height).asinh_libm())
    };
    square.sqrt() * (antiderivative(1.0 + shift) - antiderivative(shift))
}

fn sampled_mean_length(start: Vector3, end: Vector3) -> f64 {
    let pieces = 64;
    let width = 1.0 / f64::from(pieces);

    (0..pieces)
        .flat_map(|piece| {
            let middle = width * (f64::from(piece) + 0.5);
            GAUSS_LEGENDRE
                .iter()
                .map(move |(node, weight)| (middle + 0.5 * width * node, 0.5 * width * weight))
        })
        .map(|(along, weight)| weight * (start + (end - start) * along).length())
        .sum()
}

fn assert_mean_length(start: Vector3, end: Vector3, expected: f64) {
    let found = mean_length(start, end);
    let reversed = mean_length(end, start);

    assert!(
        (found - expected).abs() <= EXACT * expected.abs().max(1.0),
        "{start:?} to {end:?}: {found} against {expected}"
    );
    assert!(
        (reversed - expected).abs() <= EXACT * expected.abs().max(1.0),
        "{end:?} to {start:?}: {reversed} against {expected}"
    );
}

#[test]
fn the_mean_length_of_a_linear_normal_is_its_closed_form() {
    let pairs = [
        (Vector3::new(1.0, 2.0, 0.5), Vector3::new(-0.5, 3.0, 2.0)),
        (Vector3::new(0.3, -1.0, 0.2), Vector3::new(4.0, 0.1, -0.7)),
        (Vector3::new(-1.0, 1e-3, 0.0), Vector3::new(1.0, 1e-3, 0.0)),
        (Vector3::new(2.0, 0.5, 0.0), Vector3::new(0.0, 0.5, 0.0)),
    ];

    for (start, end) in pairs {
        assert_mean_length(start, end, textbook_mean_length(start, end));
    }
}

#[test]
fn the_mean_length_holds_where_the_textbook_form_cancels() {
    let start = Vector3::new(1.0, 2.0, 0.5);
    let nearly = [1e-6, 1e-9, 1e-13];

    for change in nearly {
        let end = start + Vector3::new(change, -2.0 * change, 0.5 * change);

        assert_mean_length(start, end, sampled_mean_length(start, end));
    }
    assert_mean_length(start, start, start.length());
}

#[test]
fn the_mean_length_of_a_normal_along_its_own_line_is_exact() {
    assert_mean_length(
        Vector3::new(-1.0, 0.0, 0.0),
        Vector3::new(3.0, 0.0, 0.0),
        1.25,
    );
    assert_mean_length(
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(3.0, 0.0, 0.0),
        2.0,
    );
    assert_mean_length(Vector3::ZERO, Vector3::new(0.0, 0.0, -2.0), 1.0);
    assert_mean_length(Vector3::ZERO, Vector3::ZERO, 0.0);
}
