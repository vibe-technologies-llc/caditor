use caditor_document::SavedViews;
use egui::{
    Align, CornerRadius, Id, Label, Layout, Popup, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, TextStyle, TextWrapMode, Ui, UiBuilder, pos2, vec2,
};

use crate::{
    analysis::AnalysisCommand,
    appearance::{self, BORDER_WIDTH, CONTROL_HEIGHT, SPACE_M, SPACE_S, WIDGET_RADIUS},
    commands::{CameraMove, Command, CommandFrame, Offer, Scope, StandardView},
    display_style::DisplayStyle,
    editing::{SketchEditing, Tool},
    files::{self, FileCommand, Files},
    history::HistoryCommand,
    icons, logo,
    model::{Action, Model},
    preferences::PreferencesCommand,
    selection::SelectionFilter,
    shape_modes::ShapeMode,
    sketch_tools::ConstraintTool,
    view_aids::ViewAids,
    widgets::{self, Tone},
    window_frame::{self, Chrome},
};

pub const SEARCH_LABEL: &str = "Search commands";
pub const MODEL_DETAILS: &str = "Model details";
pub const COPY_PATH: &str = "Copy file location";
const SEARCH_WIDTH: f32 = 240.0;
const NAME_ROOM: f32 = 160.0;
const TITLE_PADDING: f32 = SPACE_M;
const DETAILS_WIDTH: f32 = 280.0;
const WAYS_TO_DRAW: &str = "Ways to draw shapes";
const SKETCH_ONLY: &str = "Only while a sketch is being edited";
const NOT_HERE: &str = "Not available right now";
const NOT_SAVED: &str = "Save the model to start keeping its versions";
const MODEL_PATTERNS: [&[Command]; 1] = [&[Command::LinearPattern, Command::CircularPattern]];
const MODEL_PRIMITIVES: [&[Command]; 1] = [&[
    Command::NewBox,
    Command::NewCylinder,
    Command::NewSphere,
    Command::NewTorus,
]];
const MODEL_DATUMS: [&[Command]; 1] =
    [&[Command::DatumPlane, Command::DatumAxis, Command::DatumPoint]];
const MODEL_BODIES: [&[Command]; 2] = [
    &[
        Command::Move,
        Command::CopyBody,
        Command::Mirror,
        Command::Split,
        Command::Scale,
    ],
    &[
        Command::BodyAppearance,
        Command::RenameBody,
        Command::RemoveBody,
    ],
];
const MODEL_FEATURES: [&[Command]; 3] = [
    &[
        Command::EditFeature,
        Command::CloseFeature,
        Command::RenameFeature,
        Command::MoveFeatureUp,
        Command::MoveFeatureDown,
        Command::SuppressFeature,
        Command::DeleteFeature,
    ],
    &[
        Command::GroupFeatures,
        Command::RenameGroup,
        Command::Ungroup,
    ],
    &[Command::FilterFeatures],
];
const MODEL_ROLLBACK: [&[Command]; 1] = [&[
    Command::RollToHere,
    Command::RollbackUp,
    Command::RollbackDown,
    Command::RollToEnd,
]];
const MODEL_PARAMETERS: [&[Command]; 1] = [&[
    Command::AddParameter,
    Command::MoveParameterUp,
    Command::MoveParameterDown,
    Command::ParameterNote,
    Command::DeleteParameter,
]];
const MODEL_RECOMPUTE: [&[Command]; 1] = [&[
    Command::Recompute,
    Command::CancelRecompute,
    Command::ShowFirstFailed,
    Command::UpdateReferences,
]];

