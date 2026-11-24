use std::time::Duration;

use caditor_document::{Document, Evaluation};
use egui::{Area, Frame, Id, Key, Modifiers, Order, Pos2, Vec2, vec2};

use crate::{
    icons,
    selection::{self, Pickable},
    widgets,
};

pub const HOLD_TO_LIST: Duration = Duration::from_millis(500);
pub const NOTHING_UNDER: &str = "Nothing that can be picked lies under the pointer";
pub const LIST_TITLE: &str = "Under the pointer";
pub const LIST_CAPTION: &str = "Nearest first · Shift or Ctrl: add · Esc: close";
const LIST_ID: &str = "pick-list";
const LIST_WIDTH: f32 = 340.0;
const ANCHOR_OFFSET: Vec2 = vec2(6.0, 6.0);

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub pickable: Pickable,
    pub words: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickList {
    anchor: Pos2,
    size: Option<Vec2>,
    rows: Vec<Row>,
    revision: u64,
    pointed: Option<usize>,
    focused: Option<usize>,
    focus_first: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Open,
    Chosen { pickable: Pickable, toggle: bool },
    Closed,
}

pub fn rows(
    document: &Document,
    evaluation: &Evaluation,
    pickables: Vec<Pickable>,
    whole_bodies: bool,
) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    for pickable in pickables {
        let body = pickable.body().filter(|_| whole_bodies);

        if body.is_some() && rows.iter().any(|row| row.pickable.body() == body) {
            continue;
        }

        let words = match body {
            Some(body) => selection::body_name(document, body).to_owned(),
            None => pickable.describe(document, evaluation),
        };

        rows.push(Row { pickable, words });
    }
    rows
}

impl PickList {
    pub fn new(pointer: Pos2, rows: Vec<Row>, revision: u64) -> Self {
        Self {
            anchor: pointer + ANCHOR_OFFSET,
            size: None,
            rows,
            revision,
            pointed: None,
            focused: None,
            focus_first: true,
        }
    }

    pub fn opened(&self) -> u64 {
        self.revision
    }

    pub fn highlighted(&self) -> Option<Pickable> {
        self.pointed
            .or(self.focused)
            .and_then(|index| self.rows.get(index))
            .map(|row| row.pickable)
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn show(&mut self, ctx: &egui::Context, document: &Document) -> Outcome {
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            return Outcome::Closed;
        }

        let toggle = ctx.input(|input| input.modifiers.shift || input.modifiers.command);
        let width = widgets::fitting_width(ctx, LIST_WIDTH);
        let highlighted = self.pointed.or(self.focused);
        let opening = widgets::menu_opening_down(ctx, self.anchor, self.size);
        let mut pointed = None;
        let mut focused = None;
        let mut chosen = None;
        let mut measured_height = 0.0f32;

        let shown = Area::new(Id::unique(LIST_ID))
            .order(Order::Foreground)
            .fixed_pos(opening)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_max_width(width);

                    let header_top = ui.cursor().top();

                    ui.label(widgets::strong(LIST_TITLE));
                    widgets::column_caption(ui, LIST_CAPTION);
                    ui.separator();

                    let header_height = ui.cursor().top() - header_top;
                    let (_, list_height) = widgets::measured_menu(ui, |ui| {
                        for (index, row) in self.rows.iter().enumerate() {
                            let glyph = icons::pickable(row.pickable, document);
                            let response = widgets::menu_choice(
                                ui,
                                glyph,
                                &row.words,
                                None,
                                highlighted == Some(index),
                            );

                            if self.focus_first && index == 0 {
                                response.request_focus();
                                focused = Some(0);
                            }
                            if response.hovered() {
                                pointed = Some(index);
                            }
                            if response.has_focus() {
                                focused = Some(index);
                            }
                            if response.clicked() {
                                chosen = Some(row.pickable);
                            }
                        }
                    });

                    measured_height =
                        header_height + list_height + ui.spacing().menu_margin.sum().y;
                });
            });

        self.size = Some(vec2(
            shown.response.rect.width(),
            measured_height.max(shown.response.rect.height()),
        ));
        self.focus_first = false;
        self.pointed = pointed;
        self.focused = focused;
        if let Some(pickable) = chosen {
            return Outcome::Chosen { pickable, toggle };
        }

        let pressed_outside = ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| !shown.response.rect.contains(position))
        });
        if pressed_outside {
            Outcome::Closed
        } else {
            Outcome::Open
        }
    }
}
