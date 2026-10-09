use std::ops::Deref;

use caditor_expression::{Dimension, Expression, Quantity, Unit, format_number};

use crate::sketch_tools::{rounded_for_display, rounded_to_decimals};

const READOUT_DECIMALS_IN_MILLIMETRES: f64 = 2.0;
const MEASURED_LENGTH_DECIMALS: f64 = 3.0;
const MEASURED_AREA_DECIMALS: f64 = 2.0;
const MEASURED_VOLUME_DECIMALS: f64 = 1.0;
const MEASURED_SECOND_MOMENT_DECIMALS: f64 = 1.0;
const MAX_MEASURED_DECIMALS: f64 = 9.0;
const ANGLE_DECIMALS: usize = 2;
const RADIAN_DECIMALS: usize = 4;
const MODEL_LENGTH_STEP_IN_MILLIMETRES: f64 = 0.001;
const MODEL_ANGLE_STEP_IN_DEGREES: f64 = 0.001;

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

    pub fn grid_text(self, spacing_millimetres: f64) -> String {
        let spacing = spacing_millimetres / self.millimetres();
        let decimals = (-spacing.log10().floor()).max(0.0) as usize;
        format!("Grid {spacing:.decimals$} {}", self.symbol())
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

    pub fn measured_second_moment(self, millimetres_to_the_fourth: f64) -> String {
        self.measured_power(
            millimetres_to_the_fourth,
            4,
            MEASURED_SECOND_MOMENT_DECIMALS,
        )
    }

    pub fn measured_position(self, coordinates: [f64; 3]) -> String {
        let [x, y, z] = coordinates
            .map(|millimetres| self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS));
        format!("{x}, {y}, {z} {}", self.symbol())
    }

    pub fn spoken_length(self, millimetres: f64) -> String {
        format!("{} {}", self.spoken_number(millimetres), self.symbol())
    }

    pub fn spoken_position(self, coordinates: [f64; 3]) -> String {
        let [x, y, z] = coordinates.map(|millimetres| self.spoken_number(millimetres));
        format!("{x}, {y}, {z} {}", self.symbol())
    }

    pub fn spoken_point(self, coordinates: [f64; 2]) -> String {
        let [x, y] = coordinates.map(|millimetres| self.spoken_number(millimetres));
        format!("{x}, {y} {}", self.symbol())
    }

    fn spoken_number(self, millimetres: f64) -> String {
        let number = self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS);
        if number.contains('.') {
            number
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        } else {
            number
        }
    }

    pub fn measured_size(self, extents: [f64; 3]) -> String {
        let [x, y, z] = extents
            .map(|millimetres| self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS));
        format!("{x} × {y} × {z} {}", self.symbol())
    }

    fn measured_power(self, value: f64, power: i32, decimals_in_millimetres: f64) -> String {
        let number = self.measured_number(value, power, decimals_in_millimetres);
        let symbol = self.symbol();
        match power {
            1 => format!("{number} {symbol}"),
            2 => format!("{number} {symbol}²"),
            3 => format!("{number} {symbol}³"),
            4 => format!("{number} {symbol}⁴"),
            other => format!("{number} {symbol}^{other}"),
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
        let step = MODEL_LENGTH_STEP_IN_MILLIMETRES / self.millimetres();
        Expression::measure(
            rounded_to_decimals(millimetres / self.millimetres(), decimals_for(step)),
            self.unit(),
        )
    }

    pub fn measured_area_expression(self, square_millimetres: f64) -> Expression {
        let square = self.millimetres().powi(2);
        let step = MODEL_LENGTH_STEP_IN_MILLIMETRES.powi(2) / square;
        let value = rounded_to_decimals(square_millimetres / square, decimals_for(step));
        Expression::WithUnit(Box::new(Expression::number(value)), self.unit(), 2)
    }

    pub fn default_length(self, millimetres: f64) -> Expression {
        let value = millimetres / self.millimetres();
        if self == Self::Millimetre || value == 0.0 || !value.is_finite() {
            return Expression::measure(value, self.unit());
        }
        let magnitude = 10f64.powf(value.abs().log10().floor());
        let rounded = (value / magnitude).round() * magnitude;
        Expression::measure(
            rounded_for_display(if rounded == 0.0 { value } else { rounded }),
            self.unit(),
        )
    }

    pub fn attach(self, expression: Expression) -> Expression {
        attach_unit(expression, self.unit())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AngleUnit {
    #[default]
    Degree,
    Radian,
}

impl AngleUnit {
    pub const ALL: [Self; 2] = [Self::Degree, Self::Radian];

    pub fn label(self) -> &'static str {
        match self {
            Self::Degree => "Degrees",
            Self::Radian => "Radians",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            Self::Degree => Unit::Degree,
            Self::Radian => Unit::Radian,
        }
    }

    pub fn symbol(self) -> &'static str {
        self.unit().symbol()
    }

    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.symbol() == symbol)
    }

    pub fn text(self, degrees: f64) -> String {
        match self {
            Self::Degree => format!("{degrees:.ANGLE_DECIMALS$}°"),
            Self::Radian => format!("{:.RADIAN_DECIMALS$} rad", degrees.to_radians()),
        }
    }

    pub fn text_of_radians(self, radians: f64) -> String {
        self.text(radians.to_degrees())
    }

    pub fn readout_text(self, degrees: f64) -> String {
        match self {
            Self::Degree => format!("{degrees:.1}°"),
            Self::Radian => format!("{:.3} rad", degrees.to_radians()),
        }
    }

    pub fn measured(self, degrees: f64) -> Expression {
        let (value, step) = match self {
            Self::Degree => (degrees, MODEL_ANGLE_STEP_IN_DEGREES),
            Self::Radian => (
                degrees.to_radians(),
                MODEL_ANGLE_STEP_IN_DEGREES.to_radians(),
            ),
        };
        Expression::measure(rounded_to_decimals(value, decimals_for(step)), self.unit())
    }

    pub fn attach(self, expression: Expression) -> Expression {
        attach_unit(expression, self.unit())
    }
}