pub struct MenuContext<'a> {
    pub views: &'a SavedViews,
    pub files: &'a Files,
    pub editing: &'a SketchEditing,
    pub offers: &'a [Offer],
    pub chrome: Chrome,
    pub filter: SelectionFilter,
    pub style: DisplayStyle,
    pub snapping: bool,
    pub grid_snapping: bool,
    pub lasso: bool,
    pub select_through: bool,
    pub automatic_projection: bool,
    pub typed_dimensions: bool,
    pub glyphs: bool,
    pub aids: ViewAids,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &MenuContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let trailing_id = Id::new("menu-bar-trailing");
    let panel = egui::Panel::top("menu-bar")
        .show_separator_line(true)
        .show(ui, |ui| {
            if let Some(drag) = window_frame::drag_region(ui, context.chrome) {
                window_frame::drags_window(&drag, context.chrome, commands, actions);
            }
            let mut wrapped = false;
            egui::MenuBar::new().ui(ui, |ui| {
                if context.chrome.built_in() {
                    logo::show(ui, ui.spacing().interact_size.y);
                }
                files::menu(
                    ui,
                    model,
                    context.files,
                    context.editing,
                    context.offers,
                    commands,
                    actions,
                );
                let mut menus = Menus {
                    views: context.views,
                    visited: Vec::new(),
                    offers: context.offers,
                    filter: context.filter,
                    style: context.style,
                    snapping: context.snapping,
                    grid_snapping: context.grid_snapping,
                    lasso: context.lasso,
                    select_through: context.select_through,
                    automatic_projection: context.automatic_projection,
                    typed_dimensions: context.typed_dimensions,
                    glyphs: context.glyphs,
                    aids: context.aids,
                    commands,
                    chosen: Vec::new(),
                };
                menus.edit(ui);
                menus.view(ui);
                menus.model(ui);
                menus.sketch(ui);
                menus.help(ui);
                let chosen = std::mem::take(&mut menus.chosen);
                let visited = std::mem::take(&mut menus.visited);
                for command in chosen {
                    commands.trigger(command);
                }
                actions.extend(
                    visited
                        .into_iter()
                        .map(|index| Action::Preferences(PreferencesCommand::GoToView(index))),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if context.chrome.built_in() {
                        window_frame::remember_controls_row(ui.ctx(), ui.max_rect());
                        window_frame::controls(ui, context.chrome.state, commands, actions);
                        ui.add_space(SPACE_S);
                        ui.separator();
                        ui.add_space(SPACE_S);
                    }
                    if ui.available_width() >= widgets::remembered_width(ui, trailing_id) {
                        trailing(ui, model, context, commands, actions, trailing_id);
                    } else {
                        wrapped = true;
                    }
                });
            });
            if wrapped {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    trailing(ui, model, context, commands, actions, trailing_id);
                });
            }
        });
    window_frame::remember_bar(ui.ctx(), panel.response.rect);
}

fn trailing(
    ui: &mut Ui,
    model: &Model,
    context: &MenuContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
    id: Id,
) {
    let search = search(ui, commands);
    let spacing = ui.spacing().item_spacing.x;
    widgets::remember_width(ui, id, search.rect.width() + spacing + NAME_ROOM);
    title(ui, model, context, commands, actions);
}

fn title(
    ui: &mut Ui,
    model: &Model,
    context: &MenuContext<'_>,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let rest = ui.available_rect_before_wrap();
    let width_id = Id::new("model-title-width");
    let wanted = widgets::remembered_width(ui, width_id).min(rest.width());
    let centred = ui.ctx().content_rect().center().x - wanted / 2.0;
    let left = centred.min(rest.right() - wanted).max(rest.left());
    let area = Rect::from_min_max(pos2(left, rest.top()), rest.max);
    let shown = ui.scope_builder(
        UiBuilder::new()
            .max_rect(area)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| model_title(ui, model, context, commands, actions),
    );
    widgets::remember_width(ui, width_id, shown.inner.rect.width());
}

