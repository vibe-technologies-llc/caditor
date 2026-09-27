use std::fmt;

const DISPLAY_DECIMALS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dimension {
    length: i8,
    angle: i8,
}

impl Dimension {
    pub const NONE: Self = Self::new(0, 0);
    pub const LENGTH: Self = Self::new(1, 0);
    pub const AREA: Self = Self::new(2, 0);
    pub const VOLUME: Self = Self::new(3, 0);
    pub const ANGLE: Self = Self::new(0, 1);

    pub const fn new(length: i8, angle: i8) -> Self {
        Self { length, angle }
    }

    pub fn length_power(self) -> i8 {
        self.length
    }

    pub fn angle_power(self) -> i8 {
        self.angle
    }

    pub fn is_plain(self) -> bool {
        self == Self::NONE
    }

    pub fn times(self, other: Self) -> Option<Self> {
        Some(Self::new(
            self.length.checked_add(other.length)?,
            self.angle.checked_add(other.angle)?,
        ))
    }

    pub fn over(self, other: Self) -> Option<Self> {
        Some(Self::new(
            self.length.checked_sub(other.length)?,
            self.angle.checked_sub(other.angle)?,
        ))
    }

    pub fn power(self, exponent: i8) -> Option<Self> {
        Some(Self::new(
            self.length.checked_mul(exponent)?,
            self.angle.checked_mul(exponent)?,
        ))
    }

    pub fn square_root(self) -> Option<Self> {
        let even = self.length % 2 == 0 && self.angle % 2 == 0;
        even.then(|| Self::new(self.length / 2, self.angle / 2))
    }

    pub fn unit_symbol(self) -> String {
        let length = match self.length {
            0 => None,
            1 => Some("mm".to_owned()),
            2 => Some("mm²".to_owned()),
            3 => Some("mm³".to_owned()),
            power => Some(format!("mm^{power}")),
        };
        let angle = match self.angle {
            0 => None,
            1 => Some("°".to_owned()),
            power => Some(format!("deg^{power}")),
        };
        match (length, angle) {
            (Some(length), Some(angle)) => format!("{length}·{angle}"),
            (Some(symbol), None) | (None, Some(symbol)) => symbol,
            (None, None) => String::new(),
        }
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NONE => formatter.write_str("a plain number"),
            Self::LENGTH => formatter.write_str("a length"),
            Self::AREA => formatter.write_str("an area"),
            Self::VOLUME => formatter.write_str("a volume"),
            Self::ANGLE => formatter.write_str("an angle"),
            other => write!(formatter, "a quantity in {}", other.unit_symbol()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dimension: Dimension,
}

impl Quantity {
    pub const fn new(value: f64, dimension: Dimension) -> Self {
        Self { value, dimension }
    }

    pub const fn plain(value: f64) -> Self {
        Self::new(value, Dimension::NONE)
    }

    pub const fn length(millimetres: f64) -> Self {
        Self::new(millimetres, Dimension::LENGTH)
    }

    pub const fn angle(degrees: f64) -> Self {
        Self::new(degrees, Dimension::ANGLE)
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let number = format_number(self.value);
        match self.dimension {
            Dimension::NONE => formatter.write_str(&number),
            Dimension::ANGLE => write!(formatter, "{number}°"),
            dimension => write!(formatter, "{number} {}", dimension.unit_symbol()),
        }
    }
}

pub fn format_number(value: f64) -> String {
    let rounded = format!("{value:.DISPLAY_DECIMALS$}");
    let trimmed = if rounded.contains('.') {
        rounded.trim_end_matches('0').trim_end_matches('.')
    } else {
        rounded.as_str()
    };
    match trimmed {
        "-0" => "0".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Micrometre,
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
    Degree,
    Radian,
}

impl Unit {
    pub const ALL: [Self; 8] = [
        Self::Micrometre,
        Self::Millimetre,
        Self::Centimetre,
        Self::Metre,
        Self::Inch,
        Self::Foot,
        Self::Degree,
        Self::Radian,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Micrometre => "um",
            Self::Millimetre => "mm",
            Self::Centimetre => "cm",
            Self::Metre => "m",
            Self::Inch => "in",
            Self::Foot => "ft",
            Self::Degree => "deg",
            Self::Radian => "rad",
        }
    }

    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Micrometre => &["µm", "μm"],
            Self::Degree => &["°"],
            Self::Millimetre
            | Self::Centimetre
            | Self::Metre
            | Self::Inch
            | Self::Foot
            | Self::Radian => &[],
        }
    }

    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|unit| unit.symbol() == symbol || unit.aliases().contains(&symbol))
    }

    pub fn dimension(self) -> Dimension {
        match self {
            Self::Degree | Self::Radian => Dimension::ANGLE,
            Self::Micrometre
            | Self::Millimetre
            | Self::Centimetre
            | Self::Metre
            | Self::Inch
            | Self::Foot => Dimension::LENGTH,
        }
    }

    pub fn in_base_units(self) -> f64 {
        match self {
            Self::Micrometre => 0.001,
            Self::Millimetre | Self::Degree => 1.0,
            Self::Centimetre => 10.0,
            Self::Metre => 1000.0,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
            Self::Radian => 180.0 / std::f64::consts::PI,
        }
    }

    pub fn quantity(self, amount: f64) -> Quantity {
        Quantity::new(amount * self.in_base_units(), self.dimension())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_combine_and_describe_themselves() {
        assert_eq!(
            Dimension::LENGTH.times(Dimension::LENGTH),
            Some(Dimension::AREA)
        );
        assert_eq!(
            Dimension::VOLUME.over(Dimension::AREA),
            Some(Dimension::LENGTH)
        );
        assert_eq!(Dimension::AREA.square_root(), Some(Dimension::LENGTH));
        assert_eq!(Dimension::LENGTH.square_root(), None);
        assert_eq!(Dimension::new(i8::MAX, 0).times(Dimension::LENGTH), None);
        assert_eq!(Dimension::AREA.to_string(), "an area");
        assert_eq!(Dimension::new(-1, 1).to_string(), "a quantity in mm^-1·°");
    }

    #[test]
    fn quantities_display_with_trimmed_decimals() {
        assert_eq!(Quantity::length(40.0).to_string(), "40 mm");
        assert_eq!(Quantity::length(100.0 / 3.0).to_string(), "33.333333 mm");
        assert_eq!(Quantity::angle(-0.0000001).to_string(), "0°");
        assert_eq!(Quantity::new(12.5, Dimension::AREA).to_string(), "12.5 mm²");
        assert_eq!(Quantity::plain(3.0).to_string(), "3");
    }

    #[test]
    fn units_convert_to_millimetres_and_degrees() {
        assert_eq!(Unit::from_symbol("in"), Some(Unit::Inch));
        assert_eq!(Unit::from_symbol("µm"), Some(Unit::Micrometre));
        assert_eq!(Unit::from_symbol("°"), Some(Unit::Degree));
        assert_eq!(Unit::from_symbol("width"), None);
        assert_eq!(Unit::Inch.quantity(2.0), Quantity::length(50.8));
        assert!((Unit::Radian.quantity(std::f64::consts::PI).value - 180.0).abs() < 1e-12);
    }
}
