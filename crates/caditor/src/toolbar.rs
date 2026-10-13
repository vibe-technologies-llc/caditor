use std::cell::RefCell;

use caditor_document::{BlendKind, Datum, Document, PrimitiveKind, RegionChoice, describe_axis};
use egui::{Id, Popup, Rect, Response, Sense, Ui, Vec2, vec2};

use crate::{
    analysis::{AnalysisCommand, Kind},
    blend_panel, blend_tools, combine_tools,
    commands::{Command, CommandFrame, Keymap, Offer},
    datum_tools,
    editing::{EditingCommand, SketchEditing},
    feature_tree::count,
    files::FileCommand,
    hole_tools, icons,
    last_values::Starts,
    mate_tools,
    menu_bar::{self, MenuEntries},
    mirror_tools,
    model::{Action, Model},
    move_tools,
    offers::Offers,
    offset_face_tools,
    pattern_tools::{self, Shape},
    primitive_tools, queued, ribbon, scale_tools,
    section::SectionCommand,
    selection::Selection,
    shell_tools,
    sketch_placement::SketchTarget,
    solid_tools::{self, FaceProfile, Sweep},
    split_face_tools, split_tools, sweep_tools, thread_tools,
    toggles::ToggleStates,
    viewport::CHOOSE_PLANE_PROMPT,
    widgets::{self, CornerMenu, ToolButton},
};

pub const NEW_SKETCH_LABEL: &str = "New sketch";
pub const PLANE_LABEL: &str = "Plane";
pub const AXIS_LABEL: &str = "Axis";
pub const POINT_LABEL: &str = "Point";
pub const MEASURE_LABEL: &str = "Measure";
pub const INTERFERENCE_LABEL: &str = "Interference";
const NO_SKETCH_TO_SWEEP: &str =
    "Draw a sketch with a closed outline first, or select the curves of one already swept";
const MEASURE_HOVER: &str =
    "Measure the selection: distances, angles, lengths, areas and the mass properties of bodies";
const INTERFERENCE_HOVER: &str = "Find where bodies overlap or touch, with the volume they share";
const CLICK_AFTER_FIXING: &str = ", then click here.";
const RIBBON: &str = "main-ribbon";
const COMPACT_RIBBON: &str = "main-ribbon-compact";
const COMPACT_KEY: &str = "main-ribbon-is-compact";
pub const SWEEP_HOVER: &str =
    "Carry the closed regions of a sketch along a path of sketch curves or body edges";

pub struct ToolbarContext<'a> {
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
    pub offers: &'a Offers,
    pub measuring: bool,
    pub checking_interference: bool,
    pub analysing: Option<Kind>,
    pub combing: bool,
    pub isocurving: bool,
    pub last_offers: &'a [Offer],
    pub toggles: ToggleStates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    History,
    Sketch,
    Solid,
    Modify,
    Pattern,
    Reference,
    Analyse,
}

impl Group {
    const ALL: [Self; 7] = [
        Self::History,
        Self::Sketch,
        Self::Solid,
        Self::Modify,
        Self::Pattern,
        Self::Reference,
        Self::Analyse,
    ];

