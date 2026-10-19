use std::f64::consts::PI;

use caditor_geometry::Point2;

use super::{DrawnGear, Gear, GearCentre, GearError, GearOutline, Meshing, SpurGear};
use crate::{Sketch, id::EntityId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GearPair {
    pub driver: SpurGear,
    pub partner_teeth: u32,
    pub partner_bore: f64,
    pub direction: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PairOutline {
    pub driver: GearOutline,
    pub partner: GearOutline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawnPair {
    pub driver: DrawnGear,
    pub partner: DrawnGear,
}

impl GearPair {
    pub fn partner(&self) -> SpurGear {
        SpurGear {
            teeth: self.partner_teeth,
            profile_shift: -self.driver.profile_shift,
            bore: self.partner_bore,
            ..self.driver
        }
    }

    pub fn centre_distance(&self) -> f64 {
        self.driver.module * f64::from(self.driver.teeth + self.partner_teeth) / 2.0
    }

    pub fn partner_centre(&self, driver_centre: Point2) -> Point2 {
        let (sine, cosine) = self.direction.sin_cos();
        let distance = self.centre_distance();
        Point2::new(
            driver_centre.x + distance * cosine,
            driver_centre.y + distance * sine,
        )
    }

    pub fn partner_turn(&self, driver_turn: f64) -> f64 {
        let driver = f64::from(self.driver.teeth);
        let partner = f64::from(self.partner_teeth);
        PI + PI / partner - driver_turn * driver / partner
            + self.direction * (1.0 + driver / partner)
    }

    pub fn outlines(&self, centre: Point2) -> Result<PairOutline, GearError> {
        self.outlines_turned(centre, 0.0)
    }

    pub fn outlines_turned(
        &self,
        centre: Point2,
        driver_turn: f64,
    ) -> Result<PairOutline, GearError> {
        let driver = self.driver.outline_turned(centre, driver_turn)?;
        let partner = self
            .partner()
            .outline_turned(self.partner_centre(centre), self.partner_turn(driver_turn))
            .map_err(|error| GearError::Partner(Box::new(error)))?;
        Ok(PairOutline { driver, partner })
    }
}

impl Sketch {
    pub fn add_stored_pair(
        &mut self,
        centre: GearCentre,
        driver: Gear,
        partner: Gear,
        placing: &GearPair,
    ) -> Result<(EntityId, EntityId), GearError> {
        let at = self.wheel_centre_position(centre)?;
        let outlines = placing.outlines(at)?;
        let driver_centre = self.wheel_centre_point(centre, at)?;
        let partner_centre = self.add_point(outlines.partner.centre);
        let driver = self
            .add_parametric_gear(Gear {
                centre: driver_centre,
                ..driver
            })
            .map_err(GearError::Edit)?;
        let partner = self
            .add_parametric_gear(Gear {
                centre: partner_centre,
                meshing: partner.meshing.map(|meshing| Meshing { driver, ..meshing }),
                ..partner
            })
            .map_err(GearError::Edit)?;
        Ok((driver, partner))
    }

    pub fn add_gear_pair(
        &mut self,
        pair: &GearPair,
        centre: GearCentre,
    ) -> Result<DrawnPair, GearError> {
        let at = self.wheel_centre_position(centre)?;
        let outlines = pair.outlines(at)?;
        let driver_centre = self.wheel_centre_point(centre, at)?;
        let driver = self.draw_toothed(
            driver_centre,
            &outlines.driver.pieces,
            &outlines.driver.circles.radii(),
            outlines.driver.bore,
        )?;
        let partner_centre = self.add_point(outlines.partner.centre);
        let partner = self.draw_toothed(
            partner_centre,
            &outlines.partner.pieces,
            &outlines.partner.circles.radii(),
            outlines.partner.bore,
        )?;
        Ok(DrawnPair { driver, partner })
    }
}

#[cfg(test)]
mod stored_tests;
#[cfg(test)]
mod tests;
