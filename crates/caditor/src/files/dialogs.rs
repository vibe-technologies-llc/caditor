use std::{
    path::Path,
    time::{Duration, Instant},
};

use caditor_file::Recovered;
use egui::{Sides, Ui};

use super::{
    ExportWait, FileCommand, Files, GuardChoice, Intent, Origin, OutsideChangeChoice, Replacement,
    Report,
};
use crate::{
    appearance::{self, SPACE_S},
    commands::Command,
    history,
    model::{Model, display_name},
    widgets::{self, DialogBody, DialogWidth, Tone},
};

const QUIT_ANYWAY_AFTER: Duration = Duration::from_secs(5);
const REPORT_HEIGHT: widgets::ListHeight = widgets::ListHeight::Medium;
const REPLACED_HEIGHT: widgets::ListHeight = widgets::ListHeight::Short;
const RESTORE_SUPPRESSED_HINT: &str = "Restore with every feature suppressed, for a model that \
                                       made caditor stop. Unsuppress features one at a time, or \
                                       Undo to bring them all back.";

pub(super) fn opening(ctx: &egui::Context, path: &Path, origin: Origin) -> Option<FileCommand> {
    let name = display_name(Some(path));
    let (title, doing, cancel) = match origin {
        Origin::File => (
            format!("Opening “{name}”…"),
            "Reading the model and checking it for unsaved work…",
            "Stop opening it and keep the current model",
        ),
        Origin::Template | Origin::DefaultTemplate => (
            format!("Starting from “{name}”…"),
            "Reading the template…",
            "Stop reading the template and keep the current model",
        ),
    };
    let response = widgets::dialog(ctx, "opening", &title, DialogWidth::Medium, |ui| {
        widgets::dialog_body(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.label(widgets::muted(doing, ui));
            });
        });
        widgets::footer(ui, |ui| {
            ui.add(widgets::button("Cancel"))
                .on_hover_text(cancel)
                .clicked()
        })
    });
    (response.inner || response.should_close()).then_some(FileCommand::CancelOpen)
}

pub(super) fn guard(ctx: &egui::Context, model: &Model, intent: &Intent) -> Option<GuardChoice> {
    let name = model.display_name();
    let (consequence, discard) = match intent {
        Intent::Quit => (
            "If you close without saving, your changes will be lost.",
            "Close without saving",
        ),
        Intent::New
        | Intent::Empty
        | Intent::Template(_)
        | Intent::Sample(_)
        | Intent::Open(_)
        | Intent::Restore { .. }
        | Intent::Replace(_) => (
            "If you continue without saving, your changes will be lost.",
            "Continue without saving",
        ),
    };
    let save = if model.path().is_some() {
        Command::Save.title()
    } else {
        Command::SaveAs.title()
    };
    let title = format!("Save changes to “{name}”?");
    let response = widgets::dialog(ctx, "unsaved-changes", &title, DialogWidth::Medium, |ui| {
        widgets::dialog_body(ui, |ui| ui.label(consequence));
        widgets::footer_split(
            ui,
            |ui| {
                ui.add(widgets::danger_button(discard))
                    .clicked()
                    .then_some(GuardChoice::DontSave)
            },
            |ui| {
                if ui.add(widgets::primary_button(ui, save)).clicked() {
                    return Some(GuardChoice::Save);
                }
                ui.add(widgets::button("Cancel"))
                    .clicked()
                    .then_some(GuardChoice::Cancel)
            },
        )
    });
    let closed = response.should_close().then_some(GuardChoice::Cancel);
    response.inner.or(closed)
}

pub(super) fn closing(ctx: &egui::Context, since: Instant) -> Option<FileCommand> {
    let waited = since.elapsed();
    if waited < QUIT_ANYWAY_AFTER {
        ctx.request_repaint_after(QUIT_ANYWAY_AFTER.saturating_sub(waited));
    }
    let slow = waited >= QUIT_ANYWAY_AFTER;
    widgets::waiting_dialog(ctx, "closing", "Closing…", |ui| {
        widgets::dialog_body(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.label(widgets::muted("Finishing writing to disk…", ui));
            });
            if slow {
                ui.label("This is taking longer than usual, for example because the disk is slow.");
            }
        });
        if !slow {
            return None;
        }
        widgets::footer_split(
            ui,
            |ui| {
                ui.add(widgets::danger_button("Quit anyway"))
                    .on_hover_text(
                        "The changes you chose not to save may be offered for recovery the next \
                         time caditor starts.",
                    )
                    .clicked()
                    .then_some(FileCommand::QuitAnyway)
            },
            |_| None,
        )
    })
}

