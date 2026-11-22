use egui::Ui;

use crate::{
    editing::{EditingCommand, SketchEditing},
    hole_tools::{self, HoleStart},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    thread_tools::{self, ThreadToolError},
    widgets::{self, Tone},
};

pub const HOLE_PROMPT: &str = "Click the face of the body to drill the hole in";
pub const THREAD_PROMPT: &str = "Click the round face of a bore, shaft or boss to thread";
pub const WAITING_FOR_A_FACE: &str = "Waiting for a face";
pub const CANCEL: &str = "Cancel";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceTool {
    Hole,
    Thread,
}

impl FaceTool {
    pub fn title(self) -> &'static str {
        match self {
            Self::Hole => hole_tools::TITLE,
            Self::Thread => thread_tools::TITLE,
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Hole => HOLE_PROMPT,
            Self::Thread => THREAD_PROMPT,
        }
    }

    fn cancel_hover(self) -> String {
        format!("Start no {} (Esc)", self.title().to_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum FirstPickRefusal {
    #[error("Click a face of the body")]
    NotAFace,
    #[error(transparent)]
    Thread(ThreadToolError),
}

pub fn click(
    model: &Model,
    editing: &SketchEditing,
    tool: FaceTool,
    pickable: Pickable,
) -> Vec<Action> {
    let started = match pickable {
        Pickable::Face { .. } => started(model, editing, tool, pickable),
        _ => Err(FirstPickRefusal::NotAFace),
    };
    started.unwrap_or_else(|reason| {
        vec![Action::Inform(Notice::warning(format!(
            "{}: {reason}.",
            tool.title()
        )))]
    })
}

fn started(
    model: &Model,
    editing: &SketchEditing,
    tool: FaceTool,
    face: Pickable,
) -> Result<Vec<Action>, FirstPickRefusal> {
    let mut picked = Selection::default();

    picked.replace_with(face);
    match tool {
        FaceTool::Hole => match hole_tools::start(model, &picked, editing) {
            Ok(start @ (HoleStart::Face(_) | HoleStart::CurvedFace(_))) => {
                Ok(hole_tools::create_actions(model, start))
            }
            Ok(HoleStart::Sketch(_) | HoleStart::ChooseFace) | Err(_) => {
                Err(FirstPickRefusal::NotAFace)
            }
        },
        FaceTool::Thread => thread_tools::selected_face(model, &picked)
            .map(|source| {
                thread_tools::create_actions(model.document(), model.evaluation(), source)
            })
            .map_err(FirstPickRefusal::Thread),
    }
}

pub fn card(ui: &mut Ui, tool: FaceTool, actions: &mut Vec<Action>) {
    widgets::card(ui, |ui| {
        ui.label(widgets::section_title(tool.title()));
        widgets::callout(ui, Tone::Info, |ui| {
            ui.label(widgets::strong(WAITING_FOR_A_FACE));
            ui.label(format!("{}.", tool.prompt()));
        });
        if ui
            .add(widgets::button(CANCEL))
            .on_hover_text(tool.cancel_hover())
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::CancelFirstPick));
        }
    });
}
