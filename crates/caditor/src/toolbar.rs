use std::time::Duration;

use egui::{Button, Color32, Key, KeyboardShortcut, Modifiers, Ui};

use crate::{
    blend_panel, blend_tools,
    editing::{self, EditingCommand, SketchEditing},
    feature_tree::count,
    files::{self, Files},
    model::{Action, Model, NoticeKind, RecomputeStatus},
    selection::{Pickable, Selection},
    sketch_placement,
    solid_tools::{self, Sweep},
    viewport::CHOOSE_PLANE_PROMPT,
};

const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(150);
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const REDO_ALTERNATIVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const PROMPT_COLOR: Color32 = Color32::from_rgb(255, 214, 120);
const NEW_SKETCH_LABEL: &str = "New sketch";

pub struct ToolbarContext<'a> {
    pub files: &'a Files,
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
}

pub fn show(ui: &mut Ui, model: &Model, context: &ToolbarContext<'_>, actions: &mut Vec<Action>) {
    let files = context.files;
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            files::menu(ui, model, files, actions);
            ui.separator();
            history_buttons(ui, model, actions);
            ui.separator();
            sketch_buttons(ui, model, context.selection, context.editing, actions);
            solid_buttons(ui, model, context, actions);
            blend_buttons(ui, model, context, actions);
            ui.separator();
            recompute_status(ui, model, actions);
            if let Some(notice) = model.notice() {
                ui.separator();
                let color = match notice.kind {
                    NoticeKind::Info => ui.visuals().text_color(),
                    NoticeKind::Error => ui.visuals().error_fg_color,
                };
                ui.colored_label(color, &notice.text);
                if ui.small_button("🗙").on_hover_text("Dismiss").clicked() {
                    actions.push(Action::DismissNotice);
                }
            }
        });
    });
    if !files.is_blocking() {
        shortcuts(ui, actions);
    }
}

fn sketch_buttons(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
    actions: &mut Vec<Action>,
) {
    if editing.is_choosing_plane() {
        ui.colored_label(PROMPT_COLOR, CHOOSE_PLANE_PROMPT);
        if ui
            .button("Cancel")
            .on_hover_text("Stop choosing a plane (Esc)")
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        }
        return;
    }
    let plane = selection.iter().find_map(|pickable| match pickable {
        Pickable::Plane(plane) => Some(plane),
        Pickable::Origin
        | Pickable::Axis(_)
        | Pickable::SketchEntity { .. }
        | Pickable::SketchConstraint { .. }
        | Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. } => None,
    });
    let face = sketch_placement::selected_face(selection)
        .filter(|face| sketch_placement::is_flat(model, *face));
    let (hover, command) = match (plane, face) {
        (Some(plane), _) => (
            format!("Start a sketch on the selected {}", plane.name()),
            EditingCommand::NewSketch(Some(plane)),
        ),
        (None, Some(face)) => (
            "Start a sketch on the selected face; it follows the face when the model changes"
                .to_owned(),
            EditingCommand::NewSketchOnFace(face),
        ),
        (None, None) => (
            "Start a sketch on the plane or flat face you click next".to_owned(),
            EditingCommand::NewSketch(None),
        ),
    };
    if ui.button(NEW_SKETCH_LABEL).on_hover_text(hover).clicked() {
        actions.push(Action::Editing(command));
    }
}

fn solid_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let source = solid_tools::sweep_source(document, context.selection, context.editing);
    for sweep in Sweep::ALL {
        let text = format!("{} {}", sweep.icon(), sweep.label());
        let response = ui.add_enabled(source.is_some(), Button::new(text));
        let response = match source {
            Some(source) => {
                let sketch = document
                    .feature(source.sketch)
                    .map_or("the sketch", |feature| feature.name.as_str());
                let hover = match (sweep, source.axis) {
                    (Sweep::Extrude, _) => format!("Extrude the closed regions of {sketch}"),
                    (Sweep::Revolve, Some(axis)) => {
                        let axis = editing::edited_sketch(document, source.sketch).map_or_else(
                            || "the chosen line".to_owned(),
                            |sketch| sketch.entity_label(axis),
                        );
                        format!("Revolve the closed regions of {sketch} about {axis}")
                    }
                    (Sweep::Revolve, None) => format!(
                        "Revolve the closed regions of {sketch} about its vertical axis, or about \
                         a line you select first"
                    ),
                };
                response.on_hover_text(hover)
            }
            None => response.on_disabled_hover_text("Draw a sketch with a closed outline first"),
        };
        if response.clicked()
            && let Some(source) = source
        {
            actions.extend(solid_tools::create_actions(document, sweep, source));
        }
    }
}

