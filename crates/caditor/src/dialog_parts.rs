use egui::{
    Align, Button, CornerRadius, Frame, Id, Layout, Margin, Modal, RichText, Stroke, Ui, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, CONTROL_HEIGHT, DIALOG_MARGIN, SPACE_XS, WIDGET_RADIUS},
    widgets::{self, Named, Tone},
};

const SEGMENT_INSET: i8 = 2;
pub const MEDIUM_WIDTH: f32 = 460.0;
pub const WIDE_WIDTH: f32 = 600.0;
const MIN_BODY_HEIGHT: f32 = 96.0;

pub fn split_footer<T>(
    ui: &mut Ui,
    start: impl FnOnce(&mut Ui) -> Option<T>,
    end: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    widgets::footer(ui, |ui| {
        let ended = end(ui);
        let started = ui
            .with_layout(Layout::left_to_right(Align::Center), start)
            .inner;
        ended.or(started)
    })
}

pub struct BodyRoom {
    id: Id,
    start: f32,
    pub height: f32,
}

impl BodyRoom {
    pub fn measure(ui: &Ui, id: &str, screen_share: f32) -> Self {
        let id = Id::new((id, "below-body"));
        let below = ui
            .data(|data| data.get_temp::<f32>(id))
            .unwrap_or(CONTROL_HEIGHT);
        let above = ui.cursor().min.y - ui.min_rect().min.y;
        let height = ui.ctx().content_rect().height() * screen_share
            - above
            - below
            - 2.0 * f32::from(DIALOG_MARGIN);
        Self {
            id,
            start: 0.0,
            height: height.max(MIN_BODY_HEIGHT),
        }
    }

    pub fn body_ended(&mut self, ui: &Ui) {
        self.start = ui.cursor().min.y;
    }

    pub fn dialog_ended(&self, ui: &Ui) {
        let below = ui.cursor().min.y - self.start;
        ui.data_mut(|data| data.insert_temp(self.id, below));
    }
}

pub fn confirmation(ui: &mut Ui, question: &str, consequence: &str) {
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label(widgets::strong(question));
        ui.label(consequence);
    });
}

pub fn confirm_footer(ui: &mut Ui, confirm: &str, keep: &str) -> Option<bool> {
    split_footer(
        ui,
        |ui| {
            ui.add(widgets::danger_button(confirm))
                .clicked()
                .then_some(true)
        },
        |ui| {
            ui.add(widgets::primary_button(ui, keep))
                .clicked()
                .then_some(false)
        },
    )
}

pub fn undo_note(ui: &mut Ui, text: &str, hover: &str) -> bool {
    widgets::callout(ui, Tone::Success, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(text);
            ui.add(widgets::button("Undo"))
                .on_hover_text(hover)
                .clicked()
        })
        .inner
    })
}

pub fn titled_modal<T>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    add: impl FnOnce(&mut Ui) -> T,
) -> T {
    Modal::new(Id::new(id))
        .frame(widgets::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(widgets::fitting_width(ctx, MEDIUM_WIDTH));
            ui.heading(title);
            ui.add_space(ui.spacing().item_spacing.y);
            add(ui)
        })
        .inner
}

pub struct Segment<'a> {
    pub label: &'a str,
    pub hover: &'a str,
    pub refusal: Option<&'a str>,
}

pub fn segmented_offered(ui: &mut Ui, segments: &[Segment<'_>], selected: usize) -> Option<usize> {
    let tokens = appearance::tokens(ui);
    let mut chosen = None;
    Frame::new()
        .fill(tokens.button)
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(Margin::same(SEGMENT_INSET))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                let segment_height = CONTROL_HEIGHT - 2.0 * f32::from(SEGMENT_INSET);
                for (index, segment) in segments.iter().enumerate() {
                    let current = index == selected;
                    let color = if current {
                        tokens.accent_text
                    } else {
                        tokens.text
                    };
                    let mut button = Button::new(RichText::new(segment.label).color(color))
                        .min_size(vec2(0.0, segment_height))
                        .corner_radius(CornerRadius::same(WIDGET_RADIUS - SEGMENT_INSET as u8));
                    button = if current {
                        button
                            .fill(tokens.raised)
                            .stroke(Stroke::new(BORDER_WIDTH, tokens.accent_text))
                    } else {
                        button.frame_when_inactive(false)
                    };
                    let named = Named::new(button, segment.label).selected(current);
                    let response = ui
                        .add_enabled(segment.refusal.is_none(), named)
                        .on_hover_text(segment.hover);
                    let response = match segment.refusal {
                        Some(refusal) => response.on_disabled_hover_text(refusal),
                        None => response,
                    };
                    if response.clicked() && !current {
                        chosen = Some(index);
                    }
                }
            });
        });
    chosen
}
