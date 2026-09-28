use caditor_expression::{BinaryOperator, Dimension, Expression, Quantity, Unit, format_number};

use crate::sketch_tools::rounded_for_display;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LengthUnit {
    #[default]
    Millimetre,
    Centimetre,
    Metre,
}

impl LengthUnit {
    pub const ALL: [Self; 3] = [Self::Millimetre, Self::Centimetre, Self::Metre];

    pub fn label(self) -> &'static str {
        match self {
            Self::Millimetre => "Millimetres",
            Self::Centimetre => "Centimetres",
            Self::Metre => "Metres",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            Self::Millimetre => Unit::Millimetre,
            Self::Centimetre => Unit::Centimetre,
            Self::Metre => Unit::Metre,
        }
    }

    pub fn symbol(self) -> &'static str {
        self.unit().symbol()
    }

    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.symbol() == symbol)
    }

    pub fn millimetres(self) -> f64 {
        self.unit().quantity(1.0).value
    }

    pub fn show(self, quantity: Quantity) -> String {
        let power = quantity.dimension.length_power();
        if self == Self::Millimetre || quantity.dimension.angle_power() != 0 || power == 0 {
            return quantity.to_string();
        }
        let value = quantity.value / self.millimetres().powi(i32::from(power));
        let symbol = self.symbol();
        let number = format_number(value);
        match power {
            1 => format!("{number} {symbol}"),
            2 => format!("{number} {symbol}²"),
            3 => format!("{number} {symbol}³"),
            other => format!("{number} {symbol}^{other}"),
        }
    }

    pub fn small_length_text(self, millimetres: f64) -> String {
        let value = millimetres / self.millimetres();
        let digits = (1.0 - value.log10().floor()).clamp(0.0, 9.0) as usize;
        format!("{value:.digits$} {}", self.symbol())
    }

    pub fn length_text(self, millimetres: f64, decimals: usize) -> String {
        format!(
            "{:.decimals$} {}",
            millimetres / self.millimetres(),
            self.symbol()
        )
    }

    pub fn measured(self, millimetres: f64) -> Expression {
        Expression::Measure(
            rounded_for_display(millimetres / self.millimetres()),
            self.unit(),
        )
    }

    pub fn default_length(self, millimetres: f64) -> Expression {
        let value = millimetres / self.millimetres();
        if self == Self::Millimetre || value == 0.0 || !value.is_finite() {
            return Expression::Measure(value, self.unit());
        }
        let magnitude = 10f64.powf(value.abs().log10().floor());
        let rounded = (value / magnitude).round() * magnitude;
        Expression::Measure(
            rounded_for_display(if rounded == 0.0 { value } else { rounded }),
            self.unit(),
        )
    }

    pub fn attach(self, expression: Expression) -> Expression {
        match expression {
            Expression::Number(value) => Expression::Measure(value, self.unit()),
            Expression::Negate(inner) if matches!(*inner, Expression::Number(_)) => {
                Expression::Negate(Box::new(self.attach(*inner)))
            }
            other => Expression::binary(
                BinaryOperator::Multiply,
                other,
                Expression::Measure(1.0, self.unit()),
            ),
        }
    }

    pub fn applies_to(self, expected: Option<Dimension>, found: Dimension) -> bool {
        self != Self::Millimetre && expected == Some(Dimension::LENGTH) && found.is_plain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_show_in_the_chosen_unit() {
        assert_eq!(
            LengthUnit::Millimetre.show(Quantity::length(25.4)),
            "25.4 mm"
        );
        assert_eq!(
            LengthUnit::Centimetre.show(Quantity::length(25.4)),
            "2.54 cm"
        );
        assert_eq!(
            LengthUnit::Centimetre.show(Quantity::new(250.0, Dimension::AREA)),
            "2.5 cm²"
        );
        assert_eq!(LengthUnit::Metre.show(Quantity::angle(30.0)), "30°");
        assert_eq!(LengthUnit::Metre.show(Quantity::plain(3.0)), "3");
        assert_eq!(LengthUnit::Metre.length_text(1250.0, 2), "1.25 m");
    }

    #[test]
    fn new_lengths_are_round_numbers_in_the_chosen_unit() {
        assert_eq!(
            LengthUnit::Metre.default_length(10.0),
            Expression::Measure(0.01, Unit::Metre)
        );
        assert_eq!(
            LengthUnit::Millimetre.default_length(10.0),
            Expression::Measure(10.0, Unit::Millimetre)
        );
        assert_eq!(
            LengthUnit::Centimetre.default_length(1.0),
            Expression::Measure(0.1, Unit::Centimetre)
        );
        assert_eq!(
            LengthUnit::Centimetre.measured(25.4),
            Expression::Measure(2.54, Unit::Centimetre)
        );
    }

    #[test]
    fn plain_input_takes_the_chosen_unit() {
        assert_eq!(
            LengthUnit::Centimetre.attach(Expression::Number(2.0)),
            Expression::Measure(2.0, Unit::Centimetre)
        );
        let sum = Expression::binary(
            BinaryOperator::Add,
            Expression::Number(1.0),
            Expression::Number(2.0),
        );
        let attached = LengthUnit::Metre.attach(sum.clone());
        assert_eq!(
            attached,
            Expression::binary(
                BinaryOperator::Multiply,
                sum,
                Expression::Measure(1.0, Unit::Metre)
            )
        );
        assert!(LengthUnit::Metre.applies_to(Some(Dimension::LENGTH), Dimension::NONE));
        assert!(!LengthUnit::Millimetre.applies_to(Some(Dimension::LENGTH), Dimension::NONE));
        assert!(!LengthUnit::Metre.applies_to(Some(Dimension::ANGLE), Dimension::NONE));
        assert_eq!(LengthUnit::from_symbol("cm"), Some(LengthUnit::Centimetre));
        assert_eq!(LengthUnit::from_symbol("in"), None);
    }
}
