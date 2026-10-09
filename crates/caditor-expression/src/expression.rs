use std::{collections::BTreeSet, fmt};

use crate::{
    ParameterId, ParseError,
    quantity::{Dimension, Quantity, Unit},
};

const MISSING_PARAMETER: &str = "⟨missing⟩";
const EQUALITY_TOLERANCE: f64 = 1e-9;
pub(crate) const STORED_REFERENCE: char = '$';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

impl BinaryOperator {
    fn symbol(self) -> &'static str {
        match self {
            Self::Add => " + ",
            Self::Subtract => " - ",
            Self::Multiply => " * ",
            Self::Divide => " / ",
            Self::Power => "^",
            Self::Less => " < ",
            Self::LessOrEqual => " <= ",
            Self::Greater => " > ",
            Self::GreaterOrEqual => " >= ",
            Self::Equal => " == ",
            Self::NotEqual => " != ",
        }
    }

    fn precedence(self) -> Precedence {
        match self {
            Self::Add | Self::Subtract => Precedence::Sum,
            Self::Multiply | Self::Divide => Precedence::Product,
            Self::Power => Precedence::Power,
            Self::Less
            | Self::LessOrEqual
            | Self::Greater
            | Self::GreaterOrEqual
            | Self::Equal
            | Self::NotEqual => Precedence::Comparison,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    Comparison,
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
    Between(usize, usize),
}

impl Arity {
    pub fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exactly(expected) => count == expected,
            Self::AtLeast(minimum) => count >= minimum,
            Self::Between(minimum, maximum) => (minimum..=maximum).contains(&count),
        }
    }
}

impl fmt::Display for Arity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Exactly(1) => formatter.write_str("1 value"),
            Self::Exactly(count) => write!(formatter, "{count} values"),
            Self::AtLeast(count) => write!(formatter, "{count} or more values"),
            Self::Between(minimum, maximum) => write!(formatter, "{minimum} or {maximum} values"),
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
    Mod,
    Hypot,
    Exp,
    Ln,
    Sign,
    Clamp,
    If,
    Cbrt,
    Log10,
    Log2,
    Trunc,
    And,
    Or,
    Not,
}

impl Function {
    pub const ALL: [Self; 28] = [
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
        Self::Mod,
        Self::Hypot,
        Self::Exp,
        Self::Ln,
        Self::Sign,
        Self::Clamp,
        Self::If,
        Self::Cbrt,
        Self::Log10,
        Self::Log2,
        Self::Trunc,
        Self::And,
        Self::Or,
        Self::Not,
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
            Self::Mod => "mod",
            Self::Hypot => "hypot",
            Self::Exp => "exp",
            Self::Ln => "ln",
            Self::Sign => "sign",
            Self::Clamp => "clamp",
            Self::If => "if",
            Self::Cbrt => "cbrt",
            Self::Log10 => "log10",
            Self::Log2 => "log2",
            Self::Trunc => "trunc",
            Self::And => "and",
            Self::Or => "or",
            Self::Not => "not",
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
            Self::And | Self::Or => Arity::AtLeast(2),
            Self::Floor | Self::Ceil | Self::Round | Self::Trunc => Arity::Between(1, 2),
            Self::Atan2 | Self::Mod | Self::Hypot => Arity::Exactly(2),
            Self::Clamp | Self::If => Arity::Exactly(3),
            Self::Sqrt
            | Self::Cbrt
            | Self::Abs
            | Self::Exp
            | Self::Ln
            | Self::Log10
            | Self::Log2
            | Self::Not
            | Self::Sign
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
    Tau,
    E,
}

impl Constant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Tau => "tau",
            Self::E => "e",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pi" | "π" => Some(Self::Pi),
            "tau" | "τ" => Some(Self::Tau),
            "e" => Some(Self::E),
            _ => None,
        }
    }

    pub fn value(self) -> Quantity {
        match self {
            Self::Pi => Quantity::plain(std::f64::consts::PI),
            Self::Tau => Quantity::plain(std::f64::consts::TAU),
            Self::E => Quantity::plain(std::f64::consts::E),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Compare,
    Pair(Function),
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
    #[error("{function} needs a value above zero")]
    NotPositive { function: Function },
    #[error("it raises a negative number to a fractional power")]
    NegativeBase,
    #[error("clamp needs its lower limit below its upper one")]
    InvertedLimits,
    #[error("the step of {function} must be above zero")]
    InvalidStep { function: Function },
    #[error(
        "it gives a plain number, so it is unclear whether it means degrees or radians; add deg \
         or rad, as in (pi / 2) rad"
    )]
    PlainAngle,
    #[error("{function} was given the wrong number of values")]
    WrongArgumentCount { function: Function },
    #[error("an exponent must be a plain number, not {found}")]
    DimensionedExponent { found: Dimension },
    #[error("{base} can only be raised to a whole power")]
    FractionalPower { base: Dimension },
    #[error("the {} root of {found} cannot be expressed in units", if *.function == Function::Cbrt { "cube" } else { "square" })]
    UnitlessRoot {
        function: Function,
        found: Dimension,
    },
    #[error("it takes the square root of a negative number")]
    NegativeRoot,
    #[error("the units of the result are too complex")]
    TooComplex,
    #[error("the result is too large or not a number")]
    NotFinite,
    #[error(
        "it gives {found}, but {expected} is needed{}",
        if *.divides_by_unit {
            "; a unit applies only to the number right before it, so in 1 / 2 mm the 2 mm is the \
             divisor, and (1 / 2) mm gives the quotient its unit"
        } else {
            ""
        }
    )]
    WrongKind {
        expected: Dimension,
        found: Dimension,
        divides_by_unit: bool,
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
        Operation::Pair(function) => {
            format!("{function} needs values of the same kind, not {left} and {right}")
        }
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
    WithUnit(Box<Expression>, Unit, i8),
}

