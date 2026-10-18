use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity};

use super::{GearError, MAX_TEETH, MIN_TEETH, SpurGear};
use crate::{
    id::EntityId,
    text::{GEAR_CURVES, LETTER_CURVES, TEXT_BITS},
};

const GEAR_BITS: u32 = TEXT_BITS;
const TOOTH_BITS: u32 = 11;
const SEGMENT_BITS: u32 = 12;
const WHOLE_TOLERANCE: f64 = 1e-9;
pub const BORE_TOOTH: usize = (1 << TOOTH_BITS) - 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Gear {
    pub centre: EntityId,
    pub module: Expression,
    pub teeth: Expression,
    pub pressure_angle: Expression,
    pub profile_shift: Expression,
    pub root_fillet: Expression,
    pub bore: Expression,
    pub turn: Expression,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GearValues {
    pub spur: SpurGear,
    pub turn: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GearValueError {
    #[error("its module cannot be worked out: {0}")]
    Module(EvalError),
    #[error("its number of teeth cannot be worked out: {0}")]
    Teeth(EvalError),
    #[error("its number of teeth must be a whole number, not {value}")]
    TeethNotWhole { value: f64 },
    #[error("its pressure angle cannot be worked out: {0}")]
    PressureAngle(EvalError),
    #[error("its profile shift cannot be worked out: {0}")]
    ProfileShift(EvalError),
    #[error("its root fillet cannot be worked out: {0}")]
    RootFillet(EvalError),
    #[error("its bore cannot be worked out: {0}")]
    Bore(EvalError),
    #[error("its turn cannot be worked out: {0}")]
    Turn(EvalError),
    #[error("its turn is too large to use")]
    TurnNotFinite,
    #[error("{0}")]
    Build(Box<GearError>),
}

impl GearValueError {
    fn failure(&self) -> Option<&EvalError> {
        match self {
            Self::Module(error)
            | Self::Teeth(error)
            | Self::PressureAngle(error)
            | Self::ProfileShift(error)
            | Self::RootFillet(error)
            | Self::Bore(error)
            | Self::Turn(error) => Some(error),
            _ => None,
        }
    }

    pub fn failed_parameter(&self) -> Option<ParameterId> {
        match self.failure()? {
            EvalError::ParameterFailed { id, .. } => Some(*id),
            _ => None,
        }
    }

    pub fn failed_name(&self) -> Option<&str> {
        match self.failure()? {
            EvalError::ParameterFailed { name, .. } => Some(name),
            _ => None,
        }
    }
}

impl Gear {
    pub fn expressions(&self) -> [&Expression; 7] {
        [
            &self.module,
            &self.teeth,
            &self.pressure_angle,
            &self.profile_shift,
            &self.root_fillet,
            &self.bore,
            &self.turn,
        ]
    }

    pub fn expressions_mut(&mut self) -> [&mut Expression; 7] {
        [
            &mut self.module,
            &mut self.teeth,
            &mut self.pressure_angle,
            &mut self.profile_shift,
            &mut self.root_fillet,
            &mut self.bore,
            &mut self.turn,
        ]
    }

    pub fn heap_size(&self) -> usize {
        self.expressions()
            .into_iter()
            .map(Expression::heap_size)
            .sum()
    }

    pub fn parameters(&self) -> BTreeSet<ParameterId> {
        self.expressions()
            .into_iter()
            .flat_map(Expression::parameters)
            .collect()
    }

    pub fn uses(&self, parameter: ParameterId) -> bool {
        self.expressions()
            .into_iter()
            .any(|expression| expression.uses(parameter))
    }

    pub fn values<F>(&self, value_of: &F) -> Result<GearValues, GearValueError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let module = self
            .module
            .evaluate_as(Dimension::LENGTH, value_of)
            .map_err(GearValueError::Module)?;
        let teeth = self
            .teeth
            .evaluate_as(Dimension::NONE, value_of)
            .map_err(GearValueError::Teeth)?;
        let pressure_angle = self
            .pressure_angle
            .evaluate_as(Dimension::ANGLE, value_of)
            .map_err(GearValueError::PressureAngle)?;
        let profile_shift = self
            .profile_shift
            .evaluate_as(Dimension::NONE, value_of)
            .map_err(GearValueError::ProfileShift)?;
        let root_fillet = self
            .root_fillet
            .evaluate_as(Dimension::LENGTH, value_of)
            .map_err(GearValueError::RootFillet)?;
        let bore = self
            .bore
            .evaluate_as(Dimension::LENGTH, value_of)
            .map_err(GearValueError::Bore)?;
        let turn = self
            .turn
            .evaluate_as(Dimension::ANGLE, value_of)
            .map_err(GearValueError::Turn)?;
        if !turn.is_finite() {
            return Err(GearValueError::TurnNotFinite);
        }
        let spur = SpurGear {
            module,
            teeth: whole_teeth(teeth)?,
            pressure_angle,
            profile_shift,
            root_fillet,
            bore,
        };
        spur.check()
            .map_err(|error| GearValueError::Build(Box::new(error)))?;
        Ok(GearValues { spur, turn })
    }

    pub fn literal_values(&self) -> Option<GearValues> {
        self.values(&|_| Err(EvalError::ParameterMissing)).ok()
    }
}

fn whole_teeth(value: f64) -> Result<u32, GearValueError> {
    let rounded = value.round();
    if !value.is_finite() || (value - rounded).abs() > WHOLE_TOLERANCE {
        return Err(GearValueError::TeethNotWhole { value });
    }
    if !(f64::from(MIN_TEETH)..=f64::from(MAX_TEETH)).contains(&rounded) {
        let teeth = rounded.clamp(0.0, f64::from(u32::MAX)) as u32;
        return Err(GearValueError::Build(Box::new(
            GearError::TeethOutOfRange { teeth },
        )));
    }
    Ok(rounded as u32)
}

pub fn gear_curve(gear: EntityId, tooth: usize, segment: usize) -> Option<u64> {
    let tooth = u64::try_from(tooth)
        .ok()
        .filter(|tooth| *tooth < 1 << TOOTH_BITS)?;
    let segment = u64::try_from(segment)
        .ok()
        .filter(|segment| *segment < 1 << SEGMENT_BITS)?;
    let gear = gear.raw() & ((1 << GEAR_BITS) - 1);
    Some(
        LETTER_CURVES
            | GEAR_CURVES
            | gear << (TOOTH_BITS + SEGMENT_BITS)
            | tooth << SEGMENT_BITS
            | segment,
    )
}

pub(crate) fn gear_curve_gear(raw: u64) -> Option<u64> {
    let tagged = raw & (LETTER_CURVES | GEAR_CURVES) == LETTER_CURVES | GEAR_CURVES;
    (tagged && !EntityId::from_raw(raw).is_reference())
        .then_some((raw & !(LETTER_CURVES | GEAR_CURVES)) >> (TOOTH_BITS + SEGMENT_BITS))
}

pub(crate) fn same_gear_bits(gear: EntityId, bits: u64) -> bool {
    gear.raw() & ((1 << GEAR_BITS) - 1) == bits
}

#[cfg(test)]
mod tests;
