use caditor_geometry::Point3;
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Role, Sense, Stroke, Ui, Vec2,
    WidgetInfo, accesskit::Live, pos2, vec2,
};

use crate::{
    appearance::SPACE_S,
    canvas::{self, ChipLook, FEWER_KEYS, HintList, Hints, MORE_KEYS},
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
    let response = ui.interact(chip, ui.scope_id().with(MORE_KEYS_ID), Sense::click());
    let expanded = *more_keys;
    let name = if expanded { FEWER_KEYS } else { MORE_KEYS };
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
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