fn blend_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    actions: &mut Vec<Action>,
) {
    let source = blend_tools::selected_edges(context.selection);
    for kind in blend_tools::KINDS {
        let text = format!("{} {}", blend_tools::icon(kind), kind.title());
        let response = ui.add_enabled(source.is_ok(), Button::new(text));
        let response = match &source {
            Ok(source) => response.on_hover_text(format!(
                "{} ({})",
                blend_panel::describe_kind(kind),
                count(source.edges.len(), "edge", "edges")
            )),
            Err(reason) => response.on_disabled_hover_text(format!(
                "{}. {reason}, then click here.",
                blend_panel::describe_kind(kind)
            )),
        };
        if response.clicked()
            && let Ok(source) = &source
        {
            actions.extend(blend_tools::create_actions(
                model.document(),
                model.evaluation(),
                kind,
                source,
            ));
        }
    }
}

fn history_buttons(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    let buttons = [
        (
            "⟲ Undo",
            model.undo_label(),
            UNDO,
            Action::Undo,
            "Nothing to undo",
        ),
        (
            "⟳ Redo",
            model.redo_label(),
            REDO,
            Action::Redo,
            "Nothing to redo",
        ),
    ];
    for (text, label, shortcut, action, idle) in buttons {
        let response = ui.add_enabled(label.is_some(), Button::new(text));
        let response = match label {
            Some(label) => {
                let verb = text.trim_start_matches(|character: char| !character.is_alphabetic());
                let keys = ui.ctx().format_shortcut(&shortcut);
                response.on_hover_text(format!("{verb} {label} ({keys})"))
            }
            None => response.on_disabled_hover_text(idle),
        };
        if response.clicked() {
            actions.push(action);
        }
    }
}

fn recompute_status(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>) {
    match model.status() {
        RecomputeStatus::Running { since } if since.elapsed() >= SHOW_PROGRESS_AFTER => {
            ui.spinner();
            let text = match model.progress() {
                Some(progress) if progress.total > 0 => format!(
                    "Recomputing {} of {}…",
                    (progress.done + 1).min(progress.total),
                    progress.total
                ),
                Some(_) | None => "Recomputing…".to_owned(),
            };
            ui.label(text);
            if ui.button("Cancel").clicked() {
                actions.push(Action::CancelRecompute);
            }
            ui.ctx().request_repaint_after(PROGRESS_REFRESH);
        }
        RecomputeStatus::Running { since } => {
            ui.ctx()
                .request_repaint_after(SHOW_PROGRESS_AFTER.saturating_sub(since.elapsed()));
            summary(ui, model);
        }
        RecomputeStatus::UpToDate => summary(ui, model),
        RecomputeStatus::Cancelled => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Recompute cancelled, so some features are outdated",
            );
            if ui.button("Recompute").clicked() {
                actions.push(Action::Recompute);
            }
        }
        RecomputeStatus::Stopped => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Recompute stopped unexpectedly. Your model is safe.",
            );
            if ui.button("Restart").clicked() {
                actions.push(Action::Recompute);
            }
        }
    }
}

fn summary(ui: &mut Ui, model: &Model) {
    match model.evaluation().failed_count() {
        0 => {
            ui.weak("Up to date");
        }
        failed => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("⚑ {} failed", count(failed, "feature", "features")),
            );
        }
    }
}

fn shortcuts(ui: &mut Ui, actions: &mut Vec<Action>) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    files::shortcuts(ui, actions);
    let (redo, undo) = ui.input_mut(|input| {
        let redo = input.consume_shortcut(&REDO) || input.consume_shortcut(&REDO_ALTERNATIVE);
        (redo, input.consume_shortcut(&UNDO))
    });
    if redo {
        actions.push(Action::Redo);
    } else if undo {
        actions.push(Action::Undo);
    }
}