fn decimals_for(step: f64) -> f64 {
    (-step.log10()).ceil().max(0.0)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Units {
    pub length: LengthUnit,
    pub angle: AngleUnit,
}

impl Deref for Units {
    type Target = LengthUnit;

    fn deref(&self) -> &LengthUnit {
        &self.length
    }
}

impl From<LengthUnit> for Units {
    fn from(length: LengthUnit) -> Self {
        Self {
            length,
            angle: AngleUnit::default(),
        }
    }
}

impl Units {
    pub fn show(self, quantity: Quantity) -> String {
        let angle = quantity.dimension == Dimension::ANGLE;
        if angle && self.angle == AngleUnit::Radian {
            return self.angle.text(quantity.value);
        }
        self.length.show(quantity)
    }

    pub fn attach_plain(self, expected: Option<Dimension>, found: Dimension) -> Option<Unit> {
        if !found.is_plain() {
            return None;
        }
        match expected {
            Some(Dimension::LENGTH) if self.length != LengthUnit::Millimetre => {
                Some(self.length.unit())
            }
            Some(Dimension::ANGLE) if self.angle == AngleUnit::Radian => Some(self.angle.unit()),
            _ => None,
        }
    }
}

pub fn attach_unit(expression: Expression, unit: Unit) -> Expression {
    match expression {
        Expression::Number(value) => Expression::measure(value, unit),
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
    fn angles_show_and_attach_in_the_chosen_unit() {
        let radians = Units {
            length: LengthUnit::Millimetre,
            angle: AngleUnit::Radian,
        };

        assert_eq!(AngleUnit::Degree.text(45.0), "45.00°");
        assert_eq!(AngleUnit::Radian.text(180.0), "3.1416 rad");
        assert_eq!(
            AngleUnit::Radian.text_of_radians(std::f64::consts::FRAC_PI_2),
            "1.5708 rad"
        );
        assert_eq!(AngleUnit::Degree.readout_text(30.04), "30.0°");
        assert_eq!(AngleUnit::Radian.readout_text(90.0), "1.571 rad");
        assert_eq!(radians.show(Quantity::angle(90.0)), "1.5708 rad");
        assert_eq!(radians.show(Quantity::length(5.0)), "5 mm");
        assert_eq!(Units::default().show(Quantity::angle(90.0)), "90°");
        assert_eq!(
            radians.attach_plain(Some(Dimension::ANGLE), Dimension::NONE),
            Some(Unit::Radian)
        );
        assert_eq!(
            AngleUnit::Radian.measured(100.0),
            Expression::Measure(1.74533, Unit::Radian)
        );
        assert_eq!(
            AngleUnit::Radian.measured(10.0),
            Expression::Measure(0.174533, Unit::Radian)
        );
        assert_eq!(
            AngleUnit::Degree.measured(37.5),
            Expression::Measure(37.5, Unit::Degree)
        );
        assert_eq!(AngleUnit::from_symbol("rad"), Some(AngleUnit::Radian));
        assert_eq!(AngleUnit::from_symbol("grad"), None);
    }

    #[test]
    fn measured_dimensions_keep_the_model_precision_in_any_unit() {
        assert_eq!(
            LengthUnit::Metre.measured(1234.5678),
            Expression::Measure(1.234568, Unit::Metre)
        );
        assert_eq!(
            LengthUnit::Centimetre.measured(12.34567),
            Expression::Measure(1.2346, Unit::Centimetre)
        );
        assert_eq!(
            LengthUnit::Millimetre.measured(37.253_123),
            Expression::Measure(37.253, Unit::Millimetre)
        );
        assert_eq!(
            LengthUnit::Micrometre.measured(0.012_4),
            Expression::Measure(12.0, Unit::Micrometre)
        );
        assert_eq!(
            AngleUnit::Degree.measured(30.000_4),
            Expression::Measure(30.0, Unit::Degree)
        );
    }

    #[test]
    fn the_grid_spacing_names_its_power_of_ten_in_the_chosen_unit() {
        assert_eq!(LengthUnit::Millimetre.grid_text(10.0), "Grid 10 mm");
        assert_eq!(LengthUnit::Millimetre.grid_text(0.01), "Grid 0.01 mm");
        assert_eq!(
            LengthUnit::Millimetre.grid_text(100_000.0),
            "Grid 100000 mm"
        );
        assert_eq!(LengthUnit::Centimetre.grid_text(1.0), "Grid 0.1 cm");
        assert_eq!(LengthUnit::Metre.grid_text(1000.0), "Grid 1 m");
        assert_eq!(LengthUnit::Micrometre.grid_text(0.01), "Grid 10 um");
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
        assert_eq!(
            AngleUnit::Degree.text_of_radians(std::f64::consts::FRAC_PI_4),
            "45.00°"
        );
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
        let metres = Units::from(LengthUnit::Metre);
        assert_eq!(
            metres.attach_plain(Some(Dimension::LENGTH), Dimension::NONE),
            Some(Unit::Metre)
        );
        assert_eq!(
            Units::default().attach_plain(Some(Dimension::LENGTH), Dimension::NONE),
            None
        );
        assert_eq!(
            metres.attach_plain(Some(Dimension::ANGLE), Dimension::NONE),
            None
        );
        assert_eq!(
            metres.attach_plain(Some(Dimension::LENGTH), Dimension::LENGTH),
            None
        );
        assert_eq!(LengthUnit::from_symbol("cm"), Some(LengthUnit::Centimetre));
        assert_eq!(LengthUnit::from_symbol("um"), Some(LengthUnit::Micrometre));
        assert_eq!(LengthUnit::from_symbol("in"), None);
    }
}
