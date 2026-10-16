use std::time::Duration;

use caditor_document::{CarriedParameters, Feature, FeatureId, copied_picks, feature_parameters};
use caditor_file::ClipboardError;
use egui::Ui;

use crate::{
    clipboard,
    commands::{Command, CommandFrame, Pasted},
    editing::SketchEditing,
    feature_tree::count,
    model::{Action, Model, Notice},
    panels::PanelState,
    pasting::{self, NOT_PASTING, PasteRequest, Planned, STILL_PASTING},
    widgets,
};

const NOTHING_CHOSEN: &str = "Choose the features to copy in the tree first";
const IN_SKETCH: &str = "Finish the sketch first; in a sketch, paste pastes sketch geometry";
const NOTHING_TO_PASTE: &str = "Nothing was pasted: copy features in the tree first.";
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
#[error("Nothing was copied: {0}.")]
struct CopyError(#[from] ClipboardError);

pub fn commands(
    model: &Model,
    editing: &SketchEditing,
    targets: &[&Feature],
    detail: Option<String>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let copyable = if targets.is_empty() {
        Err(NOTHING_CHOSEN)
    } else {
        Ok(())
    };
    if commands.invoke_detailed(Command::CopyFeatures, detail, &copyable) {
        match copied_text(model, targets) {
            Ok(text) => {
                commands.copy(text);
                actions.push(Action::Inform(Notice::success(format!(
                    "Copied {}.",
                    described(targets)
                ))));
            }
            Err(error) => actions.push(Action::Inform(Notice::warning(error.to_string()))),
        }
    }
    follow_finished_paste(model, state);
    let pastable = match editing.active() {
        Some(_) => Err(IN_SKETCH),
        None if model.is_pasting() => Err(STILL_PASTING),
        None => Ok(()),
    };
    if commands.invoke(Command::PasteFeatures, &pastable) {
        match commands.pasted() {
            Pasted::Unread => commands.ask_for_paste(Command::PasteFeatures),
            Pasted::Nothing => actions.push(Action::Inform(Notice::warning(NOTHING_TO_PASTE))),
            Pasted::Text(text) => paste(model, text, state, actions),
        }
    }
}

fn described(features: &[&Feature]) -> String {
    match features {
        [only] => format!("“{}”", only.name),
        _ => count(features.len(), "feature", "features"),
    }
}

fn copied_text(model: &Model, targets: &[&Feature]) -> Result<String, CopyError> {
    let features: Vec<Feature> = targets.iter().map(|feature| (*feature).clone()).collect();
    let parameters = CarriedParameters::of(
        model.document(),
        model.parameters(),
        features.iter().flat_map(feature_parameters),
    );
    Ok(caditor_file::features_clipboard_text(
        &features,
        &parameters,
        &clipboard::source(model),
    )?)
}

fn paste(model: &Model, text: &str, state: &mut PanelState, actions: &mut Vec<Action>) {
    let copied = match caditor_file::read_features_clipboard(text, &clipboard::source(model)) {
        Ok(copied) => copied,
        Err(error) => {
            actions.push(Action::Inform(Notice::warning(format!(
                "Nothing was pasted: {error}."
            ))));
            return;
        }
    };
    if copied_picks(&copied.features) > 0 {
        actions.push(Action::Paste(Box::new(PasteRequest {
            features: copied.features,
            parameters: copied.parameters,
            origin: copied.origin,
            notes: copied.notes,
        })));
        return;
    }
    let planned =
        model
            .document()
            .paste_features(&copied.features, &copied.parameters, copied.origin);
    match pasting::planned(planned, &copied.notes) {
        Planned::Paste {
            transaction,
            pasted,
            notice,
        } => {
            choose_pasted(&pasted, state);
            actions.push(Action::Apply(transaction));
            if let Some(notice) = notice {
                actions.push(Action::Inform(notice));
            }
        }
        Planned::Refused(notice) => actions.push(Action::Inform(notice)),
    }
}

fn choose_pasted(pasted: &[FeatureId], state: &mut PanelState) {
    if let Some(first) = pasted.first() {
        state.reveal(*first);
    }
    state.choose_all(pasted);
}

fn follow_finished_paste(model: &Model, state: &mut PanelState) {
    let Some(finished) = model.pasted() else {
        return;
    };
    if state.paste_shown >= finished.serial {
        return;
    }
    state.paste_shown = finished.serial;
    choose_pasted(&finished.features, state);
}

pub fn activity(
    ui: &mut Ui,
    model: &Model,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let running = model.paste_progress().ok_or(NOT_PASTING);
    let mut cancel = commands.invoke(Command::CancelPaste, &running);
    if let Ok(progress) = running {
        let hover = commands.with_keys(
            Command::CancelPaste,
            "Stop finding the faces and edges the copies pick, and paste nothing",
        );
        cancel |= widgets::progress(pasting::progress_text(progress))
            .cancel(hover)
            .show(ui);
        ui.ctx().request_repaint_after(PROGRESS_REFRESH);
    }
    if cancel {
        actions.push(Action::CancelPaste);
    }
}