impl Expression {
    pub fn binary(operator: BinaryOperator, left: Self, right: Self) -> Self {
        Self::Binary(operator, Box::new(left), Box::new(right))
    }

    pub fn number(value: f64) -> Self {
        Self::signed_literal(value, Self::Number)
    }

    pub fn measure(value: f64, unit: Unit) -> Self {
        Self::signed_literal(value, |magnitude| Self::Measure(magnitude, unit))
    }

    fn signed_literal(value: f64, literal: impl Fn(f64) -> Self) -> Self {
        if value.is_sign_negative() {
            Self::Negate(Box::new(literal(-value)))
        } else {
            literal(value)
        }
    }

    pub fn is_literal(&self) -> bool {
        match self {
            Self::Number(_) | Self::Measure(..) => true,
            Self::Negate(inner) => inner.is_literal(),
            Self::WithUnit(inner, ..) => matches!(**inner, Self::Number(_)),
            Self::Constant(_) | Self::Parameter(_) | Self::Binary(..) | Self::Call(..) => false,
        }
    }

    pub fn heap_size(&self) -> usize {
        let node = size_of::<Self>();
        match self {
            Self::Number(_) | Self::Measure(..) | Self::Constant(_) | Self::Parameter(_) => 0,
            Self::Negate(inner) | Self::WithUnit(inner, ..) => node + inner.heap_size(),
            Self::Binary(_, left, right) => 2 * node + left.heap_size() + right.heap_size(),
            Self::Call(_, arguments) => {
                arguments.len() * node + arguments.iter().map(Self::heap_size).sum::<usize>()
            }
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
            Self::Negate(inner) | Self::WithUnit(inner, ..) => inner.uses(parameter),
            Self::Binary(_, left, right) => left.uses(parameter) || right.uses(parameter),
            Self::Call(_, arguments) => arguments.iter().any(|argument| argument.uses(parameter)),
            Self::Number(_) | Self::Measure(..) | Self::Constant(_) => false,
        }
    }

    pub fn inlining(&self, parameter: ParameterId, replacement: &Self) -> Result<Self, ParseError> {
        self.substituting(&|id| (id == parameter).then(|| replacement.clone()))
    }

    pub fn substituting(
        &self,
        replacement_of: &dyn Fn(ParameterId) -> Option<Self>,
    ) -> Result<Self, ParseError> {
        let mut substituted = self.clone();
        substituted.replace_parameters(replacement_of);
        Self::parse_stored(&substituted.to_stored_text())
    }