fn model_title(
    ui: &mut Ui,
    model: &Model,
    context: &MenuContext<'_>,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let tokens = appearance::tokens(ui);
    let text = if context.chrome.state.focused {
        tokens.text
    } else {
        tokens.text_muted
    };
    let background = ui.painter().add(Shape::Noop);
    let name = model.display_name();
    let group = ui
        .horizontal(|ui| {
            ui.add_space(TITLE_PADDING);
            widgets::icon_label(ui, icons::FILE, tokens.text_muted);
            ui.add(
                Label::new(
                    RichText::new(&name)
                        .text_style(TextStyle::Button)
                        .color(text),
                )
                .truncate()
                .selectable(false),
            );
            if model.is_dirty() {
                widgets::pill(ui, Tone::Neutral, "Unsaved");
            }
            if let Some(sketch) = edited_sketch(model, context.editing) {
                widgets::icon_label(ui, icons::BREADCRUMB, tokens.text_muted);
                widgets::icon_label(ui, icons::SKETCH, tokens.text_muted);
                ui.add(
                    Label::new(
                        RichText::new(sketch)
                            .text_style(TextStyle::Button)
                            .color(text),
                    )
                    .truncate()
                    .selectable(false),
                );
            }
            ui.add_space(TITLE_PADDING);
        })
        .response;
    let response = ui.interact(group.rect, Id::new("model-title"), Sense::click_and_drag());
    let response = widgets::named(response, &format!("{name}, {MODEL_DETAILS}"));
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    let fill = if response.is_pointer_button_down_on() {
        Some(tokens.pressed)
    } else if response.hovered() || open {
        Some(tokens.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().set(
            background,
            egui::epaint::RectShape::filled(group.rect, CornerRadius::same(WIDGET_RADIUS), fill),
        );
    }
    if response.has_focus() {
        widgets::paint_focus_ring(ui, group.rect);
    }
    if context.chrome.built_in() && response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let hover = match model.path() {
        Some(path) => path.display().to_string(),
        None => "Not saved to a file yet".to_owned(),
    };
    let response = response.on_hover_text(hover);
    Popup::menu(&response)
        .width(DETAILS_WIDTH)
        .show(|ui| model_details(ui, model, commands, actions));
    response
}

fn edited_sketch(model: &Model, editing: &SketchEditing) -> Option<String> {
    let feature = editing.feature()?;
    model
        .document()
        .feature(feature)
        .map(|feature| feature.name.clone())
}

fn model_details(
    ui: &mut Ui,
    model: &Model,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    ui.set_min_width(DETAILS_WIDTH);
    ui.label(widgets::section_title(&model.display_name()));
    match model.path() {
        Some(path) => ui.label(widgets::muted(path.display().to_string(), ui)),
        None => ui.label(widgets::muted("Not saved to a file yet", ui)),
    };
    ui.horizontal_wrapped(|ui| match (model.is_dirty(), model.path()) {
        (true, _) => widgets::pill(ui, Tone::Warning, "Unsaved changes"),
        (false, Some(_)) => widgets::pill(ui, Tone::Success, "All changes saved"),
        (false, None) => widgets::pill(ui, Tone::Neutral, "Nothing to save yet"),
    });
    ui.separator();
    let item = |ui: &mut Ui, command: Command| {
        widgets::menu_item(
            ui,
            icons::command(command),
            &command.title(),
            commands.keys(command),
        )
    };
    if item(ui, Command::Save).clicked() {
        actions.push(Action::File(FileCommand::Save));
    }
    if item(ui, Command::SaveAs).clicked() {
        actions.push(Action::File(FileCommand::SaveAs));
    }
    let saved = model.path().is_some();
    let history = ui
        .add_enabled_ui(saved, |ui| item(ui, Command::VersionHistory))
        .inner
        .on_disabled_hover_text(NOT_SAVED);
    if history.clicked() {
        actions.push(Action::File(FileCommand::History(HistoryCommand::Show)));
    }
    if item(ui, Command::ModelProperties).clicked() {
        actions.push(Action::Preferences(PreferencesCommand::ShowModelProperties));
    }
    if let Some(path) = model.path() {
        let copy = widgets::menu_item(ui, icons::COPY_PATH, COPY_PATH, None)
            .on_hover_text("Copy the model's full path to the clipboard");
        if copy.clicked() {
            ui.ctx().copy_text(path.display().to_string());
        }
    }
}

struct Menus<'a, 'b> {
    views: &'a SavedViews,
    visited: Vec<usize>,
    offers: &'a [Offer],
    filter: SelectionFilter,
    style: DisplayStyle,
    snapping: bool,
    grid_snapping: bool,
    lasso: bool,
    select_through: bool,
    automatic_projection: bool,
    typed_dimensions: bool,
    glyphs: bool,
    aids: ViewAids,
    commands: &'a CommandFrame<'b>,
    chosen: Vec<Command>,
}

impl Menus<'_, '_> {
    fn availability(&self, command: Command) -> Result<(), String> {
        match self.offers.iter().find(|offer| offer.command == command) {
            Some(offer) => offer.availability.clone(),
            None if command.scope() == Scope::Sketch => Err(SKETCH_ONLY.to_owned()),
            None => Err(NOT_HERE.to_owned()),
        }
    }

    fn item(&mut self, ui: &mut Ui, command: Command) {
        let availability = self.availability(command);
        let response = ui.add_enabled_ui(availability.is_ok(), |ui| {
            widgets::menu_item(
                ui,
                icons::command(command),
                &command.title(),
                self.commands.keys(command),
            )
        });
        let response = match &availability {
            Ok(()) => response.inner,
            Err(reason) => response.inner.on_disabled_hover_text(reason),
        };
        if response.clicked() {
            self.chosen.push(command);
        }
    }

    fn choice(&mut self, ui: &mut Ui, command: Command, chosen: bool) {
        let availability = self.availability(command);
        let response = ui.add_enabled_ui(availability.is_ok(), |ui| {
            widgets::menu_choice(
                ui,
                icons::command(command),
                &command.title(),
                self.commands.keys(command),
                chosen,
            )
        });
        let response = match &availability {
            Ok(()) => response.inner,
            Err(reason) => response.inner.on_disabled_hover_text(reason),
        };
        if response.clicked() {
            self.chosen.push(command);
        }
    }

    fn items(&mut self, ui: &mut Ui, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            self.item(ui, command);
        }
    }

    fn edit(&mut self, ui: &mut Ui) {
        top_menu(ui, "Edit", |ui| {
            self.items(ui, [Command::Undo, Command::Redo, Command::UndoHistory]);
            ui.separator();
            self.item(ui, Command::DeleteSelection);
            ui.separator();
            self.items(
                ui,
                [
                    Command::SelectAllShapes,
                    Command::SelectTangentEdges,
                    Command::SelectTangentFaces,
                    Command::SelectHole,
                    Command::SelectFaceEdges,
                    Command::SelectBody,
                ],
            );
            ui.separator();
            self.item(ui, Command::Palette);
        });
    }

    fn view(&mut self, ui: &mut Ui) {
        top_menu(ui, "View", |ui| {
            self.item(ui, Command::FitView);
            submenu(
                ui,
                icons::command(Command::View(StandardView::Front)),
                "Standard views",
                |ui| {
                    self.items(ui, StandardView::ALL.map(Command::View));
                },
            );
            submenu(
                ui,
                icons::command(Command::SavedViews),
                "Saved views",
                |ui| {
                    self.items(ui, [Command::SaveView, Command::SavedViews]);
                    if !self.views.named.is_empty() {
                        ui.separator();
                    }
                    for (index, named) in self.views.named.iter().enumerate() {
                        let shown = widgets::menu_item(
                            ui,
                            icons::command(Command::View(StandardView::Isometric)),
                            &named.name,
                            None,
                        );
                        if shown.clicked() {
                            self.visited.push(index);
                        }
                    }
                    ui.separator();
                    self.items(ui, [Command::SetHomeView, Command::ResetHomeView]);
                },
            );
            submenu(
                ui,
                icons::command(Command::Camera(CameraMove::OrbitLeft)),
                "Camera",
                |ui| {
                    self.items(ui, CameraMove::ALL.map(Command::Camera));
                },
            );
            submenu(
                ui,
                icons::command(Command::Filter(SelectionFilter::Everything)),
                "Selection filter",
                |ui| {
                    for filter in SelectionFilter::ALL {
                        self.choice(ui, Command::Filter(filter), filter == self.filter);
                    }
                    ui.separator();
                    self.item(ui, Command::CycleSelectionPriority);
                },
            );
            submenu(
                ui,
                icons::command(Command::Style(DisplayStyle::ShadedWithEdges)),
                "Display style",
                |ui| {
                    for style in DisplayStyle::ALL {
                        self.choice(ui, Command::Style(style), style == self.style);
                    }
                },
            );
            ui.separator();
            self.item(ui, Command::ToggleProjection);
            self.choice(ui, Command::AutomaticProjection, self.automatic_projection);
            self.choice(ui, Command::ToggleSnapping, self.snapping);
            self.choice(ui, Command::ToggleGridSnapping, self.grid_snapping);
            self.choice(ui, Command::ToggleLasso, self.lasso);
            self.choice(ui, Command::ToggleSelectThrough, self.select_through);
            self.choice(ui, Command::ToggleGlyphs, self.glyphs);
            self.choice(ui, Command::ToggleCentresOfMass, self.aids.centres_of_mass);
            ui.separator();
            self.item(ui, Command::FullScreen);
            ui.separator();
            self.item(ui, Command::Measure);
            self.item(ui, Command::Interference);
            self.item(ui, Command::Analysis(AnalysisCommand::Draft));
            self.item(ui, Command::Analysis(AnalysisCommand::Radius));
            self.item(ui, Command::Analysis(AnalysisCommand::Reach));
            self.item(ui, Command::Analysis(AnalysisCommand::Curvature));
            self.item(ui, Command::Analysis(AnalysisCommand::Zebra));
            self.item(ui, Command::Analysis(AnalysisCommand::Chrome));
            self.item(ui, Command::Analysis(AnalysisCommand::Comb));
            ui.separator();
            self.items(
                ui,
                [
                    Command::HideSelection,
                    Command::HideOthers,
                    Command::ToggleVisibility,
                    Command::ShowAll,
                    Command::TogglePrincipal,
                ],
            );
            ui.separator();
            self.items(
                ui,
                [
                    Command::HighlightNext,
                    Command::HighlightPrevious,
                    Command::ActivateHighlighted,
                    Command::ListUnderPointer,
                ],
            );
            ui.separator();
            self.items(
                ui,
                [
                    Command::LargerInterface,
                    Command::SmallerInterface,
                    Command::NormalInterface,
                ],
            );
        });
    }

    fn model(&mut self, ui: &mut Ui) {
        top_menu(ui, "Model", |ui| {
            self.items(
                ui,
                [
                    Command::NewSketch,
                    Command::Extrude,
                    Command::Revolve,
                    Command::Hole,
                    Command::Thread,
                ],
            );
            self.group(ui, Command::NewBox, "Primitives", &MODEL_PRIMITIVES);
            ui.separator();
            self.items(
                ui,
                [
                    Command::Fillet,
                    Command::Chamfer,
                    Command::Shell,
                    Command::OffsetFace,
                    Command::Combine,
                ],
            );
            self.group(ui, Command::LinearPattern, "Patterns", &MODEL_PATTERNS);
            self.group(ui, Command::DatumPlane, "Datums", &MODEL_DATUMS);
            ui.separator();
            self.group(ui, Command::Move, "Bodies", &MODEL_BODIES);
            self.group(ui, Command::EditFeature, "Features", &MODEL_FEATURES);
            self.group(ui, Command::RollToHere, "Rollback", &MODEL_ROLLBACK);
            self.group(ui, Command::AddParameter, "Parameters", &MODEL_PARAMETERS);
            self.group(ui, Command::Recompute, "Recompute", &MODEL_RECOMPUTE);
        });
    }

    fn group(&mut self, ui: &mut Ui, shown: Command, title: &str, sections: &[&[Command]]) {
        submenu(ui, icons::command(shown), title, |ui| {
            for (index, section) in sections.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                self.items(ui, section.iter().copied());
            }
        });
    }

    fn sketch(&mut self, ui: &mut Ui) {
        top_menu(ui, "Sketch", |ui| {
            self.item(ui, Command::FinishSketch);
            ui.separator();
            let (drawing, modifying): (Vec<Tool>, Vec<Tool>) = Tool::ALL
                .into_iter()
                .filter(|tool| !tool.dimensions())
                .partition(|tool| *tool == Tool::Select || tool.draws());
            self.items(ui, drawing.into_iter().map(Command::SketchTool));
            ui.separator();
            submenu(ui, icons::tool(Tool::Rectangle), WAYS_TO_DRAW, |ui| {
                for tool in [
                    Tool::Rectangle,
                    Tool::Circle,
                    Tool::Polygon,
                    Tool::Slot,
                    Tool::BlendCurve,
                ] {
                    if tool != Tool::Rectangle {
                        ui.separator();
                    }
                    self.items(ui, ShapeMode::of_tool(tool).map(Command::ShapeMode));
                }
            });
            self.items(
                ui,
                [Command::ReverseArc, Command::MoreSides, Command::FewerSides],
            );
            self.choice(ui, Command::ToggleTypedDimensions, self.typed_dimensions);
            ui.separator();
            self.item(ui, Command::Construction);
            self.items(ui, modifying.into_iter().map(Command::SketchTool));
            self.items(
                ui,
                [
                    Command::SplitCurve,
                    Command::BreakCurves,
                    Command::MoveGeometry,
                    Command::RotateGeometry,
                    Command::ScaleGeometry,
                    Command::SelectAll,
                ],
            );
            self.items(
                ui,
                [
                    Command::CopyGeometry,
                    Command::CutGeometry,
                    Command::PasteGeometry,
                ],
            );
            self.item(ui, Command::IntersectBody);
            ui.separator();
            let (dimensions, geometric): (Vec<ConstraintTool>, Vec<ConstraintTool>) =
                ConstraintTool::ALL
                    .into_iter()
                    .partition(|tool| tool.is_dimension());
            submenu(
                ui,
                icons::constraint(ConstraintTool::Coincident),
                "Constraints",
                |ui| self.items(ui, geometric.into_iter().map(Command::Constraint)),
            );
            submenu(
                ui,
                icons::constraint(ConstraintTool::Distance),
                "Dimensions",
                |ui| {
                    self.item(ui, Command::SketchTool(Tool::Dimension));
                    ui.separator();
                    self.items(ui, dimensions.into_iter().map(Command::Constraint));
                },
            );
        });
    }

    fn help(&mut self, ui: &mut Ui) {
        top_menu(ui, "Help", |ui| {
            self.items(
                ui,
                [
                    Command::Welcome,
                    Command::Palette,
                    Command::KeyboardShortcuts,
                    Command::Messages,
                    Command::About,
                ],
            );
        });
    }
}

