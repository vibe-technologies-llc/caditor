use std::sync::LazyLock;

use caditor_geometry::{Point2, Vector2, larger};
use caditor_kernel::Interrupted;
use parking_lot::Mutex;

use crate::{INTER, INTER_ITALIC, Instance, font::Typeface, overlap::Segment};

pub const REGULAR_WEIGHT: f64 = 400.0;
pub const BOLD_WEIGHT: f64 = 700.0;
const FALLBACK_LINE_PITCH: f64 = 1.6;

static INTER_FACE: LazyLock<Option<Mutex<Typeface<'static>>>> =
    LazyLock::new(|| Typeface::parse(INTER, Some(INTER_ITALIC)).map(Mutex::new));

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

impl Style {
    fn weight(self) -> f64 {
        if self.bold {
            BOLD_WEIGHT
        } else {
            REGULAR_WEIGHT
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alignment {
    #[default]
    Left,
    Centre,
    Right,
}

impl Alignment {
    fn share(self) -> f64 {
        match self {
            Self::Left => 0.0,
            Self::Centre => 0.5,
            Self::Right => 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Letter {
    pub character: char,
    pub position: usize,
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lettered {
    pub letters: Vec<Letter>,
    pub width: f64,
    pub line_widths: Vec<f64>,
    pub line_pitch: f64,
    pub missing: Vec<char>,
}

impl Lettered {
    pub fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.letters.iter().flat_map(|letter| &letter.segments)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LetteringError {
    #[error("the bundled Inter font could not be read")]
    FontUnreadable,
    #[error("the lettering was cancelled")]
    Cancelled(#[from] Interrupted),
}

pub fn lettered(
    content: &str,
    style: Style,
    alignment: Alignment,
) -> Result<Lettered, LetteringError> {
    let face = INTER_FACE.as_ref().ok_or(LetteringError::FontUnreadable)?;
    let mut typeface = face.lock();
    Ok(lay_out(&mut typeface, content, style, alignment)?)
}

fn lay_out(
    typeface: &mut Typeface<'_>,
    content: &str,
    style: Style,
    alignment: Alignment,
) -> Result<Lettered, Interrupted> {
    let instance = typeface.instance(style.weight(), style.italic);
    let cap_height = typeface
        .cap_height(instance)
        .unwrap_or_else(|| typeface.units_per_em())
        .max(f64::MIN_POSITIVE);
    let scale = 1.0 / cap_height;

    let pitch = typeface
        .line_height(instance)
        .map_or(FALLBACK_LINE_PITCH, |height| height * scale);
    let mut lettered = Lettered::default();
    let mut position = 0;

    for (index, line) in content.split('\n').enumerate() {
        let mut laid = lay_line(typeface, instance, scale, line, position)?;

        let shift = Vector2::new(-alignment.share() * laid.width, -pitch * index as f64);

        for mut letter in laid.letters {
            letter.segments = letter
                .segments
                .iter()
                .map(|segment| segment.mapped(|point| point + shift))
                .collect();
            lettered.letters.push(letter);
        }
        lettered.missing.append(&mut laid.missing);
        lettered.line_widths.push(laid.width);
        position += line.chars().count() + 1;
    }
    lettered.width = lettered.line_widths.iter().copied().fold(0.0, larger);
    lettered.line_pitch = pitch;
    Ok(lettered)
}

fn lay_line(
    typeface: &mut Typeface<'_>,
    instance: Instance,
    scale: f64,
    line: &str,
    first_position: usize,
) -> Result<Lettered, Interrupted> {
    let mut lettered = Lettered::default();
    let mut pen = 0.0;
    let mut previous = None;

    for (offset, character) in line.chars().enumerate() {
        if character.is_control() {
            continue;
        }

        let position = first_position + offset;
        let glyph = typeface.glyph(character, instance);

        if let (Some(before), Some(glyph)) = (previous, glyph) {
            pen += typeface.kerning(instance, before, glyph) * scale;
        }

        let advance = typeface.advance(instance, glyph) * scale;

        match glyph {
            Some(glyph) if !character.is_whitespace() => {
                let outline = typeface.outline(instance, glyph)?;
                let offset = Vector2::new(pen, 0.0);

                let segments = outline
                    .iter()
                    .map(|segment| segment.mapped(|point| scaled(point, scale) + offset))
                    .collect();

                lettered.letters.push(Letter {
                    character,
                    position,
                    segments,
                });
            }
            None if !character.is_whitespace() => lettered.missing.push(character),
            Some(_) | None => {}
        }
        pen += advance;
        previous = glyph;
    }
    lettered.width = pen;
    Ok(lettered)
}

fn scaled(point: Point2, scale: f64) -> Point2 {
    Point2::new(point.x * scale, point.y * scale)
}
