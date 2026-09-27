use std::time::Duration;

use egui::{Button, Key, KeyboardShortcut, Modifiers, Ui};

use crate::{
    feature_tree::count,
    model::{Action, Model, RecomputeStatus},
};

const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(150);
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const REDO_ALTERNATIVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);

pub fn show(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            history_buttons(ui, model, actions);
            ui.separator();
            recompute_status(ui, model, actions);
            if let Some(notice) = model.notice() {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, notice);
                if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                    actions.push(Action::DismissNotice);
                }
            }
        });
    });
    shortcuts(ui, actions);
}

fn history_buttons(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let buttons = [
        (
            "⟲ Undo",
            model.undo_label(),
            UNDO,
            Action::Undo,
            "Nothing to undo",
        ),
        (
            "⟳ Redo",
            model.redo_label(),
            REDO,
            Action::Redo,
            "Nothing to redo",
        ),
    ];
    for (text, label, shortcut, action, idle) in buttons {
        let response = ui.add_enabled(label.is_some(), Button::new(text));
        let response = match label {
            Some(label) => {
                let verb = text.trim_start_matches(|character: char| !character.is_alphabetic());
                let keys = ui.ctx().format_shortcut(&shortcut);
                response.on_hover_text(format!("{verb} {label} ({keys})"))
            }
            None => response.on_disabled_hover_text(idle),
        };
        if response.clicked() {
            actions.push(action);
        }
    }
}

fn recompute_status(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    match model.status() {
        RecomputeStatus::Running { since } if since.elapsed() >= SHOW_PROGRESS_AFTER => {
            ui.spinner();
            let text = match model.progress() {
                Some(progress) if progress.total > 0 => format!(
                    "Recomputing {} of {}…",
                    (progress.done + 1).min(progress.total),
                    progress.total
                ),
                Some(_) | None => "Recomputing…".to_owned(),
            };
            ui.label(text);
            if ui.button("Cancel").clicked() {
                actions.push(Action::CancelRecompute);
            }
            ui.ctx().request_repaint_after(PROGRESS_REFRESH);
        }
        RecomputeStatus::Running { since } => {
            ui.ctx()
                .request_repaint_after(SHOW_PROGRESS_AFTER.saturating_sub(since.elapsed()));
            summary(ui, model);
        }
        RecomputeStatus::UpToDate => summary(ui, model),
        RecomputeStatus::Cancelled => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Recompute cancelled, so some features are outdated",
            );
            if ui.button("Recompute").clicked() {
                actions.push(Action::Recompute);
            }
        }
        RecomputeStatus::Stopped => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Recompute stopped unexpectedly. Your model is safe.",
            );
            if ui.button("Restart").clicked() {
                actions.push(Action::Recompute);
            }
        }
    }
}

fn summary(ui: &mut Ui, model: &Model) {
    match model.evaluation().failed_count() {
        0 => {
            ui.weak("✔ Up to date");
        }
        failed => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("⚠ {} failed", count(failed, "feature", "features")),
            );
        }
    }
}

fn shortcuts(ui: &mut Ui, actions: &mut Vec<Action>) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (redo, undo) = ui.input_mut(|input| {
        let redo = input.consume_shortcut(&REDO) || input.consume_shortcut(&REDO_ALTERNATIVE);
        (redo, input.consume_shortcut(&UNDO))
    });
    if redo {
        actions.push(Action::Redo);
    } else if undo {
        actions.push(Action::Undo);
    }
}
