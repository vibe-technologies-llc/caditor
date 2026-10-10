use std::sync::Arc;

use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, accesskit::Live, vec2,
};

use crate::{
    appearance::CONTROL_HEIGHT,
    fonts,
    scene_palette::{Canvas, Contrast},
};

const CONTRAST_KEY: &str = "canvas-contrast";
const CANVAS_KEY: &str = "canvas-lightness";
pub const CUBE_DIMMEST_LABELLED_FACING: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    pub backdrop: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub dimension: Color32,
    pub hovered: Color32,
    pub selected: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub prompt: Color32,
    pub snap: Color32,
    pub measure: Color32,
    pub focus: Color32,
    pub key_cap: Color32,
    pub key_cap_edge: Color32,
    pub control_hovered: Color32,
    pub control_pressed: Color32,
    pub control_edge: Color32,
    pub cube_face: Color32,
    pub cube_hovered: Color32,
    pub cube_pressed: Color32,
    pub cube_label: Color32,
    pub cube_label_on_hover: Color32,
    pub cube_edge: Color32,
    pub cube_divider: Color32,
}

const DARK_BACKDROP: Color32 = Color32::from_rgba_premultiplied(16, 18, 23, 225);
const DARK_SELECTED: Color32 = Color32::from_rgb(96, 176, 255);
const DARK_HOVERED: Color32 = Color32::from_rgb(255, 196, 84);
const DARK_SNAP: Color32 = Color32::from_rgb(80, 226, 236);

pub const DARK_CHROME: Chrome = Chrome {
    backdrop: DARK_BACKDROP,
    panel: Color32::from_rgb(DARK_BACKDROP.r(), DARK_BACKDROP.g(), DARK_BACKDROP.b()),
    text: Color32::from_rgb(228, 231, 238),
    muted: Color32::from_rgb(170, 176, 188),
    dimension: Color32::from_rgb(200, 206, 222),
    hovered: DARK_HOVERED,
    selected: DARK_SELECTED,
    error: Color32::from_rgb(255, 128, 118),
    warning: Color32::from_rgb(242, 190, 80),
    prompt: Color32::from_rgb(255, 214, 120),
    snap: DARK_SNAP,
    measure: DARK_SNAP,
    focus: DARK_SELECTED,
    key_cap: Color32::from_rgb(46, 51, 62),
    key_cap_edge: Color32::from_rgb(88, 95, 110),
    control_hovered: Color32::from_rgba_premultiplied(40, 45, 56, 235),
    control_pressed: Color32::from_rgba_premultiplied(58, 65, 80, 240),
    control_edge: Color32::from_rgba_premultiplied(70, 76, 90, 200),
    cube_face: Color32::from_rgb(58, 64, 76),
    cube_hovered: DARK_HOVERED,
    cube_pressed: Color32::from_rgb(222, 158, 50),
    cube_label: Color32::from_rgb(225, 228, 235),
    cube_label_on_hover: Color32::from_rgb(30, 24, 12),
    cube_edge: Color32::from_rgb(120, 130, 150),
    cube_divider: Color32::from_rgb(32, 36, 44),
};

const LIGHT_SELECTED: Color32 = Color32::from_rgb(0, 80, 176);
const LIGHT_SNAP: Color32 = Color32::from_rgb(0, 90, 104);

