use std::ops::Range;

use crate::{
    ParameterId,
    expression::{Arity, BinaryOperator, Constant, Expression, Function, STORED_REFERENCE},
    quantity::Unit,
};

pub const MAX_LENGTH: usize = 1000;
const MAX_STORED_LENGTH: usize = 64 * MAX_LENGTH;
const MAX_TREE_DEPTH: usize = MAX_LENGTH;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{kind}")]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseErrorKind {
    #[error("Enter a value, such as 10 mm or width / 2")]
    Empty,
    #[error("The expression is too long; keep it under {MAX_LENGTH} characters")]
    TooLong,
    #[error("The expression is nested too deeply")]
    TooDeep,
    #[error("'{0}' cannot be used in an expression")]
    UnexpectedCharacter(char),
    #[error("'{0}' is not a valid number")]
    InvalidNumber(String),
    #[error("'{0}' is too large a number")]
    NumberTooLarge(String),
    #[error("Expected {expected} but found '{found}'")]
    Unexpected {
        expected: &'static str,
        found: String,
    },
    #[error("The expression ends where {expected} was expected")]
    UnexpectedEnd { expected: &'static str },
    #[error("This '(' is never closed")]
    UnclosedParenthesis,
    #[error("There is no parameter named '{0}'")]
    UnknownParameter(String),
    #[error("'{0}' is not a valid parameter reference")]
    InvalidReference(String),
    #[error("'{0}' is not a function")]
    UnknownFunction(String),
    #[error("{0} needs its values in parentheses, as in {0}(x)")]
    MissingArguments(Function),
    #[error("{function} takes {expected}")]
    WrongArgumentCount { function: Function, expected: Arity },
    #[error("A unit must follow a number, as in 10 {0}")]
    UnitWithoutNumber(&'static str),
    #[error("caditor works in SI units; write {suggestion} instead of {found}")]
    ImperialUnit {
        found: String,
        suggestion: &'static str,
    },
    #[error("'{found}' is not a unit; {hint}")]
    UnknownUnit { found: String, hint: String },
}

const PLURAL_LENGTH: usize = 3;
const UNIT_LIST: &str =
    "units are um, mm, cm, m, deg and rad, and mm² or mm³ for areas and volumes";

fn spelled_unit(name: &str) -> Option<Unit> {
    let lower = name.to_lowercase();
    let trimmed = if lower.chars().count() >= PLURAL_LENGTH {
        lower.trim_end_matches('s')
    } else {
        lower.as_str()
    };
    if let Some(unit) = Unit::from_symbol(&lower).or_else(|| Unit::from_symbol(trimmed)) {
        return Some(unit);
    }
    Some(match trimmed {
        "micrometre" | "micrometer" | "micron" => Unit::Micrometre,
        "millimetre" | "millimeter" => Unit::Millimetre,
        "centimetre" | "centimeter" => Unit::Centimetre,
        "metre" | "meter" => Unit::Metre,
        "degree" => Unit::Degree,
        "radian" => Unit::Radian,
        "inche" | "inch" => Unit::Inch,
        "foot" | "feet" => Unit::Foot,
        _ => return None,
    })
}

fn unit_with_power(name: &str) -> Option<(Unit, i8)> {
    let (stem, exponent) = match name.strip_suffix('²') {
        Some(stem) => (stem, 2),
        None => match name.strip_suffix('³') {
            Some(stem) => (stem, 3),
            None => (name, 1),
        },
    };
    let unit = Unit::from_symbol(stem)?;
    (exponent == 1 || unit.dimension() == crate::Dimension::LENGTH).then_some((unit, exponent))
}

const OPERAND: &str = "a number, name or '('";
const OPERATOR: &str = "an operator such as + or *";
const CLOSING: &str = "')'";
const SEPARATOR: &str = "',' or ')'";

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Number(f64),
    Name(String),
    Reference(ParameterId),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Open,
    Close,
    Comma,
    Compare(BinaryOperator),
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    span: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum References {
    ByName,
    Stored,
}

pub fn parse(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<ParameterId>,
) -> Result<Expression, ParseError> {
    parse_with(text, MAX_LENGTH, References::ByName, resolve)
}

pub fn parse_stored(text: &str) -> Result<Expression, ParseError> {
    parse_with(text, MAX_STORED_LENGTH, References::Stored, &|_| None)
}

fn parse_with(
    text: &str,
    max_length: usize,
    references: References,
    resolve: &dyn Fn(&str) -> Option<ParameterId>,
) -> Result<Expression, ParseError> {
    if text.chars().count() > max_length {
        return Err(ParseError {
            kind: ParseErrorKind::TooLong,
            span: 0..text.len(),
        });
    }
    let tokens = lex(text, references)?;
    if tokens.is_empty() {
        return Err(ParseError {
            kind: ParseErrorKind::Empty,
            span: 0..text.len(),
        });
    }
    let mut parser = Parser {
        text,
        tokens,
        position: 0,
        depth: 0,
        resolve,
        references,
    };
    let expression = parser.expression()?;
    match parser.peek() {
        Some(token) => Err(parser.unexpected(token.clone(), OPERATOR)),
        None => Ok(expression),
    }
}

fn lex(text: &str, references: References) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::new();
    let mut position = 0;
    while let Some(character) = text.get(position..).and_then(|rest| rest.chars().next()) {
        let start = position;
        let kind = if character.is_whitespace() {
            position += character.len_utf8();
            continue;
        } else if character.is_ascii_digit() || character == '.' {
            position = number_end(text, start);
            let literal = text.get(start..position).unwrap_or_default();
            let value = literal.parse::<f64>().map_err(|_| ParseError {
                kind: ParseErrorKind::InvalidNumber(literal.to_owned()),
                span: start..position,
            })?;
            if !value.is_finite() {
                return Err(ParseError {
                    kind: ParseErrorKind::NumberTooLarge(literal.to_owned()),
                    span: start..position,
                });
            }
            TokenKind::Number(value)
        } else if character == STORED_REFERENCE && references == References::Stored {
            position = reference_end(text, start);
            let literal = text.get(start..position).unwrap_or_default();
            let id = literal
                .get(STORED_REFERENCE.len_utf8()..)
                .and_then(|digits| digits.parse::<u64>().ok())
                .ok_or_else(|| ParseError {
                    kind: ParseErrorKind::InvalidReference(literal.to_owned()),
                    span: start..position,
                })?;
            TokenKind::Reference(ParameterId::from_raw(id))
        } else if character.is_alphabetic() || character == '_' || character == '°' {
            position = name_end(text, start, character);
            TokenKind::Name(text.get(start..position).unwrap_or_default().to_owned())
        } else if let Some((operator, width)) = comparison_at(text, start) {
            position += width;
            TokenKind::Compare(operator)
        } else {
            position += character.len_utf8();
            match character {
                '+' => TokenKind::Plus,
                '-' | '−' => TokenKind::Minus,
                '*' | '×' | '·' => TokenKind::Star,
                '/' | '÷' => TokenKind::Slash,
                '^' => TokenKind::Caret,
                '(' => TokenKind::Open,
                ')' => TokenKind::Close,
                ',' => TokenKind::Comma,
                other => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedCharacter(other),
                        span: start..position,
                    });
                }
            }
        };
        tokens.push(Token {
            kind,
            span: start..position,
        });
    }
    Ok(tokens)
}

