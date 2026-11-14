use crate::{
    appearance::{self, SPACE_M, SPACE_S},
    guide::Page,
    history::ago,
    icons,
    model::Model,
    widgets::{self, DialogWidth},
};

pub const TITLE: &str = "Recent messages";
pub const EMPTY: &str = "Nothing has been reported yet.";
pub const COPY_ALL: &str = "Copy all";
const COPY_ALL_HOVER: &str =
    "Copy every message with its kind and age, newest first, to paste into a report";
const LIST_HEIGHT: widgets::ListHeight = widgets::ListHeight::Tall;

pub fn copied(model: &Model) -> String {
    model
        .recorded_notices()
        .map(|recorded| {
            format!(
                "{}: {} ({})",
                recorded.notice.kind.label(),
                recorded.notice.text,
                ago(recorded.at)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn dialog(ctx: &egui::Context, model: &Model) -> bool {
    let response = widgets::helped_dialog(
        ctx,
        "messages",
        TITLE,
        DialogWidth::Medium,
        Page::Window,
        |ui| {
            ui.label(widgets::muted(
                "What caditor told you in the status bar, newest first, so a failed save is still \
             here after a later message replaced it.",
                ui,
            ));
            ui.add_space(SPACE_M);
            let tokens = appearance::tokens(ui);
            let mut any = false;
            widgets::DialogBody::default()
                .list(ui.ctx(), LIST_HEIGHT)
                .show(ui, |ui| {
                    for recorded in model.recorded_notices() {
                        any = true;
                        let kind = recorded.notice.kind;
                        let tone = kind.tone();
                        ui.horizontal_top(|ui| {
                            widgets::described_icon(
                                ui,
                                tone.icon(),
                                tone.color(&tokens),
                                kind.label(),
                            );
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
            widgets::footer(ui, |ui| {
                let button = widgets::small_button(ui, icons::COPY, COPY_ALL);
                let copy = ui
                    .add_enabled(any, button)
                    .on_hover_text(COPY_ALL_HOVER)
                    .on_disabled_hover_text(EMPTY);
                if copy.clicked() {
                    ui.ctx().copy_text(copied(model));
                }
            });
        },
    );
    response.should_close()
}
