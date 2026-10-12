use caditor_geometry::Point3;
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, WidgetInfo,
    WidgetType, accesskit::Live, pos2, vec2,
};

use crate::{
    appearance::{SPACE_M, SPACE_S, SPACE_SM},
    canvas::{self, ChipLook, FEWER_KEYS, HintList, Hints, MORE_KEYS},
    commands::Command,
    icons,
    scene_palette::{ScenePalette, color32},
};

pub const POINTER_OFFSET: Vec2 = vec2(14.0, 10.0);
const STACK_GAP: f32 = SPACE_S;
const NOTE_WIDTH: f32 = 300.0;
const DESCRIPTION_WIDTH: f32 = 360.0;
pub const NOTE_SECONDS: f64 = 2.5;
const NOTE_FADE_SECONDS: f64 = 0.8;
pub const HOVER_DELAY: f64 = 0.45;
pub const ORBIT_MARK_LINGER: f64 = 0.4;
const PIVOT_RADIUS: f32 = 6.0;
const PIVOT_DOT: f32 = 2.0;
const PIVOT_STROKE: f32 = 2.0;
const PIVOT_HALO: f32 = 4.0;
const PROMPT_MARGIN: f32 = 16.0;
const PROMPT_MAX_WIDTH: f32 = 720.0;
const EMPTY_WIDTH: f32 = 420.0;
const EMPTY_BUTTON_PADDING: f32 = SPACE_M;
const EMPTY_GAP: f32 = SPACE_SM;
const MORE_KEYS_ID: &str = "more keys";

#[derive(Debug, Clone, PartialEq)]
pub struct RefusalNote {
    text: String,
    since: f64,
}

