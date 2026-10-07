use std::time::Duration;

use caditor_document::{FeatureState, Progress};
use egui::{
    Align, CornerRadius, CursorIcon, Frame, Id, Label, Layout, Margin, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, UiBuilder, WidgetText, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, FOCUS_WIDTH, ICON_SIZE, SPACE_M, SPACE_S},
    commands::{Command, CommandFrame},
    feature_tree::count,
    files::{self, Files},
    icons,
    model::{Action, Model, Notice, NoticeKind, RecomputeStatus},
    offers::Offers,
    panels::{Focus, PanelState},
    preferences::{Appearance, PreferenceChange, PreferencesCommand},
    selection::SelectionFilter,
    widgets::{self, Named, Tone},
};

pub const UP_TO_DATE: &str = "Up to date";
pub const CANCELLED: &str = "Recompute cancelled";
pub const STOPPED: &str = "Recompute stopped";
const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(150);
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);
const SHOW_FEATURE_TIME_AFTER: Duration = Duration::from_secs(2);
const MIN_SELECTION_WIDTH: f32 = 120.0;
const DIVIDER_WIDTH: f32 = SPACE_S;
const BAR_MARGIN: Margin = Margin::symmetric(SPACE_M as i8, SPACE_S as i8);
const NO_NOTICE: &str = "There is no notice to dismiss";
const NOT_RECOMPUTING: &str = "Nothing is being recomputed";
const NOTHING_FAILED: &str = "No feature has failed";

pub struct StatusContext<'a> {
    pub files: &'a Files,
    pub offers: &'a Offers,
    pub appearance: &'a Appearance,
    pub filter: SelectionFilter,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &StatusContext<'_>,
    panels: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    recompute_commands(model, commands, actions);
    let failed = first_failed(model).ok_or(NOTHING_FAILED);
    if commands.invoke(Command::ShowFirstFailed, &failed)
        && let Ok(feature) = failed
    {
        panels.request_focus(Focus::Feature(feature));
    }
    let dismissible = model.notice().map(|_| ()).ok_or(NO_NOTICE);
    if commands.invoke(Command::DismissNotice, &dismissible) {
        actions.push(Action::DismissNotice);
    }
    let trailing_id = Id::new("status-trailing");
    let frame = Frame::side_top_panel(ui.style()).inner_margin(BAR_MARGIN);
    egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
        let notice_width = notice_width(ui, model);
        let mut trailing_wrapped = false;
        let mut notice_wrapped = false;
        ui.horizontal(|ui| {
            recompute_status(ui, model, panels, actions);
            divided(ui, |ui| {
                files::activity(ui, model, context.files, commands, actions);
            });
            let fixed = widgets::remembered_width(ui, trailing_id) + selection_icon_room(ui);
            let available = ui.available_width();
            trailing_wrapped = available < fixed + MIN_SELECTION_WIDTH;
            let inline_notice = notice_width.filter(|width| {
                !trailing_wrapped && available >= fixed + MIN_SELECTION_WIDTH + width
            });
            notice_wrapped = notice_width.is_some() && inline_notice.is_none();
            if !trailing_wrapped {
                let selection_room = available - fixed - inline_notice.unwrap_or(0.0);
                trailing(
                    ui,
                    model,
                    context,
                    trailing_id,
                    selection_room,
                    inline_notice.is_some(),
                    actions,
                );
            }
        });
        if trailing_wrapped {
            ui.horizontal(|ui| {
                let fixed = widgets::remembered_width(ui, trailing_id) + selection_icon_room(ui);
                let selection_room = ui.available_width() - fixed;
                trailing(
                    ui,
                    model,
                    context,
                    trailing_id,
                    selection_room,
                    false,
                    actions,
                );
            });
        }
        if notice_wrapped {
            ui.horizontal(|ui| notice(ui, model, actions, true));
        }
    });
}

fn trailing(
    ui: &mut Ui,
    model: &Model,
    context: &StatusContext<'_>,
    id: Id,
    selection_room: f32,
    with_notice: bool,
    actions: &mut Vec<Action>,
) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if interface_size(ui, context.appearance, actions) {
            divider(ui);
        }
        unit(ui, model, actions);
        divider(ui);
        if context.filter != SelectionFilter::Everything {
            filter(ui, context.filter, actions);
            divider(ui);
        }
        widgets::remember_width(ui, id, ui.min_rect().width());
        selection(
            ui,
            &context.offers.described,
            context.offers.selected,
            selection_room,
        );
        if with_notice {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                divider(ui);
                notice(ui, model, actions, false);
            });
        }
    });
}

