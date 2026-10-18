use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity};

use crate::{constraint::MAX_LENGTH, id::EntityId};

pub const MAX_TEXT_CHARS: usize = 200;

const LETTER_CURVES: u64 = 1 << 63;
const TEXT_BITS: u32 = 40;
const LETTER_BITS: u32 = 11;
const SEGMENT_BITS: u32 = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub anchor: EntityId,
    pub content: String,
    pub height: Expression,
    pub angle: Expression,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSize {
    pub height: f64,
    pub angle: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TextError {
    #[error("text needs at least one letter")]
    Blank,
    #[error("text is limited to {MAX_TEXT_CHARS} characters, and this has {count}")]
    TooLong { count: usize },
    #[error("text is one line, without line breaks or other control characters")]
    ControlCharacter,
    #[error("its height cannot be worked out: {0}")]
    Height(EvalError),
    #[error("its angle cannot be worked out: {0}")]
    Angle(EvalError),
    #[error("a text height must be greater than zero")]
    HeightNotPositive,
    #[error("a text height cannot be more than {} m", MAX_LENGTH / 1_000.0)]
    HeightTooLarge,
    #[error("its angle is too large to use")]
    AngleNotFinite,
}

impl Text {
    pub fn heap_size(&self) -> usize {
        self.content.capacity() + self.height.heap_size() + self.angle.heap_size()
    }

    pub fn check_content(content: &str) -> Result<(), TextError> {
        if content.chars().any(char::is_control) {
            return Err(TextError::ControlCharacter);
        }
        if content.trim().is_empty() {
            return Err(TextError::Blank);
        }
        let count = content.chars().count();
        if count > MAX_TEXT_CHARS {
            return Err(TextError::TooLong { count });
        }
        Ok(())
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        let mut used = self.height.parameters();
        used.extend(self.angle.parameters());
        used
    }

    pub fn uses(&self, parameter: ParameterId) -> bool {
        self.height.uses(parameter) || self.angle.uses(parameter)
    }

    pub fn size<F>(&self, value_of: &F) -> Result<TextSize, TextError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let height = self
            .height
            .evaluate_as(Dimension::LENGTH, value_of)
            .map_err(TextError::Height)?;
        let angle = self
            .angle
            .evaluate_as(Dimension::ANGLE, value_of)
            .map_err(TextError::Angle)?;
        TextSize { height, angle }.checked()
    }

    pub fn literal_size(&self) -> Option<TextSize> {
        self.size(&|_| Err(EvalError::ParameterMissing)).ok()
    }
}

impl TextSize {
    pub fn checked(self) -> Result<Self, TextError> {
        if !self.height.is_finite() || self.height > MAX_LENGTH {
            return Err(TextError::HeightTooLarge);
        }
        if self.height <= 0.0 {
            return Err(TextError::HeightNotPositive);
        }
        if !self.angle.is_finite() {
            return Err(TextError::AngleNotFinite);
        }
        Ok(self)
    }
}

pub fn letter_curve(text: EntityId, letter: usize, segment: usize) -> Option<u64> {
    let letter = u64::try_from(letter)
        .ok()
        .filter(|letter| *letter < 1 << LETTER_BITS)?;
    let segment = u64::try_from(segment)
        .ok()
        .filter(|segment| *segment < 1 << SEGMENT_BITS)?;
    let text = text.raw() & ((1 << TEXT_BITS) - 1);
    Some(LETTER_CURVES | text << (LETTER_BITS + SEGMENT_BITS) | letter << SEGMENT_BITS | segment)
}

pub(crate) fn letter_curve_text(raw: u64) -> Option<u64> {
    (raw & LETTER_CURVES != 0 && !EntityId::from_raw(raw).is_reference())
        .then_some((raw & !LETTER_CURVES) >> (LETTER_BITS + SEGMENT_BITS))
}

pub(crate) fn same_text_bits(text: EntityId, bits: u64) -> bool {
    text.raw() & ((1 << TEXT_BITS) - 1) == bits
}

#[cfg(test)]
mod tests;
