use std::sync::Arc;

use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, accesskit::Live, vec2,
};

use crate::{appearance::CONTROL_HEIGHT, fonts, scene_palette::Contrast};

pub const BACKDROP: Color32 = Color32::from_rgba_premultiplied(16, 18, 23, 225);
pub const PANEL: Color32 = Color32::from_rgb(BACKDROP.r(), BACKDROP.g(), BACKDROP.b());
const CONTRAST_KEY: &str = "canvas-contrast";
pub const TEXT: Color32 = Color32::from_rgb(228, 231, 238);
pub const MUTED: Color32 = Color32::from_rgb(170, 176, 188);
pub const DIMENSION: Color32 = Color32::from_rgb(200, 206, 222);
pub const HOVERED: Color32 = Color32::from_rgb(255, 196, 84);
pub const SELECTED: Color32 = Color32::from_rgb(96, 176, 255);
pub const ERROR: Color32 = Color32::from_rgb(255, 128, 118);
pub const WARNING: Color32 = Color32::from_rgb(242, 190, 80);
pub const PROMPT: Color32 = Color32::from_rgb(255, 214, 120);
pub const SNAP: Color32 = Color32::from_rgb(80, 226, 236);
pub const MEASURE: Color32 = SNAP;
pub const FOCUS: Color32 = SELECTED;

pub const KEY_CAP: Color32 = Color32::from_rgb(46, 51, 62);
pub const KEY_CAP_EDGE: Color32 = Color32::from_rgb(88, 95, 110);
pub const CONTROL_HOVERED: Color32 = Color32::from_rgba_premultiplied(40, 45, 56, 235);
pub const CONTROL_PRESSED: Color32 = Color32::from_rgba_premultiplied(58, 65, 80, 240);
pub const CONTROL_EDGE: Color32 = Color32::from_rgba_premultiplied(70, 76, 90, 200);

pub const CUBE_FACE: Color32 = Color32::from_rgb(58, 64, 76);
pub const CUBE_HOVERED: Color32 = HOVERED;
pub const CUBE_PRESSED: Color32 = Color32::from_rgb(222, 158, 50);
pub const CUBE_LABEL: Color32 = Color32::from_rgb(225, 228, 235);
pub const CUBE_LABEL_ON_HOVER: Color32 = Color32::from_rgb(30, 24, 12);
pub const CUBE_EDGE: Color32 = Color32::from_rgb(120, 130, 150);
pub const CUBE_DIVIDER: Color32 = Color32::from_rgb(32, 36, 44);
pub const CUBE_DIMMEST_LABELLED_FACING: f64 = 0.3;

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

pub fn cube_face(facing: f64) -> Color32 {
    let brightness = CUBE_DARKEST_SHADE + CUBE_SHADE_RANGE * facing.clamp(0.0, 1.0);
    let shade = |channel: u8| (f64::from(channel) * brightness).min(255.0) as u8;
    Color32::from_rgb(
        shade(CUBE_FACE.r()),
        shade(CUBE_FACE.g()),
        shade(CUBE_FACE.b()),
    )
}

pub fn set_contrast(ctx: &egui::Context, contrast: Contrast) {
    ctx.data_mut(|data| data.insert_temp(Id::new(CONTRAST_KEY), contrast));
}

fn contrast(ctx: &egui::Context) -> Contrast {
    ctx.data(|data| data.get_temp(Id::new(CONTRAST_KEY)))
        .unwrap_or_default()
}

pub fn backdrop(contrast: Contrast) -> Color32 {
    match contrast {
        Contrast::Standard => BACKDROP,
        Contrast::High => PANEL,
    }
}

