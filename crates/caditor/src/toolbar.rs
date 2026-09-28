use std::time::Duration;

use caditor_document::{BlendKind, Datum};
use egui::{Button, Ui};

use crate::{
    blend_panel, blend_tools,
    commands::{Command, CommandFrame},
    datum_tools,
    editing::{EditingCommand, SketchEditing},
    feature_tree::count,
    files::{self, Files},
    model::{Action, Model, NoticeKind, RecomputeStatus},
    selection::{Pickable, Selection},
    shell_tools, sketch_placement,
    solid_tools::{self, Sweep},
    viewport::CHOOSE_PLANE_PROMPT,
};

const SHOW_PROGRESS_AFTER: Duration = Duration::from_millis(150);
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);
const NEW_SKETCH_LABEL: &str = "New sketch";
const PALETTE_LABEL: &str = "🔍 Commands";
const NO_SKETCH_TO_SWEEP: &str = "Draw a sketch with a closed outline first";

pub struct ToolbarContext<'a> {
    pub files: &'a Files,
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let files = context.files;
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            files::menu(ui, model, files, context.editing, commands, actions);
            help_menu(ui, commands);
            palette_button(ui, commands);
            ui.separator();
            history_buttons(ui, model, commands, actions);
            ui.separator();
            sketch_buttons(ui, model, context, commands, actions);
            solid_buttons(ui, model, context, commands, actions);
            blend_buttons(ui, model, context, commands, actions);
            shell_button(ui, model, context, commands, actions);
            ui.separator();
            datum_buttons(ui, model, context, commands, actions);
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
}

fn help_menu(ui: &mut Ui, commands: &mut CommandFrame<'_>) {
    let mut chosen = None;
    ui.menu_button("Help", |ui| {
        for command in [
            Command::Welcome,
            Command::Palette,
            Command::KeyboardShortcuts,
        ] {
            let mut button = Button::new(command.title());
            if let Some(keys) = commands.keys(command) {
                button = button.shortcut_text(keys);
            }
            if ui.add(button).clicked() {
                chosen = Some(command);
            }
        }
    });
    if let Some(command) = chosen {
        commands.trigger(command);
    }
}

fn palette_button(ui: &mut Ui, commands: &mut CommandFrame<'_>) {
    let hover = commands.with_keys(Command::Palette, "Search every command by name");
    if ui.button(PALETTE_LABEL).on_hover_text(hover).clicked() {
        commands.trigger(Command::Palette);
    }
}

fn sketch_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let (selection, editing) = (context.selection, context.editing);
    if editing.is_choosing_plane() {
        ui.colored_label(ui.visuals().warn_fg_color, CHOOSE_PLANE_PROMPT);
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
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::Datum(_) => None,
    });
    let datum = datum_tools::selected_datum_plane(model.document(), selection);
    let face = sketch_placement::selected_face(selection)
        .filter(|face| sketch_placement::is_flat(model, *face));
    let (hover, command) = match (plane, datum, face) {
        (Some(plane), _, _) => (
            format!("Start a sketch on the selected {}", plane.name()),
            EditingCommand::NewSketch(Some(plane)),
        ),
        (None, Some(datum), _) => (
            format!(
                "Start a sketch on {}; it follows the plane when the model changes",
                model
                    .document()
                    .feature(datum)
                    .map_or("the selected plane", |datum| datum.name.as_str())
            ),
            EditingCommand::NewSketchOnDatum(datum),
        ),
        (None, None, Some(face)) => (
            "Start a sketch on the selected face; it follows the face when the model changes"
                .to_owned(),
            EditingCommand::NewSketchOnFace(face),
        ),
        (None, None, None) => (
            "Start a sketch on the plane or flat face you click next".to_owned(),
            EditingCommand::NewSketch(None),
        ),
    };
    let hover = commands.with_keys(Command::NewSketch, &hover);
    let invoked = commands.available(Command::NewSketch);
    if ui.button(NEW_SKETCH_LABEL).on_hover_text(hover).clicked() || invoked {
        actions.push(Action::Editing(command));
    }
}

fn solid_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let source = solid_tools::sweep_source(document, context.selection, context.editing)
        .map(|source| solid_tools::with_model_axis(model, context.selection, source));
    for sweep in Sweep::ALL {
        let command = match sweep {
            Sweep::Extrude => Command::Extrude,
            Sweep::Revolve => Command::Revolve,
        };
        let invoked = commands.invoke(command, &source.as_ref().ok_or(NO_SKETCH_TO_SWEEP));
        let text = format!("{} {}", sweep.icon(), sweep.label());
        let response = ui.add_enabled(source.is_some(), Button::new(text));
        let response = match &source {
            Some(source) => {
                let sketch = document
                    .feature(source.sketch)
                    .map_or("the sketch", |feature| feature.name.as_str());
                let hover = match (sweep, &source.axis) {
                    (Sweep::Extrude, _) => format!("Extrude the closed regions of {sketch}"),
                    (Sweep::Revolve, Some(axis)) => {
                        let axis = solid_tools::axis_name(document, source.sketch, axis);
                        format!("Revolve the closed regions of {sketch} about {axis}")
                    }
                    (Sweep::Revolve, None) => format!(
                        "Revolve the closed regions of {sketch} about its vertical axis, or about \
                         a line or axis you select first"
                    ),
                };
                response.on_hover_text(commands.with_keys(command, &hover))
            }
            None => response.on_disabled_hover_text(NO_SKETCH_TO_SWEEP),
        };
        if (response.clicked() || invoked)
            && let Some(source) = &source
        {
            actions.extend(solid_tools::create_actions(
                document,
                sweep,
                source.clone(),
                model.length_unit(),
            ));
        }
    }
}

