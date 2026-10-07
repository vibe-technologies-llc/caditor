use caditor_document::{BlendKind, Datum, describe_axis};
use egui::{Frame, Response, Ui, Vec2};

use crate::{
    blend_panel, blend_tools, combine_tools,
    commands::{Command, CommandFrame},
    datum_tools,
    editing::{EditingCommand, SketchEditing},
    feature_tree::count,
    hole_tools, icons, mirror_tools,
    model::{Action, Model},
    move_tools,
    offers::Offers,
    pattern_tools::{self, Shape},
    ribbon, scale_tools,
    selection::{Pickable, Selection},
    shell_tools,
    solid_tools::{self, Sweep},
    split_tools,
    viewport::CHOOSE_PLANE_PROMPT,
    widgets::ToolButton,
};

pub const NEW_SKETCH_LABEL: &str = "New sketch";
pub const PLANE_LABEL: &str = "Plane";
pub const AXIS_LABEL: &str = "Axis";
pub const POINT_LABEL: &str = "Point";
pub const MEASURE_LABEL: &str = "Measure";
pub const INTERFERENCE_LABEL: &str = "Interference";
const NO_SKETCH_TO_SWEEP: &str = "Draw a sketch with a closed outline first";
const MEASURE_HOVER: &str =
    "Measure the selection: distances, angles, lengths, areas and the mass properties of bodies";
const INTERFERENCE_HOVER: &str = "Find where bodies overlap or touch, with the volume they share";
const RIBBON: &str = "main-ribbon";

pub struct ToolbarContext<'a> {
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
    pub offers: &'a Offers,
    pub measuring: bool,
    pub checking_interference: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    History,
    Sketch,
    Solid,
    Modify,
    Pattern,
    Reference,
    Inspect,
}

impl Group {
    const ALL: [Self; 7] = [
        Self::History,
        Self::Sketch,
        Self::Solid,
        Self::Modify,
        Self::Pattern,
        Self::Reference,
        Self::Inspect,
    ];

    fn caption(self) -> &'static str {
        match self {
            Self::History => "History",
            Self::Sketch => "Sketch",
            Self::Solid => "Solid",
            Self::Modify => "Modify",
            Self::Pattern => "Pattern",
            Self::Reference => "Reference",
            Self::Inspect => "Inspect",
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let frame = Frame::side_top_panel(ui.style()).inner_margin(ribbon::BAR_MARGIN);
    egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(ribbon::TOOL_GAP);
        let full = ui.available_width();
        let widths = ribbon::remembered_widths(ui, RIBBON, &Group::ALL);
        let rows = ribbon::rows(&widths, full, full);
        let captions = rows.len() == 1;
        for (index, row) in rows.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(ribbon::ROW_GAP);
            }
            ui.horizontal_top(|ui| {
                ribbon::row(ui, RIBBON, &row, |ui, group| {
                    let caption = captions.then(|| group.caption());
                    ribbon::captioned(ui, caption, |ui| {
                        ui.horizontal_top(|ui| {
                            group_buttons(ui, group, model, context, commands, actions);
                        })
                        .response
                        .rect
                        .width()
                    })
                });
            });
        }
    });
}

fn group_buttons(
    ui: &mut Ui,
    group: Group,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    match group {
        Group::History => history_buttons(ui, model, commands, actions),
        Group::Sketch => sketch_buttons(ui, model, context, commands, actions),
        Group::Solid => {
            solid_buttons(ui, model, context, commands, actions);
            hole_button(ui, model, context, commands, actions);
        }
        Group::Modify => {
            blend_buttons(ui, model, context, commands, actions);
            shell_button(ui, model, context, commands, actions);
            combine_button(ui, model, context, commands, actions);
            move_button(ui, model, context, commands, actions);
            mirror_button(ui, model, context, commands, actions);
            scale_button(ui, model, context, commands, actions);
        }
        Group::Pattern => pattern_buttons(ui, model, context, commands, actions),
        Group::Reference => datum_buttons(ui, model, context, commands, actions),
        Group::Inspect => {
            measure_button(ui, context.measuring, commands);
            interference_button(ui, context.checking_interference, commands);
        }
    }
}

fn measure_button(ui: &mut Ui, measuring: bool, commands: &mut CommandFrame<'_>) {
    let button =
        ToolButton::new(icons::command(Command::Measure), MEASURE_LABEL).selected(measuring);
    let help = Ok(commands.with_keys(Command::Measure, MEASURE_HOVER));
    if ribbon::explained(ui.add(button), MEASURE_LABEL, &help).clicked() {
        commands.trigger(Command::Measure);
    }
}

fn interference_button(ui: &mut Ui, checking: bool, commands: &mut CommandFrame<'_>) {
    let button = ToolButton::new(icons::command(Command::Interference), INTERFERENCE_LABEL)
        .selected(checking);
    let help = Ok(commands.with_keys(Command::Interference, INTERFERENCE_HOVER));
    if ribbon::explained(ui.add(button), INTERFERENCE_LABEL, &help).clicked() {
        commands.trigger(Command::Interference);
    }
}

