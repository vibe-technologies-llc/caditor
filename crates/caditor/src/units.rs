use std::ops::Deref;

use caditor_expression::{
    Dimension, Expression, Quantity, Unit, format_decimals, format_number, format_places,
    trim_decimals,
};

use crate::sketch_tools::{rounded_for_display, rounded_to_decimals};

const READOUT_DECIMALS_IN_MILLIMETRES: f64 = 2.0;
const MEASURED_LENGTH_DECIMALS: f64 = 3.0;
const MEASURED_AREA_DECIMALS: f64 = 2.0;
const MEASURED_VOLUME_DECIMALS: f64 = 1.0;
const MEASURED_SECOND_MOMENT_DECIMALS: f64 = 1.0;
const MAX_MEASURED_DECIMALS: f64 = 9.0;
const ANGLE_DECIMALS: usize = 2;
const RADIAN_DECIMALS: usize = 4;
const ANGLE_READOUT_DECIMALS: usize = 1;
const RADIAN_READOUT_DECIMALS: usize = 3;
pub const MOST_DECIMALS: u8 = 6;
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

    pub fn small_length_text(self, millimetres: f64) -> String {
        let value = millimetres / self.millimetres();
        let digits = (1.0 - value.log10().floor()).clamp(0.0, 9.0) as usize;
        format!("{value:.digits$} {}", self.symbol())
    }

    pub fn grid_text(self, spacing_millimetres: f64) -> String {
        let spacing = spacing_millimetres / self.millimetres();
        let decimals = (-spacing.log10().floor()).max(0.0) as usize;
        format!("Grid {spacing:.decimals$} {}", self.symbol())
    }

    pub fn measured(self, millimetres: f64) -> Expression {
        let step = MODEL_LENGTH_STEP_IN_MILLIMETRES / self.millimetres();
        Expression::measure(
            rounded_to_decimals(millimetres / self.millimetres(), decimals_for(step)),
            self.unit(),
        )
    }

    pub fn measured_area_expression(self, square_millimetres: f64) -> Expression {
        self.measured_power_expression(square_millimetres, 2)
    }

    pub fn measured_volume_expression(self, cubic_millimetres: f64) -> Expression {
        self.measured_power_expression(cubic_millimetres, 3)
    }

    fn measured_power_expression(self, millimetres_to_the_power: f64, power: i8) -> Expression {
        let scale = self.millimetres().powi(i32::from(power));
        let step = MODEL_LENGTH_STEP_IN_MILLIMETRES.powi(i32::from(power)) / scale;
        let value = rounded_to_decimals(millimetres_to_the_power / scale, decimals_for(step));
        Expression::WithUnit(Box::new(Expression::number(value)), self.unit(), power)
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
pub enum Decimals {
    #[default]
    Automatic,
    Fixed(u8),
}

impl Decimals {
    fn places_or(self, automatic: usize) -> usize {
        match self {
            Self::Automatic => automatic,
            Self::Fixed(places) => usize::from(places.min(MOST_DECIMALS)),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrailingZeros {
    #[default]
    Shown,
    Hidden,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NumberFormat {
    pub lengths: Decimals,
    pub angles: Decimals,
    pub trailing_zeros: TrailingZeros,
}

impl NumberFormat {
    fn places(self, value: f64, places: usize) -> String {
        match self.trailing_zeros {
            TrailingZeros::Shown => format_places(value, places),
            TrailingZeros::Hidden => format_decimals(value, places),
        }
    }

    fn readout(self, value: f64, decimals: Decimals, automatic: usize) -> String {
        match (decimals, self.trailing_zeros) {
            (Decimals::Automatic, TrailingZeros::Shown) => format!("{value:.automatic$}"),
            _ => self.places(value, decimals.places_or(automatic)),
        }
    }

    fn value(self, value: f64, decimals: Decimals) -> String {
        match decimals {
            Decimals::Automatic => format_number(value),
            Decimals::Fixed(_) => self.places(value, decimals.places_or(0)),
        }
    }

    fn rounded(self, value: f64, decimals: Decimals, automatic: f64) -> String {
        let places = match decimals {
            Decimals::Automatic => automatic.clamp(0.0, MAX_MEASURED_DECIMALS),
            Decimals::Fixed(places) => f64::from(places.min(MOST_DECIMALS)),
        };
        let step = 10f64.powf(places);
        let places = places as usize;
        let rounded = (value * step).round() / step;
        let shown = if rounded == 0.0 { 0.0 } else { rounded };
        match self.trailing_zeros {
            TrailingZeros::Shown => format!("{shown:.places$}"),
            TrailingZeros::Hidden => format_decimals(shown, places),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Units {
    pub length: LengthUnit,
    pub angle: AngleUnit,
    pub numbers: NumberFormat,
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
            ..Self::default()
        }
    }
}

impl Units {
    pub fn show(self, quantity: Quantity) -> String {
        let dimension = quantity.dimension;
        if dimension == Dimension::ANGLE {
            return self.angle_value(quantity.value);
        }
        let power = dimension.length_power();
        let decimals = if dimension.angle_power() == 0 {
            self.numbers.lengths
        } else {
            self.numbers.angles
        };
        if self.length == LengthUnit::Millimetre || dimension.angle_power() != 0 || power == 0 {
            let number = match dimension {
                Dimension::NONE => format_number(quantity.value),
                _ => self.numbers.value(quantity.value, decimals),
            };
            return match dimension {
                Dimension::NONE => number,
                dimension => format!("{number} {}", dimension.unit_symbol()),
            };
        }
        let value = quantity.value / self.length.millimetres().powi(i32::from(power));
        let number = self.numbers.value(value, decimals);
        let symbol = self.length.symbol();
        match power {
            1 => format!("{number} {symbol}"),
            2 => format!("{number} {symbol}²"),
            3 => format!("{number} {symbol}³"),
            other => format!("{number} {symbol}^{other}"),
        }
    }

    fn angle_value(self, degrees: f64) -> String {
        match self.angle {
            AngleUnit::Degree => format!("{}°", self.numbers.value(degrees, self.numbers.angles)),
            AngleUnit::Radian => self.angle_text(degrees),
        }
    }

    fn with_symbol(self, number: &str, power: i32) -> String {
        let symbol = self.length.symbol();
        match power {
            1 => format!("{number} {symbol}"),
            2 => format!("{number} {symbol}²"),
            3 => format!("{number} {symbol}³"),
            4 => format!("{number} {symbol}⁴"),
            other => format!("{number} {symbol}^{other}"),
        }
    }

    pub fn readout_text(self, millimetres: f64) -> String {
        let automatic = (READOUT_DECIMALS_IN_MILLIMETRES + self.length.millimetres().log10())
            .round()
            .max(0.0) as usize;
        let value = millimetres / self.length.millimetres();
        let number = self.numbers.readout(value, self.numbers.lengths, automatic);
        format!("{number} {}", self.length.symbol())
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
        format!("{x}, {y}, {z} {}", self.length.symbol())
    }

    pub fn measured_size(self, extents: [f64; 3]) -> String {
        let [x, y, z] = extents
            .map(|millimetres| self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS));
        format!("{x} × {y} × {z} {}", self.length.symbol())
    }

    pub fn spoken_length(self, millimetres: f64) -> String {
        format!(
            "{} {}",
            self.spoken_number(millimetres),
            self.length.symbol()
        )
    }

    pub fn spoken_position(self, coordinates: [f64; 3]) -> String {
        let [x, y, z] = coordinates.map(|millimetres| self.spoken_number(millimetres));
        format!("{x}, {y}, {z} {}", self.length.symbol())
    }

    pub fn spoken_point(self, coordinates: [f64; 2]) -> String {
        let [x, y] = coordinates.map(|millimetres| self.spoken_number(millimetres));
        format!("{x}, {y} {}", self.length.symbol())
    }

    fn spoken_number(self, millimetres: f64) -> String {
        let number = self.measured_number(millimetres, 1, MEASURED_LENGTH_DECIMALS);
        trim_decimals(&number).to_owned()
    }

    fn measured_power(self, value: f64, power: i32, decimals_in_millimetres: f64) -> String {
        let number = self.measured_number(value, power, decimals_in_millimetres);
        self.with_symbol(&number, power)
    }

    fn measured_number(self, value: f64, power: i32, decimals_in_millimetres: f64) -> String {
        let scale = self.length.millimetres();
        let scaled = value / scale.powi(power);
        let automatic = (decimals_in_millimetres + f64::from(power) * scale.log10()).round();
        self.numbers
            .rounded(scaled, self.numbers.lengths, automatic)
    }

    pub fn angle_text(self, degrees: f64) -> String {
        let angles = self.numbers.angles;
        match self.angle {
            AngleUnit::Degree => {
                format!("{}°", self.numbers.readout(degrees, angles, ANGLE_DECIMALS))
            }
            AngleUnit::Radian => format!(
                "{} rad",
                self.numbers
                    .readout(degrees.to_radians(), angles, RADIAN_DECIMALS)
            ),
        }
    }

    pub fn angle_text_of_radians(self, radians: f64) -> String {
        self.angle_text(radians.to_degrees())
    }

    pub fn angle_readout(self, degrees: f64) -> String {
        let angles = self.numbers.angles;
        match self.angle {
            AngleUnit::Degree => format!(
                "{}°",
                self.numbers
                    .readout(degrees, angles, ANGLE_READOUT_DECIMALS)
            ),
            AngleUnit::Radian => format!(
                "{} rad",
                self.numbers
                    .readout(degrees.to_radians(), angles, RADIAN_READOUT_DECIMALS)
            ),
        }
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

    fn in_angle(angle: AngleUnit) -> Units {
        Units {
            angle,
            ..Units::default()
        }
    }

    #[test]
    fn quantities_show_in_the_chosen_unit() {
        assert_eq!(
            Units::from(LengthUnit::Millimetre).show(Quantity::length(25.4)),
            "25.4 mm"
        );
        assert_eq!(
            Units::from(LengthUnit::Centimetre).show(Quantity::length(25.4)),
            "2.54 cm"
        );
        assert_eq!(
            Units::from(LengthUnit::Centimetre).show(Quantity::new(250.0, Dimension::AREA)),
            "2.5 cm²"
        );
        assert_eq!(
            Units::from(LengthUnit::Metre).show(Quantity::angle(30.0)),
            "30°"
        );
        assert_eq!(
            Units::from(LengthUnit::Metre).show(Quantity::plain(3.0)),
            "3"
        );
        assert_eq!(
            Units::from(LengthUnit::Micrometre).show(Quantity::length(0.25)),
            "250 um"
        );
    }

    #[test]
    fn angles_show_and_attach_in_the_chosen_unit() {
        let radians = in_angle(AngleUnit::Radian);

        assert_eq!(in_angle(AngleUnit::Degree).angle_text(45.0), "45.00°");
        assert_eq!(in_angle(AngleUnit::Radian).angle_text(180.0), "3.1416 rad");
        assert_eq!(
            in_angle(AngleUnit::Radian).angle_text_of_radians(std::f64::consts::FRAC_PI_2),
            "1.5708 rad"
        );
        assert_eq!(in_angle(AngleUnit::Degree).angle_readout(30.04), "30.0°");
        assert_eq!(in_angle(AngleUnit::Radian).angle_readout(90.0), "1.571 rad");
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
        assert_eq!(
            Units::from(LengthUnit::Millimetre).readout_text(12.345),
            "12.35 mm"
        );
        assert_eq!(
            Units::from(LengthUnit::Centimetre).readout_text(12.345),
            "1.235 cm"
        );
        assert_eq!(
            Units::from(LengthUnit::Metre).readout_text(1250.004),
            "1.25000 m"
        );
        assert_eq!(
            Units::from(LengthUnit::Micrometre).readout_text(0.0123),
            "12 um"
        );
    }

    #[test]
    fn measurements_resolve_a_micrometre_in_every_unit_and_never_show_minus_zero() {
        assert_eq!(
            Units::from(LengthUnit::Millimetre).measured_length(12.3456),
            "12.346 mm"
        );
        assert_eq!(
            Units::from(LengthUnit::Centimetre).measured_length(12.3456),
            "1.2346 cm"
        );
        assert_eq!(
            Units::from(LengthUnit::Metre).measured_length(1250.0),
            "1.250000 m"
        );
        assert_eq!(
            Units::from(LengthUnit::Micrometre).measured_length(0.0123),
            "12 um"
        );
        assert_eq!(
            Units::from(LengthUnit::Millimetre).measured_length(-1e-9),
            "0.000 mm"
        );
        assert_eq!(
            Units::from(LengthUnit::Millimetre).measured_area(200.0),
            "200.00 mm²"
        );
        assert_eq!(
            Units::from(LengthUnit::Centimetre).measured_area(200.0),
            "2.0000 cm²"
        );
        assert_eq!(
            Units::from(LengthUnit::Millimetre).measured_volume(1000.0),
            "1000.0 mm³"
        );
        assert_eq!(
            Units::from(LengthUnit::Millimetre).measured_position([1.0, -2.5, -0.0000001]),
            "1.000, -2.500, 0.000 mm"
        );
        assert_eq!(
            in_angle(AngleUnit::Degree).angle_text_of_radians(std::f64::consts::FRAC_PI_4),
            "45.00°"
        );
    }

    fn formatted(lengths: Decimals, angles: Decimals, trailing_zeros: TrailingZeros) -> Units {
        Units {
            numbers: NumberFormat {
                lengths,
                angles,
                trailing_zeros,
            },
            ..Units::default()
        }
    }

    #[test]
    fn fixed_decimal_places_reach_every_readout_and_hidden_zeros_are_dropped() {
        let one = formatted(Decimals::Fixed(1), Decimals::Fixed(3), TrailingZeros::Shown);

        assert_eq!(one.measured_length(12.3456), "12.3 mm");
        assert_eq!(one.measured_length(-0.01), "0.0 mm");
        assert_eq!(one.measured_area(200.0), "200.0 mm²");
        assert_eq!(one.readout_text(12.345), "12.3 mm");
        assert_eq!(
            one.measured_position([1.0, -2.24, 0.0]),
            "1.0, -2.2, 0.0 mm"
        );
        assert_eq!(one.show(Quantity::length(25.4)), "25.4 mm");
        assert_eq!(one.show(Quantity::length(25.0)), "25.0 mm");
        assert_eq!(one.show(Quantity::plain(2.5)), "2.5");
        assert_eq!(one.angle_text(45.0), "45.000°");
        assert_eq!(one.angle_readout(30.04), "30.040°");
        assert_eq!(one.show(Quantity::angle(90.0)), "90.000°");
        assert_eq!(one.spoken_length(12.0), "12 mm");

        let trimmed = formatted(
            Decimals::Automatic,
            Decimals::Automatic,
            TrailingZeros::Hidden,
        );

        assert_eq!(trimmed.measured_length(12.5), "12.5 mm");
        assert_eq!(trimmed.measured_length(-1e-9), "0 mm");
        assert_eq!(trimmed.readout_text(12.0), "12 mm");
        assert_eq!(trimmed.angle_text(45.0), "45°");
        assert_eq!(trimmed.show(Quantity::length(25.4)), "25.4 mm");

        let whole = formatted(
            Decimals::Fixed(0),
            Decimals::Fixed(0),
            TrailingZeros::Hidden,
        );
        let centimetres = Units {
            length: LengthUnit::Centimetre,
            ..whole
        };

        assert_eq!(whole.measured_length(12.6), "13 mm");
        assert_eq!(centimetres.measured_length(126.0), "13 cm");
        assert_eq!(
            centimetres.show(Quantity::new(260.0, Dimension::AREA)),
            "3 cm²"
        );
    }

    #[test]
    fn the_default_format_is_each_readout_s_own() {
        let default = Units::default();
        let explicit = formatted(
            Decimals::Automatic,
            Decimals::Automatic,
            TrailingZeros::Shown,
        );

        assert_eq!(default, explicit);
        assert_eq!(default.measured_length(12.5), "12.500 mm");
        assert_eq!(default.readout_text(12.5), "12.50 mm");
        assert_eq!(default.angle_readout(30.0), "30.0°");
        assert_eq!(default.show(Quantity::length(100.0 / 3.0)), "33.333333 mm");
        assert_eq!(default.show(Quantity::length(1.5e-5)), "1.5e-5 mm");
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
