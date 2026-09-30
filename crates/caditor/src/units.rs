use caditor_expression::{Dimension, Expression, Quantity, Unit, format_number};

use crate::sketch_tools::rounded_for_display;

const READOUT_DECIMALS_IN_MILLIMETRES: f64 = 2.0;
const MEASURED_LENGTH_DECIMALS: f64 = 3.0;
const MEASURED_AREA_DECIMALS: f64 = 2.0;
const MEASURED_VOLUME_DECIMALS: f64 = 1.0;
const MAX_MEASURED_DECIMALS: f64 = 9.0;
const ANGLE_DECIMALS: usize = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LengthUnit {
    Micrometre,
    #[default]
    Millimetre,
    Centimetre,
    Metre,
}

impl LengthUnit {
    pub const ALL: [Self; 4] = [
        Self::Micrometre,
        Self::Millimetre,
        Self::Centimetre,
        Self::Metre,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Micrometre => "Micrometres",
            Self::Millimetre => "Millimetres",
            Self::Centimetre => "Centimetres",
            Self::Metre => "Metres",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            Self::Micrometre => Unit::Micrometre,
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

    pub fn readout_text(self, millimetres: f64) -> String {
        let decimals = (READOUT_DECIMALS_IN_MILLIMETRES + self.millimetres().log10())
            .round()
            .max(0.0) as usize;
        format!(
            "{:.decimals$} {}",
            millimetres / self.millimetres(),
            self.symbol()
        )
    }

    pub fn measured_length(self, millimetres: f64) -> String {
        self.measured_power(millimetres, 1, MEASURED_LENGTH_DECIMALS)
    }

    pub fn measured_area(self, square_millimetres: f64) -> String {
        self.measured_power(square_millimetres, 2, MEASURED_AREA_DECIMALS)
    }

    pub fn measured_volume(self, cubic_millimetres: f64) -> String {
        self.measured_power(cubic_millimetres, 3, MEASURED_VOLUME_DECIMALS)
    }

    pub fn measured_position(self, coordinates: [f64; 3]) -> String {
        let [x, y, z] = coordinates
            .map(|millimetres| self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS));
        format!("{x}, {y}, {z} {}", self.symbol())
    }

    fn measured_power(self, value: f64, power: i32, decimals_in_millimetres: f64) -> String {
        let number = self.measured_number(value, power, decimals_in_millimetres);
        let symbol = self.symbol();
        match power {
            1 => format!("{number} {symbol}"),
            2 => format!("{number} {symbol}²"),
            _ => format!("{number} {symbol}³"),
        }
    }

    fn measured_number(self, value: f64, power: i32, decimals_in_millimetres: f64) -> String {
        let scaled = value / self.millimetres().powi(power);
        let decimals = (decimals_in_millimetres + f64::from(power) * self.millimetres().log10())
            .round()
            .clamp(0.0, MAX_MEASURED_DECIMALS);
        let step = 10f64.powf(decimals);
        let rounded = (scaled * step).round() / step;
        let shown = if rounded == 0.0 { 0.0 } else { rounded };
        let decimals = decimals as usize;
        format!("{shown:.decimals$}")
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
        attach_unit(expression, self.unit())
    }

    pub fn applies_to(self, expected: Option<Dimension>, found: Dimension) -> bool {
        self != Self::Millimetre && expected == Some(Dimension::LENGTH) && found.is_plain()
    }
}

pub fn angle_text(radians: f64) -> String {
    format!("{:.ANGLE_DECIMALS$}°", radians.to_degrees())
}

pub fn attach_unit(expression: Expression, unit: Unit) -> Expression {
    match expression {
        Expression::Number(value) => Expression::Measure(value, unit),
        Expression::Negate(inner) if matches!(*inner, Expression::Number(_)) => {
            Expression::Negate(Box::new(attach_unit(*inner, unit)))
        }
        other => Expression::WithUnit(Box::new(other), unit, 1),
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
        assert_eq!(
            LengthUnit::Micrometre.show(Quantity::length(0.25)),
            "250 um"
        );
    }

    #[test]
    fn the_cursor_readout_resolves_a_hundredth_of_a_millimetre_in_every_unit() {
        assert_eq!(LengthUnit::Millimetre.readout_text(12.345), "12.35 mm");
        assert_eq!(LengthUnit::Centimetre.readout_text(12.345), "1.235 cm");
        assert_eq!(LengthUnit::Metre.readout_text(1250.004), "1.25000 m");
        assert_eq!(LengthUnit::Micrometre.readout_text(0.0123), "12 um");
    }

    #[test]
    fn measurements_resolve_a_micrometre_in_every_unit_and_never_show_minus_zero() {
        assert_eq!(LengthUnit::Millimetre.measured_length(12.3456), "12.346 mm");
        assert_eq!(LengthUnit::Centimetre.measured_length(12.3456), "1.2346 cm");
        assert_eq!(LengthUnit::Metre.measured_length(1250.0), "1.250000 m");
        assert_eq!(LengthUnit::Micrometre.measured_length(0.0123), "12 um");
        assert_eq!(LengthUnit::Millimetre.measured_length(-1e-9), "0.000 mm");
        assert_eq!(LengthUnit::Millimetre.measured_area(200.0), "200.00 mm²");
        assert_eq!(LengthUnit::Centimetre.measured_area(200.0), "2.0000 cm²");
        assert_eq!(LengthUnit::Millimetre.measured_volume(1000.0), "1000.0 mm³");
        assert_eq!(
            LengthUnit::Millimetre.measured_position([1.0, -2.5, -0.0000001]),
            "1.000, -2.500, 0.000 mm"
        );
        assert_eq!(angle_text(std::f64::consts::FRAC_PI_4), "45.00°");
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
            caditor_expression::BinaryOperator::Add,
            Expression::Number(1.0),
            Expression::Number(2.0),
        );
        let attached = LengthUnit::Metre.attach(sum.clone());
        assert_eq!(
            attached,
            Expression::WithUnit(Box::new(sum), Unit::Metre, 1)
        );
        assert!(LengthUnit::Metre.applies_to(Some(Dimension::LENGTH), Dimension::NONE));
        assert!(!LengthUnit::Millimetre.applies_to(Some(Dimension::LENGTH), Dimension::NONE));
        assert!(!LengthUnit::Metre.applies_to(Some(Dimension::ANGLE), Dimension::NONE));
        assert_eq!(LengthUnit::from_symbol("cm"), Some(LengthUnit::Centimetre));
        assert_eq!(LengthUnit::from_symbol("um"), Some(LengthUnit::Micrometre));
        assert_eq!(LengthUnit::from_symbol("in"), None);
    }
}