    fn replace_parameters(&mut self, replacement_of: &dyn Fn(ParameterId) -> Option<Self>) {
        match self {
            Self::Parameter(id) => {
                if let Some(replacement) = replacement_of(*id) {
                    *self = replacement;
                }
            }
            Self::Negate(inner) | Self::WithUnit(inner, ..) => {
                inner.replace_parameters(replacement_of);
            }
            Self::Binary(_, left, right) => {
                left.replace_parameters(replacement_of);
                right.replace_parameters(replacement_of);
            }
            Self::Call(_, arguments) => {
                for argument in arguments {
                    argument.replace_parameters(replacement_of);
                }
            }
            Self::Number(_) | Self::Measure(..) | Self::Constant(_) => {}
        }
    }

    fn collect_parameters(&self, found: &mut BTreeSet<ParameterId>) {
        match self {
            Self::Parameter(id) => {
                found.insert(*id);
            }
            Self::Negate(inner) | Self::WithUnit(inner, ..) => inner.collect_parameters(found),
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
        if expected == Dimension::ANGLE && result.dimension.is_plain() && !self.is_literal() {
            return Err(EvalError::PlainAngle);
        }
        if result.dimension == expected || result.dimension.is_plain() {
            Ok(result.value)
        } else {
            Err(EvalError::WrongKind {
                expected,
                found: result.dimension,
                divides_by_unit: self.divides_by_unit(expected, result.dimension),
            })
        }
    }

    fn divides_by_unit(&self, expected: Dimension, found: Dimension) -> bool {
        let Self::Binary(BinaryOperator::Divide, _, divisor) = self else {
            return false;
        };
        matches!(**divisor, Self::Measure(..) | Self::WithUnit(..))
            && Dimension::NONE.over(expected) == Some(found)
    }

    fn evaluate_unchecked<F>(&self, value_of: &F) -> Result<Quantity, EvalError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let mut current = Pending {
            node: self,
            first_operand: 0,
        };
        let mut parents: Vec<Pending<'_>> = Vec::new();
        let mut operands: Vec<Quantity> = Vec::new();
        loop {
            let gathered = operands.get(current.first_operand..).unwrap_or_default();
            match current.node.next_step(gathered, value_of) {
                Step::Descend(child) => {
                    parents.push(current);
                    current = Pending {
                        node: child,
                        first_operand: operands.len(),
                    };
                }
                Step::Done(result) => {
                    let value = finite(result?)?;
                    operands.truncate(current.first_operand);
                    let Some(parent) = parents.pop() else {
                        return Ok(value);
                    };
                    operands.push(value);
                    current = parent;
                }
            }
        }
    }