fn comparison_at(text: &str, start: usize) -> Option<(BinaryOperator, usize)> {
    let rest = text.get(start..)?;
    [
        ("<=", BinaryOperator::LessOrEqual),
        (">=", BinaryOperator::GreaterOrEqual),
        ("==", BinaryOperator::Equal),
        ("!=", BinaryOperator::NotEqual),
        ("≤", BinaryOperator::LessOrEqual),
        ("≥", BinaryOperator::GreaterOrEqual),
        ("≠", BinaryOperator::NotEqual),
        ("<", BinaryOperator::Less),
        (">", BinaryOperator::Greater),
    ]
    .into_iter()
    .find(|(symbol, _)| rest.starts_with(symbol))
    .map(|(symbol, operator)| (operator, symbol.len()))
}

fn reference_end(text: &str, start: usize) -> usize {
    let digits_start = start + STORED_REFERENCE.len_utf8();
    text.get(digits_start..)
        .and_then(|rest| {
            rest.char_indices()
                .find(|(_, character)| !character.is_ascii_digit())
                .map(|(offset, _)| digits_start + offset)
        })
        .unwrap_or(text.len())
}

fn number_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let digit_run = |mut end: usize| {
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        end
    };
    let mut end = start;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
    {
        end += 1;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            end = digit_run(exponent);
        }
    }
    end
}

