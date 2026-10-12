use egui::Ui;

use crate::widgets::{self, Tone};

pub fn confirmation(ui: &mut Ui, question: &str, consequence: &str) {
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label(widgets::strong(question));
        ui.label(consequence);
    });
}

pub fn confirm_footer(ui: &mut Ui, confirm: &str, keep: &str) -> Option<bool> {
    widgets::footer_split(
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
