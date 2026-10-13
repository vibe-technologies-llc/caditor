use std::{fmt::Debug, hash::Hash, sync::Arc};

use egui::{
    Frame, Galley, Id, Label, Margin, Popup, Rect, Response, RichText, Sense, Stroke, Style,
    TextStyle, TextWrapMode, Ui, WidgetText, pos2, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, SPACE_L, SPACE_M, SPACE_S, SPACE_XS},
    commands::Command,
    menu_bar::MenuEntries,
    widgets::{self, CornerMenu},
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
pub const MOST_LABELLED_SHARE: f32 = 0.2;
const CAPTION_GAP: f32 = SPACE_XS;
const WIDTH_TOLERANCE: f32 = 0.5;

pub fn bar_frame(style: &Style) -> Frame {
    Frame::side_top_panel(style).inner_margin(BAR_MARGIN)
}

pub fn labelled_too_tall(ui: &Ui, rows: usize) -> bool {
    rows_height(ui, rows) > MOST_LABELLED_SHARE * ui.ctx().content_rect().height()
}

pub fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

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

pub fn rows_height(ui: &Ui, rows: usize) -> f32 {
    let count = rows as f32;
    count * height(ui, false) + (count - 1.0).max(0.0) * ROW_GAP
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
    show_caption: bool,
    content: impl FnOnce(&mut Ui) -> f32,
) -> (Rect, f32) {
    let galley = caption.map(|caption| caption_galley(ui, caption));
    captioned_by(ui, galley, show_caption, content, |ui, rect, galley| {
        ui.put(rect, Label::new(galley).selectable(false));
    })
}

pub fn captioned_by(
    ui: &mut Ui,
    galley: Option<Arc<Galley>>,
    show_caption: bool,
    content: impl FnOnce(&mut Ui) -> f32,
    caption: impl FnOnce(&mut Ui, Rect, Arc<Galley>),
) -> (Rect, f32) {
    let shown = ui.vertical(|ui| {
        let content = content(ui);
        let Some(galley) = galley else {
            return content;
        };
        let natural = content.max(galley.size().x);
        if show_caption {
            let top = ui.min_rect().bottom() + CAPTION_GAP;
            let width = ui.min_rect().width().max(galley.size().x);
            let height = ui.text_style_height(&TextStyle::Small);
            let rect = Rect::from_min_size(pos2(ui.min_rect().left(), top), vec2(width, height));
            caption(ui, rect, galley);
        }
        natural
    });
    (shown.response.rect, shown.inner)
}

#[derive(Debug, Clone, Copy)]
pub struct GroupTools {
    pub width: f32,
    pub host: Rect,
    pub compact_host: bool,
}

pub struct GroupMenu<'a> {
    pub id: Id,
    pub name: &'a str,
    pub contents: &'a str,
}

impl GroupMenu<'_> {
    fn corner(&self) -> CornerMenu<'_> {
        CornerMenu {
            id: self.id,
            name: self.name,
            contents: self.contents,
            on_selected: false,
        }
    }
}

pub fn menu_group(
    ui: &mut Ui,
    title: Option<Arc<Galley>>,
    show_caption: bool,
    menu: &GroupMenu<'_>,
    content: impl FnOnce(&mut Ui) -> GroupTools,
) -> ((Rect, f32), Option<Response>) {
    let titled = show_caption && title.is_some();
    let title_width = title
        .as_ref()
        .map_or(0.0, |title| widgets::group_title_width(ui, title));
    let mut corner = None;
    let mut caption = None;
    let shown = captioned_by(
        ui,
        title,
        show_caption,
        |ui| {
            let tools = content(ui);
            if !titled {
                corner = Some(corner_trigger(ui, tools, menu));
            }
            tools.width.max(title_width)
        },
        |ui, rect, title| {
            let rect =
                Rect::from_min_size(rect.min, vec2(rect.width().max(title_width), rect.height()));
            let height = rect.height().max(title.size().y);
            let strip = Rect::from_min_size(rect.min, vec2(rect.width(), height));
            ui.allocate_rect(strip, Sense::hover());
            caption = Some(widgets::group_title(
                ui,
                rect,
                menu.id.with("title"),
                title,
                menu.name,
            ));
        },
    );
    (shown, caption.or(corner))
}

fn corner_trigger(ui: &mut Ui, tools: GroupTools, menu: &GroupMenu<'_>) -> Response {
    let corner = menu.corner();
    if tools.compact_host {
        widgets::compact_corner_menu_button(ui, tools.host, &corner)
    } else {
        widgets::corner_menu_button(ui, tools.host, &corner)
    }
}

pub fn group_menu(
    trigger: &Response,
    entries: MenuEntries<'_>,
    body: impl FnOnce(&mut Ui, &mut MenuEntries<'_>),
) -> Vec<Command> {
    let mut entries = if trigger.clicked() {
        entries.focusing_first()
    } else {
        entries
    };
    let shown = Popup::menu(trigger).show(|ui| {
        widgets::fitted_menu(ui, |ui| body(ui, &mut entries));
        let picked = entries.take_chosen();
        if !picked.is_empty() {
            ui.close();
        }
        picked
    });
    shown.map(|shown| shown.inner).unwrap_or_default()
}

pub fn menu_sections<S: AsRef<[Command]>>(
    ui: &mut Ui,
    entries: &mut MenuEntries<'_>,
    sections: &[S],
    mut entry: impl FnMut(&mut Ui, &mut MenuEntries<'_>, Command),
) {
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            ui.separator();
        }
        for &command in section.as_ref() {
            entry(ui, entries, command);
        }
    }
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

pub fn caption_galley(ui: &Ui, text: &str) -> Arc<Galley> {
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
    fn names_are_listed_in_words() {
        assert_eq!(listed(&[]), "");
        assert_eq!(listed(&["Offset"]), "Offset");
        assert_eq!(listed(&["Offset", "Blend curve"]), "Offset and Blend curve");
        assert_eq!(
            listed(&["Offset", "Tangent circle", "Blend curve"]),
            "Offset, Tangent circle and Blend curve"
        );
    }

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