pub fn paint_backdrop(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, RADIUS, backdrop(contrast(painter.ctx())));
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
        let galley = painter.layout(text.to_owned(), small(), MUTED, wrap_width);
        Self {
            size: galley.size(),
            galley,
            key: false,
        }
    }

    fn key(painter: &Painter, key: &str) -> Self {
        let galley = painter.layout_no_wrap(key.to_owned(), emphasis(), TEXT);
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
    if atom.key {
        painter.rect(
            rect,
            RADIUS,
            KEY_CAP,
            Stroke::new(KEY_EDGE_WIDTH, KEY_CAP_EDGE),
            StrokeKind::Inside,
        );
        painter.galley(rect.min + KEY_PADDING, atom.galley, TEXT);
    } else {
        painter.galley(rect.min, atom.galley, MUTED);
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
    let fill = if response.is_pointer_button_down_on() {
        CONTROL_PRESSED
    } else if response.hovered() || response.has_focus() {
        CONTROL_HOVERED
    } else {
        BACKDROP
    };
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.rect(
        rect,
        RADIUS,
        fill,
        Stroke::new(KEY_EDGE_WIDTH, CONTROL_EDGE),
        StrokeKind::Inside,
    );
    let glyph = painter.layout_no_wrap(glyph.to_owned(), icon(), TEXT);
    let label = text.map(|text| painter.layout_no_wrap(text.to_owned(), emphasis(), TEXT));
    let label_width = label
        .as_ref()
        .map_or(0.0, |label| BUTTON_GAP + label.size().x);
    let left = rect.center().x - (glyph.size().x + label_width) / 2.0;
    let centred = |height: f32| rect.center().y - height / 2.0;
    let label_left = left + glyph.size().x + BUTTON_GAP;
    painter.galley(Pos2::new(left, centred(glyph.size().y)), glyph, TEXT);
    if let Some(label) = label {
        let label_top = centred(label.size().y);
        painter.galley(Pos2::new(label_left, label_top), label, TEXT);
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
        Stroke::new(FOCUS_WIDTH, FOCUS),
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

    const BELOW: [Color32; 3] = [Color32::BLACK, Color32::WHITE, HOVERED];

    #[test]
    fn canvas_text_is_readable_over_its_backdrop_on_any_background() {
        for below in BELOW {
            let backdrop = over(BACKDROP, below);
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
                ("focus", FOCUS),
            ] {
                let ratio = contrast_ratio(color, backdrop);
                assert!(ratio >= READABLE, "{what} over {below:?} is {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn high_contrast_labels_sit_on_an_opaque_backdrop_holding_every_colour_to_seven_to_one() {
        let backdrop = backdrop(Contrast::High);
        assert_eq!(backdrop.a(), 255);
        for (what, color) in [
            ("text", TEXT),
            ("muted", MUTED),
            ("dimension", DIMENSION),
            ("hovered", HOVERED),
            ("selected", SELECTED),
            ("error", ERROR),
            ("warning", WARNING),
            ("prompt", PROMPT),
            ("snap", SNAP),
            ("measure", MEASURE),
            ("focus", FOCUS),
        ] {
            let ratio = contrast_ratio(color, backdrop);
            assert!(ratio >= HIGHLY_READABLE, "{what} is {ratio:.2}:1");
        }
        assert_eq!(super::backdrop(Contrast::Standard), BACKDROP);
    }

    #[test]
    fn key_caps_and_canvas_controls_keep_their_text_readable_in_every_state() {
        assert!(contrast_ratio(TEXT, KEY_CAP) >= HIGHLY_READABLE);
        assert!(contrast_ratio(TEXT, PANEL) >= HIGHLY_READABLE);
        assert!(contrast_ratio(MUTED, PANEL) >= READABLE);
        assert!(contrast_ratio(ERROR, PANEL) >= READABLE);
        for below in BELOW {
            for (what, fill) in [
                ("control", BACKDROP),
                ("hovered control", CONTROL_HOVERED),
                ("pressed control", CONTROL_PRESSED),
            ] {
                let ratio = contrast_ratio(TEXT, over(fill, below));
                assert!(
                    ratio >= READABLE,
                    "text on a {what} over {below:?} is {ratio:.2}:1"
                );
            }
        }
    }

    #[test]
    fn view_cube_labels_are_readable_on_every_cell_state() {
        let steps = 10;
        for step in 0..=steps {
            let facing = CUBE_DIMMEST_LABELLED_FACING
                + (1.0 - CUBE_DIMMEST_LABELLED_FACING) * f64::from(step) / f64::from(steps);
            let ratio = contrast_ratio(CUBE_LABEL, cube_face(facing));
            assert!(ratio >= READABLE, "facing {facing:.2} is {ratio:.2}:1");
        }
        for (what, fill) in [("hovered", CUBE_HOVERED), ("pressed", CUBE_PRESSED)] {
            let ratio = contrast_ratio(CUBE_LABEL_ON_HOVER, fill);
            assert!(ratio >= READABLE, "the {what} cell is {ratio:.2}:1");
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
