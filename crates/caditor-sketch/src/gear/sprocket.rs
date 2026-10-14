use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Vector2};

use super::{DrawnGear, GearCentre, GearError, GearPiece};
use crate::{
    Sketch,
    constraint::MAX_LENGTH,
    curve::{ArcGeometry, Faceting},
};

pub const MIN_SPROCKET_TEETH: u32 = 9;
pub const MAX_SPROCKET_TEETH: u32 = 150;
const SEAT_RADIUS_RATIO: f64 = 0.505;
const SEAT_ANGLE_DEGREES: f64 = 130.0;
const SEAT_ANGLE_TEETH_DEGREES: f64 = 90.0;
const FLANK_MIN_FACTOR: f64 = 0.12;
const FLANK_MIN_OFFSET: f64 = 2.0;
const FLANK_MAX_FACTOR: f64 = 0.008;
const FLANK_MAX_OFFSET: f64 = 180.0;
const TIP_PITCHES: f64 = 1.125;
const TIP_TEETH_PITCHES: f64 = 0.8;
const MIN_TIP_LAND: f64 = 0.02;
const MIN_RIM_PITCHES: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainSize {
    pub designation: &'static str,
    pub pitch: f64,
    pub roller: f64,
}

impl ChainSize {
    pub const DEFAULT: Self = Self::new("08B", 12.7, 8.51);

    pub const ALL: [Self; 22] = [
        Self::new("05B", 8.0, 5.0),
        Self::new("06B", 9.525, 6.35),
        Self::new("08A", 12.7, 7.95),
        Self::DEFAULT,
        Self::new("10A", 15.875, 10.16),
        Self::new("10B", 15.875, 10.16),
        Self::new("12A", 19.05, 11.91),
        Self::new("12B", 19.05, 12.07),
        Self::new("16A", 25.4, 15.88),
        Self::new("16B", 25.4, 15.88),
        Self::new("20A", 31.75, 19.05),
        Self::new("20B", 31.75, 19.05),
        Self::new("24A", 38.1, 22.23),
        Self::new("24B", 38.1, 25.4),
        Self::new("28A", 44.45, 25.4),
        Self::new("28B", 44.45, 27.94),
        Self::new("32A", 50.8, 28.58),
        Self::new("32B", 50.8, 29.21),
        Self::new("40A", 63.5, 39.68),
        Self::new("40B", 63.5, 39.37),
        Self::new("48A", 76.2, 47.63),
        Self::new("48B", 76.2, 48.26),
    ];

    const fn new(designation: &'static str, pitch: f64, roller: f64) -> Self {
        Self {
            designation,
            pitch,
            roller,
        }
    }

