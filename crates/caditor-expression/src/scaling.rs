use std::{collections::BTreeMap, ptr};

use crate::{BinaryOperator, Dimension, EvalError, Expression, Function, ParameterId, Quantity};

impl Expression {
    pub fn with_lengths_scaled<F>(&self, factor: f64, plain_power: i8, value_of: &F) -> Self
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let results = Results::bottom_up(self, value_of);

        Scaling {
            factor,
            dimension_of: |node: &Expression| results.dimension(node),
        }
        .node(self, plain_power)
    }

    pub fn times_number(self, factor: f64) -> Self {
        Self::binary(BinaryOperator::Multiply, self, Self::number(factor))
    }
}

struct Results(BTreeMap<*const Expression, Option<Quantity>>);

impl Results {
    fn bottom_up<F>(root: &Expression, value_of: &F) -> Self
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let mut order = Vec::new();
        let mut pending = vec![root];

        while let Some(node) = pending.pop() {
            order.push(node);
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

        let mut results = BTreeMap::new();

        for node in order.into_iter().rev() {
            let result = node.evaluate_over(value_of, &|child| {
                results.get(&ptr::from_ref(child)).copied().flatten()
            });

            results.insert(ptr::from_ref(node), result);
        }
        Self(results)
    }

    fn dimension(&self, node: &Expression) -> Option<Dimension> {
        self.0
            .get(&ptr::from_ref(node))
            .copied()
            .flatten()
            .map(|quantity| quantity.dimension)
    }
}

struct Scaling<D> {
    factor: f64,
    dimension_of: D,
}

impl<D> Scaling<D>
where
    D: Fn(&Expression) -> Option<Dimension>,
{
    fn dimension(&self, node: &Expression) -> Option<Dimension> {
        (self.dimension_of)(node)
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
    use std::cell::Cell;

    use super::*;
    use crate::{Unit, parse::MAX_LENGTH};

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

    fn by_evaluating_every_subtree(expression: &Expression, plain_power: i8) -> Expression {
        Scaling {
            factor: 2.0,
            dimension_of: |node: &Expression| {
                node.evaluate(&values)
                    .ok()
                    .map(|quantity| quantity.dimension)
            },
        }
        .node(expression, plain_power)
    }

    fn long_chain(limit: usize) -> String {
        let terms = [
            "if($1 > $2, max($0, 1 mm, 2 mm, 3 mm), hypot($0, $1 * 2 mm))",
            "min($0 + 5, abs(-$0), 7 mm) / 2",
            "($1 / 1 mm + 2) mm * $1",
            "clamp($0, 1, $3) + 4 mm^2 / 2 mm",
            "if(1 / 0, $0, $1) + (1 mm)^2 / $0",
        ];
        let mut cycle = terms.iter().cycle();
        let mut text = String::new();

        loop {
            let group: Vec<&str> = cycle.by_ref().take(4).copied().collect();
            let joined = format!(
                "(({} + {}) + ({} + {}))",
                group[0], group[1], group[2], group[3]
            );
            let next = if text.is_empty() {
                joined
            } else {
                format!("{text} + {joined}")
            };

            if next.len() > limit {
                return text;
            }
            text = next;
        }
    }

    #[test]
    fn bottom_up_dimensions_scale_as_evaluating_every_subtree_did() {
        let samples = [
            "5 mm",
            "$0 + 5",
            "2 * $0",
            "$1",
            "sqrt($1) + $0",
            "(5 + 3) * 2 + $0",
            "$0 / 2 + 1 mm",
            "if($1 > 2, $0 + 3, 5)",
            "if($1 > 5, 1 / 0, $0 + 3)",
            "if(1 / 0, $0, 7)",
            "if($0, 1, 2) + $0",
            "or($1 > 2, 1 / 0) * $0",
            "and($1 > 9, $2) + 4",
            "max($0, $2, 3) + 1",
            "max($0, 3, 1 mm) - 2",
            "clamp(5, 1, 7) mm + $0",
            "hypot(3, 4) + $0",
            "asin(2) + $0 + 5",
            "$2 + 5 + $0",
            "(1 / 0) + $0 * 3",
            "(2 + 3)^2 * $0",
            "$0^2 + 5",
            "($0 * $0) / 4 + 3 mm^2",
            "-($0 + 5) * 2",
            "3 * (4 + 5) / 2 + 1 mm",
            "round($0 / 3 mm) * 5 + 2",
            "5 deg + 30 deg * 2",
            "sin(30 deg) * $0 + 5",
        ];

        for sample in samples {
            let expression = Expression::parse_stored(sample).unwrap();

            for plain_power in [0, 1, 2] {
                assert_eq!(
                    expression.with_lengths_scaled(2.0, plain_power, &values),
                    by_evaluating_every_subtree(&expression, plain_power),
                    "{sample} at {plain_power}"
                );
            }
        }

        let chain = Expression::parse_stored(&long_chain(1500)).unwrap();

        assert_eq!(
            chain.with_lengths_scaled(2.0, 1, &values),
            by_evaluating_every_subtree(&chain, 1)
        );
    }

    #[test]
    fn the_largest_stored_expression_reads_each_parameter_once() {
        let text = long_chain(64 * MAX_LENGTH);
        let expression = Expression::parse_stored(&text).unwrap();
        let reads = Cell::new(0);
        let counted = |id: ParameterId| {
            reads.set(reads.get() + 1);
            values(id)
        };

        let scaled = expression.with_lengths_scaled(2.0, 1, &counted);

        assert!(text.len() > 60_000, "{}", text.len());
        assert_eq!(reads.get(), text.matches('$').count());
        assert_ne!(scaled, expression);
        assert!(scaled.depth() >= expression.depth());
    }
}
