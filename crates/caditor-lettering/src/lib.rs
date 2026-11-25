mod font;
mod layout;
mod overlap;
mod signs;

#[cfg(test)]
mod tests;

pub use skrifa::GlyphId;

pub use crate::{
    font::{DrawnSign, Instance, Typeface},
    layout::{
        Alignment, BOLD_WEIGHT, Letter, Lettered, LetteringError, REGULAR_WEIGHT, Style, lettered,
    },
    overlap::{Segment, merged},
};

pub const INTER: &[u8] = include_bytes!("../../caditor/assets/fonts/InterVariable.ttf");
pub const INTER_ITALIC: &[u8] =
    include_bytes!("../../caditor/assets/fonts/InterVariable-Italic.ttf");