    pub fn find(designation: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|size| size.designation.eq_ignore_ascii_case(designation.trim()))
    }

    pub fn seat_radius(&self) -> f64 {
        SEAT_RADIUS_RATIO * self.roller
    }

    fn valid(&self) -> bool {
        self.pitch.is_finite()
            && self.roller.is_finite()
            && self.roller > 0.0
            && self.pitch > self.roller
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprocket {
    pub chain: ChainSize,
    pub teeth: u32,
    pub bore: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SprocketCircles {
    pub pitch: f64,
    pub tip: f64,
    pub root: f64,
}

impl SprocketCircles {
    pub fn radii(&self) -> [f64; 3] {
        [self.pitch, self.tip, self.root]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SprocketOutline {
    pub centre: Point2,
    pub circles: SprocketCircles,
    pub pieces: Vec<GearPiece>,
    pub bore: Option<f64>,
}

impl SprocketOutline {
    pub fn faceted(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        let bore = self
            .bore
            .map(|radius| ArcGeometry::full_circle(self.centre, radius).faceted(faceting));
        self.pieces
            .iter()
            .map(|piece| piece.faceted(faceting))
            .chain(bore)
            .collect()
    }

    pub fn construction(&self, faceting: Faceting) -> Vec<Vec<Point2>> {
        self.circles
            .radii()
            .into_iter()
            .map(|radius| ArcGeometry::full_circle(self.centre, radius).faceted(faceting))
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct ToothForm {
    seat_centre: Point2,
    seat_end: Point2,
    flank_centre: Point2,
    tip: Point2,
}

impl Sprocket {
    pub fn pitch_radius(&self) -> f64 {
        self.chain.pitch / (2.0 * (PI / f64::from(self.teeth)).sin())
    }

    pub fn tip_radius(&self) -> f64 {
        let teeth = f64::from(self.teeth);
        self.pitch_radius() + self.chain.pitch * (TIP_PITCHES - TIP_TEETH_PITCHES / teeth) / 2.0
            - self.chain.roller / 2.0
    }

    pub fn root_radius(&self) -> f64 {
        self.pitch_radius() - self.chain.seat_radius()
    }

    pub fn seating_angle(&self) -> f64 {
        (SEAT_ANGLE_DEGREES - SEAT_ANGLE_TEETH_DEGREES / f64::from(self.teeth)).to_radians()
    }

    pub fn flank_radius(&self) -> f64 {
        let teeth = f64::from(self.teeth);
        let least = FLANK_MIN_FACTOR * self.chain.roller * (teeth + FLANK_MIN_OFFSET);
        let most = FLANK_MAX_FACTOR * self.chain.roller * (teeth * teeth + FLANK_MAX_OFFSET);
        0.5 * (least + most)
    }

    pub fn circles(&self) -> SprocketCircles {
        SprocketCircles {
            pitch: self.pitch_radius(),
            tip: self.tip_radius(),
            root: self.root_radius(),
        }
    }

    pub fn largest_bore(&self) -> f64 {
        2.0 * (self.root_radius() - MIN_RIM_PITCHES * self.chain.pitch).max(0.0)
    }

    fn tooth_form(&self) -> Result<ToothForm, GearError> {
        let pointed = GearError::SprocketPointed { teeth: self.teeth };
        let seat_radius = self.chain.seat_radius();
        let seat_centre = Point2::new(self.pitch_radius(), 0.0);
        let half = self.seating_angle() / 2.0;
        let outward = Vector2::new(-half.cos(), half.sin());
        let seat_end = seat_centre + outward * seat_radius;
        let flank_radius = self.flank_radius();
        let flank_centre = seat_centre + outward * (seat_radius - flank_radius);
        let tip = first_crossing(flank_centre, flank_radius, seat_end, self.tip_radius())
            .ok_or_else(|| pointed.clone())?;
        let land = (PI / f64::from(self.teeth) - tip.y.atan2(tip.x)) * 2.0 * self.tip_radius();
        if land < MIN_TIP_LAND * self.chain.pitch || tip.y <= seat_end.y {
            return Err(pointed);
        }
        Ok(ToothForm {
            seat_centre,
            seat_end,
            flank_centre,
            tip,
        })
    }

    pub fn outline(&self, centre: Point2) -> Result<SprocketOutline, GearError> {
        if !(MIN_SPROCKET_TEETH..=MAX_SPROCKET_TEETH).contains(&self.teeth) {
            return Err(GearError::SprocketTeethOutOfRange { teeth: self.teeth });
        }
        if !self.chain.valid() {
            return Err(GearError::ChainNotPositive);
        }
        if self.bore.is_nan() || self.bore < 0.0 {
            return Err(GearError::BoreNegative);
        }
        if self.bore > self.largest_bore() {
            return Err(GearError::BoreTooLarge {
                bore: self.bore,
                largest: self.largest_bore(),
            });
        }
        let circles = self.circles();
        if !(centre.is_finite() && centre.length() + circles.tip <= MAX_LENGTH) {
            return Err(GearError::OutOfReach);
        }
        let form = self.tooth_form()?;
        let seat_end = form.seat_end;
        let seat = GearPiece::Arc {
            center: form.seat_centre,
            about_gear: false,
            from: Point2::new(seat_end.x, -seat_end.y),
            to: seat_end,
            counter_clockwise: false,
        };
        let upper = GearPiece::Arc {
            center: form.flank_centre,
            about_gear: false,
            from: seat_end,
            to: form.tip,
            counter_clockwise: false,
        };
        let lower = upper.mirrored_backwards();
        let step = TAU / f64::from(self.teeth);
        let place = |turn: f64| {
            let (sine, cosine) = turn.sin_cos();
            move |point: Point2| {
                Point2::new(
                    centre.x + point.x * cosine - point.y * sine,
                    centre.y + point.x * sine + point.y * cosine,
                )
            }
        };
        let mut pieces = Vec::new();
        for gap in 0..self.teeth {
            let here = place(step * f64::from(gap));
            let next = place(step * f64::from((gap + 1) % self.teeth));
            pieces.push(lower.placed(&here));
            pieces.push(seat.placed(&here));
            pieces.push(upper.placed(&here));
            pieces.push(GearPiece::Arc {
                center: centre,
                about_gear: true,
                from: here(form.tip),
                to: next(lower.from()),
                counter_clockwise: true,
            });
        }
        Ok(SprocketOutline {
            centre,
            circles,
            pieces,
            bore: (self.bore > 0.0).then_some(self.bore / 2.0),
        })
    }
}

fn first_crossing(
    flank_centre: Point2,
    flank_radius: f64,
    from: Point2,
    tip_radius: f64,
) -> Option<Point2> {
    let distance = flank_centre.length();
    if distance <= 0.0 {
        return None;
    }
    let along = (tip_radius * tip_radius - flank_radius * flank_radius + distance * distance)
        / (2.0 * distance);
    let height_squared = tip_radius * tip_radius - along * along;
    if height_squared < 0.0 {
        return None;
    }
    let direction = flank_centre / distance;
    let across = Vector2::new(-direction.y, direction.x) * height_squared.sqrt();
    let base = direction * along;
    let start = (from - flank_centre).to_angle();
    let sweep = |point: Point2| (start - (point - flank_centre).to_angle()).rem_euclid(TAU);
    [base + across, base - across]
        .into_iter()
        .min_by(|a, b| sweep(*a).total_cmp(&sweep(*b)))
}

impl Sketch {
    pub fn add_sprocket(
        &mut self,
        sprocket: &Sprocket,
        centre: GearCentre,
    ) -> Result<DrawnGear, GearError> {
        let at = self.wheel_centre_position(centre)?;
        let outline = sprocket.outline(at)?;
        let centre = self.wheel_centre_point(centre, at)?;
        self.draw_toothed(
            centre,
            &outline.pieces,
            &outline.circles.radii(),
            outline.bore,
        )
    }
}

#[cfg(test)]
mod tests;
