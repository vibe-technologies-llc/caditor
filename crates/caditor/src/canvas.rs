use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Vec2, vec2};

pub const BACKDROP: Color32 = Color32::from_rgba_premultiplied(16, 18, 23, 225);
pub const TEXT: Color32 = Color32::from_rgb(228, 231, 238);
pub const MUTED: Color32 = Color32::from_rgb(170, 176, 188);
pub const DIMENSION: Color32 = Color32::from_rgb(200, 206, 222);
pub const HOVERED: Color32 = Color32::from_rgb(255, 196, 84);
pub const SELECTED: Color32 = Color32::from_rgb(110, 184, 255);
pub const ERROR: Color32 = Color32::from_rgb(255, 128, 118);
pub const WARNING: Color32 = Color32::from_rgb(242, 190, 80);
pub const PROMPT: Color32 = Color32::from_rgb(255, 214, 120);
pub const SNAP: Color32 = Color32::from_rgb(80, 226, 236);
pub const MEASURE: Color32 = Color32::from_rgb(80, 226, 236);

const PADDING: Vec2 = vec2(5.0, 2.0);
const CORNER_RADIUS: f32 = 3.0;

pub fn label(
    painter: &Painter,
    anchor: Pos2,
    align: Align2,
    text: impl ToString,
    font: FontId,
    color: Color32,
) -> Rect {
    wrapped_label(painter, anchor, align, text, font, color, f32::INFINITY)
}

pub fn wrapped_label(
    painter: &Painter,
    anchor: Pos2,
    align: Align2,
    text: impl ToString,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Rect {
    let wrap_width = (max_width - PADDING.x * 2.0).max(0.0);
    let galley = painter.layout(text.to_string(), font, color, wrap_width);
    let rect = align.anchor_size(anchor, galley.size() + PADDING * 2.0);
    painter.rect_filled(rect, CORNER_RADIUS, BACKDROP);
    painter.galley(rect.min + PADDING, galley, color);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::tests::{HIGHLY_READABLE, READABLE, contrast_ratio};

    fn over(background: Color32) -> Color32 {
        let uncovered = 1.0 - f32::from(BACKDROP.a()) / 255.0;
        let channel = |backdrop: u8, below: u8| {
            (f32::from(backdrop) + uncovered * f32::from(below)).round() as u8
        };
        Color32::from_rgb(
            channel(BACKDROP.r(), background.r()),
            channel(BACKDROP.g(), background.g()),
            channel(BACKDROP.b(), background.b()),
        )
    }

    #[test]
    fn canvas_text_is_readable_over_its_backdrop_on_any_background() {
        for below in [
            Color32::BLACK,
            Color32::WHITE,
            Color32::from_rgb(255, 196, 84),
        ] {
            let backdrop = over(below);
            assert!(contrast_ratio(TEXT, backdrop) >= HIGHLY_READABLE);
            for (what, color) in [
                ("muted", MUTED),
                ("dimension", DIMENSION),
                ("hovered", HOVERED),
                ("selected", SELECTED),
                ("error", ERROR),
                ("warning", WARNING),
                ("prompt", PROMPT),
                ("snap", SNAP),
                ("measure", MEASURE),
            ] {
                let ratio = contrast_ratio(color, backdrop);
                assert!(ratio >= READABLE, "{what} over {below:?} is {ratio:.2}:1");
            }
        }
    }
}