    fn next_step<'a, F>(&'a self, gathered: &[Quantity], value_of: &F) -> Step<'a>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        match self {
            Self::Number(value) => Step::Done(Ok(Quantity::plain(*value))),
            Self::Measure(amount, unit) => Step::Done(Ok(unit.quantity(*amount))),
            Self::Constant(constant) => Step::Done(Ok(constant.value())),
            Self::Parameter(id) => Step::Done(value_of(*id)),
            Self::Negate(inner) => match gathered {
                [] => Step::Descend(inner),
                [value, ..] => Step::Done(Ok(Quantity::new(-value.value, value.dimension))),
            },
            Self::Binary(operator, left, right) => match gathered {
                [] => Step::Descend(left),
                [_] => Step::Descend(right),
                [left, right, ..] => Step::Done(binary(*operator, *left, *right)),
            },
            Self::Call(Function::If, arguments) => {
                let [condition, chosen, otherwise] = arguments.as_slice() else {
                    return Step::Done(Err(EvalError::WrongArgumentCount {
                        function: Function::If,
                    }));
                };
                match gathered {
                    [] => Step::Descend(condition),
                    [condition] => match plain_number(Function::If, *condition) {
                        Ok(holds) if holds != 0.0 => Step::Descend(chosen),
                        Ok(_) => Step::Descend(otherwise),
                        Err(error) => Step::Done(Err(error)),
                    },
                    [_, value, ..] => Step::Done(Ok(*value)),
                }
            }
            Self::Call(function @ (Function::And | Function::Or), arguments) => {
                let deciding = *function == Function::Or;
                if let Some(latest) = gathered.last() {
                    match plain_number(*function, *latest) {
                        Ok(value) if (value != 0.0) == deciding => {
                            return Step::Done(Ok(truth(deciding)));
                        }
                        Ok(_) => {}
                        Err(error) => return Step::Done(Err(error)),
                    }
                }
                match arguments.get(gathered.len()) {
                    Some(next) => Step::Descend(next),
                    None => Step::Done(Ok(truth(!deciding))),
                }
            }
            Self::Call(function, arguments) => match arguments.get(gathered.len()) {
                Some(next) => Step::Descend(next),
                None => Step::Done(call(*function, gathered)),
            },
            Self::WithUnit(inner, unit, exponent) => match gathered {
                [] => Step::Descend(inner),
                [value, ..] => Step::Done(
                    power(unit.quantity(1.0), Quantity::plain(f64::from(*exponent)))
                        .and_then(|unit| binary(BinaryOperator::Multiply, *value, unit)),
                ),
            },
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
                Self::Negate(inner) | Self::WithUnit(inner, ..) => pending.push((inner, depth + 1)),
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
            | Self::Call(..)
            | Self::WithUnit(..) => Precedence::Atom,
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
            Self::WithUnit(inner, unit, exponent) => {
                let bare = matches!(
                    **inner,
                    Self::Parameter(_) | Self::Constant(_) | Self::Call(..)
                ) || matches!(**inner, Self::Number(value) if !value.is_sign_negative());
                inner.write_wrapped(text, style, !bare);
                text.push(' ');
                text.push_str(unit.symbol());
                match exponent {
                    1 => {}
                    2 => text.push('²'),
                    3 => text.push('³'),
                    other => {
                        text.push('^');
                        text.push_str(&other.to_string());
                    }
                }
            }
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
                    BinaryOperator::Less
                    | BinaryOperator::LessOrEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterOrEqual
                    | BinaryOperator::Equal
                    | BinaryOperator::NotEqual => {
                        (left.precedence() <= own, right.precedence() <= own)
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
        BinaryOperator::Less
        | BinaryOperator::LessOrEqual
        | BinaryOperator::Greater
        | BinaryOperator::GreaterOrEqual
        | BinaryOperator::Equal
        | BinaryOperator::NotEqual => {
            unify(Operation::Compare, left, right)?;
            let (a, b) = (left.value, right.value);
            let equal = (a - b).abs() <= EQUALITY_TOLERANCE * a.abs().max(b.abs()).max(1.0);
            let holds = match operator {
                BinaryOperator::Less => a < b && !equal,
                BinaryOperator::LessOrEqual => a < b || equal,
                BinaryOperator::Greater => a > b && !equal,
                BinaryOperator::GreaterOrEqual => a > b || equal,
                BinaryOperator::Equal => equal,
                _ => !equal,
            };
            Ok(Quantity::plain(if holds { 1.0 } else { 0.0 }))
        }
    }
}

