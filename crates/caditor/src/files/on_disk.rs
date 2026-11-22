use std::path::{Path, PathBuf};

use caditor_file::{DiskState, FileDigest, ReadFailure};
use egui::{Align2, Area, Id, Label, Order, Rect, vec2};

use crate::{
    model::display_name,
    widgets::{self, Tone},
};

pub const RELOAD: &str = "Reload";
pub const DISMISS: &str = "Dismiss";
const WIDTH: f32 = 460.0;
const TOP_CLEARANCE: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskChoice {
    Reload,
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadChoice {
    SaveCopy,
    Discard,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outside {
    Changed { head: Option<FileDigest> },
    Gone,
    Unreadable(ReadFailure),
}

impl Outside {
    pub fn of(state: DiskState) -> Option<Self> {
        match state {
            DiskState::AsLastSeen => None,
            DiskState::Changed { head } => Some(Self::Changed { head }),
            DiskState::Gone => Some(Self::Gone),
            DiskState::Unreadable(failure) => Some(Self::Unreadable(failure)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutsideChange {
    pub path: PathBuf,
    pub session: u64,
    pub found: Outside,
}

impl OutsideChange {
    pub fn reloadable(&self) -> bool {
        matches!(self.found, Outside::Changed { .. })
    }

    pub fn text(&self) -> String {
        let name = display_name(Some(&self.path));
        match &self.found {
            Outside::Changed { .. } => format!(
                "“{name}” was changed by another program, such as a sync client or caditor on \
                 another computer. Reload to open that version here, or keep editing: saving \
                 asks before replacing it."
            ),
            Outside::Gone => format!(
                "“{name}” is no longer on disk: another program moved or deleted it. The model \
                 stays open here, and saving writes the file again."
            ),
            Outside::Unreadable(failure) => format!(
                "caditor can no longer read “{name}”: {failure}. The model stays open here; save \
                 it once the file can be reached again, or save it under another name."
            ),
        }
    }
}

#[derive(Debug, Default)]
pub struct DiskWatch {
    checking: bool,
    shown: Option<OutsideChange>,
    dismissed: Option<OutsideChange>,
    asking: bool,
}

impl DiskWatch {
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    pub fn start_check(&mut self) {
        self.checking = true;
    }

    pub fn check_stopped(&mut self) {
        self.checking = false;
    }

    pub fn compared(&mut self, path: PathBuf, session: u64, found: Option<Outside>) {
        self.checking = false;

        let Some(found) = found else {
            self.forget();
            return;
        };
        let change = OutsideChange {
            path,
            session,
            found,
        };

        if self.dismissed.as_ref() == Some(&change) || self.shown.as_ref() == Some(&change) {
            return;
        }
        self.dismissed = None;
        self.shown = Some(change);
    }

    pub fn shown(&self) -> Option<&OutsideChange> {
        self.shown.as_ref().filter(|_| !self.asking)
    }

    pub fn reloadable(&self) -> Option<&Path> {
        self.shown
            .as_ref()
            .filter(|change| change.reloadable())
            .map(|change| change.path.as_path())
    }

    pub fn dismiss(&mut self) {
        self.asking = false;
        self.dismissed = self.shown.take();
    }

    pub fn ask(&mut self) {
        self.asking = self.reloadable().is_some();
    }

    pub fn asking(&self) -> Option<&Path> {
        self.reloadable().filter(|_| self.asking)
    }

    pub fn stop_asking(&mut self) {
        self.asking = false;
    }

    pub fn forget(&mut self) {
        self.shown = None;
        self.dismissed = None;
        self.asking = false;
    }

    pub fn forget_unless(&mut self, path: Option<&Path>, session: u64) -> bool {
        let about = |change: &OutsideChange| {
            path == Some(change.path.as_path()) && change.session == session
        };
        let stale = self.shown.as_ref().is_some_and(|change| !about(change));

        if stale {
            self.shown = None;
            self.asking = false;
        }
        if self.dismissed.as_ref().is_some_and(|change| !about(change)) {
            self.dismissed = None;
        }
        stale
    }
}

pub fn show(ctx: &egui::Context, viewport: Rect, change: &OutsideChange) -> Option<DiskChoice> {
    let anchor = viewport.center_top() + vec2(0.0, TOP_CLEARANCE);
    Area::new(Id::unique("outside-change-notice"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            widgets::dialog_frame(ctx)
                .show(ui, |ui| {
                    ui.set_width(widgets::fitting_width(ctx, WIDTH));
                    widgets::callout(ui, Tone::Warning, |ui| {
                        let told = ui.add(Label::new(change.text()).wrap());

                        widgets::announced(ui, &told, false);
                        ui.horizontal_wrapped(|ui| {
                            if change.reloadable()
                                && ui
                                    .add(widgets::primary_button(ui, RELOAD))
                                    .on_hover_text(
                                        "Open the file as it is on disk now, in place of the \
                                         model shown here",
                                    )
                                    .clicked()
                            {
                                return Some(DiskChoice::Reload);
                            }
                            ui.add(widgets::button(DISMISS))
                                .on_hover_text("Keep editing the model shown here")
                                .clicked()
                                .then_some(DiskChoice::Dismiss)
                        })
                        .inner
                    })
                })
                .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(found: Outside) -> OutsideChange {
        OutsideChange {
            path: PathBuf::from("plate.caditor"),
            session: 1,
            found,
        }
    }

    #[test]
    fn a_dismissed_state_is_not_shown_again_until_the_file_changes_again() {
        let mut watch = DiskWatch::default();
        let first = Outside::Changed { head: None };

        watch.compared(PathBuf::from("plate.caditor"), 1, Some(first.clone()));

        assert_eq!(watch.shown(), Some(&change(first.clone())));

        watch.dismiss();
        watch.compared(PathBuf::from("plate.caditor"), 1, Some(first.clone()));

        assert_eq!(watch.shown(), None);

        watch.compared(PathBuf::from("plate.caditor"), 1, Some(Outside::Gone));

        assert_eq!(watch.shown(), Some(&change(Outside::Gone)));
        assert_eq!(watch.reloadable(), None);

        watch.dismiss();
        watch.compared(PathBuf::from("plate.caditor"), 1, None);
        watch.compared(PathBuf::from("plate.caditor"), 1, Some(Outside::Gone));

        assert_eq!(watch.shown(), Some(&change(Outside::Gone)));
    }

    #[test]
    fn a_notice_about_another_model_is_dropped() {
        let mut watch = DiskWatch::default();

        watch.compared(
            PathBuf::from("plate.caditor"),
            1,
            Some(Outside::Changed { head: None }),
        );
        watch.ask();

        assert_eq!(watch.asking(), Some(Path::new("plate.caditor")));
        assert!(!watch.forget_unless(Some(Path::new("plate.caditor")), 1));
        assert!(watch.forget_unless(Some(Path::new("plate.caditor")), 2));
        assert_eq!(watch.shown(), None);
        assert_eq!(watch.asking(), None);
    }
}
