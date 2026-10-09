use caditor_document::{
    CarriedParameters, Feature, FeaturePaste, LeftOut, PasteError, feature_parameters,
};

use crate::{
    clipboard,
    commands::{Command, CommandFrame, Pasted},
    editing::SketchEditing,
    feature_tree::count,
    model::{Action, Model, Notice},
    panels::PanelState,
};

const NOTHING_CHOSEN: &str = "Choose the features to copy in the tree first";
const IN_SKETCH: &str = "Finish the sketch first; in a sketch, paste pastes sketch geometry";
const NOTHING_TO_PASTE: &str = "Nothing was pasted: copy features in the tree first.";

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
                actions.push(Action::Inform(Notice::info(format!(
                    "Copied {}.",
                    described(targets)
                ))));
            }
            Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
        }
    }
    let pastable = match editing.active() {
        Some(_) => Err(IN_SKETCH),
        None => Ok(()),
    };
    if commands.invoke(Command::PasteFeatures, &pastable) {
        match commands.pasted() {
            Pasted::Unread => commands.ask_for_paste(Command::PasteFeatures),
            Pasted::Nothing => actions.push(Action::Inform(Notice::info(NOTHING_TO_PASTE))),
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

fn copied_text(model: &Model, targets: &[&Feature]) -> Result<String, String> {
    let features: Vec<Feature> = targets.iter().map(|feature| (*feature).clone()).collect();
    let parameters = CarriedParameters::of(
        model.document(),
        model.parameters(),
        features.iter().flat_map(feature_parameters),
    );
    caditor_file::features_clipboard_text(&features, &parameters, &clipboard::source(model))
        .map_err(|error| format!("Nothing was copied: {error}."))
}

fn paste(model: &Model, text: &str, state: &mut PanelState, actions: &mut Vec<Action>) {
    let copied = match caditor_file::read_features_clipboard(text, &clipboard::source(model)) {
        Ok(copied) => copied,
        Err(error) => {
            actions.push(Action::Inform(Notice::info(format!(
                "Nothing was pasted: {error}."
            ))));
            return;
        }
    };
    let planned =
        model
            .document()
            .paste_features(&copied.features, &copied.parameters, copied.origin);
    match planned {
        Ok(paste) => {
            let note = paste_note(&paste, &copied.notes);
            if let Some(first) = paste.pasted.first() {
                state.reveal(*first);
            }
            state.choose_all(&paste.pasted);
            actions.push(Action::Apply(paste.transaction));
            if let Some(note) = note {
                actions.push(Action::Inform(Notice::info(note)));
            }
        }
        Err(PasteError::NothingPasted { left_out }) => {
            let mut parts = vec!["Nothing was pasted.".to_owned()];
            parts.extend(left_out.iter().map(left_out_sentence));
            parts.extend(copied.notes);
            actions.push(Action::Inform(Notice::info(parts.join(" "))));
        }
        Err(error) => actions.push(Action::Inform(Notice::info(format!(
            "Nothing was pasted: {error}."
        )))),
    }
}

fn left_out_sentence(left_out: &LeftOut) -> String {
    format!(
        "“{}” was left out because {}.",
        left_out.name, left_out.reason
    )
}

fn paste_note(paste: &FeaturePaste, notes: &[String]) -> Option<String> {
    let mut parts: Vec<String> = paste.left_out.iter().map(left_out_sentence).collect();
    if paste.inlined > 0 {
        let one = paste.inlined == 1;
        parts.push(format!(
            "{} written as {}, since this model has no parameter of {} name.",
            count(paste.inlined, "value", "values"),
            if one { "a number" } else { "numbers" },
            if one { "its" } else { "their" },
        ));
    }
    parts.extend(notes.iter().cloned());
    if parts.is_empty() {
        return None;
    }
    let pasted = count(paste.pasted.len(), "feature", "features");
    Some(format!("Pasted {pasted}. {}", parts.join(" ")))
}
