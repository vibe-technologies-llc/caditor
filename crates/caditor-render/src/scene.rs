use std::num::NonZeroU32;

use caditor_geometry::{Plane, Point3};
use glam::DVec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PickId(NonZeroU32);

impl PickId {
    pub fn from_index(index: usize) -> Option<Self> {
        let raw = u32::try_from(index).ok()?.checked_add(1)?;
        NonZeroU32::new(raw).map(Self)
    }

    pub fn from_raw(raw: u32) -> Option<Self> {
        NonZeroU32::new(raw).map(Self)
    }

    pub fn index(self) -> usize {
        (self.0.get() - 1) as usize
    }

    pub(crate) fn raw(pick: Option<Self>) -> u32 {
        pick.map_or(0, |id| id.0.get())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Color {
    pub const fn from_rgba8(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red: red as f32 / 255.0,
            green: green as f32 / 255.0,
            blue: blue as f32 / 255.0,
            alpha: alpha as f32 / 255.0,
        }
    }

    pub const fn from_rgb8(red: u8, green: u8, blue: u8) -> Self {
        Self::from_rgba8(red, green, blue, 255)
    }

    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }

    pub(crate) fn to_array(self) -> [f32; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Reference,
    Model,
}

pub(crate) enum Primitive {
    Line,
    Marker,
}

impl Layer {
    pub(crate) fn depth_bias(self, primitive: Primitive) -> f32 {
        match (self, primitive) {
            (Self::Reference, Primitive::Line) => 1.00001,
            (Self::Reference, Primitive::Marker) => 1.00002,
            (Self::Model, Primitive::Line) => 1.00003,
            (Self::Model, Primitive::Marker) => 1.00004,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start: Point3,
    pub end: Point3,
    pub color: Color,
    pub width: f32,
    pub layer: Layer,
    pub pick: Option<PickId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub position: Point3,
    pub color: Color,
    pub diameter: f32,
    pub layer: Layer,
    pub pick: Option<PickId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub convex_outline: Vec<Point3>,
    pub color: Color,
    pub pick: Option<PickId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub plane: Plane,
    pub color: Color,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub lines: Vec<Line>,
    pub markers: Vec<Marker>,
    pub fills: Vec<Fill>,
    pub grid: Option<Grid>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickHit {
    pub id: PickId,
    pub offset_px: f32,
    pub position: Point3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickResult {
    pub cursor: DVec2,
    pub hits: Vec<PickHit>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_ids_round_trip_and_never_encode_zero() {
        let id = PickId::from_index(0).unwrap();
        assert_eq!(PickId::raw(Some(id)), 1);
        assert_eq!(id.index(), 0);
        assert_eq!(PickId::raw(None), 0);
        assert_eq!(PickId::from_raw(0), None);
        assert_eq!(PickId::from_raw(42).map(PickId::index), Some(41));
        assert_eq!(PickId::from_index(u32::MAX as usize), None);
    }
}