fn divider(ui: &mut Ui) {
    let height = ui.text_style_height(&TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(vec2(DIVIDER_WIDTH, height), Sense::hover());
    let stroke = Stroke::new(BORDER_WIDTH, appearance::tokens(ui).border_strong);
    ui.painter().vline(rect.center().x, rect.y_range(), stroke);
}

fn divider_room(ui: &Ui) -> f32 {
    DIVIDER_WIDTH + 2.0 * ui.spacing().item_spacing.x
}

fn divided(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let spacing = ui.spacing().item_spacing.x;
    let lead = DIVIDER_WIDTH + spacing;
    let row = ui.spacing().interact_size.y;
    let origin = ui.cursor().min + vec2(lead, 0.0);
    let room = Rect::from_min_size(origin, vec2((ui.available_width() - lead).max(0.0), row));
    let mut segment = ui.new_child(
        UiBuilder::new()
            .max_rect(room)
            .layout(Layout::left_to_right(Align::Center)),
    );
    add(&mut segment);
    let drawn = segment.min_rect();
    if drawn.width() > 0.0 {
        divider(ui);
        ui.advance_cursor_after_rect(Rect::from_min_max(origin, drawn.max.max(origin)));
    }
}

fn selection_icon_room(ui: &Ui) -> f32 {
    ICON_SIZE + ui.spacing().item_spacing.x
}

fn recompute_commands(model: &Model, commands: &mut CommandFrame<'_>, actions: &mut Vec<Action>) {
    if commands.available(Command::Recompute) {
        actions.push(Action::Recompute);
    }
    let running = match model.status() {
        RecomputeStatus::Running { .. } => Ok(()),
        RecomputeStatus::UpToDate | RecomputeStatus::Cancelled | RecomputeStatus::Stopped => {
            Err(NOT_RECOMPUTING)
        }
    };
    if commands.invoke(Command::CancelRecompute, &running) {
        actions.push(Action::CancelRecompute);
    }
}

fn recomputing_text(progress: &Progress) -> String {
    let position = format!(
        "Recomputing {} of {}",
        (progress.done + 1).min(progress.total),
        progress.total
    );
    let Some(name) = &progress.running else {
        return format!("{position}…");
    };
    let running_for = progress.running_for();
    if running_for < SHOW_FEATURE_TIME_AFTER {
        return format!("{position}: {name}…");
    }
    format!("{position}: {name}, for {} s…", running_for.as_secs())
}

fn recompute_status(
    ui: &mut Ui,
    model: &Model,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    match model.status() {
        RecomputeStatus::Running { since } if since.elapsed() >= SHOW_PROGRESS_AFTER => {
            ui.spinner();
            let text = match model.progress() {
                Some(progress) if progress.total > 0 => recomputing_text(&progress),
                Some(_) | None => "Recomputing…".to_owned(),
            };
            ui.label(text);
            let cancel = ui
                .add(widgets::button("Cancel"))
                .on_hover_text("Stop recomputing; features not yet recomputed stay outdated");
            if cancel.clicked() {
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
            widgets::announced_status_pill(ui, Tone::Warning, CANCELLED)
                .on_hover_text("Some features are outdated until the model is recomputed");
            let recompute = ui
                .add(widgets::button("Recompute"))
                .on_hover_text("Bring the outdated features up to date");
            if recompute.clicked() {
                actions.push(Action::Recompute);
            }
        }
        RecomputeStatus::Stopped => {
            widgets::announced_status_pill(ui, Tone::Error, STOPPED)
                .on_hover_text("Recompute stopped unexpectedly. Your model is safe.");
            let restart = ui
                .add(widgets::button("Restart"))
                .on_hover_text("Start recomputing again. Your model is safe.");
            if restart.clicked() {
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
            if failed_pill(ui, &text).clicked()
                && let Some(feature) = first_failed(model)
            {
                panels.request_focus(Focus::Feature(feature));
            }
        }
    }
}

fn failed_pill(ui: &mut Ui, text: &str) -> Response {
    let tokens = appearance::tokens(ui);
    let pill = widgets::status_pill(ui, Tone::Error, text);
    let response = widgets::named(pill.interact(Sense::click()), text)
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Show the first failed feature");
    widgets::announced(ui, &response, true);
    let pressed = response.is_pointer_button_down_on();
    if pressed || response.hovered() {
        let width = if pressed { FOCUS_WIDTH } else { BORDER_WIDTH };
        let radius = CornerRadius::from(response.rect.height() / 2.0);
        ui.painter().rect_stroke(
            response.rect,
            radius,
            Stroke::new(width, tokens.error),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        widgets::paint_focus_ring(ui, response.rect);
    }
    response
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

fn notice_text(ui: &Ui, notice: &Notice) -> RichText {
    match notice.kind {
        NoticeKind::Info => RichText::new(&notice.text),
        NoticeKind::Error => RichText::new(&notice.text).color(appearance::tokens(ui).error),
    }
}

fn notice_width(ui: &Ui, model: &Model) -> Option<f32> {
    let notice = model.notice()?;
    let text = WidgetText::from(notice_text(ui, notice)).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let icons = 2.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
    Some(text.size().x + icons + divider_room(ui))
}

fn notice(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>, wrap: bool) {
    let Some(notice) = model.notice() else {
        return;
    };
    let tokens = appearance::tokens(ui);
    let (glyph, color) = match notice.kind {
        NoticeKind::Info => (icons::INFO, tokens.accent_text),
        NoticeKind::Error => (icons::FAILED, tokens.error),
    };
    widgets::icon_label(ui, glyph, color);
    let text = notice_text(ui, notice);
    let dismiss_id = Id::new("status-notice-dismiss");
    let dismiss = widgets::remembered_width(ui, dismiss_id).max(ui.spacing().interact_size.y);
    let dismiss_width = dismiss + ui.spacing().item_spacing.x;
    ui.scope(|ui| {
        ui.set_max_width((ui.available_width() - dismiss_width).max(0.0));
        let label = Label::new(text);
        let response = ui.add(if wrap { label.wrap() } else { label.truncate() });
        widgets::announced(ui, &response, notice.kind == NoticeKind::Error);
    });
    let dismiss = widgets::icon_button(ui, icons::CLOSE, "Dismiss");
    widgets::remember_width(ui, dismiss_id, dismiss.rect.width());
    if dismiss.clicked() {
        actions.push(Action::DismissNotice);
    }
}

fn selection(ui: &mut Ui, described: &[String], selected: usize, room: f32) {
    let tokens = appearance::tokens(ui);
    let text = match (described, selected) {
        (_, 0) => RichText::new("Nothing selected").color(tokens.text_muted),
        ([only], 1) => RichText::new(only).color(tokens.text),
        _ => RichText::new(format!("{} selected", count(selected, "item", "items")))
            .color(tokens.text),
    };
    let icon_room = selection_icon_room(ui);
    let response = ui
        .scope(|ui| {
            ui.set_max_width(room.max(MIN_SELECTION_WIDTH) - icon_room);
            ui.add(Label::new(text).truncate())
        })
        .inner;
    let more = selected.saturating_sub(described.len());
    match (described, more) {
        ([], _) => {}
        ([only], 0) => {
            response.on_hover_text(only);
        }
        (listed, 0) => {
            response.on_hover_text(listed.join("\n"));
        }
        (listed, more) => {
            response.on_hover_text(format!("{}\nand {more} more", listed.join("\n")));
        }
    }
    widgets::icon_label(ui, icons::SELECTION, appearance::tokens(ui).text_muted);
}

fn filter(ui: &mut Ui, filter: SelectionFilter, actions: &mut Vec<Action>) {
    let button =
        widgets::small_button(ui, icons::command(Command::Filter(filter)), filter.status());
    let response = ui.add(Named::new(button, filter.status())).on_hover_text(
        "Clicks and the highlight keys skip everything else. Click to select anything again.",
    );
    if response.clicked() {
        actions.push(Action::Filter(SelectionFilter::Everything));
    }
}

fn unit(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let unit = model.length_unit();
    let button = widgets::small_button(ui, icons::UNIT, unit.symbol());
    let name = format!("Length unit: {}", unit.label().to_lowercase());
    let response = ui.add(Named::new(button, name)).on_hover_text(format!(
        "Lengths are shown in {}. Click to change it in Preferences.",
        unit.label().to_lowercase()
    ));
    if response.clicked() {
        actions.push(Action::Preferences(PreferencesCommand::Show));
    }
}

fn interface_size(ui: &mut Ui, appearance: &Appearance, actions: &mut Vec<Action>) -> bool {
    if (appearance.scale - 1.0).abs() < f32::EPSILON {
        return false;
    }
    let text = format!("{:.0}%", appearance.scale * 100.0);
    let response = ui
        .add(widgets::button(text))
        .on_hover_text("Interface size. Click to go back to normal size.");
    if response.clicked() {
        actions.push(Action::Preferences(PreferencesCommand::Change(
            PreferenceChange::Scale(1.0),
        )));
    }
    true
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn progress(running: Option<&str>, started_ago: Duration) -> Progress {
        Progress {
            done: 2,
            total: 5,
            running: running.map(str::to_owned),
            since: Instant::now() - started_ago,
        }
    }

    #[test]
    fn a_recompute_names_its_feature_and_how_long_it_has_run_once_slow() {
        assert_eq!(
            recomputing_text(&progress(Some("Pocket"), Duration::ZERO)),
            "Recomputing 3 of 5: Pocket…"
        );
        assert_eq!(
            recomputing_text(&progress(Some("Pocket"), Duration::from_secs(7))),
            "Recomputing 3 of 5: Pocket, for 7 s…"
        );
        assert_eq!(
            recomputing_text(&progress(None, Duration::ZERO)),
            "Recomputing 3 of 5…"
        );
    }
}