impl RefusalNote {
    pub fn new(text: impl Into<String>, since: f64) -> Self {
        Self {
            text: text.into(),
            since,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn opacity(&self, now: f64) -> Option<f32> {
        let age = now - self.since;
        if !(0.0..NOTE_SECONDS).contains(&age) {
            return None;
        }
        let fading_for = NOTE_SECONDS - age;
        Some((fading_for / NOTE_FADE_SECONDS).min(1.0) as f32)
    }

    pub fn repaint_after(&self, now: f64) -> f64 {
        let age = now - self.since;
        let fade_starts = NOTE_SECONDS - NOTE_FADE_SECONDS;
        if age < fade_starts {
            fade_starts - age
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoverDelay {
    shown: Option<(String, f64)>,
}

impl HoverDelay {
    pub fn waited(&mut self, text: Option<&str>, now: f64) -> Waited {
        let Some(text) = text else {
            self.shown = None;
            return Waited::Nothing;
        };
        let since = match &self.shown {
            Some((shown, since)) if shown == text => *since,
            _ => {
                self.shown = Some((text.to_owned(), now));
                now
            }
        };
        let waited = now - since;
        if waited >= HOVER_DELAY {
            Waited::Shown
        } else {
            Waited::Waiting(HOVER_DELAY - waited)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waited {
    Nothing,
    Waiting(f64),
    Shown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitMark {
    pub pivot: Point3,
    pub at: f64,
    pub held: bool,
}

impl OrbitMark {
    pub fn shown(&self, now: f64) -> bool {
        self.held || now - self.at < ORBIT_MARK_LINGER
    }
}

pub fn paint_pivot(ui: &Ui, at: Pos2, palette: &ScenePalette) {
    let painter = ui.painter();
    let mark = color32(palette.snap);
    let halo = color32(palette.outline);
    painter.circle_stroke(at, PIVOT_RADIUS, Stroke::new(PIVOT_HALO, halo));
    painter.circle_stroke(at, PIVOT_RADIUS, Stroke::new(PIVOT_STROKE, mark));
    painter.circle_filled(at, PIVOT_DOT + PIVOT_STROKE / 2.0, halo);
    painter.circle_filled(at, PIVOT_DOT, mark);
}

pub struct PointerLabel {
    pub name: &'static str,
    pub text: String,
    pub font: FontId,
    pub color: Color32,
    pub opacity: f32,
    pub live: Option<Live>,
    pub max_width: f32,
}

impl PointerLabel {
    pub fn new(name: &'static str, text: impl Into<String>, color: Color32) -> Self {
        Self {
            name,
            text: text.into(),
            font: canvas::small(),
            color,
            opacity: 1.0,
            live: None,
            max_width: NOTE_WIDTH,
        }
    }

    #[must_use]
    pub fn live(self, live: Live) -> Self {
        Self {
            live: Some(live),
            ..self
        }
    }

    #[must_use]
    pub fn faded(self, opacity: f32) -> Self {
        Self { opacity, ..self }
    }

    #[must_use]
    pub fn description(self) -> Self {
        Self {
            font: canvas::body(),
            max_width: DESCRIPTION_WIDTH,
            ..self
        }
    }
}

pub fn paint_beside_pointer(
    ui: &Ui,
    pointer: Pos2,
    view: Rect,
    labels: Vec<PointerLabel>,
) -> Vec<Rect> {
    let room = view.shrink(canvas::MARGIN / 2.0);
    let laid: Vec<(PointerLabel, canvas::Label)> = labels
        .into_iter()
        .map(|label| {
            let width = label.max_width.min(room.width());
            let chip = canvas::Label::new(
                ui.painter(),
                &label.text,
                label.font.clone(),
                label.color,
                width,
            );
            (label, chip)
        })
        .collect();
    let height: f32 = laid
        .iter()
        .map(|(_, chip)| chip.size().y + STACK_GAP)
        .sum::<f32>()
        - STACK_GAP;
    let width = laid
        .iter()
        .map(|(_, chip)| chip.size().x)
        .fold(0.0, f32::max);
    let below = pointer.y + POINTER_OFFSET.y;
    let top = if below + height <= room.bottom() {
        below
    } else {
        (pointer.y - POINTER_OFFSET.y - height).max(room.top())
    };
    let right_side = pointer.x + POINTER_OFFSET.x;
    let left = if right_side + width <= room.right() {
        right_side
    } else {
        (pointer.x - POINTER_OFFSET.x - width).max(room.left())
    };
    let mut y = top;
    let mut placed = Vec::new();
    for (label, chip) in laid {
        let size = chip.size();
        let mut painter = ui.painter().clone();
        painter.multiply_opacity(label.opacity);
        let shown = chip.paint(&painter, pos2(left, y));
        canvas::announce(ui, shown, label.name, &label.text, label.live);
        placed.push(shown);
        y += size.y + STACK_GAP;
    }
    placed
}

pub fn first_clear(candidates: &[Rect], obstacles: &[Rect], view: Rect) -> Option<Rect> {
    candidates.iter().copied().find(|candidate| {
        view.contains_rect(*candidate) && !obstacles.iter().any(|kept| candidate.intersects(*kept))
    })
}

pub struct Prompt<'a> {
    pub text: &'a str,
    pub keys: &'a HintList,
}

pub fn show_prompt(
    ui: &Ui,
    view: Rect,
    band: Rect,
    bottom_reserved: f32,
    prompt: &Prompt<'_>,
    more_keys: &mut bool,
) -> Rect {
    let painter = ui.painter();
    let width = band.width().min(PROMPT_MAX_WIDTH);
    let title = canvas::Label::new(
        painter,
        prompt.text,
        canvas::title(),
        canvas::chrome(painter.ctx()).prompt,
        width,
    );
    let hints = (!prompt.keys.is_empty())
        .then(|| Hints::folded(painter, prompt.keys, width, Align::Center, *more_keys));
    let hints_height = hints.as_ref().map_or(0.0, |hints| hints.size().y);
    let reach = PROMPT_MARGIN + title.size().y + canvas::MARGIN / 2.0 + hints_height;
    let hints = hints.filter(|_| view.top() + reach <= view.bottom() - bottom_reserved);
    let hints_width = hints.as_ref().map_or(0.0, |hints| hints.size().x);
    let half = title.size().x.max(hints_width) / 2.0;
    let x = view
        .center()
        .x
        .min(band.right() - half)
        .max(band.left() + half);
    let shown = title.paint_at(
        painter,
        pos2(x, view.top() + PROMPT_MARGIN),
        Align2::CENTER_TOP,
    );
    let mut area = shown;
    if let Some(hints) = hints {
        let min = Align2::CENTER_TOP
            .anchor_size(pos2(x, shown.bottom() + canvas::MARGIN / 2.0), hints.size())
            .min;
        let look = hints
            .chip_rect(min)
            .map(|chip| more_keys_chip(ui, chip, more_keys))
            .unwrap_or_default();
        area = area.union(hints.paint_with(painter, min, look));
    }
    let spoken = if prompt.keys.is_empty() {
        prompt.text.to_owned()
    } else {
        format!("{}. {}", prompt.text, prompt.keys.spoken_folded(*more_keys))
    };
    canvas::announce(ui, area, "prompt", &spoken, Some(Live::Polite));
    area
}

fn more_keys_chip(ui: &Ui, chip: Rect, more_keys: &mut bool) -> ChipLook {
    let response = ui.interact(chip, ui.id().with(MORE_KEYS_ID), Sense::click());
    let expanded = *more_keys;
    let name = if expanded { FEWER_KEYS } else { MORE_KEYS };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(expanded));
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if response.clicked() {
        *more_keys = !expanded;
    }
    ChipLook {
        hovered: response.hovered(),
        pressed: response.is_pointer_button_down_on(),
        focused: response.has_focus(),
    }
}

pub struct Start {
    pub command: Command,
    pub label: &'static str,
    pub hover: String,
}

pub struct EmptyState<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub starts: &'a [Start],
}

pub fn show_empty_state(
    ui: &Ui,
    view: Rect,
    band: Rect,
    empty: &EmptyState<'_>,
) -> (Rect, Option<Command>) {
    let painter = ui.painter();
    let chrome = canvas::chrome(painter.ctx());
    let width = band.width().min(EMPTY_WIDTH);
    let padding = Vec2::splat(canvas::MARGIN);
    let inner = (width - padding.x * 2.0).max(0.0);
    let title = painter.layout(empty.title.to_owned(), canvas::title(), chrome.text, inner);
    let text = painter.layout(empty.text.to_owned(), canvas::body(), chrome.muted, inner);
    let buttons: Vec<(Vec2, &Start)> = empty
        .starts
        .iter()
        .map(|start| {
            let glyph = painter.layout_no_wrap(
                icons::command(start.command).to_owned(),
                canvas::icon(),
                chrome.text,
            );
            let label =
                painter.layout_no_wrap(start.label.to_owned(), canvas::emphasis(), chrome.text);
            let size = vec2(
                glyph.size().x + SPACE_SM + label.size().x + EMPTY_BUTTON_PADDING * 2.0,
                canvas::BUTTON_HEIGHT,
            );
            (size, start)
        })
        .collect();
    let mut rows: Vec<Vec<(Vec2, &Start)>> = Vec::new();
    for (size, start) in buttons {
        let fits = rows.last().is_some_and(|row| {
            let used: f32 = row.iter().map(|(size, _)| size.x + EMPTY_GAP).sum();
            used + size.x <= inner
        });
        match rows.last_mut() {
            Some(row) if fits => row.push((size, start)),
            _ => rows.push(vec![(size, start)]),
        }
    }
    let rows_height = rows.len() as f32 * (canvas::BUTTON_HEIGHT + EMPTY_GAP);
    let content = vec2(
        inner,
        title.size().y + EMPTY_GAP + text.size().y + EMPTY_GAP * 2.0 + rows_height - EMPTY_GAP,
    );
    let size = content + padding * 2.0;
    let half = size.x / 2.0;
    let x = view
        .center()
        .x
        .min(band.right() - half)
        .max(band.left() + half);
    let card = Align2::CENTER_TOP.anchor_size(pos2(x, view.top() + PROMPT_MARGIN), size);
    canvas::paint_backdrop(painter, card);
    let mut y = card.top() + padding.y;
    let title_height = title.size().y;
    painter.galley(pos2(card.left() + padding.x, y), title, chrome.text);
    y += title_height + EMPTY_GAP;
    let text_height = text.size().y;
    painter.galley(pos2(card.left() + padding.x, y), text, chrome.muted);
    canvas::announce(
        ui,
        card,
        "empty model",
        &format!("{} {}", empty.title, empty.text),
        None,
    );
    y += text_height + EMPTY_GAP * 2.0;
    let mut chosen = None;
    for row in rows {
        let row_width: f32 =
            row.iter().map(|(size, _)| size.x + EMPTY_GAP).sum::<f32>() - EMPTY_GAP;
        let mut x = card.center().x - row_width / 2.0;
        for (size, start) in row {
            let rect = Rect::from_min_size(pos2(x, y), size);
            let response = canvas::button(
                ui,
                rect,
                ui.id().with(("empty model", start.label)),
                icons::command(start.command),
                start.label,
            )
            .on_hover_text(&start.hover);
            if response.clicked() {
                chosen = Some(start.command);
            }
            x += size.x + EMPTY_GAP;
        }
        y += canvas::BUTTON_HEIGHT + EMPTY_GAP;
    }
    (card, chosen)
}