pub const LIGHT_CHROME: Chrome = Chrome {
    backdrop: Color32::from_rgba_premultiplied(224, 225, 226, 230),
    panel: Color32::from_rgb(248, 249, 251),
    text: Color32::from_rgb(24, 27, 33),
    muted: Color32::from_rgb(64, 70, 82),
    dimension: Color32::from_rgb(36, 48, 76),
    hovered: Color32::from_rgb(120, 66, 0),
    selected: LIGHT_SELECTED,
    error: Color32::from_rgb(160, 16, 16),
    warning: Color32::from_rgb(120, 70, 0),
    prompt: Color32::from_rgb(110, 60, 0),
    snap: LIGHT_SNAP,
    measure: LIGHT_SNAP,
    focus: LIGHT_SELECTED,
    key_cap: Color32::from_rgb(232, 235, 240),
    key_cap_edge: Color32::from_rgb(150, 156, 168),
    control_hovered: Color32::from_rgba_premultiplied(218, 221, 226, 240),
    control_pressed: Color32::from_rgba_premultiplied(206, 210, 217, 245),
    control_edge: Color32::from_rgba_premultiplied(118, 122, 132, 200),
    cube_face: Color32::from_rgb(200, 206, 216),
    cube_hovered: DARK_HOVERED,
    cube_pressed: Color32::from_rgb(222, 158, 50),
    cube_label: Color32::from_rgb(24, 27, 33),
    cube_label_on_hover: Color32::from_rgb(30, 24, 12),
    cube_edge: Color32::from_rgb(110, 118, 134),
    cube_divider: Color32::from_rgb(170, 176, 186),
};

impl Chrome {
    pub fn backdrop(&self, contrast: Contrast) -> Color32 {
        match contrast {
            Contrast::Standard => self.backdrop,
            Contrast::High => self.panel,
        }
    }

    pub fn cube_face(&self, facing: f64) -> Color32 {
        let brightness = CUBE_DARKEST_SHADE + CUBE_SHADE_RANGE * facing.clamp(0.0, 1.0);
        let shade = |channel: u8| (f64::from(channel) * brightness).min(255.0) as u8;
        Color32::from_rgb(
            shade(self.cube_face.r()),
            shade(self.cube_face.g()),
            shade(self.cube_face.b()),
        )
    }
}

pub const SMALL_SIZE: f32 = 11.5;
pub const BODY_SIZE: f32 = 13.0;
pub const TITLE_SIZE: f32 = 15.0;
pub const ICON_SIZE: f32 = 14.0;
pub const PADDING: Vec2 = vec2(5.0, 2.0);
pub const RADIUS: f32 = 3.0;
pub const MARGIN: f32 = 12.0;
pub const BUTTON_HEIGHT: f32 = CONTROL_HEIGHT;

const HINT_PADDING: Vec2 = vec2(6.0, 4.0);
const KEY_PADDING: Vec2 = vec2(4.0, 1.0);
const KEY_EDGE_WIDTH: f32 = 1.0;
const ATOM_GAP: f32 = 4.0;
const ITEM_GAP: f32 = 14.0;
const ROW_GAP: f32 = 4.0;
const MIN_WRAPPED_ACTION: f32 = 48.0;
const HINT_SEPARATOR: &str = "   ";
const KEY_SEPARATOR: &str = ": ";
const ALTERNATIVE: &str = " or ";
const ALTERNATIVE_WORD: &str = "or";
const BUTTON_GAP: f32 = 5.0;
const FOCUS_GAP: f32 = 2.0;
const FOCUS_WIDTH: f32 = 2.0;
const CUBE_DARKEST_SHADE: f64 = 0.75;
const CUBE_SHADE_RANGE: f64 = 0.35;

pub fn small() -> FontId {
    FontId::proportional(SMALL_SIZE)
}

pub fn body() -> FontId {
    FontId::proportional(BODY_SIZE)
}

pub fn title() -> FontId {
    FontId::new(TITLE_SIZE, fonts::medium())
}

pub fn emphasis() -> FontId {
    FontId::new(SMALL_SIZE, fonts::medium())
}

pub fn readout() -> FontId {
    FontId::monospace(SMALL_SIZE)
}

pub fn icon() -> FontId {
    FontId::new(ICON_SIZE, fonts::icons())
}

pub fn set_contrast(ctx: &egui::Context, contrast: Contrast) {
    ctx.data_mut(|data| data.insert_temp(Id::new(CONTRAST_KEY), contrast));
}

fn contrast(ctx: &egui::Context) -> Contrast {
    ctx.data(|data| data.get_temp(Id::new(CONTRAST_KEY)))
        .unwrap_or_default()
}

pub fn set_canvas(ctx: &egui::Context, canvas: Canvas) {
    ctx.data_mut(|data| data.insert_temp(Id::new(CANVAS_KEY), canvas));
}

