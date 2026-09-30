use caditor_document::{BlendKind, Datum, describe_axis};
use egui::{Response, Ui};

use crate::{
    blend_panel, blend_tools,
    commands::{Command, CommandFrame},
    datum_tools,
    editing::{EditingCommand, SketchEditing},
    feature_tree::count,
    icons,
    model::{Action, Model},
    offers::Offers,
    pattern_tools::{self, Shape},
    selection::{Pickable, Selection},
    shell_tools,
    solid_tools::{self, Sweep},
    viewport::CHOOSE_PLANE_PROMPT,
    widgets::{self, Tone, ToolButton},
};

pub const NEW_SKETCH_LABEL: &str = "New sketch";
pub const PLANE_LABEL: &str = "Plane";
pub const AXIS_LABEL: &str = "Axis";
const NO_SKETCH_TO_SWEEP: &str = "Draw a sketch with a closed outline first";

pub struct ToolbarContext<'a> {
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
    pub offers: &'a Offers,
    pub measuring: bool,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = TOOL_GAP;
            history_buttons(ui, model, commands, actions);
            ui.separator();
            sketch_buttons(ui, model, context, commands, actions);
            ui.separator();
            solid_buttons(ui, model, context, commands, actions);
            ui.separator();
            blend_buttons(ui, model, context, commands, actions);
            shell_button(ui, model, context, commands, actions);
            ui.separator();
            pattern_buttons(ui, model, context, commands, actions);
            ui.separator();
            datum_buttons(ui, model, context, commands, actions);
            ui.separator();
            measure_button(ui, context.measuring, commands);
        });
    });
}

pub const MEASURE_LABEL: &str = "Measure";
const MEASURE_HOVER: &str =
    "Measure the selection: distances, angles, lengths, areas and the mass properties of bodies";

fn measure_button(ui: &mut Ui, measuring: bool, commands: &mut CommandFrame<'_>) {
    let response = ui
        .add(ToolButton::new(icons::command(Command::Measure), MEASURE_LABEL).selected(measuring))
        .on_hover_text(commands.with_keys(Command::Measure, MEASURE_HOVER));
    if response.clicked() {
        commands.trigger(Command::Measure);
    }
}

const TOOL_GAP: f32 = 2.0;

fn tool(ui: &mut Ui, command: Command, label: &str, enabled: bool) -> Response {
    ui.add_enabled(enabled, ToolButton::new(icons::command(command), label))
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
        widgets::pill(ui, Tone::Info, CHOOSE_PLANE_PROMPT);
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
        | Pickable::Vertex { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::Datum(_) => None,
    });
    let datum = datum_tools::selected_datum_plane(model.document(), selection);
    let face = context.offers.sketch_face;
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
    let response = tool(ui, Command::NewSketch, NEW_SKETCH_LABEL, true).on_hover_text(hover);
    if response.clicked() || invoked {
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
        .map(|source| solid_tools::with_model_axis(source, context.offers.model_axis.as_ref()));
    for sweep in Sweep::ALL {
        let command = match sweep {
            Sweep::Extrude => Command::Extrude,
            Sweep::Revolve => Command::Revolve,
        };
        let invoked = commands.invoke(command, &source.as_ref().ok_or(NO_SKETCH_TO_SWEEP));
        let response = tool(ui, command, sweep.label(), source.is_some());
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
        let response = tool(ui, command, kind.title(), source.is_ok());
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
    let plane = context.offers.datum_plane.clone();
    let invoked = commands.invoke(Command::DatumPlane, &plane);
    let response = tool(ui, Command::DatumPlane, PLANE_LABEL, plane.is_ok());
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

    let axis = context.offers.datum_axis.clone();
    let invoked = commands.invoke(Command::DatumAxis, &axis);
    let response = tool(ui, Command::DatumAxis, AXIS_LABEL, axis.is_ok());
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
    let source = &context.offers.shell;
    let invoked = commands.invoke(Command::Shell, source);
    let response = tool(ui, Command::Shell, shell_tools::TITLE, source.is_ok());
    let response = match source {
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
        && let Ok(source) = source
    {
        actions.extend(shell_tools::create_actions(
            model.document(),
            model.evaluation(),
            source,
            model.length_unit(),
        ));
    }
}

fn pattern_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let source = &context.offers.pattern;
    for shape in Shape::ALL {
        let command = match shape {
            Shape::Linear => Command::LinearPattern,
            Shape::Circular => Command::CircularPattern,
        };
        let invoked = commands.invoke(command, source);
        let response = tool(ui, command, shape.title(), source.is_ok());
        let response = match source {
            Ok(source) => {
                let body = document
                    .feature(source.body)
                    .map_or("the body", |body| body.name.as_str());
                let hover = match (shape, &source.axis) {
                    (Shape::Linear, Some(axis)) => format!(
                        "Repeat the body of {body} along {}",
                        describe_axis(document, axis)
                    ),
                    (Shape::Linear, None) => format!(
                        "Repeat the body of {body} along the X axis, or along an edge or axis \
                         you select first"
                    ),
                    (Shape::Circular, Some(axis)) => format!(
                        "Repeat the body of {body} around {}",
                        describe_axis(document, axis)
                    ),
                    (Shape::Circular, None) => format!(
                        "Repeat the body of {body} around the Z axis, or around an axis or \
                         round face you select first"
                    ),
                };
                response.on_hover_text(commands.with_keys(command, &hover))
            }
            Err(reason) => response.on_disabled_hover_text(format!(
                "{}. {reason}, then click here.",
                shape.description()
            )),
        };
        if (response.clicked() || invoked)
            && let Ok(source) = source
        {
            actions.extend(pattern_tools::create_actions(model, shape, source));
        }
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
            model.undo_label(),
            Command::Undo,
            Action::Undo,
            "Nothing to undo",
        ),
        (
            model.redo_label(),
            Command::Redo,
            Action::Redo,
            "Nothing to redo",
        ),
    ];
    for (label, command, action, idle) in buttons {
        let invoked = commands.invoke(command, &label.ok_or(idle));
        let verb = command.title();
        let response = tool(ui, command, &verb, label.is_some());
        let response = match label {
            Some(label) => {
                response.on_hover_text(commands.with_keys(command, &format!("{verb} {label}")))
            }
            None => response.on_disabled_hover_text(idle),
        };
        if response.clicked() || invoked {
            actions.push(action);
        }
    }
}