fn tool(ui: &mut Ui, command: Command, label: &str, help: &Result<String, String>) -> Response {
    let button = ToolButton::new(icons::command(command), label);
    ribbon::explained(ui.add_enabled(help.is_ok(), button), label, help)
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
        let button =
            ToolButton::new(icons::command(Command::NewSketch), NEW_SKETCH_LABEL).selected(true);
        let help = Ok(format!(
            "{CHOOSE_PLANE_PROMPT}, or click here to stop choosing (Esc)"
        ));
        if ribbon::explained(ui.add(button), NEW_SKETCH_LABEL, &help).clicked() {
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
    let help = Ok(commands.with_keys(Command::NewSketch, &hover));
    let invoked = commands.available(Command::NewSketch);
    let response = tool(ui, Command::NewSketch, NEW_SKETCH_LABEL, &help);
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
        let help = match &source {
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
                Ok(commands.with_keys(command, &hover))
            }
            None => Err(NO_SKETCH_TO_SWEEP.to_owned()),
        };
        let response = tool(ui, command, sweep.label(), &help);
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

fn hole_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = hole_tools::start(model, context.selection, context.editing);
    let invoked = commands.invoke(Command::Hole, &source);
    let help = match &source {
        Ok(_) => Ok(commands.with_keys(Command::Hole, hole_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            hole_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Hole, hole_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(hole_tools::create_actions(model, source));
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
        let help = match &source {
            Ok(source) => Ok(commands.with_keys(
                command,
                &format!(
                    "{} ({})",
                    blend_panel::describe_kind(kind),
                    count(source.edges.len(), "edge", "edges")
                ),
            )),
            Err(reason) => Err(format!(
                "{}. {reason}, then click here.",
                blend_panel::describe_kind(kind)
            )),
        };
        let response = tool(ui, command, kind.title(), &help);
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
    let help = match &plane {
        Ok(_) => Ok(commands.with_keys(
            Command::DatumPlane,
            "Add a plane offset from the selected plane or flat face (the XY plane when none is \
             selected) and turned about a selected axis, or through three selected points, \
             midway between two planes, or through an axis and a point",
        )),
        Err(reason) => Err(format!("{reason}.")),
    };
    let response = tool(ui, Command::DatumPlane, PLANE_LABEL, &help);
    if (response.clicked() || invoked)
        && let Ok(plane) = plane
    {
        actions.extend(datum_tools::create_actions(document, plane));
    }

    let axis = context.offers.datum_axis.clone();
    let invoked = commands.invoke(Command::DatumAxis, &axis);
    let help = match &axis {
        Ok(_) => Ok(commands.with_keys(
            Command::DatumAxis,
            "Add an axis along the selected edge, round face or axis, where the two selected \
             planes meet, through two points, or square to a plane through a point",
        )),
        Err(reason) => Err(format!("Add an axis. {reason}, then click here.")),
    };
    let response = tool(ui, Command::DatumAxis, AXIS_LABEL, &help);
    if (response.clicked() || invoked)
        && let Ok(axis) = axis
    {
        actions.extend(datum_tools::create_actions(document, Datum::Axis(axis)));
    }

    let point = context.offers.datum_point.clone();
    let invoked = commands.invoke(Command::DatumPoint, &point);
    let help = match &point {
        Ok(_) => Ok(commands.with_keys(
            Command::DatumPoint,
            "Add a point at the selected corner, round edge's centre, sketch point or datum point \
             (the origin when none is selected), moved by typed offsets",
        )),
        Err(reason) => Err(format!("Add a point. {reason}, then click here.")),
    };
    let response = tool(ui, Command::DatumPoint, POINT_LABEL, &help);
    if (response.clicked() || invoked)
        && let Ok(point) = point
    {
        actions.extend(datum_tools::create_actions(document, Datum::Point(point)));
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
    let help = match source {
        Ok(source) => Ok(commands.with_keys(
            Command::Shell,
            &format!(
                "{} ({} open)",
                shell_tools::DESCRIPTION,
                count(source.faces.len(), "face", "faces")
            ),
        )),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            shell_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Shell, shell_tools::TITLE, &help);
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

fn move_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let body = &context.offers.movement;
    let invoked = commands.invoke(Command::Move, body);
    let help = match body {
        Ok(_) => Ok(commands.with_keys(Command::Move, move_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            move_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Move, move_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(body) = body
    {
        actions.extend(move_tools::create_actions(model, *body, false));
    }

    if commands.invoke(Command::CopyBody, body)
        && let Ok(body) = body
    {
        actions.extend(move_tools::create_actions(model, *body, true));
    }
}

fn mirror_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.mirror;
    let invoked = commands.invoke(Command::Mirror, source);
    let help = match source {
        Ok(_) => Ok(commands.with_keys(Command::Mirror, mirror_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            mirror_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Mirror, mirror_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(mirror_tools::create_actions(model, source));
    }

    let split = &context.offers.split;
    if commands.invoke(Command::Split, split)
        && let Ok(split) = split
    {
        actions.extend(split_tools::create_actions(model, split));
    }
}

fn scale_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let body = &context.offers.scale;
    let invoked = commands.invoke(Command::Scale, body);
    let help = match body {
        Ok(_) => Ok(commands.with_keys(Command::Scale, scale_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            scale_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Scale, scale_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(body) = body
    {
        actions.extend(scale_tools::create_actions(model, *body));
    }
}

fn combine_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let pair = &context.offers.combine;
    let invoked = commands.invoke(Command::Combine, pair);
    let help = match pair {
        Ok(_) => Ok(commands.with_keys(Command::Combine, combine_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            combine_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, Command::Combine, combine_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(pair) = pair
    {
        actions.extend(combine_tools::create_actions(model, *pair));
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
        let help = match source {
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
                Ok(commands.with_keys(command, &hover))
            }
            Err(reason) => Err(format!(
                "{}. {reason}, then click here.",
                shape.description()
            )),
        };
        let response = tool(ui, command, shape.title(), &help);
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
        let help = match label {
            Some(label) => Ok(commands.with_keys(command, &format!("{verb} {label}"))),
            None => Err(idle.to_owned()),
        };
        let response = tool(ui, command, &verb, &help);
        if response.clicked() || invoked {
            actions.push(action);
        }
    }
}
