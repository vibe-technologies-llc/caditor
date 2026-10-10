use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use caditor_file::{History, LoadError, Loaded, SavedState};
use egui::Ui;
use jiff::{Timestamp, tz::TimeZone};

use crate::{
    appearance, icons,
    model::{Model, display_name},
    version_preview::{self, Preview, Started},
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
    Keep { index: usize, kept: bool },
    Preview(usize),
    StopPreview,
}

#[derive(Debug, Clone, PartialEq)]
enum Listing {
    Loading,
    Loaded(History),
    Failed(String),
}

#[derive(Default)]
pub struct VersionHistory {
    shown: Option<(PathBuf, Listing)>,
    restoring: Option<usize>,
    keeping: Option<usize>,
    preview: Option<Preview>,
    preview_ticket: u64,
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
        self.keeping = None;
        self.stop_preview();
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
        let still_listed = match (&self.shown, &self.preview) {
            (Some((_, Listing::Loaded(history))), Some(preview)) => {
                history.versions.iter().any(|version| {
                    version.index == preview.index
                        && version.available
                        && version.state == preview.saved
                })
            }
            _ => false,
        };
        if !still_listed {
            self.stop_preview();
        }
    }

    pub fn start_preview(&mut self, index: usize) -> Option<(PathBuf, Started)> {
        let (path, Listing::Loaded(history)) = self.shown.as_ref()? else {
            return None;
        };
        let version = history
            .versions
            .iter()
            .find(|version| version.index == index && version.available)?;
        let path = path.clone();
        let saved = version.state.clone();
        self.stop_preview();
        self.preview_ticket = self.preview_ticket.wrapping_add(1);
        let (preview, started) = Preview::start(index, saved, self.preview_ticket);
        self.preview = Some(preview);
        Some((path, started))
    }

    pub fn stop_preview(&mut self) {
        if let Some(preview) = self.preview.take() {
            preview.stop();
        }
    }

    pub fn preview_mut(&mut self, ticket: u64) -> Option<&mut Preview> {
        self.preview
            .as_mut()
            .filter(|preview| preview.ticket == ticket)
    }

    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }

    pub fn compare_preview(&mut self, model: &Model) {
        let same_file = self.path().map(PathBuf::as_path) == model.path();
        if let Some(preview) = &mut self.preview {
            preview.compare(
                model.document(),
                same_file,
                (model.session(), model.revision()),
            );
        }
    }

    pub fn start_restoring(&mut self, index: usize) -> Option<(PathBuf, SavedState)> {
        let (path, Listing::Loaded(history)) = self.shown.as_ref()? else {
            return None;
        };
        if self.keeping.is_some() {
            return None;
        }
        let version = history
            .versions
            .iter()
            .find(|version| version.index == index && version.available)?;
        self.restoring = Some(index);
        Some((path.clone(), version.state.clone()))
    }

    pub fn start_keeping(&mut self, index: usize) -> Option<PathBuf> {
        let (path, Listing::Loaded(history)) = self.shown.as_ref()? else {
            return None;
        };
        if self.keeping.is_some() || self.restoring.is_some() {
            return None;
        }
        history
            .versions
            .iter()
            .find(|version| version.index == index && version.available)?;
        self.keeping = Some(index);
        Some(path.clone())
    }

    pub fn finish_keeping(&mut self) {
        self.keeping = None;
    }

    pub fn finish_restoring(&mut self, path: &Path, result: &Result<Loaded, LoadError>) {
        let restored = self.restoring.take();
        if let (Some(index), Err(LoadError::VersionUnavailable)) = (restored, result)
            && let Some((shown, Listing::Loaded(history))) = &mut self.shown
            && shown == path
        {
            for version in &mut history.versions {
                version.available &= version.index != index;
            }
            if self
                .preview
                .as_ref()
                .is_some_and(|preview| preview.index == index)
            {
                self.stop_preview();
            }
        }
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
    let response = widgets::dialog(ctx, "version-history", TITLE, DialogWidth::Wide, |ui| {
        ui.label(widgets::muted(
            format!(
                "Every save keeps the state it replaces inside “{name}”, so you can go back to \
                 it even after closing caditor. Restoring is one change that Undo reverses. Old \
                 versions are thinned out as they age, except the ones you keep. Preview shows \
                 what a version holds before you restore it."
            ),
            ui,
        ));
        let mut command = None;
        match listing {
            Listing::Loading => {
                ui.horizontal(|ui| {
                    widgets::spinner(ui);
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
                            let glyph = if version.kept {
                                icons::KEPT_VERSION
                            } else {
                                icons::RECENT
                            };
                            widgets::icon_label(ui, glyph, muted);
                            ui.label(describe(&version.state));
                            if version.kept {
                                ui.label(widgets::strong("Kept"));
                            }
                        },
                        |ui| {
                            let previewed = history
                                .preview
                                .as_ref()
                                .is_some_and(|preview| preview.index == version.index);
                            if !version.available {
                                widgets::status_pill(ui, Tone::Error, "Damaged")
                                    .on_hover_text(DAMAGED);
                            } else if history.restoring == Some(version.index)
                                || history.keeping == Some(version.index)
                            {
                                widgets::spinner(ui);
                            } else {
                                let idle = history.restoring.is_none() && history.keeping.is_none();
                                let (label, hover, wanted) = if previewed {
                                    (
                                        version_preview::HIDE,
                                        "Close the preview of this version",
                                        HistoryCommand::StopPreview,
                                    )
                                } else {
                                    (
                                        version_preview::SHOW,
                                        "Show what this version holds and how it differs from \
                                         the model now, without changing anything",
                                        HistoryCommand::Preview(version.index),
                                    )
                                };
                                if ui
                                    .add(widgets::button(label))
                                    .on_hover_text(hover)
                                    .clicked()
                                {
                                    command = Some(wanted);
                                }
                                if ui
                                    .add_enabled(idle, widgets::button("Restore"))
                                    .on_hover_text(
                                        "Bring the model back to this version; Undo reverses it",
                                    )
                                    .on_disabled_hover_text("Another version is being changed")
                                    .clicked()
                                {
                                    command = Some(HistoryCommand::Restore(version.index));
                                }
                                let (label, hover) = if version.kept {
                                    (
                                        "Stop keeping",
                                        "Let this version be thinned out with the others as it \
                                         ages",
                                    )
                                } else {
                                    (
                                        "Keep",
                                        "Never remove this version when old versions are thinned \
                                         out",
                                    )
                                };
                                if ui
                                    .add_enabled(idle, widgets::button(label))
                                    .on_hover_text(hover)
                                    .on_disabled_hover_text("Another version is being changed")
                                    .clicked()
                                {
                                    command = Some(HistoryCommand::Keep {
                                        index: version.index,
                                        kept: !version.kept,
                                    });
                                }
                            }
                        },
                    );
                }
            });
    });
    if let Some(preview) = &history.preview
        && let Some(chosen) = version_preview::show(ui, preview)
    {
        command = Some(chosen);
    }
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

    use caditor_file::Version;

    use super::*;

    #[test]
    fn a_version_found_damaged_when_restored_is_shown_damaged_from_then_on() {
        let path = PathBuf::from("/models/plate.caditor");
        let state = SavedState {
            saved_at: UNIX_EPOCH,
            label: None,
        };
        let versions = (0..2)
            .map(|index| Version {
                index,
                state: state.clone(),
                available: true,
                kept: false,
            })
            .collect();
        let mut history = VersionHistory::default();
        history.open(path.clone());
        history.listed(
            &path,
            Ok(History {
                current: None,
                versions,
            }),
        );

        assert!(history.start_restoring(1).is_some());
        history.finish_restoring(&path, &Err(LoadError::VersionUnavailable));

        assert_eq!(history.start_restoring(1), None);
        assert!(history.start_restoring(0).is_some());
        history.finish_restoring(&path, &Err(LoadError::Empty));
        assert!(history.start_restoring(0).is_some());
    }

    #[test]
    fn keeping_waits_for_its_result_and_refuses_damaged_or_missing_versions() {
        let path = PathBuf::from("/models/plate.caditor");
        let state = SavedState {
            saved_at: UNIX_EPOCH,
            label: None,
        };
        let versions = [true, false]
            .into_iter()
            .enumerate()
            .map(|(index, available)| Version {
                index,
                state: state.clone(),
                available,
                kept: false,
            })
            .collect();
        let mut history = VersionHistory::default();
        assert_eq!(history.start_keeping(0), None);
        history.open(path.clone());
        assert_eq!(history.start_keeping(0), None);
        history.listed(
            &path,
            Ok(History {
                current: None,
                versions,
            }),
        );

        assert_eq!(history.start_keeping(1), None);
        assert_eq!(history.start_keeping(5), None);
        assert_eq!(history.start_keeping(0), Some(path));
        assert_eq!(history.start_keeping(0), None);
        assert_eq!(history.start_restoring(0), None);
        history.finish_keeping();
        assert!(history.start_keeping(0).is_some());
        history.close();
        assert_eq!(history.start_keeping(0), None);
    }

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