fn blend_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = blend_tools::selected_edges(context.selection);
    for kind in blend_tools::KINDS {
        let command = match kind {
            BlendKind::Fillet => Command::Fillet,
            BlendKind::Chamfer => Command::Chamfer,
        };
        let invoked = commands.invoke(command, &source);
        let text = format!("{} {}", blend_tools::icon(kind), kind.title());
        let response = ui.add_enabled(source.is_ok(), Button::new(text));
        let response = match &source {
            Ok(source) => response.on_hover_text(commands.with_keys(
                command,
                &format!(
                    "{} ({})",
                    blend_panel::describe_kind(kind),
                    count(source.edges.len(), "edge", "edges")
                ),
            )),
            Err(reason) => response.on_disabled_hover_text(format!(
                "{}. {reason}, then click here.",
                blend_panel::describe_kind(kind)
            )),
        };
        if (response.clicked() || invoked)
            && let Ok(source) = &source
        {
            actions.extend(blend_tools::create_actions(
                model.document(),
                model.evaluation(),
                kind,
                source,
                model.length_unit(),
            ));
        }
    }
}

fn datum_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let end = document.features().len();
    let plane = datum_tools::plane_from_selection(model, context.selection, end);
    let invoked = commands.invoke(Command::DatumPlane, &plane);
    let text = format!("{} Plane", datum_tools::PLANE_ICON);
    let response = ui.add_enabled(plane.is_ok(), Button::new(text));
    let response = match &plane {
        Ok(_) => response.on_hover_text(commands.with_keys(
            Command::DatumPlane,
            "Add a plane offset from the selected plane or flat face (the XY plane when none is \
             selected), turned about the selected axis or straight edge if there is one",
        )),
        Err(reason) => response.on_disabled_hover_text(format!("{reason}.")),
    };
    if (response.clicked() || invoked)
        && let Ok(plane) = plane
    {
        actions.extend(datum_tools::create_actions(document, Datum::Plane(plane)));
    }

    let axis = datum_tools::axis_from_selection(model, context.selection, end);
    let invoked = commands.invoke(Command::DatumAxis, &axis);
    let text = format!("{} Axis", datum_tools::AXIS_ICON);
    let response = ui.add_enabled(axis.is_ok(), Button::new(text));
    let response = match &axis {
        Ok(_) => response.on_hover_text(commands.with_keys(
            Command::DatumAxis,
            "Add an axis along the selected edge, round face or axis, or where the two selected \
             planes meet",
        )),
        Err(reason) => {
            response.on_disabled_hover_text(format!("Add an axis. {reason}, then click here."))
        }
    };
    if (response.clicked() || invoked)
        && let Ok(axis) = axis
    {
        actions.extend(datum_tools::create_actions(document, Datum::Axis(axis)));
    }
}

fn shell_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = shell_tools::selected_faces(model, context.selection);
    let invoked = commands.invoke(Command::Shell, &source);
    let text = format!("{} {}", shell_tools::ICON, shell_tools::TITLE);
    let response = ui.add_enabled(source.is_ok(), Button::new(text));
    let response = match &source {
        Ok(source) => response.on_hover_text(commands.with_keys(
            Command::Shell,
            &format!(
                "{} ({} open)",
                shell_tools::DESCRIPTION,
                count(source.faces.len(), "face", "faces")
            ),
        )),
        Err(reason) => response.on_disabled_hover_text(format!(
            "{}. {reason}, then click here.",
            shell_tools::DESCRIPTION
        )),
    };
    if (response.clicked() || invoked)
        && let Ok(source) = &source
    {
        actions.extend(shell_tools::create_actions(
            model.document(),
            model.evaluation(),
            source,
            model.length_unit(),
        ));
    }
}

fn history_buttons(
    ui: &mut Ui,
    model: &Model,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let buttons = [
        (
            "⟲ Undo",
            model.undo_label(),
            Command::Undo,
            Action::Undo,
            "Nothing to undo",
        ),
        (
            "⟳ Redo",
            model.redo_label(),
            Command::Redo,
            Action::Redo,
            "Nothing to redo",
        ),
    ];
    for (text, label, command, action, idle) in buttons {
        let invoked = commands.invoke(command, &label.ok_or(idle));
        let response = ui.add_enabled(label.is_some(), Button::new(text));
        let response = match label {
            Some(label) => {
                let verb = text.trim_start_matches(|character: char| !character.is_alphabetic());
                response.on_hover_text(commands.with_keys(command, &format!("{verb} {label}")))
            }
            None => response.on_disabled_hover_text(idle),
        };
        if response.clicked() || invoked {
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