pub fn canvas(ctx: &egui::Context) -> Canvas {
    ctx.data(|data| data.get_temp(Id::new(CANVAS_KEY)))
        .unwrap_or_default()
}

pub fn chrome(ctx: &egui::Context) -> &'static Chrome {
    canvas(ctx).chrome()
}

pub fn paint_backdrop(painter: &Painter, rect: Rect) {
    let ctx = painter.ctx();
    painter.rect_filled(rect, RADIUS, chrome(ctx).backdrop(contrast(ctx)));
}

pub fn announce(ui: &Ui, rect: Rect, name: &str, text: &str, live: Option<Live>) {
    let response = ui.interact(rect, ui.id().with(("canvas text", name)), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    if let Some(live) = live {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_live(live));
    }
}

pub fn chip_size(text: Vec2) -> Vec2 {
    text + PADDING * 2.0
}

pub struct Label {
    galley: Arc<Galley>,
    color: Color32,
}

impl Label {
    pub fn new(
        painter: &Painter,
        text: impl ToString,
        font: FontId,
        color: Color32,
        max_width: f32,
    ) -> Self {
        let wrap_width = (max_width - PADDING.x * 2.0).max(0.0);
        Self {
            galley: painter.layout(text.to_string(), font, color, wrap_width),
            color,
        }
    }

    pub fn size(&self) -> Vec2 {
        chip_size(self.galley.size())
    }

    pub fn paint(self, painter: &Painter, min: Pos2) -> Rect {
        let rect = Rect::from_min_size(min, self.size());
        paint_backdrop(painter, rect);
        painter.galley(rect.min + PADDING, self.galley, self.color);
        rect
    }

    pub fn paint_at(self, painter: &Painter, anchor: Pos2, align: Align2) -> Rect {
        let rect = align.anchor_size(anchor, self.size());
        self.paint(painter, rect.min)
    }
}

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
    Label::new(painter, text, font, color, max_width).paint_at(painter, anchor, align)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintPart<'a> {
    Keys { keys: Vec<&'a str>, action: &'a str },
    Text(&'a str),
}

pub fn hint_parts(text: &str) -> Vec<HintPart<'_>> {
    text.split(HINT_SEPARATOR)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once(KEY_SEPARATOR)
                .and_then(|(keys, action)| {
                    let keys: Vec<&str> = keys.split(ALTERNATIVE).collect();
                    let all_keys = keys
                        .iter()
                        .all(|key| !key.is_empty() && !key.contains(char::is_whitespace));
                    all_keys.then_some(HintPart::Keys { keys, action })
                })
                .unwrap_or(HintPart::Text(part))
        })
        .collect()
}

#[cfg(test)]
pub fn hint_texts(text: &str) -> Vec<&str> {
    hint_parts(text)
        .into_iter()
        .flat_map(|part| match part {
            HintPart::Keys { keys, action } => {
                let mut texts = Vec::new();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        texts.push(ALTERNATIVE_WORD);
                    }
                    texts.push(key);
                }
                texts.push(action);
                texts
            }
            HintPart::Text(text) => vec![text],
        })
        .collect()
}

struct Atom {
    galley: Arc<Galley>,
    key: bool,
    size: Vec2,
}

impl Atom {
    fn text(painter: &Painter, text: &str, wrap_width: f32) -> Self {
        let muted = chrome(painter.ctx()).muted;
        let galley = painter.layout(text.to_owned(), small(), muted, wrap_width);
        Self {
            size: galley.size(),
            galley,
            key: false,
        }
    }

    fn key(painter: &Painter, key: &str) -> Self {
        let text = chrome(painter.ctx()).text;
        let galley = painter.layout_no_wrap(key.to_owned(), emphasis(), text);
        Self {
            size: galley.size() + KEY_PADDING * 2.0,
            galley,
            key: true,
        }
    }
}

struct Item {
    atoms: Vec<Atom>,
}