fn name_end(text: &str, start: usize, first: char) -> usize {
    let after_first = start + first.len_utf8();
    if first == '°' {
        return after_first;
    }
    text.get(after_first..)
        .and_then(|rest| {
            rest.char_indices()
                .find(|(_, character)| !(character.is_alphanumeric() || *character == '_'))
                .map(|(offset, _)| after_first + offset)
        })
        .unwrap_or(text.len())
}

fn deepened(depth: usize, right: &Expression, span: Range<usize>) -> Result<usize, ParseError> {
    let deeper = depth.max(right.depth()) + 1;
    if deeper > MAX_TREE_DEPTH {
        return Err(ParseError {
            kind: ParseErrorKind::TooDeep,
            span,
        });
    }
    Ok(deeper)
}

struct Parser<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    position: usize,
    depth: usize,
    resolve: &'a dyn Fn(&str) -> Option<ParameterId>,
    references: References,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|token| &token.kind)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn unexpected(&self, token: Token, expected: &'static str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::Unexpected {
                expected,
                found: self
                    .text
                    .get(token.span.clone())
                    .unwrap_or_default()
                    .to_owned(),
            },
            span: token.span,
        }
    }

    fn ended(&self, expected: &'static str) -> ParseError {
        ParseError {
            kind: ParseErrorKind::UnexpectedEnd { expected },
            span: self.text.len()..self.text.len(),
        }
    }

    fn nested<T>(
        &mut self,
        span: Range<usize>,
        parse: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        if self.depth >= MAX_DEPTH {
            return Err(ParseError {
                kind: ParseErrorKind::TooDeep,
                span,
            });
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn expression(&mut self) -> Result<Expression, ParseError> {
        let left = self.sum()?;
        let Some(Token {
            kind: TokenKind::Compare(operator),
            span,
        }) = self.peek().cloned()
        else {
            return Ok(left);
        };
        self.advance();
        let right = self.sum()?;
        deepened(left.depth(), &right, span)?;
        Ok(Expression::binary(operator, left, right))
    }

    fn sum(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.term()?;
        let mut depth = left.depth();
        loop {
            let operator = match self.peek_kind() {
                Some(TokenKind::Plus) => BinaryOperator::Add,
                Some(TokenKind::Minus) => BinaryOperator::Subtract,
                _ => return Ok(left),
            };
            let span = self.advance().map(|token| token.span).unwrap_or_default();
            let right = self.term()?;
            depth = deepened(depth, &right, span)?;
            left = Expression::binary(operator, left, right);
        }
    }

    fn term(&mut self) -> Result<Expression, ParseError> {
        let mut left = self.unary()?;
        let mut depth = left.depth();
        loop {
            let operator = match self.peek_kind() {
                Some(TokenKind::Star) => BinaryOperator::Multiply,
                Some(TokenKind::Slash) => BinaryOperator::Divide,
                _ => return Ok(left),
            };
            let span = self.advance().map(|token| token.span).unwrap_or_default();
            let right = self.unary()?;
            depth = deepened(depth, &right, span)?;
            left = Expression::binary(operator, left, right);
        }
    }

    fn unary(&mut self) -> Result<Expression, ParseError> {
        let Some(token) = self.peek().cloned() else {
            return Err(self.ended(OPERAND));
        };
        match token.kind {
            TokenKind::Minus => {
                self.advance();
                let operand = self.nested(token.span, Self::unary)?;
                Ok(Expression::Negate(Box::new(operand)))
            }
            TokenKind::Plus => {
                self.advance();
                self.nested(token.span, Self::unary)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Expression, ParseError> {
        let base = self.primary()?;
        let Some(caret) = self
            .peek()
            .filter(|token| token.kind == TokenKind::Caret)
            .cloned()
        else {
            return Ok(base);
        };
        self.advance();
        let exponent = self.nested(caret.span, Self::unary)?;
        Ok(Expression::binary(BinaryOperator::Power, base, exponent))
    }

    fn primary(&mut self) -> Result<Expression, ParseError> {
        let atom = self.atom()?;
        self.unit_after(atom)
    }

    fn atom(&mut self) -> Result<Expression, ParseError> {
        let Some(token) = self.advance() else {
            return Err(self.ended(OPERAND));
        };
        match token.kind {
            TokenKind::Number(value) => Ok(Expression::Number(value)),
            TokenKind::Name(ref name) => self.name(name, token.span.clone()),
            TokenKind::Reference(id) => Ok(Expression::Parameter(id)),
            TokenKind::Open => {
                let inner = self.nested(token.span.clone(), Self::expression)?;
                match self.advance() {
                    Some(Token {
                        kind: TokenKind::Close,
                        ..
                    }) => Ok(inner),
                    Some(other) => Err(self.unexpected(other, CLOSING)),
                    None => Err(ParseError {
                        kind: ParseErrorKind::UnclosedParenthesis,
                        span: token.span,
                    }),
                }
            }
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Caret
            | TokenKind::Close
            | TokenKind::Comma
            | TokenKind::Compare(_) => Err(self.unexpected(token, OPERAND)),
        }
    }

    fn unit_after(&mut self, atom: Expression) -> Result<Expression, ParseError> {
        let Some(Token {
            kind: TokenKind::Name(name),
            span,
        }) = self.peek().cloned()
        else {
            return Ok(atom);
        };
        let Some((unit, exponent)) = unit_with_power(&name) else {
            let known = Function::from_name(&name).is_some()
                || Constant::from_name(&name).is_some()
                || (self.resolve)(&name).is_some();
            return match spelled_unit(&name) {
                Some(unit) if !known => Err(ParseError {
                    kind: ParseErrorKind::UnknownUnit {
                        found: name,
                        hint: format!("write {}, as {UNIT_LIST}", unit.symbol()),
                    },
                    span,
                }),
                _ => Ok(atom),
            };
        };
        if self.references == References::ByName && matches!(unit, Unit::Inch | Unit::Foot) {
            return Err(ParseError {
                kind: ParseErrorKind::ImperialUnit {
                    found: name,
                    suggestion: "mm, cm or m",
                },
                span,
            });
        }
        self.advance();
        Ok(match atom {
            Expression::Number(value) if exponent == 1 => Expression::Measure(value, unit),
            other => Expression::WithUnit(Box::new(other), unit, exponent),
        })
    }

    fn name(&mut self, name: &str, span: Range<usize>) -> Result<Expression, ParseError> {
        let error = |kind| ParseError {
            kind,
            span: span.clone(),
        };
        if self.peek_kind() == Some(&TokenKind::Open) {
            let function = Function::from_name(name)
                .ok_or_else(|| error(ParseErrorKind::UnknownFunction(name.to_owned())))?;
            return self.call(function, span.clone());
        }
        if let Some(constant) = Constant::from_name(name) {
            return Ok(Expression::Constant(constant));
        }
        if let Some((unit, _)) = unit_with_power(name) {
            return Err(error(ParseErrorKind::UnitWithoutNumber(unit.symbol())));
        }
        if let Some(function) = Function::from_name(name) {
            return Err(error(ParseErrorKind::MissingArguments(function)));
        }
        (self.resolve)(name)
            .map(Expression::Parameter)
            .ok_or_else(|| error(ParseErrorKind::UnknownParameter(name.to_owned())))
    }

    fn call(&mut self, function: Function, span: Range<usize>) -> Result<Expression, ParseError> {
        let open = self
            .advance()
            .map(|token| token.span)
            .unwrap_or(span.clone());
        let mut arguments = Vec::new();
        if self.peek_kind() == Some(&TokenKind::Close) {
            self.advance();
        } else {
            loop {
                arguments.push(self.nested(open.clone(), Self::expression)?);
                match self.advance() {
                    Some(Token {
                        kind: TokenKind::Comma,
                        ..
                    }) => {}
                    Some(Token {
                        kind: TokenKind::Close,
                        ..
                    }) => break,
                    Some(other) => return Err(self.unexpected(other, SEPARATOR)),
                    None => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnclosedParenthesis,
                            span: open,
                        });
                    }
                }
            }
        }
        let expected = function.arity();
        if !expected.accepts(arguments.len()) {
            return Err(ParseError {
                kind: ParseErrorKind::WrongArgumentCount { function, expected },
                span,
            });
        }
        Ok(Expression::Call(function, arguments))
    }
}
