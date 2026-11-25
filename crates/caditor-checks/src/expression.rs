use caditor_expression::{
    BinaryOperator, EvalError, Expression, Function, ParameterId, ParseErrorKind, Quantity,
};

pub const WIDTH: ParameterId = ParameterId::from_raw(1);
pub const COUNT: ParameterId = ParameterId::from_raw(2);
pub const TURN: ParameterId = ParameterId::from_raw(3);

const NAMES: [(&str, ParameterId); 3] = [("width", WIDTH), ("count", COUNT), ("turn", TURN)];
const WIDTH_MILLIMETRES: f64 = 10.0;
const COUNT_VALUE: f64 = 4.0;
const TURN_DEGREES: f64 = 30.0;
const SCALE: f64 = 2.0;
const PLAIN_POWERS: [i8; 3] = [0, 1, 2];
const AGREEMENT: f64 = 1e-9;
const SMALLEST_KEPT_EXACT: f64 = 1e-280;

pub fn resolve(name: &str) -> Option<ParameterId> {
    NAMES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, id)| *id)
}

fn name_of(id: ParameterId) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|(_, known)| *known == id)
        .map(|(name, _)| *name)
}

pub fn value(id: ParameterId) -> Result<Quantity, EvalError> {
    match id {
        WIDTH => Ok(Quantity::length(WIDTH_MILLIMETRES)),
        COUNT => Ok(Quantity::plain(COUNT_VALUE)),
        TURN => Ok(Quantity::angle(TURN_DEGREES)),
        _ => Err(EvalError::ParameterMissing),
    }
}

fn scaled_value(id: ParameterId) -> Result<Quantity, EvalError> {
    match id {
        WIDTH => Ok(Quantity::length(WIDTH_MILLIMETRES * SCALE)),
        other => value(other),
    }
}

fn is_too_large(kind: &ParseErrorKind) -> bool {
    matches!(kind, ParseErrorKind::TooLong | ParseErrorKind::TooDeep)
}

fn nodes(root: &Expression) -> Vec<&Expression> {
    let mut found = Vec::new();
    let mut pending = vec![root];

    while let Some(node) = pending.pop() {
        found.push(node);
        match node {
            Expression::Negate(inner) | Expression::WithUnit(inner, ..) => pending.push(inner),
            Expression::Binary(_, left, right) => {
                pending.push(left);
                pending.push(right);
            }
            Expression::Call(_, arguments) => pending.extend(arguments),
            Expression::Number(_)
            | Expression::Measure(..)
            | Expression::Constant(_)
            | Expression::Parameter(_) => {}
        }
    }
    found
}

fn literals_are_finite(expression: &Expression) -> bool {
    nodes(expression).into_iter().all(|node| match node {
        Expression::Number(value) | Expression::Measure(value, _) => value.is_finite(),
        _ => true,
    })
}

fn scales_with_lengths(node: &Expression) -> bool {
    match node {
        Expression::Binary(operator, ..) => matches!(
            operator,
            BinaryOperator::Add
                | BinaryOperator::Subtract
                | BinaryOperator::Multiply
                | BinaryOperator::Divide
                | BinaryOperator::Power
        ),
        Expression::Call(
            Function::Floor | Function::Ceil | Function::Round | Function::Trunc,
            arguments,
        ) => arguments.len() > 1,
        _ => true,
    }
}

struct Magnitudes {
    smallest: f64,
    largest: f64,
}

fn magnitudes(expression: &Expression) -> Magnitudes {
    let mut found = Magnitudes {
        smallest: f64::INFINITY,
        largest: 0.0,
    };

    for node in nodes(expression) {
        if let Ok(result) = node.evaluate(&value) {
            let magnitude = result.value.abs();

            if magnitude > 0.0 {
                found.smallest = found.smallest.min(magnitude);
            }
            found.largest = found.largest.max(magnitude);
        }
    }
    found
}

fn assert_stored_round_trip(expression: &Expression) {
    let stored = expression.to_stored_text();

    match Expression::parse_stored(&stored) {
        Ok(again) => assert_eq!(&again, expression, "stored as {stored}"),
        Err(error) => assert!(
            is_too_large(&error.kind) || !literals_are_finite(expression),
            "the stored text {stored} is refused: {error}"
        ),
    }
}

fn assert_display_round_trip(expression: &Expression) {
    if expression
        .parameters()
        .into_iter()
        .any(|id| name_of(id).is_none())
    {
        return;
    }

    let shown = expression.to_text(&name_of);

    match Expression::parse(&shown, &resolve) {
        Ok(again) => assert_eq!(&again, expression, "shown as {shown}"),
        Err(error) => assert!(
            is_too_large(&error.kind),
            "the display text {shown} is refused: {error}"
        ),
    }
}

struct Unscaled {
    result: Quantity,
    largest: f64,
}

fn comparable(expression: &Expression) -> Option<Unscaled> {
    let result = expression.evaluate(&value).ok()?;

    if !nodes(expression).into_iter().all(scales_with_lengths) {
        return None;
    }

    let Magnitudes { smallest, largest } = magnitudes(expression);

    (smallest >= SMALLEST_KEPT_EXACT).then_some(Unscaled { result, largest })
}

fn assert_scaled_consistently(
    expression: &Expression,
    plain_power: i8,
    unscaled: Option<&Unscaled>,
) {
    let scaled = expression.with_lengths_scaled(SCALE, plain_power, &value);

    assert_stored_round_trip(&scaled);

    let Some(Unscaled {
        result: old,
        largest,
    }) = unscaled
    else {
        return;
    };

    let power = if old.dimension.is_plain() {
        plain_power
    } else {
        old.dimension.length_power()
    };
    let expected = old.value * SCALE.powi(i32::from(power));
    let stored = scaled.to_stored_text();

    match scaled.evaluate(&scaled_value) {
        Ok(new) => {
            assert_eq!(
                new.dimension, old.dimension,
                "{stored} changed kind when scaled"
            );
            assert!(
                (new.value - expected).abs() <= AGREEMENT * expected.abs().max(*largest).max(1.0),
                "{stored} gives {} scaled by {SCALE} at power {plain_power} instead of {expected}",
                new.value
            );
        }
        Err(EvalError::NotFinite) => {}
        Err(error) => panic!("{stored} fails when scaled by {SCALE}: {error}"),
    }
}

fn assert_holds(expression: &Expression) {
    assert_stored_round_trip(expression);
    assert_display_round_trip(expression);

    let unscaled = comparable(expression);

    for plain_power in PLAIN_POWERS {
        assert_scaled_consistently(expression, plain_power, unscaled.as_ref());
    }
}

pub fn check_text(text: &str) {
    if let Ok(parsed) = Expression::parse(text, &resolve) {
        assert_holds(&parsed);
    }
    if let Ok(parsed) = Expression::parse_stored(text) {
        assert_holds(&parsed);
    }
}