impl Item {
    fn width(&self) -> f32 {
        let gaps = self.atoms.len().saturating_sub(1) as f32 * ATOM_GAP;
        self.atoms.iter().map(|atom| atom.size.x).sum::<f32>() + gaps
    }

    fn height(&self) -> f32 {
        self.atoms
            .iter()
            .map(|atom| atom.size.y)
            .fold(0.0, f32::max)
    }

    fn of(painter: &Painter, part: &HintPart<'_>, inner_width: f32) -> Self {
        match part {
            HintPart::Text(text) => Self {
                atoms: vec![Atom::text(painter, text, inner_width)],
            },
            HintPart::Keys { keys, action } => {
                let mut item = Self {
                    atoms: Self::of_keys(painter, keys),
                };
                let wrap = (inner_width - item.width() - ATOM_GAP).max(MIN_WRAPPED_ACTION);
                item.atoms.push(Atom::text(painter, action, wrap));
                item
            }
        }
    }

    fn of_keys(painter: &Painter, keys: &[&str]) -> Vec<Atom> {
        let mut atoms = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                atoms.push(Atom::text(painter, ALTERNATIVE_WORD, f32::INFINITY));
            }
            atoms.push(Atom::key(painter, key));
        }
        atoms
    }
}

struct Row {
    items: Vec<Item>,
    width: f32,
    height: f32,
}

pub struct Hints {
    rows: Vec<Row>,
    align: Align,
    padding: Vec2,
    size: Vec2,
}

impl Hints {
    pub fn new(painter: &Painter, text: &str, max_width: f32, align: Align) -> Self {
        Self::laid_out(painter, text, max_width, align, HINT_PADDING)
    }

    pub fn bare(painter: &Painter, text: &str, max_width: f32, align: Align) -> Self {
        Self::laid_out(painter, text, max_width, align, Vec2::ZERO)
    }

    fn laid_out(
        painter: &Painter,
        text: &str,
        max_width: f32,
        align: Align,
        padding: Vec2,
    ) -> Self {
        let inner_width = (max_width - padding.x * 2.0).max(0.0);
        let mut rows: Vec<Row> = Vec::new();
        for part in hint_parts(text) {
            let item = Item::of(painter, &part, inner_width);
            let (width, height) = (item.width(), item.height());
            match rows.last_mut() {
                Some(row) if row.width + ITEM_GAP + width <= inner_width => {
                    row.width += ITEM_GAP + width;
                    row.height = row.height.max(height);
                    row.items.push(item);
                }
                _ => rows.push(Row {
                    items: vec![item],
                    width,
                    height,
                }),
            }
        }
        let width = rows.iter().map(|row| row.width).fold(0.0, f32::max);
        let gaps = rows.len().saturating_sub(1) as f32 * ROW_GAP;
        let height = rows.iter().map(|row| row.height).sum::<f32>() + gaps;
        Self {
            rows,
            align,
            padding,
            size: vec2(width, height) + padding * 2.0,
        }
    }

    pub fn size(&self) -> Vec2 {
        self.size
    }

    pub fn paint_at(self, painter: &Painter, anchor: Pos2, align: Align2) -> Rect {
        let rect = align.anchor_size(anchor, self.size);
        self.paint(painter, rect.min)
    }

    pub fn paint(self, painter: &Painter, min: Pos2) -> Rect {
        let rect = Rect::from_min_size(min, self.size);
        if self.rows.is_empty() {
            return rect;
        }
        if self.padding != Vec2::ZERO {
            paint_backdrop(painter, rect);
        }
        let inner = rect.shrink2(self.padding);
        let mut top = inner.top();
        for row in self.rows {
            let mut x = match self.align {
                Align::Min => inner.left(),
                Align::Center => inner.center().x - row.width / 2.0,
                Align::Max => inner.right() - row.width,
            };
            for item in row.items {
                for atom in item.atoms {
                    let atom_rect = Rect::from_min_size(
                        Pos2::new(x, top + (row.height - atom.size.y) / 2.0),
                        atom.size,
                    );
                    paint_atom(painter, atom_rect, atom);
                    x = atom_rect.right() + ATOM_GAP;
                }
                x += ITEM_GAP - ATOM_GAP;
            }
            top += row.height + ROW_GAP;
        }
        rect
    }
}

