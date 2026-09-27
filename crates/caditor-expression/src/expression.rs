use std::{collections::BTreeSet, fmt};

use crate::{
    ParameterId,
    quantity::{Dimension, Quantity, Unit},
};

const MISSING_PARAMETER: &str = "⟨missing⟩";
pub(crate) const STORED_REFERENCE: char = '$';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
}

impl BinaryOperator {
    fn symbol(self) -> &'static str {
        match self {
            Self::Add => " + ",
            Self::Subtract => " - ",
            Self::Multiply => " * ",
            Self::Divide => " / ",
            Self::Power => "^",
        }
    }

    fn precedence(self) -> Precedence {
        match self {
            Self::Add | Self::Subtract => Precedence::Sum,
            Self::Multiply | Self::Divide => Precedence::Product,
            Self::Power => Precedence::Power,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Sum,
    Product,
    Negation,
    Power,
    Atom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arity {
    Exactly(usize),
    AtLeast(usize),
}

impl Arity {
    pub fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exactly(expected) => count == expected,
            Self::AtLeast(minimum) => count >= minimum,
        }
    }
}

impl fmt::Display for Arity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exactly(1) => formatter.write_str("1 value"),
            Self::Exactly(count) => write!(formatter, "{count} values"),
            Self::AtLeast(count) => write!(formatter, "{count} or more values"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Function {
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Round,
    Min,
    Max,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
}

impl Function {
    pub const ALL: [Self; 14] = [
        Self::Sqrt,
        Self::Abs,
        Self::Floor,
        Self::Ceil,
        Self::Round,
        Self::Min,
        Self::Max,
        Self::Sin,
        Self::Cos,
        Self::Tan,
        Self::Asin,
        Self::Acos,
        Self::Atan,
        Self::Atan2,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sqrt => "sqrt",
            Self::Abs => "abs",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
            Self::Round => "round",
            Self::Min => "min",
            Self::Max => "max",
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Tan => "tan",
            Self::Asin => "asin",
            Self::Acos => "acos",
            Self::Atan => "atan",
            Self::Atan2 => "atan2",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|function| function.name() == name)
    }

    pub fn arity(self) -> Arity {
        match self {
            Self::Min | Self::Max => Arity::AtLeast(1),
            Self::Atan2 => Arity::Exactly(2),
            Self::Sqrt
            | Self::Abs
            | Self::Floor
            | Self::Ceil
            | Self::Round
            | Self::Sin
            | Self::Cos
            | Self::Tan
            | Self::Asin
            | Self::Acos
            | Self::Atan => Arity::Exactly(1),
        }
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constant {
    Pi,
}

impl Constant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pi => "pi",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pi" | "π" => Some(Self::Pi),
            _ => None,
        }
    }

    pub fn value(self) -> Quantity {
        match self {
            Self::Pi => Quantity::plain(std::f64::consts::PI),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Compare,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EvalError {
    #[error("it divides by zero")]
    DivisionByZero,
    #[error("{}", mismatch_message(*.operation, *.left, *.right))]
    Mismatch {
        operation: Operation,
        left: Dimension,
        right: Dimension,
    },
    #[error("{function} needs an angle or a plain number, not {found}")]
    NeedsAngle {
        function: Function,
        found: Dimension,
    },
    #[error("{function} needs a plain number, not {found}")]
    NeedsPlainNumber {
        function: Function,
        found: Dimension,
    },
    #[error("{function} needs a value between -1 and 1")]
    OutOfDomain { function: Function },
    #[error("{function} was given the wrong number of values")]
    WrongArgumentCount { function: Function },
    #[error("an exponent must be a plain number, not {found}")]
    DimensionedExponent { found: Dimension },
    #[error("{base} can only be raised to a whole power")]
    FractionalPower { base: Dimension },
    #[error("the square root of {found} cannot be expressed in units")]
    UnitlessRoot { found: Dimension },
    #[error("it takes the square root of a negative number")]
    NegativeRoot,
    #[error("the units of the result are too complex")]
    TooComplex,
    #[error("the result is too large or not a number")]
    NotFinite,
    #[error("it gives {found}, but {expected} is needed")]
    WrongKind {
        expected: Dimension,
        found: Dimension,
    },
    #[error("it uses {name}, which has an error")]
    ParameterFailed { id: ParameterId, name: String },
    #[error("it uses a parameter that no longer exists")]
    ParameterMissing,
}

fn mismatch_message(operation: Operation, left: Dimension, right: Dimension) -> String {
    match operation {
        Operation::Add => format!("{right} cannot be added to {left}"),
        Operation::Subtract => format!("{right} cannot be subtracted from {left}"),
        Operation::Compare => format!("{left} cannot be compared with {right}"),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Number(f64),
    Measure(f64, Unit),
    Constant(Constant),
    Parameter(ParameterId),
    Negate(Box<Expression>),
    Binary(BinaryOperator, Box<Expression>, Box<Expression>),
    Call(Function, Vec<Expression>),
}

impl Expression {
    pub fn binary(operator: BinaryOperator, left: Self, right: Self) -> Self {
        Self::Binary(operator, Box::new(left), Box::new(right))
    }

    pub fn is_literal(&self) -> bool {
        match self {
            Self::Number(_) | Self::Measure(..) => true,
            Self::Negate(inner) => inner.is_literal(),
            Self::Constant(_) | Self::Parameter(_) | Self::Binary(..) | Self::Call(..) => false,
        }
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        let mut found = BTreeSet::new();
        self.collect_parameters(&mut found);
        found
    }

    pub fn uses(&self, parameter: ParameterId) -> bool {
        match self {
            Self::Parameter(id) => *id == parameter,
            Self::Negate(inner) => inner.uses(parameter),
            Self::Binary(_, left, right) => left.uses(parameter) || right.uses(parameter),
            Self::Call(_, arguments) => arguments.iter().any(|argument| argument.uses(parameter)),
            Self::Number(_) | Self::Measure(..) | Self::Constant(_) => false,
        }
    }

    fn collect_parameters(&self, found: &mut BTreeSet<ParameterId>) {
        match self {
            Self::Parameter(id) => {
                found.insert(*id);
            }
            Self::Negate(inner) => inner.collect_parameters(found),
            Self::Binary(_, left, right) => {
                left.collect_parameters(found);
                right.collect_parameters(found);
            }
            Self::Call(_, arguments) => {
                for argument in arguments {
                    argument.collect_parameters(found);
                }
            }
            Self::Number(_) | Self::Measure(..) | Self::Constant(_) => {}
        }
    }

    pub fn evaluate<F>(&self, value_of: &F) -> Result<Quantity, EvalError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let result = self.evaluate_unchecked(value_of)?;
        if result.value.is_finite() {
            Ok(result)
        } else {
            Err(EvalError::NotFinite)
        }
    }

    pub fn evaluate_as<F>(&self, expected: Dimension, value_of: &F) -> Result<f64, EvalError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let result = self.evaluate(value_of)?;
        if result.dimension == expected || result.dimension.is_plain() {
            Ok(result.value)
        } else {
            Err(EvalError::WrongKind {
                expected,
                found: result.dimension,
            })
        }
    }

    fn evaluate_unchecked<F>(&self, value_of: &F) -> Result<Quantity, EvalError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        match self {
            Self::Number(value) => Ok(Quantity::plain(*value)),
            Self::Measure(amount, unit) => Ok(unit.quantity(*amount)),
            Self::Constant(constant) => Ok(constant.value()),
            Self::Parameter(id) => value_of(*id),
            Self::Negate(inner) => {
                let value = inner.evaluate_unchecked(value_of)?;
                Ok(Quantity::new(-value.value, value.dimension))
            }
            Self::Binary(operator, left, right) => binary(
                *operator,
                left.evaluate_unchecked(value_of)?,
                right.evaluate_unchecked(value_of)?,
            ),
            Self::Call(function, arguments) => {
                let values = arguments
                    .iter()
                    .map(|argument| argument.evaluate_unchecked(value_of))
                    .collect::<Result<Vec<_>, _>>()?;
                call(*function, &values)
            }
        }
    }

    pub fn to_text<'a>(&self, name_of: &dyn Fn(ParameterId) -> Option<&'a str>) -> String {
        let mut text = String::new();
        let style = Style {
            reference: &|id, text: &mut String| {
                text.push_str(name_of(id).unwrap_or(MISSING_PARAMETER));
            },
            number: display_number,
        };
        self.write(&mut text, &style);
        text
    }

    pub fn to_stored_text(&self) -> String {
        let mut text = String::new();
        let style = Style {
            reference: &|id, text: &mut String| {
                text.push(STORED_REFERENCE);
                text.push_str(&id.raw().to_string());
            },
            number: exact_number,
        };
        self.write(&mut text, &style);
        text
    }

    pub(crate) fn depth(&self) -> usize {
        let mut deepest = 0;
        let mut pending = vec![(self, 1)];
        while let Some((expression, depth)) = pending.pop() {
            deepest = deepest.max(depth);
            match expression {
                Self::Negate(inner) => pending.push((inner, depth + 1)),
                Self::Binary(_, left, right) => {
                    pending.push((left, depth + 1));
                    pending.push((right, depth + 1));
                }
                Self::Call(_, arguments) => {
                    pending.extend(arguments.iter().map(|argument| (argument, depth + 1)));
                }
                Self::Number(_) | Self::Measure(..) | Self::Constant(_) | Self::Parameter(_) => {}
            }
        }
        deepest
    }

    fn precedence(&self) -> Precedence {
        match self {
            Self::Number(value) | Self::Measure(value, _) if value.is_sign_negative() => {
                Precedence::Negation
            }
            Self::Number(_)
            | Self::Measure(..)
            | Self::Constant(_)
            | Self::Parameter(_)
            | Self::Call(..) => Precedence::Atom,
            Self::Negate(_) => Precedence::Negation,
            Self::Binary(operator, ..) => operator.precedence(),
        }
    }

    fn write(&self, text: &mut String, style: &Style<'_>) {
        match self {
            Self::Number(value) => text.push_str(&(style.number)(*value)),
            Self::Measure(value, unit) => {
                text.push_str(&(style.number)(*value));
                text.push(' ');
                text.push_str(unit.symbol());
            }
            Self::Constant(constant) => text.push_str(constant.name()),
            Self::Parameter(id) => (style.reference)(*id, text),
            Self::Negate(inner) => {
                text.push('-');
                inner.write_wrapped(text, style, inner.precedence() < Precedence::Negation);
            }
            Self::Binary(operator, left, right) => {
                let own = operator.precedence();
                let (left_parens, right_parens) = match operator {
                    BinaryOperator::Power => (
                        left.precedence() <= Precedence::Power,
                        right.precedence() < Precedence::Negation,
                    ),
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => {
                        (left.precedence() < own, right.precedence() <= own)
                    }
                };
                left.write_wrapped(text, style, left_parens);
                text.push_str(operator.symbol());
                right.write_wrapped(text, style, right_parens);
            }
            Self::Call(function, arguments) => {
                text.push_str(function.name());
                text.push('(');
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        text.push_str(", ");
                    }
                    argument.write(text, style);
                }
                text.push(')');
            }
        }
    }

    fn write_wrapped(&self, text: &mut String, style: &Style<'_>, parenthesize: bool) {
        if parenthesize {
            text.push('(');
            self.write(text, style);
            text.push(')');
        } else {
            self.write(text, style);
        }
    }
}