    fn caption(self) -> &'static str {
        match self {
            Self::History => "History",
            Self::Sketch => "Sketch",
            Self::Solid => "Solid",
            Self::Modify => "Modify",
            Self::Pattern => "Pattern",
            Self::Reference => "Reference",
            Self::Analyse => "Analyse",
        }
    }

    fn menu_name(self) -> String {
        format!("{} commands", self.caption())
    }

    fn sections(self) -> &'static [&'static [Command]] {
        match self {
            Self::History => &[&[
                Command::Undo,
                Command::Redo,
                Command::RepeatLast,
                Command::UndoHistory,
            ]],
            Self::Sketch => &[&[
                Command::NewSketch,
                Command::EditSketch,
                Command::LookAtSketch,
            ]],
            Self::Solid => &[
                &[
                    Command::Extrude,
                    Command::Revolve,
                    Command::Sweep,
                    Command::Hole,
                    Command::Thread,
                ],
                &[
                    Command::NewBox,
                    Command::NewCylinder,
                    Command::NewSphere,
                    Command::NewTorus,
                    Command::NewCone,
                    Command::NewWedge,
                    Command::NewPrism,
                ],
            ],
            Self::Modify => &[
                &[
                    Command::Fillet,
                    Command::Chamfer,
                    Command::Shell,
                    Command::OffsetFace,
                    Command::SplitFace,
                    Command::Combine,
                ],
                &[
                    Command::Move,
                    Command::CopyBody,
                    Command::Mirror,
                    Command::MirrorFaces,
                    Command::Split,
                    Command::Scale,
                    Command::ScaleModel,
                ],
                &[Command::Mate, Command::MateAngle],
            ],
            Self::Pattern => &[&[
                Command::LinearPattern,
                Command::CircularPattern,
                Command::CurvePattern,
                Command::PointPattern,
            ]],
            Self::Reference => &[
                &[
                    Command::DatumPlane,
                    Command::DatumAxis,
                    Command::DatumPoint,
                    Command::CoordinateSystem,
                ],
                &[Command::InsertImage],
            ],
            Self::Analyse => &[
                &[Command::Measure, Command::Interference],
                &[
                    Command::Analysis(AnalysisCommand::Draft),
                    Command::Analysis(AnalysisCommand::Radius),
                    Command::Analysis(AnalysisCommand::Reach),
                    Command::Analysis(AnalysisCommand::Curvature),
                    Command::Analysis(AnalysisCommand::Zebra),
                    Command::Analysis(AnalysisCommand::Chrome),
                ],
                &[
                    Command::Analysis(AnalysisCommand::Comb),
                    Command::Analysis(AnalysisCommand::Isocurves),
                ],
                &[
                    Command::Section(SectionCommand::Toggle),
                    Command::Section(SectionCommand::Add),
                ],
                &[Command::ToggleCentresOfMass],
            ],
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) -> Vec<Command> {
    let frame = ribbon::bar_frame(ui.style());
    let keymap = commands.keymap();
    let chosen = RefCell::new(Vec::new());
    egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(ribbon::TOOL_GAP);
        let full = ui.available_width();
        let labelled = ribbon::rows(
            &ribbon::remembered_widths(ui, RIBBON, &Group::ALL),
            full,
            full,
        );
        let compact = ribbon::labelled_too_tall(ui, labelled.len());
        let (ribbon_name, rows) = if compact {
            let widths = ribbon::remembered_widths(ui, COMPACT_RIBBON, &Group::ALL);
            (COMPACT_RIBBON, ribbon::rows(&widths, full, full))
        } else {
            (RIBBON, labelled)
        };
        ui.data_mut(|data| data.insert_temp(Id::new(COMPACT_KEY), compact));
        let captions = !compact && rows.len() == 1;
        for (index, row) in rows.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(ribbon::ROW_GAP);
            }
            ui.horizontal_top(|ui| {
                ribbon::row(ui, ribbon_name, &row, |ui, group| {
                    let title = (!compact).then(|| ribbon::caption_galley(ui, group.caption()));
                    ribbon::captioned_by(
                        ui,
                        title,
                        captions,
                        |ui| {
                            let tools = ui
                                .horizontal_top(|ui| {
                                    group_buttons(ui, group, model, context, commands, actions);
                                })
                                .response
                                .rect;
                            if !captions {
                                let response = corner_menu(ui, tools, group, compact);
                                group_menu(&response, group, model, context, keymap, &chosen);
                            }
                            tools.width()
                        },
                        |ui, rect, title| {
                            let height = rect.height().max(title.size().y);
                            let strip = Rect::from_min_size(rect.min, vec2(rect.width(), height));
                            ui.allocate_rect(strip, Sense::hover());
                            let id = ui.id().with(("group-title", group));
                            let response =
                                widgets::group_title(ui, rect, id, title, &group.menu_name());
                            group_menu(&response, group, model, context, keymap, &chosen);
                        },
                    )
                });
            });
        }
    });
    chosen.into_inner()
}

