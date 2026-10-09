use crate::{BinaryOperator, Dimension, EvalError, Expression, Function, ParameterId, Quantity};

impl Expression {
    pub fn with_lengths_scaled<F>(&self, factor: f64, plain_power: i8, value_of: &F) -> Self
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        Scaling { factor, value_of }.node(self, plain_power)
    }

    pub fn times_number(self, factor: f64) -> Self {
        Self::binary(BinaryOperator::Multiply, self, Self::number(factor))
    }
}

struct Scaling<'a, F> {
    factor: f64,
    value_of: &'a F,
}

impl<F> Scaling<'_, F>
where
    F: Fn(ParameterId) -> Result<Quantity, EvalError>,
{
    fn dimension(&self, node: &Expression) -> Option<Dimension> {
        node.evaluate(self.value_of)
            .ok()
            .map(|quantity| quantity.dimension)
    }

    fn is_plain(&self, node: &Expression) -> bool {
        self.dimension(node).is_some_and(Dimension::is_plain)
    }

    fn times(&self, power: i8) -> f64 {
        self.factor.powi(i32::from(power))
    }

    fn wrapped(&self, node: &Expression, context: i8) -> Expression {
        if context != 0 && self.is_plain(node) {
            node.clone().times_number(self.times(context))
        } else {
            node.clone()
        }
    }

    fn node(&self, node: &Expression, context: i8) -> Expression {
        match node {
            Expression::Number(value) if context != 0 => {
                Expression::Number(value * self.times(context))
            }
            Expression::Number(_) => node.clone(),
            Expression::Measure(value, unit) => {
                Expression::Measure(value * self.times(unit.dimension().length_power()), *unit)
            }
            Expression::Constant(_) | Expression::Parameter(_) => self.wrapped(node, context),
            Expression::Negate(inner) => Expression::Negate(Box::new(self.node(inner, context))),
            Expression::WithUnit(inner, unit, exponent) => {
                let power = unit.dimension().length_power().saturating_mul(*exponent);
                let scaled = match **inner {
                    Expression::Number(value) => Expression::Number(value * self.times(power)),
                    _ if power == 0 => self.node(inner, 0),
                    _ => self.node(inner, 0).times_number(self.times(power)),
                };
                Expression::WithUnit(Box::new(scaled), *unit, *exponent)
            }
            Expression::Binary(operator, left, right) => {
                self.binary(node, *operator, (left, right), context)
            }
            Expression::Call(Function::If, arguments) => Expression::Call(
                Function::If,
                arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        self.node(argument, if index == 0 { 0 } else { context })
                    })
                    .collect(),
            ),
            Expression::Call(function, arguments) if same_kind(*function) => {
                let contexts = self.contexts(arguments.iter(), context);
                Expression::Call(
                    *function,
                    arguments
                        .iter()
                        .zip(contexts)
                        .map(|(argument, context)| self.node(argument, context))
                        .collect(),
                )
            }
            Expression::Call(function, arguments) => {
                let scaled = Expression::Call(
                    *function,
                    arguments
                        .iter()
                        .map(|argument| self.node(argument, 0))
                        .collect(),
                );
                if context != 0 && self.is_plain(node) {
                    scaled.times_number(self.times(context))
                } else {
                    scaled
                }
            }
        }
    }

    fn binary(
        &self,
        node: &Expression,
        operator: BinaryOperator,
        (left, right): (&Expression, &Expression),
        context: i8,
    ) -> Expression {
        let rebuilt = |left: Expression, right: Expression| {
            Expression::Binary(operator, Box::new(left), Box::new(right))
        };
        match operator {
            BinaryOperator::Add | BinaryOperator::Subtract => {
                let contexts = self.contexts([left, right].into_iter(), context);
                let [left_context, right_context] = contexts.as_slice() else {
                    return node.clone();
                };
                rebuilt(
                    self.node(left, *left_context),
                    self.node(right, *right_context),
                )
            }
            BinaryOperator::Less
            | BinaryOperator::LessOrEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterOrEqual
            | BinaryOperator::Equal
            | BinaryOperator::NotEqual => {
                let contexts = self.contexts([left, right].into_iter(), 0);
                let [left_context, right_context] = contexts.as_slice() else {
                    return node.clone();
                };
                rebuilt(
                    self.node(left, *left_context),
                    self.node(right, *right_context),
                )
            }
            BinaryOperator::Multiply | BinaryOperator::Divide
                if context != 0 && self.is_plain(node) =>
            {
                let carrier = if self.is_plain(left) {
                    Some(true)
                } else if operator == BinaryOperator::Multiply && self.is_plain(right) {
                    Some(false)
                } else {
                    None
                };
                match carrier {
                    Some(true) => rebuilt(self.node(left, context), self.node(right, 0)),
                    Some(false) => rebuilt(self.node(left, 0), self.node(right, context)),
                    None => rebuilt(self.node(left, 0), self.node(right, 0))
                        .times_number(self.times(context)),
                }
            }
            BinaryOperator::Multiply | BinaryOperator::Divide => {
                rebuilt(self.node(left, 0), self.node(right, 0))
            }
            BinaryOperator::Power => {
                let scaled = rebuilt(self.node(left, 0), right.clone());
                if context != 0 && self.is_plain(node) {
                    scaled.times_number(self.times(context))
                } else {
                    scaled
                }
            }
        }
    }

    fn contexts<'a>(
        &self,
        arguments: impl Iterator<Item = &'a Expression>,
        context: i8,
    ) -> Vec<i8> {
        let dimensions: Vec<Option<Dimension>> =
            arguments.map(|argument| self.dimension(argument)).collect();
        let shared = dimensions
            .iter()
            .flatten()
            .find(|dimension| !dimension.is_plain())
            .map(|dimension| dimension.length_power());
        dimensions
            .iter()
            .map(|dimension| match (dimension, shared) {
                (Some(dimension), _) if !dimension.is_plain() => 0,
                (Some(_), Some(power)) => power,
                (Some(_), None) => context,
                (None, _) => 0,
            })
            .collect()
    }
}