struct Style<'a> {
    reference: &'a dyn Fn(ParameterId, &mut String),
    number: fn(f64) -> String,
}

fn display_number(value: f64) -> String {
    value.to_string()
}

fn exact_number(value: f64) -> String {
    let positional = value.to_string();
    let scientific = format!("{value:e}");
    if scientific.len() < positional.len() {
        scientific
    } else {
        positional
    }
}

fn unify(operation: Operation, left: Quantity, right: Quantity) -> Result<Dimension, EvalError> {
    if left.dimension == right.dimension || right.dimension.is_plain() {
        Ok(left.dimension)
    } else if left.dimension.is_plain() {
        Ok(right.dimension)
    } else {
        Err(EvalError::Mismatch {
            operation,
            left: left.dimension,
            right: right.dimension,
        })
    }
}

fn binary(
    operator: BinaryOperator,
    left: Quantity,
    right: Quantity,
) -> Result<Quantity, EvalError> {
    match operator {
        BinaryOperator::Add => Ok(Quantity::new(
            left.value + right.value,
            unify(Operation::Add, left, right)?,
        )),
        BinaryOperator::Subtract => Ok(Quantity::new(
            left.value - right.value,
            unify(Operation::Subtract, left, right)?,
        )),
        BinaryOperator::Multiply => Ok(Quantity::new(
            left.value * right.value,
            left.dimension
                .times(right.dimension)
                .ok_or(EvalError::TooComplex)?,
        )),
        BinaryOperator::Divide => {
            if right.value == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Quantity::new(
                left.value / right.value,
                left.dimension
                    .over(right.dimension)
                    .ok_or(EvalError::TooComplex)?,
            ))
        }
        BinaryOperator::Power => power(left, right),
    }
}

