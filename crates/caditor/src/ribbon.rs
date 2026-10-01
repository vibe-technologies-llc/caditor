use std::{fmt::Debug, hash::Hash, sync::Arc};

use egui::{
    Galley, Id, Label, Margin, Rect, Response, RichText, Stroke, TextStyle, TextWrapMode, Ui,
    WidgetText, pos2, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, SPACE_L, SPACE_M, SPACE_S, SPACE_XS},
    widgets,
};

pub const TOOL_GAP: f32 = widgets::COMPACT_TOOL_GAP;
pub const DIVIDER_SPACE: f32 = SPACE_L;
pub const GROUP_SPACE: f32 = DIVIDER_SPACE + TOOL_GAP;
pub const ROW_GAP: f32 = SPACE_XS;
pub const BAR_MARGIN: Margin = Margin {
    left: SPACE_M as i8,
    right: SPACE_M as i8,
    top: SPACE_S as i8,
    bottom: SPACE_S as i8,
};
const CAPTION_GAP: f32 = SPACE_XS;
const WIDTH_TOLERANCE: f32 = 0.5;

pub fn width_id(ribbon: &'static str, group: impl Hash + Debug) -> Id {
    Id::new((ribbon, "group-width", group))
}

pub fn remembered_widths<G: Copy + Hash + Debug>(
    ui: &Ui,
    ribbon: &'static str,
    groups: &[G],
) -> Vec<(G, f32)> {
    groups
        .iter()
        .map(|&group| {
            (
                group,
                widgets::remembered_width(ui, width_id(ribbon, group)),
            )
        })
        .collect()
}

pub fn rows<G: Copy>(widths: &[(G, f32)], first: f32, rest: f32) -> Vec<Vec<G>> {
    let mut rows = Vec::new();
    let mut row: Vec<G> = Vec::new();
    let mut used = 0.0;
    for &(group, width) in widths {
        let room = if rows.is_empty() { first } else { rest };
        let needed = used + GROUP_SPACE + width;
        if row.is_empty() {
            used = width;
        } else if needed <= room + WIDTH_TOLERANCE {
            used = needed;
        } else {
            rows.push(std::mem::take(&mut row));
            used = width;
        }
        row.push(group);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

pub fn row<G: Copy + Hash + Debug>(
    ui: &mut Ui,
    ribbon: &'static str,
    groups: &[G],
    mut draw: impl FnMut(&mut Ui, G) -> (Rect, f32),
) {
    let divider = Stroke::new(BORDER_WIDTH, appearance::tokens(ui).border_strong);
    let mut previous: Option<f32> = None;
    for &group in groups {
        if previous.is_some() {
            ui.add_space(DIVIDER_SPACE);
        }
        let (rect, natural) = draw(ui, group);
        if let Some(right) = previous {
            ui.painter()
                .vline((right + rect.left()) / 2.0, rect.y_range(), divider);
        }
        previous = Some(rect.right());
        widgets::remember_width(ui, width_id(ribbon, group), natural);
    }
}

pub fn height(ui: &Ui, captions: bool) -> f32 {
    let band = widgets::tool_height(ui);
    if captions {
        band + CAPTION_GAP + ui.text_style_height(&TextStyle::Small)
    } else {
        band
    }
}

pub fn captioned(
    ui: &mut Ui,
    caption: Option<&str>,
    content: impl FnOnce(&mut Ui) -> f32,
) -> (Rect, f32) {
    let galley = caption.map(|caption| caption_galley(ui, caption));
    let shown = ui.vertical(|ui| {
        let content = content(ui);
        let Some(galley) = galley else {
            return content;
        };
        let top = ui.min_rect().bottom() + CAPTION_GAP;
        let width = ui.min_rect().width().max(galley.size().x);
        let height = ui.text_style_height(&TextStyle::Small);
        let rect = Rect::from_min_size(pos2(ui.min_rect().left(), top), vec2(width, height));
        let natural = content.max(galley.size().x);
        ui.put(rect, Label::new(galley).selectable(false));
        natural
    });
    (shown.response.rect, shown.inner)
}

pub fn explained(response: Response, title: &str, help: &Result<String, String>) -> Response {
    let tip = |ui: &mut Ui, text: &str| {
        ui.label(widgets::strong(title));
        ui.label(text);
    };
    match help {
        Ok(text) => response.on_hover_ui(|ui| tip(ui, text)),
        Err(text) => response.on_disabled_hover_ui(|ui| tip(ui, text)),
    }
}

fn caption_galley(ui: &Ui, text: &str) -> Arc<Galley> {
    let muted = appearance::tokens(ui).text_muted;
    WidgetText::from(
        RichText::new(text)
            .text_style(TextStyle::Small)
            .color(muted),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Small,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTHS: [(char, f32); 4] = [('a', 200.0), ('b', 50.0), ('c', 500.0), ('d', 50.0)];

    #[test]
    fn groups_fill_each_row_before_wrapping_and_the_first_row_can_leave_room() {
        assert_eq!(
            rows(&WIDTHS, 1000.0, 1000.0),
            vec![vec!['a', 'b', 'c', 'd']]
        );
        assert_eq!(
            rows(&WIDTHS, 700.0, 900.0),
            vec![vec!['a', 'b'], vec!['c', 'd']]
        );
        assert_eq!(
            rows(&WIDTHS, 300.0, 300.0),
            vec![vec!['a', 'b'], vec!['c'], vec!['d']]
        );
        assert_eq!(
            rows(&WIDTHS, 100.0, 100.0),
            vec![vec!['a'], vec!['b'], vec!['c'], vec!['d']]
        );
    }
}