pub(super) fn finishing_exports(
    ctx: &egui::Context,
    running: &[String],
    cancellable: bool,
    waiting: &ExportWait,
) -> Option<FileCommand> {
    let waited = waiting.since.elapsed();
    let uncancellable = waiting.stopping || !cancellable;
    let slow = uncancellable && waited >= QUIT_ANYWAY_AFTER;
    if uncancellable && !slow {
        ctx.request_repaint_after(QUIT_ANYWAY_AFTER.saturating_sub(waited));
    }
    let response = widgets::dialog(
        ctx,
        "closing-exports",
        "Closing…",
        DialogWidth::Medium,
        |ui| {
            widgets::dialog_body(ui, |ui| {
                for description in running {
                    ui.horizontal(|ui| {
                        widgets::spinner(ui);
                        ui.label(description);
                    });
                }
                let explanation = match (cancellable, waiting.stopping) {
                    (false, _) => "caditor closes as soon as the file is written.",
                    (true, true) => {
                        "Stopping the export. caditor closes as soon as it has stopped."
                    }
                    (true, false) => {
                        "caditor closes when the export is done. Cancelling it writes no file."
                    }
                };
                ui.label(widgets::muted(explanation, ui));
            });
            widgets::footer_split(
                ui,
                |ui| {
                    if slow {
                        ui.add(widgets::danger_button("Quit anyway"))
                            .on_hover_text(
                                "What is still being written is left unfinished and may leave a \
                             temporary file beside the place it was writing to.",
                            )
                            .clicked()
                            .then_some(FileCommand::QuitAnyway)
                    } else if uncancellable {
                        None
                    } else {
                        ui.add(widgets::danger_button("Cancel export and quit"))
                            .on_hover_text(
                                "Stop the export without writing the file, then close caditor",
                            )
                            .clicked()
                            .then_some(FileCommand::StopExportsAndQuit)
                    }
                },
                |ui| {
                    ui.add(widgets::primary_button(ui, "Don't quit"))
                        .on_hover_text("Keep caditor open and let the writing carry on")
                        .clicked()
                        .then_some(FileCommand::KeepExporting)
                },
            )
        },
    );
    let closed = response
        .should_close()
        .then_some(FileCommand::KeepExporting);
    response.inner.or(closed)
}

pub(super) fn confirm_replace(ctx: &egui::Context, replacement: &Replacement) -> Option<bool> {
    let kind = replacement.kind();
    let replaced = replacement.replaced();
    let folder = replaced
        .first()
        .and_then(|path| path.parent())
        .map(|folder| folder.display().to_string())
        .unwrap_or_default();
    let title = match replaced {
        [only] => format!("Replace “{}”?", display_name(Some(only))),
        _ => format!("Replace {} {kind}s?", replaced.len()),
    };
    let response = widgets::dialog(ctx, "replace-model", &title, DialogWidth::Medium, |ui| {
        if let [only] = replaced {
            let name = display_name(Some(only));
            widgets::dialog_body(ui, |ui| {
                ui.label(format!(
                    "A {kind} named “{name}” already exists in “{folder}”. Replacing it \
                     overwrites what it holds."
                ))
            });
        } else {
            ui.label(format!(
                "These {} {kind}s already exist in “{folder}”. Replacing them overwrites what \
                 they hold.",
                replaced.len()
            ));
            ui.add_space(SPACE_S);
            widgets::card(ui, |ui| {
                DialogBody::default()
                    .list(ui.ctx(), REPLACED_HEIGHT)
                    .show(ui, |ui| {
                        for path in replaced {
                            ui.add(egui::Label::new(display_name(Some(path))).wrap());
                        }
                    });
            });
        }
        widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, "Replace")).clicked() {
                return Some(true);
            }
            ui.add(widgets::button("Cancel")).clicked().then_some(false)
        })
    });
    let closed = response.should_close().then_some(false);
    response.inner.or(closed)
}

pub(super) fn changed_on_disk(ctx: &egui::Context, path: &Path) -> Option<OutsideChangeChoice> {
    let name = display_name(Some(path));
    let title = format!("“{name}” was changed by another program");
    let response = widgets::dialog(ctx, "changed-on-disk", &title, DialogWidth::Medium, |ui| {
        widgets::dialog_body(ui, |ui| {
            ui.label(format!(
                "Since you opened “{name}”, another program, such as a sync client or caditor on \
                 another computer, saved over it. Replacing it keeps that version in the file's \
                 Version History, where you can restore it. Saving a copy leaves the file as it \
                 is."
            ))
        });
        widgets::footer_split(
            ui,
            |ui| {
                ui.add(widgets::button("Replace"))
                    .on_hover_text("Save over the file; its current contents become a version")
                    .clicked()
                    .then_some(OutsideChangeChoice::Replace)
            },
            |ui| {
                if ui
                    .add(widgets::primary_button(ui, "Save a copy…"))
                    .clicked()
                {
                    return Some(OutsideChangeChoice::SaveCopy);
                }
                ui.add(widgets::button("Cancel"))
                    .clicked()
                    .then_some(OutsideChangeChoice::Cancel)
            },
        )
    });
    let closed = response
        .should_close()
        .then_some(OutsideChangeChoice::Cancel);
    response.inner.or(closed)
}