fn paint_atom(painter: &Painter, rect: Rect, atom: Atom) {
    let chrome = chrome(painter.ctx());
    if atom.key {
        painter.rect(
            rect,
            RADIUS,
            chrome.key_cap,
            Stroke::new(KEY_EDGE_WIDTH, chrome.key_cap_edge),
            StrokeKind::Inside,
        );
        painter.galley(rect.min + KEY_PADDING, atom.galley, chrome.text);
    } else {
        painter.galley(rect.min, atom.galley, chrome.muted);
    }
}

pub fn button(ui: &Ui, rect: Rect, id: Id, glyph: &str, text: &str) -> Response {
    control(ui, rect, id, glyph, Some(text), text)
}

pub fn icon_button(ui: &Ui, rect: Rect, id: Id, glyph: &str, name: &str) -> Response {
    control(ui, rect, id, glyph, None, name)
}

fn control(ui: &Ui, rect: Rect, id: Id, glyph: &str, text: Option<&str>, name: &str) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name));
    let chrome = chrome(ui.ctx());
    let fill = if response.is_pointer_button_down_on() {
        chrome.control_pressed
    } else if response.hovered() || response.has_focus() {
        chrome.control_hovered
    } else {
        chrome.backdrop
    };
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(KEY_EDGE_WIDTH, chrome.control_edge),
        StrokeKind::Inside,
    );
    let glyph = painter.layout_no_wrap(glyph.to_owned(), icon(), chrome.text);
    let label = text.map(|text| painter.layout_no_wrap(text.to_owned(), emphasis(), chrome.text));
    let label_width = label
        .as_ref()
        .map_or(0.0, |label| BUTTON_GAP + label.size().x);
    let left = rect.center().x - (glyph.size().x + label_width) / 2.0;
    let centred = |height: f32| rect.center().y - height / 2.0;
    let label_left = left + glyph.size().x + BUTTON_GAP;
    painter.galley(Pos2::new(left, centred(glyph.size().y)), glyph, chrome.text);
    if let Some(label) = label {
        let label_top = centred(label.size().y);
        painter.galley(Pos2::new(label_left, label_top), label, chrome.text);
    }
    if response.has_focus() {
        paint_focus_ring(painter, rect);
    }
    response
}