fn power(base: Quantity, exponent: Quantity) -> Result<Quantity, EvalError> {
    if !exponent.dimension.is_plain() {
        return Err(EvalError::DimensionedExponent {
            found: exponent.dimension,
        });
    }
    if base.value == 0.0 && exponent.value < 0.0 {
        return Err(EvalError::DivisionByZero);
    }
    if base.dimension.is_plain() {
        if base.value < 0.0 && exponent.value.fract() != 0.0 {
            return Err(EvalError::NegativeBase);
        }
        return Ok(Quantity::plain(base.value.powf(exponent.value)));
    }
    let whole = exponent.value.fract() == 0.0
        && exponent.value >= f64::from(i8::MIN)
        && exponent.value <= f64::from(i8::MAX);
    if exponent.value.fract() == 0.0 && !whole {
        return Err(EvalError::TooComplex);
    }
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
                    function,
                    found: first.dimension,
                })?;
            if first.value < 0.0 {
                return Err(EvalError::NegativeRoot);
            }
            Ok(Quantity::new(first.value.sqrt(), dimension))
        }
        Function::Cbrt => {
            let dimension = first.dimension.cube_root().ok_or(EvalError::UnitlessRoot {
                function,
                found: first.dimension,
            })?;
            Ok(Quantity::new(first.value.cbrt(), dimension))
        }
        Function::Abs => keep_dimension(first.value.abs()),
        Function::Floor | Function::Ceil | Function::Round | Function::Trunc => {
            let round = |value: f64| match function {
                Function::Floor => value.floor(),
                Function::Ceil => value.ceil(),
                Function::Trunc => value.trunc(),
                _ => value.round(),
            };
            match arguments.get(1) {
                Some(step) => {
                    let dimension = unify(Operation::Pair(function), first, *step)?;
                    if step.value <= 0.0 {
                        return Err(EvalError::InvalidStep { function });
                    }
                    Ok(Quantity::new(
                        round(snapped(first.value / step.value)) * step.value,
                        dimension,
                    ))
                }
                None => keep_dimension(round(snapped(first.value))),
            }
        }
        Function::Mod => {
            let divisor = *arguments
                .get(1)
                .ok_or(EvalError::WrongArgumentCount { function })?;
            let dimension = unify(Operation::Pair(function), first, divisor)?;
            if divisor.value == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            let quotient = first.value / divisor.value;
            let whole = snapped(quotient);
            let remainder = if whole == quotient {
                first.value - divisor.value * quotient.floor()
            } else {
                0.0
            };
            Ok(Quantity::new(remainder, dimension))
        }
        Function::Hypot => {
            let other = *arguments
                .get(1)
                .ok_or(EvalError::WrongArgumentCount { function })?;
            let dimension = unify(Operation::Pair(function), first, other)?;
            Ok(Quantity::new(first.value.hypot(other.value), dimension))
        }
        Function::Exp => Ok(Quantity::plain(plain_number(function, first)?.exp())),
        Function::Ln | Function::Log10 | Function::Log2 => {
            let value = plain_number(function, first)?;
            if value <= 0.0 {
                return Err(EvalError::NotPositive { function });
            }
            Ok(Quantity::plain(match function {
                Function::Log10 => value.log10(),
                Function::Log2 => value.log2(),
                _ => value.ln(),
            }))
        }
        Function::Not => Ok(truth(plain_number(function, first)? == 0.0)),
        Function::Sign => Ok(Quantity::plain(if first.value > 0.0 {
            1.0
        } else if first.value < 0.0 {
            -1.0
        } else {
            0.0
        })),
        Function::Clamp => {
            let (Some(low), Some(high)) = (arguments.get(1), arguments.get(2)) else {
                return Err(EvalError::WrongArgumentCount { function });
            };
            let dimension = unify(Operation::Pair(function), first, *low)?;
            let dimension = unify(
                Operation::Pair(function),
                Quantity::new(0.0, dimension),
                *high,
            )?;
            if low.value > high.value {
                return Err(EvalError::InvertedLimits);
            }
            Ok(Quantity::new(
                first.value.clamp(low.value, high.value),
                dimension,
            ))
        }
        Function::If | Function::And | Function::Or => {
            Err(EvalError::WrongArgumentCount { function })
        }
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
            unify(Operation::Pair(function), first, x)?;
            Ok(Quantity::angle(first.value.atan2(x.value).to_degrees()))
        }
    }
}

struct Pending<'a> {
    node: &'a Expression,
    first_operand: usize,
}

enum Step<'a> {
    Descend(&'a Expression),
    Done(Result<Quantity, EvalError>),
}

fn finite(value: Quantity) -> Result<Quantity, EvalError> {
    if value.value.is_finite() {
        Ok(value)
    } else {
        Err(EvalError::NotFinite)
    }
}

fn truth(holds: bool) -> Quantity {
    Quantity::plain(if holds { 1.0 } else { 0.0 })
}

fn snapped(quotient: f64) -> f64 {
    let nearest = quotient.round();
    if (quotient - nearest).abs() <= EQUALITY_TOLERANCE * quotient.abs().max(1.0) {
        nearest
    } else {
        quotient
    }
}

fn extreme(
    function: Function,
    first: Quantity,
    arguments: &[Quantity],
) -> Result<Quantity, EvalError> {
    arguments.iter().skip(1).try_fold(first, |best, next| {
        let dimension = unify(Operation::Pair(function), best, *next)?;
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