pub(super) fn show_report(ctx: &egui::Context, report: &Report) -> Option<FileCommand> {
    let response = widgets::dialog(
        ctx,
        "file-report",
        &report.heading,
        DialogWidth::Medium,
        |ui| {
            if let Some(intro) = report.intro {
                ui.label(intro);
            }
            widgets::card(ui, |ui| {
                DialogBody::default()
                    .list(ui.ctx(), REPORT_HEIGHT)
                    .show(ui, |ui| {
                        let tokens = appearance::tokens(ui);
                        for (index, issue) in report.issues.iter().enumerate() {
                            if index > 0 {
                                ui.add_space(SPACE_S);
                            }
                            ui.horizontal_top(|ui| {
                                widgets::icon_label(
                                    ui,
                                    Tone::Info.icon(),
                                    Tone::Info.color(&tokens),
                                );
                                ui.add(egui::Label::new(issue.as_str()).wrap());
                            });
                        }
                    });
            });
        },
    );
    response
        .should_close()
        .then_some(FileCommand::DismissReport)
}

pub(super) fn recovery(ctx: &egui::Context, files: &Files) -> Option<FileCommand> {
    let response = widgets::dialog(
        ctx,
        "recovery",
        "Recover unsaved work",
        DialogWidth::Medium,
        |ui| {
            ui.label("caditor closed before these changes were saved.");
            let mut command = None;
            widgets::dialog_body(ui, |ui| {
                for candidate in &files.recoverable {
                    widgets::card(ui, |ui| {
                        if let Some(chosen) = recovery_row(ui, files, &candidate.recovered) {
                            command = Some(chosen);
                        }
                    });
                }
            });
            widgets::footer(ui, |ui| {
                if ui
                    .add(widgets::button("Decide later"))
                    .on_hover_text(
                        "These changes will be offered again the next time caditor starts.",
                    )
                    .clicked()
                {
                    command = Some(FileCommand::HideRecovery);
                }
            });
            command
        },
    );
    let closed = response.should_close().then_some(FileCommand::HideRecovery);
    response.inner.or(closed)
}

fn recovery_row(ui: &mut Ui, files: &Files, recovered: &Recovered) -> Option<FileCommand> {
    let name = match &recovered.file {
        Some(file) => display_name(Some(file)),
        None => "Untitled model".to_owned(),
    };
    ui.label(widgets::strong(name));
    let changes = match recovered.changes() {
        1 => "1 unsaved change".to_owned(),
        count => format!("{count} unsaved changes"),
    };
    let when = recovered
        .modified
        .map(|modified| format!(", last one {}", history::ago(modified)))
        .unwrap_or_default();
    ui.label(widgets::muted(format!("{changes}{when}"), ui));
    if let Some(file) = &recovered.file {
        ui.label(widgets::muted(file.display().to_string(), ui));
    }
    for issue in &recovered.issues {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(issue);
        });
    }
    let journal = recovered.journal.clone();
    ui.add_space(SPACE_S);
    if files.confirm_discard.as_ref() == Some(&journal) {
        ui.label("Discard these changes permanently?");
        let discarding = journal.clone();
        let (discard, keep) = Sides::new().show(
            ui,
            |ui| {
                ui.add(widgets::danger_button("Discard"))
                    .clicked()
                    .then_some(FileCommand::Discard(discarding))
            },
            |ui| {
                ui.add(widgets::primary_button(ui, "Keep"))
                    .clicked()
                    .then_some(FileCommand::KeepRecovered)
            },
        );
        return discard.or(keep);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.add(widgets::primary_button(ui, "Restore")).clicked() {
            return Some(FileCommand::Restore(journal));
        }
        if ui
            .add(widgets::button("Restore suppressed"))
            .on_hover_text(RESTORE_SUPPRESSED_HINT)
            .clicked()
        {
            return Some(FileCommand::RestoreSuppressed(journal));
        }
        ui.add(widgets::button("Discard…"))
            .clicked()
            .then_some(FileCommand::AskDiscard(journal))
    })
    .inner
}
