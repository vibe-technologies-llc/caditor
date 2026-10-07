mod expression;
mod parse;
mod quantity;

use std::fmt;

pub use crate::{
    expression::{Arity, BinaryOperator, Constant, EvalError, Expression, Function, Operation},
    parse::{MAX_LENGTH, ParseError, ParseErrorKind},
    quantity::{Dimension, Quantity, Unit, format_number},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParameterId(u64);

impl ParameterId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ParameterId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl Expression {
    pub fn parse(
        text: &str,
        resolve: &dyn Fn(&str) -> Option<ParameterId>,
    ) -> Result<Self, ParseError> {
        parse::parse(text, resolve)
    }

    pub fn parse_stored(text: &str) -> Result<Self, ParseError> {
        parse::parse_stored(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("A name cannot be empty")]
    Empty,
    #[error("A name must start with a letter or '_'")]
    InvalidStart,
    #[error("A name can only contain letters, digits and '_', not '{0}'")]
    InvalidCharacter(char),
    #[error("'{0}' is a unit, so it cannot be used as a name")]
    Unit(String),
    #[error("'{0}' is a function, so it cannot be used as a name")]
    Function(String),
    #[error("'{0}' is a constant, so it cannot be used as a name")]
    Constant(String),
}

pub fn check_name(name: &str) -> Result<(), NameError> {
    let mut characters = name.chars();
    let first = characters.next().ok_or(NameError::Empty)?;
    if !(first.is_alphabetic() || first == '_') {
        return Err(NameError::InvalidStart);
    }
    if let Some(invalid) =
        characters.find(|character| !(character.is_alphanumeric() || *character == '_'))
    {
        return Err(NameError::InvalidCharacter(invalid));
    }
    if Unit::from_symbol(name).is_some() || parse::unit_with_power(name).is_some() {
        return Err(NameError::Unit(name.to_owned()));
    }
    if Function::from_name(name).is_some() {
        return Err(NameError::Function(name.to_owned()));
    }
    if Constant::from_name(name).is_some() {
        return Err(NameError::Constant(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: ParameterId = ParameterId::from_raw(0);
    const HEIGHT: ParameterId = ParameterId::from_raw(1);
    const GAP: ParameterId = ParameterId::from_raw(2);

    fn resolve(name: &str) -> Option<ParameterId> {
        match name {
            "width" => Some(WIDTH),
            "height" => Some(HEIGHT),
            "gap" => Some(GAP),
            _ => None,
        }
    }

    fn name_of(id: ParameterId) -> Option<&'static str> {
        match id.raw() {
            0 => Some("width"),
            1 => Some("height"),
            2 => Some("gap"),
            _ => None,
        }
    }

    fn value_of(id: ParameterId) -> Result<Quantity, EvalError> {
        match id.raw() {
            0 => Ok(Quantity::length(40.0)),
            1 => Ok(Quantity::length(20.0)),
            2 => Ok(Quantity::length(0.0)),
            _ => Err(EvalError::ParameterMissing),
        }
    }

    fn parse(text: &str) -> Result<Expression, ParseError> {
        Expression::parse(text, &resolve)
    }

    #[test]
    fn inlining_a_parameter_keeps_the_meaning_of_the_expression() {
        let gap = parse("width - height").unwrap();
        let cases = [
            ("gap * 3", "(width - height) * 3"),
            ("-gap", "-(width - height)"),
            (
                "max(gap, 1 mm) + gap / 2",
                "max(width - height, 1 mm) + (width - height) / 2",
            ),
            ("height", "height"),
        ];

        for (text, expected) in cases {
            let inlined = parse(text).unwrap().inlining(GAP, &gap).unwrap();
            let before = parse(text)
                .unwrap()
                .evaluate(&|id| match id {
                    GAP => Ok(Quantity::length(20.0)),
                    _ => value_of(id),
                })
                .unwrap();

            assert_eq!(inlined.to_text(&name_of), expected);
            assert_eq!(inlined.evaluate(&value_of).unwrap(), before);
            assert!(!inlined.uses(GAP));
        }
    }

    #[test]
    fn literals_built_in_code_survive_storing_as_text() {
        let built = [
            Expression::number(-2.5),
            Expression::number(-0.0),
            Expression::number(7.0),
            Expression::measure(-1.0, Unit::Millimetre),
            Expression::measure(-0.0, Unit::Degree),
            Expression::measure(1e-7, Unit::Metre),
        ];

        for expression in built {
            let stored = expression.to_stored_text();
            assert_eq!(
                Expression::parse_stored(&stored),
                Ok(expression),
                "{stored}"
            );
        }
    }

    fn evaluate(text: &str) -> Result<Quantity, EvalError> {
        parse(text).unwrap().evaluate(&value_of)
    }

    fn error_kind(text: &str) -> ParseErrorKind {
        parse(text).unwrap_err().kind
    }

    #[test]
    fn numbers_take_units_and_convert_to_base_units() {
        assert_eq!(evaluate("10"), Ok(Quantity::plain(10.0)));
        assert_eq!(evaluate("10 mm"), Ok(Quantity::length(10.0)));
        assert_eq!(
            Expression::parse_stored("2 in")
                .unwrap()
                .evaluate(&value_of),
            Ok(Quantity::length(50.8))
        );
        assert!(matches!(
            error_kind("2in"),
            ParseErrorKind::ImperialUnit { .. }
        ));
        assert_eq!(evaluate("1.5e1 cm"), Ok(Quantity::length(150.0)));
        assert_eq!(evaluate("30°"), Ok(Quantity::angle(30.0)));
        assert_eq!(evaluate(".5 m"), Ok(Quantity::length(500.0)));
    }

    #[test]
    fn arithmetic_follows_precedence_and_tracks_dimensions() {
        assert_eq!(evaluate("width + 2 * height"), Ok(Quantity::length(80.0)));
        assert_eq!(evaluate("(width + 2) * 2"), Ok(Quantity::length(84.0)));
        assert_eq!(evaluate("-2^2"), Ok(Quantity::plain(-4.0)));
        assert_eq!(evaluate("2^3^2"), Ok(Quantity::plain(512.0)));
        assert_eq!(evaluate("2^-1"), Ok(Quantity::plain(0.5)));
        assert_eq!(
            evaluate("width * height"),
            Ok(Quantity::new(800.0, Dimension::AREA))
        );
        assert_eq!(evaluate("sqrt(width * width)"), Ok(Quantity::length(40.0)));
        assert_eq!(evaluate("width / height"), Ok(Quantity::plain(2.0)));
        assert_eq!(
            evaluate("width ^ 3 / width"),
            Ok(Quantity::new(1600.0, Dimension::AREA))
        );
        assert_eq!(evaluate("max(width, 50 mm, 3)"), Ok(Quantity::length(50.0)));
        assert_eq!(
            evaluate("pi * 2 mm").map(|q| q.dimension),
            Ok(Dimension::LENGTH)
        );
    }

    #[test]
    fn angles_work_in_degrees_and_plain_numbers_in_radians() {
        let close = |text: &str, expected: f64| {
            let value = evaluate(text).unwrap().value;
            assert!((value - expected).abs() < 1e-12, "{text} gave {value}");
        };
        close("sin(30 deg)", 0.5);
        close("cos(pi / 3)", 0.5);
        close("sin(pi / 2)", 1.0);
        close("tan(0.25 rad) - tan(0.25)", 0.0);
        close("atan2(height, height)", 45.0);
        close("asin(1)", 90.0);
        assert_eq!(evaluate("asin(1)").unwrap().dimension, Dimension::ANGLE);
    }

    #[test]
    fn plain_numbers_adopt_the_dimension_they_are_combined_with() {
        assert_eq!(evaluate("width + 5"), Ok(Quantity::length(45.0)));
        assert_eq!(evaluate("5 - width"), Ok(Quantity::length(-35.0)));
        assert_eq!(evaluate("30 deg + 15"), Ok(Quantity::angle(45.0)));
    }

    #[test]
    fn roots_logarithms_truncation_and_logic_evaluate() {
        let near = |text: &str, expected: Quantity| {
            let found = evaluate(text).unwrap();
            assert_eq!(found.dimension, expected.dimension, "{text}");
            assert!(
                (found.value - expected.value).abs() < 1e-12,
                "{text}: {found:?}"
            );
        };

        near("cbrt(27 mm³)", Quantity::length(3.0));
        near("cbrt(-8)", Quantity::plain(-2.0));
        near("log10(1000)", Quantity::plain(3.0));
        near("log2(8)", Quantity::plain(3.0));
        near("trunc(-2.7 mm)", Quantity::length(-2.0));
        near("trunc(-2.7 mm, 0.5 mm)", Quantity::length(-2.5));
        near("floor(0.3, 0.1)", Quantity::plain(0.3));
        near("ceil(0.7 mm, 0.1 mm)", Quantity::length(0.7));
        near("floor(0.35, 0.1)", Quantity::plain(0.3));
        near("and(width > 1 mm, height > 1 mm)", Quantity::plain(1.0));
        near("and(gap != 0 mm, width / gap > 2)", Quantity::plain(0.0));
        near("or(gap == 0 mm, width / gap > 2)", Quantity::plain(1.0));
        near("or(0, 0, 3)", Quantity::plain(1.0));
        near("not(width < height)", Quantity::plain(1.0));

        assert_eq!(
            evaluate("cbrt(4 mm²)"),
            Err(EvalError::UnitlessRoot {
                function: Function::Cbrt,
                found: Dimension::new(2, 0),
            })
        );
        assert_eq!(
            evaluate("log10(0)"),
            Err(EvalError::NotPositive {
                function: Function::Log10
            })
        );
        assert_eq!(
            evaluate("and(1 mm, 1)"),
            Err(EvalError::NeedsPlainNumber {
                function: Function::And,
                found: Dimension::LENGTH,
            })
        );
        assert_eq!(evaluate("(2 mm) ^ 200"), Err(EvalError::TooComplex));
        assert_eq!(
            evaluate("max(width / gap, log10(0)) + and(1 mm, 1)"),
            Err(EvalError::DivisionByZero)
        );
        assert_eq!(
            evaluate("if(gap > 0 mm, width / gap, -width)"),
            Ok(Quantity::length(-40.0))
        );
        assert_eq!(
            error_kind("and(1)"),
            ParseErrorKind::WrongArgumentCount {
                function: Function::And,
                expected: Arity::AtLeast(2)
            }
        );
    }

    #[test]
    fn common_slips_get_their_own_messages() {
        assert_eq!(
            error_kind("1 < width < 5 mm"),
            ParseErrorKind::ChainedComparison
        );
        assert_eq!(
            error_kind("0,5 mm"),
            ParseErrorKind::DecimalComma("0.5".to_owned())
        );
        assert_eq!(
            error_kind("10 mm2"),
            ParseErrorKind::PowerSpelling {
                found: "mm2".to_owned(),
                suggestion: "mm²".to_owned()
            }
        );
        assert_eq!(
            error_kind("cm3 + 1"),
            ParseErrorKind::PowerSpelling {
                found: "cm3".to_owned(),
                suggestion: "cm³".to_owned()
            }
        );
        assert_eq!(
            error_kind("width² / 2"),
            ParseErrorKind::PowerSpelling {
                found: "width²".to_owned(),
                suggestion: "width^2".to_owned()
            }
        );
        assert_eq!(
            error_kind("depth²"),
            ParseErrorKind::UnknownParameter("depth²".to_owned())
        );
        assert!(parse("max(0,5)").is_ok());
        assert_eq!(check_name("mm²"), Err(NameError::Unit("mm²".to_owned())));
        assert_eq!(check_name("cm³"), Err(NameError::Unit("cm³".to_owned())));
        assert_eq!(
            check_name("cbrt"),
            Err(NameError::Function("cbrt".to_owned()))
        );
        assert_eq!(check_name("area²"), Ok(()));
    }

    #[test]
    fn function_errors_name_the_function_not_a_comparison() {
        assert_eq!(
            evaluate("hypot(1 mm, 1 deg)").unwrap_err().to_string(),
            "hypot needs values of the same kind, not a length and an angle"
        );
        assert_eq!(
            evaluate("min(1 mm, 1 deg)").unwrap_err().to_string(),
            "min needs values of the same kind, not a length and an angle"
        );
        assert_eq!(
            evaluate("width < 3 deg").unwrap_err().to_string(),
            "a length cannot be compared with an angle"
        );
    }

    #[test]
    fn a_zero_base_with_a_negative_exponent_divides_by_zero() {
        assert_eq!(evaluate("0 ^ -1"), Err(EvalError::DivisionByZero));
        assert_eq!(evaluate("gap ^ -2"), Err(EvalError::DivisionByZero));
        assert_eq!(evaluate("0 ^ 0"), Ok(Quantity::plain(1.0)));
    }

    #[test]
    fn a_unit_after_a_divisor_is_explained() {
        let length = |text: &str| {
            parse(text)
                .unwrap()
                .evaluate_as(Dimension::LENGTH, &value_of)
        };
        let message = length("1 / 2 mm").unwrap_err().to_string();
        assert!(message.starts_with("it gives a quantity in mm^-1, but a length is needed;"));
        assert!(message.contains("(1 / 2) mm"));
        assert_eq!(length("(1 / 2) mm"), Ok(0.5));
        assert_eq!(
            length("width ^ -1").unwrap_err().to_string(),
            "it gives a quantity in mm^-1, but a length is needed"
        );
    }

    #[test]
    fn evaluation_errors_are_plain_language() {
        assert_eq!(evaluate("width / gap"), Err(EvalError::DivisionByZero));
        assert_eq!(
            evaluate("width + 10 deg").unwrap_err().to_string(),
            "an angle cannot be added to a length"
        );
        assert_eq!(
            evaluate("sin(width)").unwrap_err().to_string(),
            "sin needs an angle or a plain number, not a length"
        );
        assert_eq!(
            evaluate("sqrt(width)").unwrap_err().to_string(),
            "the square root of a length cannot be expressed in units"
        );
        assert_eq!(evaluate("sqrt(-4)"), Err(EvalError::NegativeRoot));
        assert_eq!(
            evaluate("width ^ 0.5"),
            Err(EvalError::FractionalPower {
                base: Dimension::LENGTH
            })
        );
        assert_eq!(
            evaluate("asin(2)"),
            Err(EvalError::OutOfDomain {
                function: Function::Asin
            })
        );
        assert_eq!(evaluate("10 ^ 400"), Err(EvalError::NotFinite));
        assert_eq!(
            evaluate("width ^ 100 * width ^ 100"),
            Err(EvalError::TooComplex)
        );
    }

    #[test]
    fn evaluating_for_a_field_checks_the_kind_and_accepts_plain_numbers() {
        let length = |text: &str| {
            parse(text)
                .unwrap()
                .evaluate_as(Dimension::LENGTH, &value_of)
        };
        assert_eq!(length("width / 2"), Ok(20.0));
        assert_eq!(length("12"), Ok(12.0));
        assert_eq!(
            length("width * height").unwrap_err().to_string(),
            "it gives an area, but a length is needed"
        );
    }

    #[test]
    fn parse_errors_point_at_the_problem() {
        assert_eq!(error_kind("   "), ParseErrorKind::Empty);
        assert_eq!(
            error_kind("wdth"),
            ParseErrorKind::UnknownParameter("wdth".to_owned())
        );
        assert_eq!(
            error_kind("sine(3)"),
            ParseErrorKind::UnknownFunction("sine".to_owned())
        );
        assert_eq!(
            error_kind("sqrt"),
            ParseErrorKind::MissingArguments(Function::Sqrt)
        );
        assert_eq!(
            error_kind("mm * 2"),
            ParseErrorKind::UnitWithoutNumber("mm")
        );
        assert_eq!(error_kind("(1 + 2"), ParseErrorKind::UnclosedParenthesis);
        assert_eq!(
            error_kind("1 +"),
            ParseErrorKind::UnexpectedEnd {
                expected: "a number, name or '('"
            }
        );
        assert_eq!(
            error_kind("1.2.3"),
            ParseErrorKind::InvalidNumber("1.2.3".to_owned())
        );
        assert_eq!(
            error_kind("if(0, 1e999, 1) mm"),
            ParseErrorKind::NumberTooLarge("1e999".to_owned())
        );
        assert_eq!(
            Expression::parse_stored("1e-999").map(|expression| expression.to_stored_text()),
            Ok("0".to_owned())
        );
        assert_eq!(error_kind("3 $"), ParseErrorKind::UnexpectedCharacter('$'));
        assert_eq!(
            error_kind("atan2(1)"),
            ParseErrorKind::WrongArgumentCount {
                function: Function::Atan2,
                expected: Arity::Exactly(2)
            }
        );
        let error = parse("2 width").unwrap_err();
        assert_eq!(error.span, 2..7);
        assert_eq!(
            error.to_string(),
            "Expected an operator such as + or * but found 'width'"
        );
    }

    #[test]
    fn hostile_input_is_rejected_instead_of_overflowing_the_stack() {
        assert_eq!(error_kind(&"(".repeat(500)), ParseErrorKind::TooDeep);
        assert_eq!(error_kind(&"-".repeat(500)), ParseErrorKind::TooDeep);
        assert_eq!(
            error_kind(&"1".repeat(MAX_LENGTH + 1)),
            ParseErrorKind::TooLong
        );
    }

    #[test]
    fn the_deepest_stored_expressions_are_used_on_a_worker_with_the_default_stack() {
        const DEFAULT_WORKER_STACK: usize = 2 * 1024 * 1024;
        let terms = [
            "$0",
            "if($1 > $2, $0, $1)",
            "max($0, 1 mm)",
            "-hypot($0, $2)",
            "($1 / 1 mm + 2) mm",
            "$0 * 3 / 3",
        ];
        let chain = |count: usize| -> String {
            let mut text =
                terms
                    .iter()
                    .cycle()
                    .take(count)
                    .fold(String::new(), |mut text, term| {
                        text.push_str(term);
                        text.push_str(" + ");
                        text
                    });
            text.truncate(text.len() - " + ".len());
            text
        };
        let deeper_than_its_terms = crate::parse::MAX_TREE_DEPTH - 2;
        let deepest = chain(deeper_than_its_terms);
        let too_deep = chain(deeper_than_its_terms + 1);

        let worker = std::thread::Builder::new()
            .stack_size(DEFAULT_WORKER_STACK)
            .spawn(move || {
                let expression = Expression::parse_stored(&deepest).unwrap();
                let depth = expression.depth();
                let value = expression
                    .evaluate_as(Dimension::LENGTH, &value_of)
                    .unwrap();
                let printed = expression.to_text(&name_of);
                let stored = expression.to_stored_text();
                let reparsed = Expression::parse_stored(&stored).unwrap();
                let same = reparsed == expression;
                let copy = expression.clone();
                drop(expression);
                drop(reparsed);
                drop(copy);
                let refused = Expression::parse_stored(&too_deep).unwrap_err().kind;
                (depth, value, printed, same, refused)
            })
            .unwrap();
        let (depth, value, printed, same, refused) = worker.join().unwrap();

        assert_eq!(depth, crate::parse::MAX_TREE_DEPTH);
        assert_eq!(value, 166.0 * 142.0 + 80.0);
        assert!(printed.starts_with("width + if(height > gap, width, height) + max(width, 1 mm)"));
        assert!(same);
        assert_eq!(refused, ParseErrorKind::TooDeep);
    }

    #[test]
    fn printing_is_canonical_and_parses_back_to_the_same_expression() {
        let cases = [
            ("width+2*height", "width + 2 * height"),
            ("(width+2)*2", "(width + 2) * 2"),
            ("width-(height-gap)", "width - (height - gap)"),
            ("width - height - gap", "width - height - gap"),
            ("-2^2", "-2^2"),
            ("(-2)^2", "(-2)^2"),
            ("(2^3)^2", "(2^3)^2"),
            ("2^-1", "2^-1"),
            ("10mm", "10 mm"),
            ("30°", "30 deg"),
            ("--width", "--width"),
            ("max( width ,2 cm )", "max(width, 2 cm)"),
            ("(2+3) mm", "(2 + 3) mm"),
            ("width mm", "width mm"),
            ("-2 mm", "-2 mm"),
            ("(-2) mm", "(-2) mm"),
            ("2 mm²", "2 mm²"),
            ("width < 2 mm", "width < 2 mm"),
            (
                "if(width >= height, width, height)",
                "if(width >= height, width, height)",
            ),
            ("π/2", "pi / 2"),
            ("1 / (2 * width)", "1 / (2 * width)"),
        ];
        for (input, printed) in cases {
            let expression = parse(input).unwrap();
            let text = expression.to_text(&name_of);
            assert_eq!(text, printed);
            assert_eq!(parse(&text).unwrap(), expression, "{input}");
        }
    }

    #[test]
    fn stored_text_refers_by_id_and_round_trips_exactly() {
        let cases = [
            "width + 2 * height",
            "-(width - gap) / 3",
            "0.1 + 1e-7 * 12345678901234567 mm",
            "max(width, 2 cm) ^ 2 / sqrt(height * height)",
            "(width + 1) mm² / 3 mm",
            "if(width != gap, round(width, 5 mm), 0 mm)",
            "30 deg + atan2(height, gap) - pi",
        ];
        for input in cases {
            let expression = parse(input).unwrap();
            let stored = expression.to_stored_text();
            assert!(!stored.contains("width"), "{stored}");
            assert_eq!(
                Expression::parse_stored(&stored).unwrap(),
                expression,
                "{stored}"
            );
        }
        assert_eq!(parse("width / 2").unwrap().to_stored_text(), "$0 / 2");
        assert_eq!(Expression::Number(1e-300).to_stored_text(), "1e-300");
        assert_eq!(Expression::Number(0.25).to_stored_text(), "0.25");
    }

    #[test]
    fn stored_text_rejects_names_and_malformed_references() {
        let kind = |text: &str| Expression::parse_stored(text).unwrap_err().kind;
        assert_eq!(
            kind("width"),
            ParseErrorKind::UnknownParameter("width".to_owned())
        );
        assert_eq!(kind("$"), ParseErrorKind::InvalidReference("$".to_owned()));
        assert_eq!(
            kind("$99999999999999999999999"),
            ParseErrorKind::InvalidReference("$99999999999999999999999".to_owned())
        );
        assert_eq!(error_kind("$0"), ParseErrorKind::UnexpectedCharacter('$'));
        let deep = vec!["$0"; MAX_LENGTH + 2].join("+");
        assert_eq!(kind(&deep), ParseErrorKind::TooDeep);
        let longest = vec!["$0"; MAX_LENGTH * 16].join("*");
        let started = std::time::Instant::now();
        let error = Expression::parse_stored(&longest).unwrap_err();
        assert_eq!(error.kind, ParseErrorKind::TooDeep);
        assert!(error.span.start < MAX_LENGTH * 4, "{:?}", error.span);
        assert!(started.elapsed().as_secs() < 2);
    }

    #[test]
    fn references_are_by_id_so_renaming_changes_the_text() {
        let expression = parse("width / 2 + gap").unwrap();
        assert!(expression.uses(GAP));
        assert!(!expression.uses(HEIGHT));
        assert_eq!(
            expression.parameters().into_iter().collect::<Vec<_>>(),
            vec![WIDTH, GAP]
        );
        let renamed = expression.to_text(&|id| (id == WIDTH).then_some("span").or(name_of(id)));
        assert_eq!(renamed, "span / 2 + gap");
        assert!(!parse("12 mm").unwrap().uses(WIDTH));
        assert!(parse("-12 mm").unwrap().is_literal());
        assert!(!parse("width").unwrap().is_literal());
    }

    #[test]
    fn names_must_be_identifiers_that_are_not_reserved() {
        assert_eq!(check_name("width_2"), Ok(()));
        assert_eq!(check_name("Länge"), Ok(()));
        assert_eq!(check_name(""), Err(NameError::Empty));
        assert_eq!(check_name("2width"), Err(NameError::InvalidStart));
        assert_eq!(
            check_name("wide gap"),
            Err(NameError::InvalidCharacter(' '))
        );
        assert_eq!(check_name("mm"), Err(NameError::Unit("mm".to_owned())));
        assert_eq!(
            check_name("sin"),
            Err(NameError::Function("sin".to_owned()))
        );
        assert_eq!(check_name("pi"), Err(NameError::Constant("pi".to_owned())));
    }

    #[test]
    fn units_follow_any_primary_and_areas_can_be_typed() {
        assert_eq!(evaluate("(2 + 3) mm"), Ok(Quantity::length(5.0)));
        assert_eq!(evaluate("(10 / 2) cm"), Ok(Quantity::length(50.0)));
        assert_eq!(evaluate("width / 20 mm"), Ok(Quantity::plain(2.0)));
        assert_eq!(evaluate("3 cm²"), Ok(Quantity::new(300.0, Dimension::AREA)));
        assert_eq!(evaluate("2 mm³"), Ok(Quantity::new(2.0, Dimension::VOLUME)));
        assert_eq!(evaluate("pi rad"), Ok(Quantity::angle(180.0)));
        assert_eq!(evaluate("mod(-1, 360) deg"), Ok(Quantity::angle(359.0)));
    }

    #[test]
    fn misspelled_and_imperial_units_say_which_units_exist() {
        let hint = |text: &str| parse(text).unwrap_err().to_string();
        assert_eq!(
            hint("10 MM"),
            "'MM' is not a unit; write mm, as units are um, mm, cm, m, deg and rad, and mm² or \
             mm³ for areas and volumes"
        );
        assert!(hint("10 degrees").starts_with("'degrees' is not a unit; write deg"));
        assert!(hint("10 millimeters").starts_with("'millimeters' is not a unit; write mm"));
        assert_eq!(
            hint("3 ft"),
            "caditor works in SI units; write mm, cm or m instead of ft"
        );
        assert!(matches!(
            error_kind("10 height"),
            ParseErrorKind::Unexpected { .. }
        ));
    }

    #[test]
    fn rounding_takes_a_step_in_any_unit() {
        assert_eq!(evaluate("round(1.26 cm, 1 cm)"), Ok(Quantity::length(10.0)));
        assert_eq!(evaluate("floor(width, 15 mm)"), Ok(Quantity::length(30.0)));
        assert_eq!(evaluate("ceil(41 mm, 0.5 cm)"), Ok(Quantity::length(45.0)));
        assert_eq!(evaluate("round(1.26 cm)"), Ok(Quantity::length(13.0)));
        assert_eq!(
            evaluate("round(width, 0 mm)"),
            Err(EvalError::InvalidStep {
                function: Function::Round
            })
        );
    }

    #[test]
    fn rounding_and_mod_snap_a_quotient_within_the_equality_tolerance() {
        assert_eq!(evaluate("floor(2.8 mm / 0.4 mm)"), Ok(Quantity::plain(7.0)));
        assert_eq!(evaluate("ceil(0.7 / 0.1)"), Ok(Quantity::plain(7.0)));
        assert_eq!(evaluate("trunc(0.3 / 0.1)"), Ok(Quantity::plain(3.0)));
        assert_eq!(evaluate("round(2.5)"), Ok(Quantity::plain(3.0)));
        assert_eq!(evaluate("floor(2.9)"), Ok(Quantity::plain(2.0)));
        assert_eq!(evaluate("mod(0.3, 0.1)"), Ok(Quantity::plain(0.0)));
        assert_eq!(evaluate("mod(2.8 mm, 0.4 mm)"), Ok(Quantity::length(0.0)));
        assert_eq!(evaluate("mod(-1, 360)"), Ok(Quantity::plain(359.0)));
    }

    #[test]
    fn intermediate_values_must_stay_real() {
        assert_eq!(evaluate("(-4)^0.5"), Err(EvalError::NegativeBase));
        assert_eq!(
            evaluate("max((-8)^(1/3), 5 mm)"),
            Err(EvalError::NegativeBase)
        );
        assert_eq!(evaluate("exp(1000) * 0 + 1"), Err(EvalError::NotFinite));
    }

    #[test]
    fn comparisons_conditions_and_more_functions() {
        assert_eq!(evaluate("width > height"), Ok(Quantity::plain(1.0)));
        assert_eq!(evaluate("width <= height"), Ok(Quantity::plain(0.0)));
        assert_eq!(evaluate("0.1 + 0.2 == 0.3"), Ok(Quantity::plain(1.0)));
        assert_eq!(
            evaluate("if(gap > 0 mm, width / gap, 3 mm)"),
            Ok(Quantity::length(3.0))
        );
        assert_eq!(evaluate("hypot(30 mm, 40 mm)"), Ok(Quantity::length(50.0)));
        assert_eq!(evaluate("mod(width, 15 mm)"), Ok(Quantity::length(10.0)));
        assert_eq!(evaluate("sign(-gap - 1 mm)"), Ok(Quantity::plain(-1.0)));
        assert_eq!(
            evaluate("clamp(width, 0 mm, 25 mm)"),
            Ok(Quantity::length(25.0))
        );
        assert_eq!(
            evaluate("clamp(width, 5 mm, 1 mm)"),
            Err(EvalError::InvertedLimits)
        );
        assert!((evaluate("ln(e)").unwrap().value - 1.0).abs() < 1e-12);
        assert!((evaluate("exp(0) + tau / pi").unwrap().value - 3.0).abs() < 1e-12);
        assert_eq!(
            evaluate("ln(0)"),
            Err(EvalError::NotPositive {
                function: Function::Ln
            })
        );
        assert!(matches!(
            evaluate("width < 3 deg"),
            Err(EvalError::Mismatch { .. })
        ));
        assert!(check_name("tau").is_err() && check_name("e").is_err());
    }

    #[test]
    fn a_computed_plain_number_is_not_taken_for_an_angle() {
        let angle = |text: &str| {
            parse(text)
                .unwrap()
                .evaluate_as(Dimension::ANGLE, &value_of)
        };
        assert_eq!(angle("90"), Ok(90.0));
        assert!((angle("(pi / 2) rad").unwrap() - 90.0).abs() < 1e-12);
        assert!((angle("pi rad / 2").unwrap() - 90.0).abs() < 1e-12);
        assert_eq!(angle("pi / 2"), Err(EvalError::PlainAngle));
        assert_eq!(angle("width / height"), Err(EvalError::PlainAngle));
        let length = parse("width / height")
            .unwrap()
            .evaluate_as(Dimension::LENGTH, &value_of);
        assert_eq!(length, Ok(2.0));
    }
}