fn corner_menu(ui: &mut Ui, tools: Rect, group: Group, compact: bool) -> Response {
    let name = group.menu_name();
    let contents = format!(
        "Every {} command, the ones without a button included",
        group.caption()
    );
    let menu = CornerMenu {
        id: Id::new(("ribbon-group-menu", group)),
        name: &name,
        contents: &contents,
        on_selected: false,
    };
    if compact {
        widgets::compact_corner_menu_button(ui, tools, &menu)
    } else {
        widgets::corner_menu_button(ui, tools, &menu)
    }
}

fn group_menu(
    response: &Response,
    group: Group,
    model: &Model,
    context: &ToolbarContext<'_>,
    keymap: &Keymap,
    chosen: &RefCell<Vec<Command>>,
) {
    let mut entries = MenuEntries::new(context.last_offers, context.toggles, keymap);
    if response.clicked() {
        entries = entries.focusing_first();
    }
    let shown = Popup::menu(response).show(|ui| {
        widgets::fitted_menu(ui, |ui| {
            for (index, section) in group.sections().iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                for &command in *section {
                    group_entry(ui, &mut entries, command, model, context);
                }
            }
        });
        let picked = entries.take_chosen();
        if !picked.is_empty() {
            ui.close();
        }
        picked
    });
    if let Some(shown) = shown {
        chosen.borrow_mut().extend(shown.inner);
    }
}

fn group_entry(
    ui: &mut Ui,
    entries: &mut MenuEntries<'_>,
    command: Command,
    model: &Model,
    context: &ToolbarContext<'_>,
) {
    let step = match command {
        Command::Undo => Some(model.undo_label()),
        Command::Redo => Some(model.redo_label()),
        _ => None,
    };
    if let Some(step) = step {
        let title = menu_bar::step_title(command, step, entries.is_available(command));
        entries.titled_item(ui, command, &title);
    } else if command == Command::RepeatLast
        && let Some(last) = entries.detail(command)
    {
        let title = command.title_with(last);
        entries.titled_item(ui, command, &title);
    } else if let Some(on) = panel_state(context, command) {
        entries.marked(ui, command, on);
    } else {
        entries.item(ui, command);
    }
}

fn panel_state(context: &ToolbarContext<'_>, command: Command) -> Option<bool> {
    match command {
        Command::Measure => Some(context.measuring),
        Command::Interference => Some(context.checking_interference),
        Command::Analysis(AnalysisCommand::Comb) => Some(context.combing),
        Command::Analysis(AnalysisCommand::Isocurves) => Some(context.isocurving),
        Command::Analysis(analysis) => analysis.kind().map(|kind| context.analysing == Some(kind)),
        _ => context.toggles.shown(command).map(|shown| shown.is_on()),
    }
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
            sweep_button(ui, model, context, commands, actions);
            hole_button(ui, model, context, commands, actions);
            primitive_offers(model, context, commands, actions);
            thread_offer(model, context, commands, actions);
        }
        Group::Modify => {
            blend_buttons(ui, model, context, commands, actions);
            shell_button(ui, model, context, commands, actions);
            offset_face_offer(model, context, commands, actions);
            split_face_offer(model, context, commands, actions);
            combine_button(ui, model, context, commands, actions);
            move_button(ui, model, context, commands, actions);
            mirror_button(ui, model, context, commands, actions);
            scale_button(ui, model, context, commands, actions);
        }
        Group::Pattern => pattern_buttons(ui, model, context, commands, actions),
        Group::Reference => {
            datum_buttons(ui, model, context, commands, actions);
            coordinate_system_offer(model, context, commands, actions);
            image_offer(context, commands, actions);
        }
        Group::Analyse => {
            measure_button(ui, context.measuring, commands);
            interference_button(ui, context.checking_interference, commands);
        }
    }
}