fn same_kind(function: Function) -> bool {
    matches!(
        function,
        Function::Abs
            | Function::Floor
            | Function::Ceil
            | Function::Round
            | Function::Trunc
            | Function::Min
            | Function::Max
            | Function::Mod
            | Function::Hypot
            | Function::Clamp
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Unit;

    fn values(id: ParameterId) -> Result<Quantity, EvalError> {
        match id.raw() {
            0 => Ok(Quantity::length(10.0)),
            1 => Ok(Quantity::plain(4.0)),
            _ => Err(EvalError::ParameterMissing),
        }
    }

    fn scaled(text: &str, plain_power: i8) -> String {
        let expression = Expression::parse_stored(text).unwrap();
        let scaled = expression.with_lengths_scaled(2.0, plain_power, &values);
        Expression::parse_stored(&scaled.to_stored_text())
            .unwrap()
            .to_stored_text()
    }

    #[test]
    fn literals_are_scaled_by_their_length_power_and_angles_stay() {
        assert_eq!(scaled("5 mm", 1), "10 mm");
        assert_eq!(scaled("5", 1), "10");
        assert_eq!(scaled("-1.5 cm", 1), "-3 cm");
        assert_eq!(scaled("30 deg", 0), "30 deg");
        assert_eq!(scaled("4 mm²", 2), "16 mm²");
        assert_eq!(scaled("3", 0), "3");
    }

    #[test]
    fn a_plain_number_beside_a_length_takes_its_power_and_a_factor_does_not() {
        assert_eq!(scaled("$0 + 5", 1), "$0 + 10");
        assert_eq!(scaled("2 * $0", 1), "2 * $0");
        assert_eq!(scaled("$0 / 2 + 1 mm", 1), "$0 / 2 + 2 mm");
        assert_eq!(scaled("max($0, 3)", 1), "max($0, 6)");
        assert_eq!(scaled("(5 + 3) * 2", 1), "(10 + 6) * 2");
    }

    #[test]
    fn a_plain_parameter_read_as_a_length_is_multiplied() {
        assert_eq!(scaled("$1", 1), "$1 * 2");
        assert_eq!(scaled("sqrt($1)", 1), "sqrt($1) * 2");
        assert_eq!(scaled("$1", 0), "$1");
    }

    #[test]
    fn the_scaled_value_is_the_factor_to_the_power_times_the_value() {
        let expression = Expression::parse_stored("($0 + 5) * 2 + 1 cm").unwrap();
        let before = expression.evaluate(&values).unwrap();
        let scaled = expression.with_lengths_scaled(2.0, 1, &values);
        let after = scaled
            .evaluate(&|id: ParameterId| {
                values(id).map(|quantity| match quantity.dimension {
                    Dimension::LENGTH => Quantity::length(quantity.value * 2.0),
                    _ => quantity,
                })
            })
            .unwrap();

        assert_eq!(after.dimension, Dimension::LENGTH);
        assert!((after.value - 2.0 * before.value).abs() < 1e-9);
        assert!(matches!(
            Expression::Measure(1.0, Unit::Inch).with_lengths_scaled(2.0, 1, &values),
            Expression::Measure(value, Unit::Inch) if value == 2.0
        ));
    }
}