fn power(base: Quantity, exponent: Quantity) -> Result<Quantity, EvalError> {
    if !exponent.dimension.is_plain() {
        return Err(EvalError::DimensionedExponent {
            found: exponent.dimension,
        });
    }
    if base.dimension.is_plain() {
        return Ok(Quantity::plain(base.value.powf(exponent.value)));
    }
    let whole = exponent.value.fract() == 0.0
        && exponent.value >= f64::from(i8::MIN)
        && exponent.value <= f64::from(i8::MAX);
    if !whole {
        return Err(EvalError::FractionalPower {
            base: base.dimension,
        });
    }
    let exponent = exponent.value as i8;
    Ok(Quantity::new(
        base.value.powi(i32::from(exponent)),
        base.dimension
            .power(exponent)
            .ok_or(EvalError::TooComplex)?,
    ))
}

fn call(function: Function, arguments: &[Quantity]) -> Result<Quantity, EvalError> {
    if !function.arity().accepts(arguments.len()) {
        return Err(EvalError::WrongArgumentCount { function });
    }
    let first = *arguments
        .first()
        .ok_or(EvalError::WrongArgumentCount { function })?;
    let keep_dimension = |value: f64| Ok(Quantity::new(value, first.dimension));
    match function {
        Function::Sqrt => {
            let dimension = first
                .dimension
                .square_root()
                .ok_or(EvalError::UnitlessRoot {
                    found: first.dimension,
                })?;
            if first.value < 0.0 {
                return Err(EvalError::NegativeRoot);
            }
            Ok(Quantity::new(first.value.sqrt(), dimension))
        }
        Function::Abs => keep_dimension(first.value.abs()),
        Function::Floor => keep_dimension(first.value.floor()),
        Function::Ceil => keep_dimension(first.value.ceil()),
        Function::Round => keep_dimension(first.value.round()),
        Function::Min | Function::Max => extreme(function, first, arguments),
        Function::Sin => Ok(Quantity::plain(radians(function, first)?.sin())),
        Function::Cos => Ok(Quantity::plain(radians(function, first)?.cos())),
        Function::Tan => Ok(Quantity::plain(radians(function, first)?.tan())),
        Function::Asin | Function::Acos => {
            let value = plain_number(function, first)?;
            if !(-1.0..=1.0).contains(&value) {
                return Err(EvalError::OutOfDomain { function });
            }
            let angle = if function == Function::Asin {
                value.asin()
            } else {
                value.acos()
            };
            Ok(Quantity::angle(angle.to_degrees()))
        }
        Function::Atan => Ok(Quantity::angle(
            plain_number(function, first)?.atan().to_degrees(),
        )),
        Function::Atan2 => {
            let x = *arguments
                .get(1)
                .ok_or(EvalError::WrongArgumentCount { function })?;
            unify(Operation::Compare, first, x)?;
            Ok(Quantity::angle(first.value.atan2(x.value).to_degrees()))
        }
    }
}

fn extreme(
    function: Function,
    first: Quantity,
    arguments: &[Quantity],
) -> Result<Quantity, EvalError> {
    arguments.iter().skip(1).try_fold(first, |best, next| {
        let dimension = unify(Operation::Compare, best, *next)?;
        let value = if function == Function::Min {
            best.value.min(next.value)
        } else {
            best.value.max(next.value)
        };
        Ok(Quantity::new(value, dimension))
    })
}

fn radians(function: Function, argument: Quantity) -> Result<f64, EvalError> {
    match argument.dimension {
        Dimension::ANGLE => Ok(argument.value.to_radians()),
        Dimension::NONE => Ok(argument.value),
        found => Err(EvalError::NeedsAngle { function, found }),
    }
}

fn plain_number(function: Function, argument: Quantity) -> Result<f64, EvalError> {
    if argument.dimension.is_plain() {
        Ok(argument.value)
    } else {
        Err(EvalError::NeedsPlainNumber {
            function,
            found: argument.dimension,
        })
    }
}
