use caditor_geometry::Vector2;

use crate::import::svg::{
    syntax::{Length, MILLIMETRES_PER_INCH, PIXELS_PER_INCH, Unit, numbers},
    xml::Node,
};

const MILLIMETRES_PER_PIXEL: f64 = MILLIMETRES_PER_INCH / PIXELS_PER_INCH;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ViewBox {
    pub origin: Vector2,
    pub size: Vector2,
}

impl ViewBox {
    pub fn of(node: Node<'_, '_>) -> Option<Self> {
        let (values, complete) = numbers(node.attribute("viewBox")?);
        match values.as_slice() {
            &[x, y, width, height] if complete && width > 0.0 && height > 0.0 => Some(Self {
                origin: Vector2::new(x, y),
                size: Vector2::new(width, height),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fit {
    Meet,
    Slice,
    Stretch,
}

impl Fit {
    pub fn of(node: Node<'_, '_>) -> Self {
        let words: Vec<&str> = node
            .attribute("preserveAspectRatio")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .collect();
        match words.as_slice() {
            ["none", ..] => Self::Stretch,
            [_, "slice"] => Self::Slice,
            _ => Self::Meet,
        }
    }

    pub fn scale(self, factors: Vector2) -> Vector2 {
        match self {
            Self::Meet => Vector2::splat(factors.min_element()),
            Self::Slice => Vector2::splat(factors.max_element()),
            Self::Stretch => factors,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Sizing {
    pub millimetres: Vector2,
    pub viewport: Vector2,
    pub note: Option<String>,
}

impl Sizing {
    pub fn unit_scale(&self) -> f64 {
        (self.millimetres.x * self.millimetres.y).abs().sqrt()
    }
}

pub(super) fn sizing(root: Node<'_, '_>) -> Sizing {
    let side = |name: &str| {
        root.attribute(name)
            .and_then(Length::parse)
            .filter(|length| length.value > 0.0 && length.unit != Unit::Percent)
    };
    let (width, height) = (side("width"), side("height"));
    let view_box = ViewBox::of(root);
    let pixels = |length: Option<Length>| length.map(|length| length.pixels(0.0));
    let viewport = view_box.map_or_else(
        || {
            Vector2::new(
                pixels(width).unwrap_or(100.0),
                pixels(height).unwrap_or(100.0),
            )
        },
        |view_box| view_box.size,
    );
    let named = width.or(height).map(|length| length.unit);
    let Some(view_box) = view_box.filter(|_| named.is_some()) else {
        let note = if named.is_some_and(|unit| !matches!(unit, Unit::Plain | Unit::Pixels)) {
            pixel_note("Its size is given without a viewBox, so its content is in CSS pixels")
        } else if named.is_some() {
            pixel_note("The drawing is sized in pixels")
        } else {
            pixel_note("The drawing does not name a unit, so its numbers were read as CSS pixels")
        };
        return Sizing {
            millimetres: Vector2::new(MILLIMETRES_PER_PIXEL, -MILLIMETRES_PER_PIXEL),
            viewport,
            note: Some(note),
        };
    };
    let millimetres =
        |length: Option<Length>| length.map(|length| length.pixels(0.0) * MILLIMETRES_PER_PIXEL);
    let factors = match (millimetres(width), millimetres(height)) {
        (Some(width), Some(height)) => Fit::of(root).scale(Vector2::new(
            width / view_box.size.x,
            height / view_box.size.y,
        )),
        (Some(width), None) => Vector2::splat(width / view_box.size.x),
        (None, Some(height)) => Vector2::splat(height / view_box.size.y),
        (None, None) => Vector2::splat(MILLIMETRES_PER_PIXEL),
    };
    if !factors.is_finite() || factors.min_element() <= 0.0 {
        return Sizing {
            millimetres: Vector2::new(MILLIMETRES_PER_PIXEL, -MILLIMETRES_PER_PIXEL),
            viewport,
            note: Some(pixel_note(
                "The drawing's size cannot be used, so its numbers were read as CSS pixels",
            )),
        };
    }
    Sizing {
        millimetres: Vector2::new(factors.x, -factors.y),
        viewport,
        note: named.and_then(unit_note),
    }
}

fn pixel_note(lead: &str) -> String {
    format!(
        "{lead}, 96 to the inch as CSS defines them, so a pixel is {MILLIMETRES_PER_PIXEL:.4} mm."
    )
}

fn unit_note(unit: Unit) -> Option<String> {
    let name = match unit {
        Unit::Millimetres | Unit::Percent => return None,
        Unit::Plain | Unit::Pixels => return Some(pixel_note("The drawing is sized in pixels")),
        Unit::Centimetres => "centimetres",
        Unit::Inches => "inches",
        Unit::Points => "points (72 to the inch)",
        Unit::Picas => "picas (6 to the inch)",
    };
    Some(format!(
        "The drawing is sized in {name}, so it was converted to millimetres."
    ))
}