fn tool_button<'a>(ui: &Ui, glyph: &'a str, label: &'a str) -> ToolButton<'a> {
    let button = ToolButton::new(glyph, label);
    let compact = ui
        .data(|data| data.get_temp::<bool>(Id::new(COMPACT_KEY)))
        .unwrap_or(false);
    if compact { button.compact() } else { button }
}

fn measure_button(ui: &mut Ui, measuring: bool, commands: &mut CommandFrame<'_>) {
    let button =
        tool_button(ui, icons::command(Command::Measure), MEASURE_LABEL).selected(measuring);
    let help = Ok(commands.with_keys(Command::Measure, MEASURE_HOVER));
    if ribbon::explained(ui.add(button), MEASURE_LABEL, &help).clicked() {
        commands.trigger(Command::Measure);
    }
}

fn interference_button(ui: &mut Ui, checking: bool, commands: &mut CommandFrame<'_>) {
    let button = tool_button(
        ui,
        icons::command(Command::Interference),
        INTERFERENCE_LABEL,
    )
    .selected(checking);
    let help = Ok(commands.with_keys(Command::Interference, INTERFERENCE_HOVER));
    if ribbon::explained(ui.add(button), INTERFERENCE_LABEL, &help).clicked() {
        commands.trigger(Command::Interference);
    }
}

fn tool(
    ui: &mut Ui,
    commands: &mut CommandFrame<'_>,
    command: Command,
    label: &str,
    help: &Result<String, String>,
) -> Response {
    let button = tool_button(ui, icons::command(command), label);
    match help {
        Err(blocked) if commands.awaits(command) => {
            let response = ribbon::explained(ui.add(button), label, &Ok(queued_help(blocked)));
            if response.clicked() {
                commands.queue(command);
            }
            response
        }
        _ => ribbon::explained(ui.add_enabled(help.is_ok(), button), label, help),
    }
}

fn queued_help(blocked: &str) -> String {
    let stem = blocked
        .strip_suffix(CLICK_AFTER_FIXING)
        .unwrap_or_else(|| blocked.trim_end_matches('.'));
    format!("{stem}. {}", queued::QUEUE_ON_CLICK)
}

fn sketch_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let editing = context.editing;
    if editing.is_choosing_plane() {
        let button =
            tool_button(ui, icons::command(Command::NewSketch), NEW_SKETCH_LABEL).selected(true);
        let help = Ok(format!(
            "{CHOOSE_PLANE_PROMPT}, or click here to stop choosing (Esc)"
        ));
        if ribbon::explained(ui.add(button), NEW_SKETCH_LABEL, &help).clicked() {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        }
        return;
    }
    let target = context.offers.sketch_target;
    let (hover, command) = match target {
        Ok(SketchTarget::Principal(plane)) => (
            format!("Start a sketch on the selected {}", plane.name()),
            EditingCommand::NewSketch(Some(plane)),
        ),
        Ok(SketchTarget::Datum(datum)) => (
            format!(
                "Start a sketch on {}; it follows the plane when the model changes",
                datum.name(model.document())
            ),
            EditingCommand::NewSketchOnDatum(datum),
        ),
        Ok(SketchTarget::Face(face)) => (
            "Start a sketch on the selected face; it follows the face when the model changes"
                .to_owned(),
            EditingCommand::NewSketchOnFace(face),
        ),
        Ok(SketchTarget::Choose) | Err(_) => (
            "Start a sketch on the plane or flat face you click next".to_owned(),
            EditingCommand::NewSketch(None),
        ),
    };
    let help = match target {
        Ok(_) => Ok(commands.with_keys(Command::NewSketch, &hover)),
        Err(reason) => Err(reason.to_owned()),
    };
    let invoked = commands.invoke(Command::NewSketch, &target);
    let response = tool(ui, commands, Command::NewSketch, NEW_SKETCH_LABEL, &help);
    if (response.clicked() || invoked) && target.is_ok() {
        actions.push(Action::Editing(command));
    }
}

