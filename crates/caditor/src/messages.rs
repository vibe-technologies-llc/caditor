use egui::{Layout, ScrollArea};

use crate::{
    appearance::{self, SPACE_M, SPACE_S},
    history::ago,
    model::Model,
    widgets::{self, DialogWidth},
};

pub const TITLE: &str = "Recent messages";
pub const EMPTY: &str = "Nothing has been reported yet.";
const LIST_HEIGHT: f32 = 320.0;

pub fn dialog(ctx: &egui::Context, model: &Model) -> bool {
    let response = widgets::dialog(ctx, "messages", TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(
            "What caditor told you in the status bar, newest first, so a failed save is still \
             here after a later message replaced it.",
            ui,
        ));
        ui.add_space(SPACE_M);
        let tokens = appearance::tokens(ui);
        let mut any = false;
        ScrollArea::vertical()
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for recorded in model.recorded_notices() {
                    any = true;
                    let kind = recorded.notice.kind;
                    let tone = kind.tone();
                    ui.horizontal_top(|ui| {
                        widgets::described_icon(ui, tone.icon(), tone.color(&tokens), kind.label());
                        ui.vertical(|ui| {
                            ui.label(&recorded.notice.text);
                            ui.label(widgets::muted(ago(recorded.at), ui));
                        });
                    });
                    ui.add_space(SPACE_S);
                }
            });
        if !any {
            ui.label(widgets::muted(EMPTY, ui));
        }
        ui.add_space(SPACE_M);
        ui.with_layout(Layout::top_down(egui::Align::Min), |ui| {
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
        .is_some()
    });
    response.inner || response.should_close()
}
