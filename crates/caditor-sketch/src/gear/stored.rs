use std::collections::BTreeSet;

use caditor_expression::{Dimension, EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::Point2;

use super::{
    ChainSize, GearError, GearOutline, GearPair, GearPiece, GearPieceName, MAX_SPROCKET_TEETH,
    MAX_TEETH, MIN_SPROCKET_TEETH, MIN_TEETH, Sprocket, SprocketOutline, SpurGear,
};
use crate::{
    curve::Faceting,
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
    pub meshing: Option<Meshing>,
    pub chain: Option<ChainSize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Meshing {
    pub driver: EntityId,
    pub direction: Expression,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshPlacement {
    pub driver: EntityId,
    pub distance: f64,
    pub direction: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GearShape {
    Spur(SpurGear),
    Sprocket(Sprocket),
}

impl GearShape {
    pub fn teeth(&self) -> u32 {
        match self {
            Self::Spur(spur) => spur.teeth,
            Self::Sprocket(sprocket) => sprocket.teeth,
        }
    }

    pub fn bore(&self) -> f64 {
        match self {
            Self::Spur(spur) => spur.bore,
            Self::Sprocket(sprocket) => sprocket.bore,
        }
    }

    pub fn spur(&self) -> Option<&SpurGear> {
        match self {
            Self::Spur(spur) => Some(spur),
            Self::Sprocket(_) => None,
        }
    }

    pub fn sprocket(&self) -> Option<&Sprocket> {
        match self {
            Self::Sprocket(sprocket) => Some(sprocket),
            Self::Spur(_) => None,
        }
    }

    pub fn outline_turned(&self, centre: Point2, turn: f64) -> Result<ToothedOutline, GearError> {
        match self {
            Self::Spur(spur) => spur.outline_turned(centre, turn).map(ToothedOutline::Gear),
            Self::Sprocket(sprocket) => sprocket
                .outline_turned(centre, turn)
                .map(ToothedOutline::Sprocket),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToothedOutline {
    Gear(GearOutline),
    Sprocket(SprocketOutline),
}

impl ToothedOutline {
    pub fn centre(&self) -> Point2 {
        match self {
            Self::Gear(outline) => outline.centre,
            Self::Sprocket(outline) => outline.centre,
        }
    }

    pub fn tip_radius(&self) -> f64 {
        match self {
            Self::Gear(outline) => outline.circles.tip,
            Self::Sprocket(outline) => outline.circles.tip,
        }
    }

    pub fn bore(&self) -> Option<f64> {
        match self {
            Self::Gear(outline) => outline.bore,
            Self::Sprocket(outline) => outline.bore,
        }
    }

    pub fn pieces(&self) -> &[GearPiece] {
        match self {
            Self::Gear(outline) => &outline.pieces,
            Self::Sprocket(outline) => &outline.pieces,
        }
    }

    pub fn radii(&self) -> Vec<f64> {
        match self {
            Self::Gear(outline) => outline.circles.radii().to_vec(),
            Self::Sprocket(outline) => outline.circles.radii().to_vec(),
        }
    }

    pub fn named_pieces(&self) -> Vec<(GearPieceName, &GearPiece)> {
        match self {
            Self::Gear(outline) => outline.named_pieces(),
            Self::Sprocket(outline) => outline.named_pieces(),
        }
    }

    pub fn faceted(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        match self {
            Self::Gear(outline) => outline.faceted(faceting),
            Self::Sprocket(outline) => outline.faceted(faceting),
        }
    }

    pub fn construction(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        match self {
            Self::Gear(outline) => outline.construction(faceting),
            Self::Sprocket(outline) => outline.construction(faceting),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GearValues {
    pub shape: GearShape,
    pub turn: f64,
    pub placement: Option<MeshPlacement>,
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
    #[error("its direction cannot be worked out: {0}")]
    Direction(EvalError),
    #[error("its direction is too large to use")]
    DirectionNotFinite,
    #[error("the gear it meshes with is not known")]
    DriverUnknown,
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
            | Self::Turn(error)
            | Self::Direction(error) => Some(error),
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
    pub fn meshed(
        centre: EntityId,
        driver: EntityId,
        teeth: Expression,
        bore: Expression,
        direction: Expression,
    ) -> Self {
        Self {
            centre,
            module: Expression::measure(0.0, Unit::Millimetre),
            teeth,
            pressure_angle: Expression::measure(0.0, Unit::Degree),
            profile_shift: Expression::number(0.0),
            root_fillet: Expression::measure(0.0, Unit::Millimetre),
            bore,
            turn: Expression::measure(0.0, Unit::Degree),
            meshing: Some(Meshing { driver, direction }),
            chain: None,
        }
    }

    pub fn sprocket(
        centre: EntityId,
        chain: ChainSize,
        teeth: Expression,
        bore: Expression,
        turn: Expression,
    ) -> Self {
        Self {
            centre,
            module: Expression::measure(0.0, Unit::Millimetre),
            teeth,
            pressure_angle: Expression::measure(0.0, Unit::Degree),
            profile_shift: Expression::number(0.0),
            root_fillet: Expression::measure(0.0, Unit::Millimetre),
            bore,
            turn,
            meshing: None,
            chain: Some(chain),
        }
    }

    pub fn is_sprocket(&self) -> bool {
        self.chain.is_some()
    }

    pub fn driver(&self) -> Option<EntityId> {
        self.meshing.as_ref().map(|meshing| meshing.driver)
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        match &self.meshing {
            None if self.chain.is_some() => vec![&self.teeth, &self.bore, &self.turn],
            None => vec![
                &self.module,
                &self.teeth,
                &self.pressure_angle,
                &self.profile_shift,
                &self.root_fillet,
                &self.bore,
                &self.turn,
            ],
            Some(meshing) => vec![&self.teeth, &self.bore, &meshing.direction],
        }
    }

    pub fn expressions_mut(&mut self) -> Vec<&mut Expression> {
        match &mut self.meshing {
            None if self.chain.is_some() => vec![&mut self.teeth, &mut self.bore, &mut self.turn],
            None => vec![
                &mut self.module,
                &mut self.teeth,
                &mut self.pressure_angle,
                &mut self.profile_shift,
                &mut self.root_fillet,
                &mut self.bore,
                &mut self.turn,
            ],
            Some(meshing) => vec![&mut self.teeth, &mut self.bore, &mut meshing.direction],
        }
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
        if self.meshing.is_some() {
            return Err(GearValueError::DriverUnknown);
        }
        if let Some(chain) = self.chain {
            return self.sprocket_values(chain, value_of);
        }
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
        Ok(GearValues {
            shape: GearShape::Spur(spur),
            turn,
            placement: None,
        })
    }

    fn sprocket_values<F>(
        &self,
        chain: ChainSize,
        value_of: &F,
    ) -> Result<GearValues, GearValueError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let teeth = self
            .teeth
            .evaluate_as(Dimension::NONE, value_of)
            .map_err(GearValueError::Teeth)?;
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
        let sprocket = Sprocket {
            chain,
            teeth: whole_sprocket_teeth(teeth)?,
            bore,
        };
        sprocket
            .check()
            .map_err(|error| GearValueError::Build(Box::new(error)))?;
        Ok(GearValues {
            shape: GearShape::Sprocket(sprocket),
            turn,
            placement: None,
        })
    }

    pub fn meshed_values<F>(
        &self,
        driver: &GearValues,
        value_of: &F,
    ) -> Result<GearValues, GearValueError>
    where
        F: Fn(ParameterId) -> Result<Quantity, EvalError>,
    {
        let meshing = self.meshing.as_ref().ok_or(GearValueError::DriverUnknown)?;
        let teeth = self
            .teeth
            .evaluate_as(Dimension::NONE, value_of)
            .map_err(GearValueError::Teeth)?;
        let bore = self
            .bore
            .evaluate_as(Dimension::LENGTH, value_of)
            .map_err(GearValueError::Bore)?;
        let direction = meshing
            .direction
            .evaluate_as(Dimension::ANGLE, value_of)
            .map_err(GearValueError::Direction)?;
        if !direction.is_finite() {
            return Err(GearValueError::DirectionNotFinite);
        }
        let driver_shape = driver.shape.spur().ok_or(GearValueError::DriverUnknown)?;
        let pair = GearPair {
            driver: *driver_shape,
            partner_teeth: whole_teeth(teeth)?,
            partner_bore: bore,
            direction: direction.to_radians(),
        };
        let spur = pair.partner();
        spur.check().map_err(|error| {
            GearValueError::Build(Box::new(GearError::Partner(Box::new(error))))
        })?;
        Ok(GearValues {
            shape: GearShape::Spur(spur),
            turn: pair.partner_turn(driver.turn.to_radians()).to_degrees(),
            placement: Some(MeshPlacement {
                driver: meshing.driver,
                distance: pair.centre_distance(),
                direction,
            }),
        })
    }

    pub fn literal_values(&self) -> Option<GearValues> {
        self.values(&|_| Err(EvalError::ParameterMissing)).ok()
    }

    pub fn literal_meshed_values(&self, driver: &GearValues) -> Option<GearValues> {
        self.meshed_values(driver, &|_| Err(EvalError::ParameterMissing))
            .ok()
    }
}

fn whole_count(value: f64) -> Result<u32, GearValueError> {
    let rounded = value.round();
    if !value.is_finite() || (value - rounded).abs() > WHOLE_TOLERANCE {
        return Err(GearValueError::TeethNotWhole { value });
    }
    Ok(rounded.clamp(0.0, f64::from(u32::MAX)) as u32)
}

fn whole_teeth(value: f64) -> Result<u32, GearValueError> {
    let teeth = whole_count(value)?;
    if !(MIN_TEETH..=MAX_TEETH).contains(&teeth) {
        return Err(GearValueError::Build(Box::new(
            GearError::TeethOutOfRange { teeth },
        )));
    }
    Ok(teeth)
}

fn whole_sprocket_teeth(value: f64) -> Result<u32, GearValueError> {
    let teeth = whole_count(value)?;
    if !(MIN_SPROCKET_TEETH..=MAX_SPROCKET_TEETH).contains(&teeth) {
        return Err(GearValueError::Build(Box::new(
            GearError::SprocketTeethOutOfRange { teeth },
        )));
    }
    Ok(teeth)
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