fn face_sweep_help(document: &Document, sweep: Sweep, profile: &FaceProfile) -> String {
    let faces = match profile.faces.len() {
        1 => "face".to_owned(),
        count => format!("{count} faces"),
    };
    match (sweep, &profile.axis) {
        (Sweep::Extrude, _) => format!("Extrude the selected {faces} out of its body"),
        (Sweep::Revolve, Some(axis)) => format!(
            "Revolve the selected {faces} about {}",
            describe_axis(document, axis)
        ),
        (Sweep::Revolve, None) => format!("Revolve the selected {faces}"),
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
    let lone_axis = context.selection.len() == 1 && !context.offers.model_axes.is_empty();
    for sweep in Sweep::ALL {
        let command = match sweep {
            Sweep::Extrude => Command::Extrude,
            Sweep::Revolve => Command::Revolve,
        };
        let may_guess = solid_tools::may_guess_sketch(sweep, context.selection, lone_axis);
        let source = solid_tools::sweep_source(
            document,
            model.evaluation(),
            context.selection,
            context.editing,
            solid_tools::Guess::sweep_when(may_guess),
        )
        .map(|source| {
            source.map(|source| {
                solid_tools::with_model_axis(source, context.selection, &context.offers.model_axes)
            })
        });
        let face = match sweep {
            Sweep::Extrude => &context.offers.extrude_faces,
            Sweep::Revolve => &context.offers.revolve_faces,
        };
        let face = face.as_ref().map(Result::as_ref);
        let missing = match sweep {
            _ if may_guess => NO_SKETCH_TO_SWEEP,
            Sweep::Extrude => solid_tools::NOTHING_TO_EXTRUDE,
            Sweep::Revolve => solid_tools::NOTHING_TO_REVOLVE,
        };
        let available = match (face, source.as_ref()) {
            (Some(Ok(_)), _) => Ok(()),
            (Some(Err(&reason)), _) | (None, Err(&reason)) => Err(reason),
            (None, Ok(Some(_))) => Ok(()),
            (None, Ok(None)) => Err(missing),
        };
        let invoked = commands.invoke(command, &available);
        let help = match (face, source.as_ref()) {
            (Some(Ok(profile)), _) => {
                Ok(commands.with_keys(command, &face_sweep_help(document, sweep, profile)))
            }
            (Some(Err(&reason)), _) | (None, Err(&reason)) => Err(reason.to_owned()),
            (None, Ok(Some(source))) => {
                let sketch = document
                    .feature(source.sketch)
                    .map_or("the sketch", |feature| feature.name.as_str());
                let picked =
                    !solid_tools::selected_regions(context.selection, source.sketch).is_empty();
                let which = match (picked, &source.regions) {
                    (true, _) => "selected",
                    (false, RegionChoice::Chosen(_)) => "remaining",
                    (false, RegionChoice::All) => "closed",
                };
                let hover = match (sweep, &source.axis) {
                    (Sweep::Extrude, _) => format!("Extrude the {which} regions of {sketch}"),
                    (Sweep::Revolve, Some(axis)) => {
                        let axis = solid_tools::axis_name(document, source.sketch, axis);
                        format!("Revolve the {which} regions of {sketch} about {axis}")
                    }
                    (Sweep::Revolve, None) => format!(
                        "Revolve the {which} regions of {sketch} about its vertical axis, or \
                         about a line or axis you select first"
                    ),
                };
                Ok(commands.with_keys(command, &hover))
            }
            (None, Ok(None)) => Err(missing.to_owned()),
        };
        let response = tool(ui, commands, command, sweep.label(), &help);
        if !(response.clicked() || invoked) {
            continue;
        }
        match (face, &source) {
            (Some(Ok(profile)), _) => {
                actions.extend(solid_tools::create_on_faces_actions(model, sweep, profile));
            }
            (None, Ok(Some(source))) => actions.extend(solid_tools::create_actions(
                model,
                sweep,
                solid_tools::with_selected_outline(model, sweep, source.clone(), context.selection),
                &Starts::of(model),
            )),
            (Some(Err(_)), _) | (None, Err(_) | Ok(None)) => {}
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
    let source = context.offers.hole;
    let invoked = commands.invoke(Command::Hole, &source);
    let help = match &source {
        Ok(_) => Ok(commands.with_keys(Command::Hole, hole_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            hole_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, commands, Command::Hole, hole_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(hole_tools::create_actions(model, source));
    }
}

fn thread_offer(
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.thread;
    if commands.invoke(Command::Thread, source)
        && let Ok(source) = source
    {
        actions.extend(thread_tools::create_actions(
            model.document(),
            model.evaluation(),
            *source,
        ));
    }
}

fn blend_buttons(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.blend;
    for kind in blend_tools::KINDS {
        let command = match kind {
            BlendKind::Fillet => Command::Fillet,
            BlendKind::Chamfer => Command::Chamfer,
        };
        let invoked = commands.invoke(command, source);
        let help = match source {
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
        let response = tool(ui, commands, command, kind.title(), &help);
        if (response.clicked() || invoked)
            && let Ok(source) = source
        {
            actions.extend(blend_tools::create_actions(
                model.document(),
                model.evaluation(),
                kind,
                source,
                &Starts::of(model),
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
             midway between two planes, through an axis and a point or two lines in one plane, \
             or square to the selected round or curved edge",
        )),
        Err(reason) => Err(format!("{reason}.")),
    };
    let response = tool(ui, commands, Command::DatumPlane, PLANE_LABEL, &help);
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
    let response = tool(ui, commands, Command::DatumAxis, AXIS_LABEL, &help);
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
            "Add a point at the selected corner, round edge's centre, sphere or torus centre, \
             sketch point or datum point (the origin when none is selected), moved by typed \
             offsets, a distance along the selected edge, or where two lines, a line and a \
             plane, or three planes meet",
        )),
        Err(reason) => Err(format!("Add a point. {reason}, then click here.")),
    };
    let response = tool(ui, commands, Command::DatumPoint, POINT_LABEL, &help);
    if (response.clicked() || invoked)
        && let Ok(point) = point
    {
        actions.extend(datum_tools::create_actions(document, point));
    }
}

fn coordinate_system_offer(
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let frame = &context.offers.coordinate_system;
    if commands.invoke(Command::CoordinateSystem, frame)
        && let Ok(frame) = frame
    {
        actions.extend(datum_tools::create_actions(model.document(), frame.clone()));
    }
}

fn image_offer(
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let target = &context.offers.reference_image;
    if commands.invoke(Command::InsertImage, target)
        && let Ok(target) = target
    {
        actions.push(Action::File(FileCommand::InsertImage(target.clone())));
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
    let response = tool(ui, commands, Command::Shell, shell_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(shell_tools::create_actions(
            model.document(),
            model.evaluation(),
            source,
            &Starts::of(model),
        ));
    }
}

fn sweep_button(
    ui: &mut Ui,
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.sweep;
    let invoked = commands.invoke(Command::Sweep, source);
    let help = match source {
        Ok(_) => Ok(commands.with_keys(Command::Sweep, SWEEP_HOVER)),
        Err(reason) => Err(format!("{SWEEP_HOVER}. {reason}, then click here.")),
    };
    let response = tool(ui, commands, Command::Sweep, sweep_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(sweep_tools::create_actions(model, source));
    }
}

fn offset_face_offer(
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.offset_face;
    if commands.invoke(Command::OffsetFace, source)
        && let Ok(source) = source
    {
        actions.extend(offset_face_tools::create_actions(
            model.document(),
            model.evaluation(),
            source,
            model.length_unit(),
        ));
    }
}

fn split_face_offer(
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.split_face;
    if commands.invoke(Command::SplitFace, source)
        && let Ok(source) = source
    {
        actions.extend(split_face_tools::create_actions(
            model.document(),
            model.evaluation(),
            source,
        ));
    }
}

fn primitive_offers(
    model: &Model,
    context: &ToolbarContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let source = &context.offers.primitive;
    for kind in PrimitiveKind::ALL {
        if commands.invoke(primitive_tools::command(kind), source)
            && let Ok(source) = source
        {
            actions.extend(primitive_tools::create_actions(model, kind, source));
        }
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
    let response = tool(ui, commands, Command::Move, move_tools::TITLE, &help);
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
        Ok(chosen) if !chosen.mirrored.is_empty() => {
            let hover = mirror_tools::features_hover(model.document(), chosen);
            Ok(commands.with_keys(Command::Mirror, &hover))
        }
        Ok(_) => Ok(commands.with_keys(Command::Mirror, mirror_tools::DESCRIPTION)),
        Err(reason) => Err(format!(
            "{}. {reason}, then click here.",
            mirror_tools::DESCRIPTION
        )),
    };
    let response = tool(ui, commands, Command::Mirror, mirror_tools::TITLE, &help);
    if (response.clicked() || invoked)
        && let Ok(source) = source
    {
        actions.extend(mirror_tools::create_actions(model, source));
    }

    let faces = &context.offers.mirror_faces;
    if commands.invoke(Command::MirrorFaces, faces)
        && let Ok(faces) = faces
    {
        actions.extend(mirror_tools::create_faces_actions(model, faces));
    }

    let split = &context.offers.split;
    if commands.invoke(Command::Split, split)
        && let Ok(split) = split
    {
        actions.extend(split_tools::create_actions(model, split));
    }

    let mate = &context.offers.mate;
    if commands.invoke(Command::Mate, mate)
        && let Ok(mate) = mate
    {
        actions.extend(mate_tools::create_actions(model, mate));
    }

    let mate_angle = &context.offers.mate_angle;
    if commands.invoke(Command::MateAngle, mate_angle)
        && let Ok(mate) = mate_angle
    {
        actions.extend(mate_tools::create_actions(model, mate));
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
    let response = tool(ui, commands, Command::Scale, scale_tools::TITLE, &help);
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
    let response = tool(ui, commands, Command::Combine, combine_tools::TITLE, &help);
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
    for shape in Shape::ON_RIBBON {
        let command = shape.command();
        let invoked = commands.invoke(command, source);
        let help = match source {
            Ok(source) => {
                let subject = source.subject(document);
                let hover = match (shape, &source.axis) {
                    (Shape::Linear, Some(axis)) => {
                        format!("Repeat {subject} along {}", describe_axis(document, axis))
                    }
                    (Shape::Linear, None) => format!(
                        "Repeat {subject} along the X axis, or along an edge or axis you select \
                         first"
                    ),
                    (Shape::Circular, Some(axis)) => {
                        format!("Repeat {subject} around {}", describe_axis(document, axis))
                    }
                    (Shape::Circular, None) => format!(
                        "Repeat {subject} around an axis, straight edge or round face: select one \
                         first, or click it once the pattern opens"
                    ),
                    (Shape::Curve | Shape::Points, None) => format!(
                        "Repeat {subject} around the Z axis, or around an axis or round face you \
                         select first"
                    ),
                    (Shape::Curve | Shape::Points, Some(axis)) => {
                        format!("Repeat {subject} about {}", describe_axis(document, axis))
                    }
                };
                Ok(commands.with_keys(command, &hover))
            }
            Err(reason) => Err(format!(
                "{}. {reason}, then click here.",
                shape.description()
            )),
        };
        let response = tool(ui, commands, command, shape.title(), &help);
        if (response.clicked() || invoked)
            && let Ok(source) = source
        {
            actions.extend(pattern_tools::create_actions(model, shape, source));
        }
    }
    for (shape, source) in [
        (Shape::Curve, &context.offers.curve_pattern),
        (Shape::Points, &context.offers.point_pattern),
    ] {
        if commands.invoke(shape.command(), source)
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
        let response = tool(ui, commands, command, &verb, &help);
        if response.clicked() || invoked {
            actions.push(action);
        }
    }
}
