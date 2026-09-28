use std::{path::PathBuf, time::SystemTime};

use caditor_file::{History, LoadError, SavedState};
use egui::{Id, Modal, RichText, Ui};

use crate::model::{Model, display_name};

const DIALOG_WIDTH: f32 = 460.0;
const LIST_HEIGHT: f32 = 320.0;

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
    match &state.label {
        Some(label) => format!("Saved {} after “{label}”", ago(state.saved_at)),
        None => format!("Saved {}", ago(state.saved_at)),
    }
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    history: &VersionHistory,
) -> Option<HistoryCommand> {
    let (path, listing) = history.shown.as_ref()?;
    let response = Modal::new(Id::new("version-history")).show(ctx, |ui| {
        ui.set_max_width(DIALOG_WIDTH);
        ui.heading(format!("Versions of “{}”", display_name(Some(path))));
        ui.label(
            "Every save keeps the state it replaces inside the file, so you can go back to it \
             even after closing caditor. Restoring is one change that Undo reverses.",
        );
        ui.add_space(6.0);
        let mut command = None;
        match listing {
            Listing::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading the saved versions…");
                });
            }
            Listing::Failed(reason) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("The versions could not be read: {reason}."),
                );
            }
            Listing::Loaded(listed) => {
                command = versions(ui, model, history, listed);
            }
        }
        ui.add_space(8.0);
        if ui.button("Close").clicked() {
            command = Some(HistoryCommand::Hide);
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
        ui.label(RichText::new(format!("Current file: {}", describe(current))).strong());
    }
    if model.is_dirty() {
        ui.weak("Your unsaved changes stay in the undo history when you restore a version.");
    }
    if listed.versions.is_empty() {
        ui.weak(
            "There are no earlier versions yet. Each time you save a change, the state before \
             it is kept here.",
        );
        return None;
    }
    let mut command = None;
    egui::ScrollArea::vertical()
        .max_height(LIST_HEIGHT)
        .show(ui, |ui| {
            for version in &listed.versions {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(describe(&version.state));
                    if !version.available {
                        ui.weak("Damaged, cannot be restored");
                    } else if history.restoring == Some(version.index) {
                        ui.spinner();
                    } else if ui
                        .add_enabled(history.restoring.is_none(), egui::Button::new("Restore"))
                        .clicked()
                    {
                        command = Some(HistoryCommand::Restore(version.index));
                    }
                });
            }
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