pub fn paint_focus_ring(painter: &Painter, rect: Rect) {
    painter.rect_stroke(
        rect.expand(FOCUS_GAP),
        RADIUS + FOCUS_GAP,
        Stroke::new(FOCUS_WIDTH, chrome(painter.ctx()).focus),
        StrokeKind::Outside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::tests::{HIGHLY_READABLE, READABLE, contrast_ratio};

    fn over(fill: Color32, below: Color32) -> Color32 {
        let uncovered = 1.0 - f32::from(fill.a()) / 255.0;
        let channel =
            |above: u8, under: u8| (f32::from(above) + uncovered * f32::from(under)).round() as u8;
        Color32::from_rgb(
            channel(fill.r(), below.r()),
            channel(fill.g(), below.g()),
            channel(fill.b(), below.b()),
        )
    }

    const CHROMES: [(&str, Chrome); 2] = [("dark", DARK_CHROME), ("light", LIGHT_CHROME)];

    fn below(chrome: &Chrome) -> [Color32; 3] {
        [Color32::BLACK, Color32::WHITE, chrome.hovered]
    }

    fn label_colours(chrome: &Chrome) -> [(&'static str, Color32); 10] {
        [
            ("muted", chrome.muted),
            ("dimension", chrome.dimension),
            ("hovered", chrome.hovered),
            ("selected", chrome.selected),
            ("error", chrome.error),
            ("warning", chrome.warning),
            ("prompt", chrome.prompt),
            ("snap", chrome.snap),
            ("measure", chrome.measure),
            ("focus", chrome.focus),
        ]
    }

    #[test]
    fn canvas_text_is_readable_over_its_backdrop_on_any_background() {
        for (name, chrome) in CHROMES {
            for below in below(&chrome) {
                let backdrop = over(chrome.backdrop, below);
                assert!(
                    contrast_ratio(chrome.text, backdrop) >= HIGHLY_READABLE,
                    "{name}"
                );
                for (what, color) in label_colours(&chrome) {
                    let ratio = contrast_ratio(color, backdrop);
                    assert!(
                        ratio >= READABLE,
                        "{name} {what} over {below:?} is {ratio:.2}:1"
                    );
                }
            }
        }
    }

    #[test]
    fn high_contrast_labels_sit_on_an_opaque_backdrop_holding_every_colour_to_seven_to_one() {
        for (name, chrome) in CHROMES {
            let backdrop = chrome.backdrop(Contrast::High);
            assert_eq!(backdrop.a(), 255);
            let text = ("text", chrome.text);
            for (what, color) in label_colours(&chrome).into_iter().chain([text]) {
                let ratio = contrast_ratio(color, backdrop);
                assert!(ratio >= HIGHLY_READABLE, "{name} {what} is {ratio:.2}:1");
            }
            assert_eq!(chrome.backdrop(Contrast::Standard), chrome.backdrop);
        }
    }

    #[test]
    fn key_caps_and_canvas_controls_keep_their_text_readable_in_every_state() {
        for (name, chrome) in CHROMES {
            assert!(
                contrast_ratio(chrome.text, chrome.key_cap) >= HIGHLY_READABLE,
                "{name}"
            );
            assert!(
                contrast_ratio(chrome.text, chrome.panel) >= HIGHLY_READABLE,
                "{name}"
            );
            assert!(
                contrast_ratio(chrome.muted, chrome.panel) >= READABLE,
                "{name}"
            );
            assert!(
                contrast_ratio(chrome.error, chrome.panel) >= READABLE,
                "{name}"
            );
            for below in below(&chrome) {
                for (what, fill) in [
                    ("control", chrome.backdrop),
                    ("hovered control", chrome.control_hovered),
                    ("pressed control", chrome.control_pressed),
                ] {
                    let ratio = contrast_ratio(chrome.text, over(fill, below));
                    assert!(
                        ratio >= READABLE,
                        "{name} text on a {what} over {below:?} is {ratio:.2}:1"
                    );
                }
            }
        }
    }

    #[test]
    fn view_cube_labels_are_readable_on_every_cell_state() {
        let steps = 10;
        for (name, chrome) in CHROMES {
            for step in 0..=steps {
                let facing = CUBE_DIMMEST_LABELLED_FACING
                    + (1.0 - CUBE_DIMMEST_LABELLED_FACING) * f64::from(step) / f64::from(steps);
                let ratio = contrast_ratio(chrome.cube_label, chrome.cube_face(facing));
                assert!(
                    ratio >= READABLE,
                    "{name} facing {facing:.2} is {ratio:.2}:1"
                );
            }
            for (what, fill) in [
                ("hovered", chrome.cube_hovered),
                ("pressed", chrome.cube_pressed),
            ] {
                let ratio = contrast_ratio(chrome.cube_label_on_hover, fill);
                assert!(ratio >= READABLE, "{name} the {what} cell is {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn hints_split_into_keys_and_actions_and_keep_plain_text_whole() {
        assert_eq!(
            hint_parts("Right-drag: orbit   ] or [: more or fewer sides   Type x, y or a: b"),
            vec![
                HintPart::Keys {
                    keys: vec!["Right-drag"],
                    action: "orbit"
                },
                HintPart::Keys {
                    keys: vec!["]", "["],
                    action: "more or fewer sides"
                },
                HintPart::Text("Type x, y or a: b"),
            ]
        );
        assert_eq!(
            hint_texts("Esc: cancel   Middle-drag or Shift+right-drag: pan"),
            vec![
                "Esc",
                "cancel",
                "Middle-drag",
                "or",
                "Shift+right-drag",
                "pan"
            ]
        );
    }
}
