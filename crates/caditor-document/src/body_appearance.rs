use std::collections::{BTreeMap, BTreeSet};

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, format_number};
use caditor_kernel::{FaceId, FaceName, FaceReference, Solid};

use crate::values::ParameterValues;

pub const MAX_MATERIAL_NAME_CHARS: usize = 80;
pub const MAX_BODY_NAME_CHARS: usize = 120;
pub const MIN_OPACITY_PERCENT: u8 = 10;
pub const OPAQUE_PERCENT: u8 = 100;
pub const MAX_DENSITY: f64 = 100.0;
const GRAMS_PER_CUBIC_MILLIMETRE_AT_UNIT_DENSITY: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }

    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |range: std::ops::Range<usize>| {
            digits
                .get(range)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        let doubled = |index: usize| channel(index..index + 1).map(|nibble| nibble * 17);
        match digits.len() {
            6 => Some(Self::new(channel(0..2)?, channel(2..4)?, channel(4..6)?)),
            3 => Some(Self::new(doubled(0)?, doubled(1)?, doubled(2)?)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyAppearance {
    pub colour: Option<Rgb>,
    pub material: Option<String>,
    pub density: Option<Expression>,
    pub name: Option<String>,
    pub opacity: Option<u8>,
    pub faces: Vec<FaceColour>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceColour {
    pub face: FaceReference,
    pub colour: Rgb,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DensityError {
    #[error("the density cannot be worked out, since {0}")]
    Evaluation(EvalError),
    #[error("the density must be above zero, and {} is not", format_number(*.0))]
    NotAboveZero(f64),
    #[error("the density may be at most {} g/cm³, and {} is more", format_number(MAX_DENSITY), format_number(*.0))]
    TooHigh(f64),
}

impl BodyAppearance {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.density
            .iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses_parameter(&self, parameter: ParameterId) -> bool {
        self.density
            .as_ref()
            .is_some_and(|density| density.uses(parameter))
    }

    pub fn heap_size(&self) -> usize {
        self.material.as_ref().map_or(0, String::len)
            + self.density.as_ref().map_or(0, Expression::heap_size)
            + self.name.as_ref().map_or(0, String::len)
            + size_of_val(self.faces.as_slice())
            + self
                .faces
                .iter()
                .map(|face| face.face.heap_size())
                .sum::<usize>()
    }

    pub fn face_colours(&self, solid: &Solid) -> BTreeMap<FaceId, Rgb> {
        let mut named: BTreeMap<FaceName, Vec<FaceId>> = BTreeMap::new();
        if !self.faces.is_empty() {
            for (id, face) in solid.faces() {
                named.entry(face.name()).or_default().push(id);
            }
        }
        let mut colours = BTreeMap::new();
        for coloured in &self.faces {
            match named.get(&coloured.face.name()) {
                Some(faces) => {
                    for face in faces {
                        colours.insert(*face, coloured.colour);
                    }
                }
                None => {
                    if let Ok(face) = coloured.face.resolve(solid) {
                        colours.insert(face, coloured.colour);
                    }
                }
            }
        }
        colours
    }

    pub fn density_value(&self, values: &ParameterValues) -> Option<Result<f64, DensityError>> {
        self.density
            .as_ref()
            .map(|density| density_of(density, values))
    }

    pub fn mass_grams(
        &self,
        volume: f64,
        values: &ParameterValues,
    ) -> Option<Result<f64, DensityError>> {
        self.density_value(values).map(|density| {
            density.map(|density| volume * density * GRAMS_PER_CUBIC_MILLIMETRE_AT_UNIT_DENSITY)
        })
    }
}

pub fn density_of(density: &Expression, values: &ParameterValues) -> Result<f64, DensityError> {
    let value = density
        .evaluate_as(Dimension::NONE, &|id| values.value(id))
        .map_err(DensityError::Evaluation)?;
    if value <= 0.0 {
        return Err(DensityError::NotAboveZero(value));
    }
    if value > MAX_DENSITY {
        return Err(DensityError::TooHigh(value));
    }
    Ok(value)
}

pub fn material_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