fn top_menu(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.menu_button(title, |ui| widgets::fitted_menu(ui, add));
}

fn submenu(ui: &mut Ui, glyph: &str, title: &str, add: impl FnOnce(&mut Ui)) {
    let muted = appearance::tokens(ui).text_muted;
    let submenu = ui.menu_button(
        (widgets::icon(glyph).color(muted), title.to_owned()),
        |ui| widgets::fitted_menu(ui, add),
    );
    widgets::named(submenu.response, title);
}

fn search(ui: &mut Ui, commands: &mut CommandFrame<'_>) -> Response {
    let tokens = appearance::tokens(ui);
    let content_id = Id::new("search-commands-content");
    let content = widgets::remembered_width(ui, content_id);
    let size = vec2(SEARCH_WIDTH.max(content + 2.0 * SPACE_M), CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hover = commands.with_keys(Command::Palette, "Search every command by name");
    let response = widgets::named(response, SEARCH_LABEL).on_hover_text(hover);
    let active = response.hovered() || response.is_pointer_button_down_on();
    let outline = if active {
        tokens.text_muted
    } else {
        tokens.field_border
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(WIDGET_RADIUS),
        tokens.sunken,
        Stroke::new(BORDER_WIDTH, outline),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        widgets::paint_focus_ring(ui, rect);
    }
    let inside = rect.shrink2(vec2(SPACE_M, 0.0));
    let mut leading = ui.new_child(
        UiBuilder::new()
            .max_rect(inside)
            .layout(Layout::left_to_right(Align::Center)),
    );
    leading.spacing_mut().item_spacing.x = SPACE_S;
    widgets::icon_label(&mut leading, icons::SEARCH, tokens.text_muted);
    leading.add(
        Label::new(RichText::new(SEARCH_LABEL).color(tokens.text_muted))
            .selectable(false)
            .wrap_mode(TextWrapMode::Extend),
    );
    let mut needed = leading.min_rect().width();
    if let Some(keys) = commands.keys(Command::Palette) {
        let (width_id, height_id) = (Id::new("search-keys-width"), Id::new("search-keys-height"));
        let size = vec2(
            widgets::remembered_width(ui, width_id),
            widgets::remembered_width(ui, height_id),
        );
        let place =
            Rect::from_center_size(pos2(inside.right() - size.x / 2.0, inside.center().y), size);
        let mut trailing = ui.new_child(
            UiBuilder::new()
                .max_rect(place)
                .layout(Layout::left_to_right(Align::Min)),
        );
        trailing.spacing_mut().interact_size.y = 0.0;
        let caps = widgets::key_cap(&mut trailing, &keys);
        widgets::remember_width(ui, width_id, caps.rect.width());
        widgets::remember_width(ui, height_id, caps.rect.height());
        needed += SPACE_M + caps.rect.width();
    }
    widgets::remember_width(ui, content_id, needed);
    if response.clicked() {
        commands.trigger(Command::Palette);
    }
    response
}
