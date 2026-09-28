use std::time::Duration;

use caditor_document::FeatureState;
use egui::{Align, Label, Layout, RichText, TextStyle, Ui};

use crate::{
    appearance,
    feature_tree::count,
    files::{self, Files},
    icons,
    model::{Action, Model, NoticeKind, RecomputeStatus},
    panels::{Focus, PanelState},
    preferences::{Appearance, PreferenceChange, PreferencesCommand},
    selection::Selection,
    widgets::{self, Tone},
};

pub const UP_TO_DATE: &str = "Up to date";
const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(150);
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);
const NOTICE_GAP: f32 = 16.0;

pub struct StatusContext<'a> {
    pub files: &'a Files,
    pub selection: &'a Selection,
    pub appearance: &'a Appearance,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &StatusContext<'_>,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            recompute_status(ui, model, panels, actions);
            files::activity(ui, model, context.files, actions);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                interface_size(ui, context.appearance, actions);
                unit(ui, model, actions);
                selection(ui, model, context.selection);
                ui.add_space(NOTICE_GAP);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    notice(ui, model, actions);
                });
            });
        });
    });
}

fn recompute_status(
    ui: &mut Ui,
    model: &Model,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let tokens = appearance::tokens(ui);
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
            if ui.small_button("Cancel").clicked() {
                actions.push(Action::CancelRecompute);
            }
            ui.ctx().request_repaint_after(PROGRESS_REFRESH);
        }
        RecomputeStatus::Running { since } => {
            ui.ctx()
                .request_repaint_after(SHOW_PROGRESS_AFTER.saturating_sub(since.elapsed()));
            summary(ui, model, panels);
        }
        RecomputeStatus::UpToDate => summary(ui, model, panels),
        RecomputeStatus::Cancelled => {
            widgets::icon_label(ui, icons::OUTDATED, tokens.warn);
            ui.colored_label(
                tokens.warn,
                "Recompute cancelled, so some features are outdated",
            );
            if ui.small_button("Recompute").clicked() {
                actions.push(Action::Recompute);
            }
        }
        RecomputeStatus::Stopped => {
            widgets::icon_label(ui, icons::FAILED, tokens.error);
            ui.colored_label(
                tokens.error,
                "Recompute stopped unexpectedly. Your model is safe.",
            );
            if ui.small_button("Restart").clicked() {
                actions.push(Action::Recompute);
            }
        }
    }
}

fn summary(ui: &mut Ui, model: &Model, panels: &mut PanelState) {
    let tokens = appearance::tokens(ui);
    match model.evaluation().failed_count() {
        0 => {
            widgets::icon_label(ui, icons::UP_TO_DATE, tokens.success);
            ui.label(widgets::muted(UP_TO_DATE, ui));
        }
        failed => {
            let text = format!("{} failed", count(failed, "feature", "features"));
            let response = widgets::pill(ui, Tone::Error, text)
                .interact(egui::Sense::click())
                .on_hover_text("Show the first failed feature");
            if response.clicked()
                && let Some(feature) = first_failed(model)
            {
                panels.request_focus(Focus::Feature(feature));
            }
        }
    }
}

fn first_failed(model: &Model) -> Option<caditor_document::FeatureId> {
    let evaluation = model.evaluation();
    model
        .document()
        .features()
        .map(|feature| feature.id())
        .find(|id| {
            evaluation
                .feature(*id)
                .is_some_and(|status| matches!(status.state, FeatureState::Failed(_)))
        })
}

fn notice(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let Some(notice) = model.notice() else {
        return;
    };
    let tokens = appearance::tokens(ui);
    let (glyph, color) = match notice.kind {
        NoticeKind::Info => (icons::INFO, tokens.accent_text),
        NoticeKind::Error => (icons::FAILED, tokens.error),
    };
    widgets::icon_label(ui, glyph, color);
    let text = match notice.kind {
        NoticeKind::Info => RichText::new(&notice.text),
        NoticeKind::Error => RichText::new(&notice.text).color(tokens.error),
    };
    let dismiss_width = ui.spacing().interact_size.y + ui.spacing().item_spacing.x;
    ui.scope(|ui| {
        ui.set_max_width((ui.available_width() - dismiss_width).max(0.0));
        ui.add(Label::new(text).truncate());
    });
    if widgets::icon_button(ui, icons::CLOSE, "Dismiss").clicked() {
        actions.push(Action::DismissNotice);
    }
}

fn selection(ui: &mut Ui, model: &Model, selection: &Selection) {
    let described: Vec<String> = selection
        .iter()
        .map(|pickable| pickable.describe(model.document(), model.evaluation()))
        .collect();
    let text = match described.as_slice() {
        [] => "Nothing selected".to_owned(),
        [only] => only.clone(),
        many => format!("{} selected", count(many.len(), "item", "items")),
    };
    let response = ui.add(Label::new(widgets::muted(text, ui)).truncate());
    if described.len() > 1 {
        response.on_hover_text(described.join("\n"));
    }
    widgets::icon_label(ui, icons::SELECTION, appearance::tokens(ui).text_muted);
}

fn unit(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let muted = appearance::tokens(ui).text_muted;
    let response = ui
        .add(
            egui::Button::new((
                widgets::icon(icons::UNIT).color(muted),
                RichText::new(model.length_unit().symbol()).text_style(TextStyle::Body),
            ))
            .frame_when_inactive(false),
        )
        .on_hover_text(format!(
            "Lengths are shown in {}. Click to change it in Preferences.",
            model.length_unit().label().to_lowercase()
        ));
    if response.clicked() {
        actions.push(Action::Preferences(PreferencesCommand::Show));
    }
}

fn interface_size(ui: &mut Ui, appearance: &Appearance, actions: &mut Vec<Action>) {
    if (appearance.scale - 1.0).abs() < f32::EPSILON {
        return;
    }
    let text = format!("{:.0}%", appearance.scale * 100.0);
    let response = ui
        .add(egui::Button::new(widgets::muted(text, ui)).frame_when_inactive(false))
        .on_hover_text("Interface size. Click to go back to normal size.");
    if response.clicked() {
        actions.push(Action::Preferences(PreferencesCommand::Change(
            PreferenceChange::Scale(1.0),
        )));
    }
}
