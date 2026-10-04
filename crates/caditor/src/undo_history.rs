use egui::{Align, Layout, ScrollArea};

use crate::{
    appearance::SPACE_M,
    model::Model,
    widgets::{self, DialogWidth},
};

pub const TITLE: &str = "Undo history";
pub const NOTHING_YET: &str = "Nothing has been changed yet.";
pub const NOW: &str = "Now";
pub const UNDONE: &str = "Undone, Redo brings them back";
const LIST_HEIGHT: f32 = 360.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    Close,
    Undo(usize),
    Redo(usize),
}

pub fn dialog(ctx: &egui::Context, model: &Model) -> Option<Jump> {
    let response = widgets::dialog(ctx, "undo-history", TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(
            "Choose a change to go to the model as it was just after it.",
            ui,
        ));
        ui.add_space(SPACE_M);
        let undone: Vec<&str> = model.redo_labels().collect();
        let done: Vec<&str> = model.undo_labels().collect();
        let mut jump = None;
        ScrollArea::vertical()
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if !undone.is_empty() {
                    ui.label(widgets::muted(UNDONE, ui));
                }
                for (index, label) in undone.iter().enumerate().rev() {
                    if ui.add(widgets::button(*label)).clicked() {
                        jump = Some(Jump::Redo(index + 1));
                    }
                }
                ui.separator();
                ui.label(NOW);
                if done.is_empty() && undone.is_empty() {
                    ui.label(widgets::muted(NOTHING_YET, ui));
                }
                for (index, label) in done.iter().enumerate() {
                    if ui.add(widgets::button(*label)).clicked() && index > 0 {
                        jump = Some(Jump::Undo(index));
                    }
                }
            });
        ui.add_space(SPACE_M);
        let closed = ui
            .with_layout(Layout::top_down(Align::Min), |ui| {
                widgets::footer_split(
                    ui,
                    |_| None,
                    |ui| {
                        ui.add(widgets::primary_button(ui, "Close"))
                            .clicked()
                            .then_some(())
                    },
                )
            })
            .inner
            .is_some();
        jump.or(closed.then_some(Jump::Close))
    });
    response
        .inner
        .or(response.should_close().then_some(Jump::Close))
}
