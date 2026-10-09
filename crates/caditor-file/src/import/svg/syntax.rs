use caditor_geometry::{Point2, Vector2};

use crate::import::dxf::geometry::Affine;

pub(super) const PIXELS_PER_INCH: f64 = 96.0;
pub(super) const MILLIMETRES_PER_INCH: f64 = 25.4;

pub(super) struct Scanner<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Scanner<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text: text.as_bytes(),
            at: 0,
        }
    }

    pub fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    pub fn advance(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    pub fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.advance();
        }
    }

    pub fn skip_separator(&mut self) {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.advance();
            self.skip_space();
        }
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_space();
        self.peek().is_none()
    }

    pub fn rest(&self) -> &'a [u8] {
        self.text.get(self.at..).unwrap_or_default()
    }

    pub fn number(&mut self) -> Option<f64> {
        let start = self.at;
        let mut end = start;
        if matches!(self.byte(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let whole = self.digits(end);
        end += whole;
        let mut fraction = 0;
        if self.byte(end) == Some(b'.') {
            fraction = self.digits(end + 1);
            if fraction > 0 || whole > 0 {
                end += 1 + fraction;
            }
        }
        if whole == 0 && fraction == 0 {
            return None;
        }
        if matches!(self.byte(end), Some(b'e' | b'E')) {
            let mut exponent = end + 1;
            if matches!(self.byte(exponent), Some(b'+' | b'-')) {
                exponent += 1;
            }
            let digits = self.digits(exponent);
            if digits > 0 {
                end = exponent + digits;
            }
        }
        let value = std::str::from_utf8(self.text.get(start..end)?)
            .ok()?
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())?;
        self.at = end;
        Some(value)
    }

    pub fn flag(&mut self) -> Option<bool> {
        let flag = match self.peek()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.advance();
        Some(flag)
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.text.get(at).copied()
    }

    fn digits(&self, from: usize) -> usize {
        self.text
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    }
}

pub(super) fn numbers(text: &str) -> (Vec<f64>, bool) {
    let mut scanner = Scanner::new(text);
    let mut values = Vec::new();
    scanner.skip_space();
    while !scanner.at_end() {
        let Some(value) = scanner.number() else {
            return (values, false);
        };
        values.push(value);
        scanner.skip_separator();
    }
    (values, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unit {
    Plain,
    Pixels,
    Millimetres,
    Centimetres,
    Inches,
    Points,
    Picas,
    Percent,
}

impl Unit {
    fn named(name: &[u8]) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_slice() {
            b"" => Self::Plain,
            b"px" => Self::Pixels,
            b"mm" => Self::Millimetres,
            b"cm" => Self::Centimetres,
            b"in" => Self::Inches,
            b"pt" => Self::Points,
            b"pc" => Self::Picas,
            b"%" => Self::Percent,
            _ => return None,
        })
    }

    pub fn pixels(self) -> Option<f64> {
        let inch = PIXELS_PER_INCH;
        match self {
            Self::Plain | Self::Pixels => Some(1.0),
            Self::Millimetres => Some(inch / MILLIMETRES_PER_INCH),
            Self::Centimetres => Some(10.0 * inch / MILLIMETRES_PER_INCH),
            Self::Inches => Some(inch),
            Self::Points => Some(inch / 72.0),
            Self::Picas => Some(inch / 6.0),
            Self::Percent => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Length {
    pub value: f64,
    pub unit: Unit,
}

impl Length {
    pub fn parse(text: &str) -> Option<Self> {
        let mut scanner = Scanner::new(text.trim());
        let value = scanner.number()?;
        let unit = Unit::named(scanner.rest())?;
        Some(Self { value, unit })
    }

    pub fn pixels(self, reference: f64) -> f64 {
        match self.unit.pixels() {
            Some(factor) => self.value * factor,
            None => self.value / 100.0 * reference,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translation(x: f64, y: f64) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    pub fn rotation(degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn then(&self, outer: &Self) -> Self {
        Self {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            e: outer.a * self.e + outer.c * self.f + outer.e,
            f: outer.b * self.e + outer.d * self.f + outer.f,
        }
    }

    pub fn affine(&self) -> Affine {
        Affine::planar(
            Vector2::new(self.a, self.b),
            Vector2::new(self.c, self.d),
            Point2::new(self.e, self.f),
        )
    }
}

pub(super) fn transform_list(text: &str) -> Option<Matrix> {
    let mut scanner = Scanner::new(text);
    let mut matrix = Matrix::IDENTITY;
    loop {
        scanner.skip_separator();
        if scanner.at_end() {
            return Some(matrix);
        }
        let name: Vec<u8> = scanner
            .rest()
            .iter()
            .take_while(|byte| byte.is_ascii_alphabetic())
            .copied()
            .collect();
        for _ in 0..name.len() {
            scanner.advance();
        }
        scanner.skip_space();
        if scanner.peek() != Some(b'(') {
            return None;
        }
        scanner.advance();
        let mut values = Vec::new();
        loop {
            scanner.skip_space();
            if scanner.peek() == Some(b')') {
                scanner.advance();
                break;
            }
            values.push(scanner.number()?);
            scanner.skip_separator();
        }
        matrix = transform(&name, &values)?.then(&matrix);
    }
}

fn transform(name: &[u8], values: &[f64]) -> Option<Matrix> {
    Some(match (name, values) {
        (b"matrix", &[a, b, c, d, e, f]) => Matrix { a, b, c, d, e, f },
        (b"translate", &[x]) => Matrix::translation(x, 0.0),
        (b"translate", &[x, y]) => Matrix::translation(x, y),
        (b"scale", &[factor]) => Matrix::scale(factor, factor),
        (b"scale", &[x, y]) => Matrix::scale(x, y),
        (b"rotate", &[angle]) => Matrix::rotation(angle),
        (b"rotate", &[angle, x, y]) => Matrix::translation(-x, -y)
            .then(&Matrix::rotation(angle))
            .then(&Matrix::translation(x, y)),
        (b"skewX", &[angle]) => Matrix {
            c: angle.to_radians().tan(),
            ..Matrix::IDENTITY
        },
        (b"skewY", &[angle]) => Matrix {
            b: angle.to_radians().tan(),
            ..Matrix::IDENTITY
        },
        _ => return None,
    })
}
