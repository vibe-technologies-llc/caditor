use std::path::Path;

use egui::{Align2, Area, Id, Label, Order, Rect, Sides, vec2};

use crate::{appearance, icons, widgets};

pub const ON_WINDOWS: bool = cfg!(windows);
pub const TITLE: &str = "Saving on Windows";
pub const READ_MORE: &str = "Read in the guide";
pub const GOT_IT: &str = "Got it";
pub const SHOW_AGAIN: &str = "Show the reminder";
pub const DISMISS: &str = "Dismiss the reminder";
const WIDTH: f32 = 420.0;
const BOTTOM_CLEARANCE: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder<'a> {
    pub folder: Option<&'a Path>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Dismiss,
    ReadMore,
}

pub fn due<'a>(
    on_windows: bool,
    reminded: bool,
    asked: bool,
    saved_folder: Option<&'a Path>,
) -> Option<Reminder<'a>> {
    if asked {
        return Some(Reminder {
            folder: saved_folder,
        });
    }
    saved_folder
        .filter(|_| on_windows && !reminded)
        .map(|folder| Reminder {
            folder: Some(folder),
        })
}

pub fn text(reminder: Reminder<'_>) -> String {
    let place = reminder.folder.map_or_else(
        || "the folders your models are in".to_owned(),
        |folder| format!("“{}”", folder.display()),
    );
    format!(
        "Microsoft Defender scans every file caditor writes, which slows saving, the recovery \
         journal and the versions kept in {place}. You can exclude your models' folder from its \
         scanning in Windows Security › Virus & threat protection › Manage settings › Exclusions. \
         Defender then no longer checks any file there, so exclude only a folder holding your own \
         models. caditor never changes Defender's settings itself."
    )
}

pub fn show(ctx: &egui::Context, viewport: Rect, reminder: Reminder<'_>) -> Option<Choice> {
    let anchor = viewport.center_bottom() - vec2(0.0, BOTTOM_CLEARANCE);
    Area::new(Id::new("defender-reminder"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            widgets::dialog_frame(ctx)
                .show(ui, |ui| {
                    ui.set_width(widgets::fitting_width(ctx, WIDTH));
                    let closed = Sides::new()
                        .show(
                            ui,
                            |ui| {
                                let accent = appearance::tokens(ui).accent_text;
                                widgets::icon_label(ui, icons::DEFENDER, accent);
                                ui.label(widgets::section_title(TITLE));
                            },
                            |ui| widgets::icon_button(ui, icons::CLOSE, DISMISS).clicked(),
                        )
                        .1;
                    ui.add(Label::new(text(reminder)).wrap());
                    let chosen = widgets::footer_split(
                        ui,
                        |ui| {
                            ui.add(widgets::button(READ_MORE))
                                .on_hover_text(
                                    "Open the guide's page on Microsoft Defender, with each step",
                                )
                                .clicked()
                                .then_some(Choice::ReadMore)
                        },
                        |ui| {
                            ui.add(widgets::primary_button(ui, GOT_IT))
                                .on_hover_text("Preferences › General shows this again")
                                .clicked()
                                .then_some(Choice::Dismiss)
                        },
                    );
                    chosen.or(closed.then_some(Choice::Dismiss))
                })
                .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reminder_comes_once_after_a_save_on_windows_and_whenever_asked() {
        let folder = Path::new("models");
        assert_eq!(
            due(true, false, false, Some(folder)),
            Some(Reminder {
                folder: Some(folder)
            })
        );
        assert_eq!(due(true, false, false, None), None);
        assert_eq!(due(true, true, false, Some(folder)), None);
        assert_eq!(due(false, false, false, Some(folder)), None);
        assert_eq!(
            due(false, true, true, None),
            Some(Reminder { folder: None })
        );
        assert!(
            text(Reminder {
                folder: Some(folder)
            })
            .contains("“models”")
        );
    }
}
