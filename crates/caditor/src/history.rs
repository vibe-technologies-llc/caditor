use std::{path::PathBuf, time::SystemTime};

use caditor_file::{History, LoadError, SavedState};
use egui::Ui;
use jiff::{Timestamp, tz::TimeZone};

use crate::{
    appearance, icons,
    model::{Model, display_name},
    widgets::{self, DialogWidth, Tone},
};

const LIST_HEIGHT: f32 = 320.0;
pub const TITLE: &str = "Version history";
const DAMAGED: &str = "This version is damaged inside the file, so it cannot be restored. The \
                       other versions are not affected.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryCommand {
    Show,
    Hide,
    Restore(usize),
}

#[derive(Debug, Clone, PartialEq)]
enum Listing {
    Loading,
    Loaded(History),
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VersionHistory {
    shown: Option<(PathBuf, Listing)>,
    restoring: Option<usize>,
}

impl VersionHistory {
    pub fn is_open(&self) -> bool {
        self.shown.is_some()
    }

    pub fn open(&mut self, path: PathBuf) {
        self.shown = Some((path, Listing::Loading));
    }

    pub fn close(&mut self) {
        self.shown = None;
        self.restoring = None;
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.shown.as_ref().map(|(path, _)| path)
    }

    pub fn listed(&mut self, path: &PathBuf, result: Result<History, LoadError>) {
        if let Some((shown, listing)) = &mut self.shown
            && shown == path
        {
            *listing = match result {
                Ok(history) => Listing::Loaded(history),
                Err(error) => Listing::Failed(error.to_string()),
            };
        }
    }

    pub fn start_restoring(&mut self, index: usize) -> Option<(PathBuf, SavedState)> {
        let (path, Listing::Loaded(history)) = self.shown.as_ref()? else {
            return None;
        };
        let version = history
            .versions
            .iter()
            .find(|version| version.index == index && version.available)?;
        self.restoring = Some(index);
        Some((path.clone(), version.state.clone()))
    }

    pub fn finish_restoring(&mut self) {
        self.restoring = None;
    }
}

pub fn describe(state: &SavedState) -> String {
    format!("Saved {}", when_saved(state))
}

pub fn when_saved(state: &SavedState) -> String {
    let when = match absolute(state.saved_at, &TimeZone::system()) {
        Some(date) => format!("{} ({date})", ago(state.saved_at)),
        None => ago(state.saved_at),
    };
    match &state.label {
        Some(label) => format!("{when} after “{label}”"),
        None => when,
    }
}

fn absolute(time: SystemTime, zone: &TimeZone) -> Option<String> {
    let timestamp = Timestamp::try_from(time).ok()?;
    Some(
        timestamp
            .to_zoned(zone.clone())
            .strftime("%-d %b %Y at %H:%M")
            .to_string(),
    )
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    history: &VersionHistory,
) -> Option<HistoryCommand> {
    let (path, listing) = history.shown.as_ref()?;
    let name = display_name(Some(path));
    let response = widgets::dialog(ctx, "version-history", TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(
            format!(
                "Every save keeps the state it replaces inside “{name}”, so you can go back to \
                 it even after closing caditor. Restoring is one change that Undo reverses."
            ),
            ui,
        ));
        let mut command = None;
        match listing {
            Listing::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading the saved versions…");
                });
            }
            Listing::Failed(reason) => {
                widgets::callout(ui, Tone::Error, |ui| {
                    ui.label(format!("The versions could not be read: {reason}."));
                });
            }
            Listing::Loaded(listed) => {
                command = versions(ui, model, history, listed);
            }
        }
        command
    });
    let closed = response.should_close().then_some(HistoryCommand::Hide);
    response.inner.or(closed)
}

fn versions(
    ui: &mut Ui,
    model: &Model,
    history: &VersionHistory,
    listed: &History,
) -> Option<HistoryCommand> {
    if let Some(current) = &listed.current {
        ui.label(widgets::strong(format!(
            "Current file: {}",
            describe(current)
        )));
    }
    if model.is_dirty() {
        widgets::callout(ui, Tone::Info, |ui| {
            ui.label("Your unsaved changes stay in the undo history when you restore a version.");
        });
    }
    if listed.versions.is_empty() {
        ui.label(widgets::muted(
            "There are no earlier versions yet. Each time you save a change, the state before \
             it is kept here.",
            ui,
        ));
        return None;
    }
    let mut command = None;
    let height = widgets::list_height(ui.ctx(), LIST_HEIGHT);
    widgets::card(ui, |ui| {
        egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                for version in &listed.versions {
                    egui::Sides::new().shrink_left().wrap().show(
                        ui,
                        |ui| {
                            let muted = appearance::tokens(ui).text_muted;
                            widgets::icon_label(ui, icons::RECENT, muted);
                            ui.label(describe(&version.state));
                        },
                        |ui| {
                            if !version.available {
                                widgets::status_pill(ui, Tone::Error, "Damaged")
                                    .on_hover_text(DAMAGED);
                            } else if history.restoring == Some(version.index) {
                                ui.spinner();
                            } else if ui
                                .add_enabled(
                                    history.restoring.is_none(),
                                    widgets::button("Restore"),
                                )
                                .on_hover_text(
                                    "Bring the model back to this version; Undo reverses it",
                                )
                                .on_disabled_hover_text("Another version is being restored")
                                .clicked()
                            {
                                command = Some(HistoryCommand::Restore(version.index));
                            }
                        },
                    );
                }
            });
    });
    command
}

pub fn ago(time: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs();
    let (count, unit) = match seconds {
        0..60 => return "just now".to_owned(),
        60..3600 => (seconds / 60, "minute"),
        3600..86400 => (seconds / 3600, "hour"),
        _ => (seconds / 86400, "day"),
    };
    match count {
        1 => format!("1 {unit} ago"),
        count => format!("{count} {unit}s ago"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    #[test]
    fn versions_show_the_date_and_time_they_were_saved_in_the_given_zone() {
        let saved = UNIX_EPOCH + Duration::from_secs(1_790_000_000);

        assert_eq!(
            absolute(saved, &TimeZone::UTC).as_deref(),
            Some("21 Sep 2026 at 14:13")
        );
        assert_eq!(
            absolute(saved, &TimeZone::fixed(jiff::tz::offset(3))).as_deref(),
            Some("21 Sep 2026 at 17:13")
        );
    }
}
