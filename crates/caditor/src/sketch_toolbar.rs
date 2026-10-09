use std::sync::Arc;

use caditor_document::{Feature, FeatureId};
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Id, Label, Layout, Margin, Popup, Rect, Response,
    RichText, Stroke, TextStyle, TextWrapMode, Ui, Vec2, WidgetText, vec2,
};

use crate::{
    annotations,
    appearance::{self, CONTROL_HEIGHT, ICON_SIZE, SPACE_M, SPACE_S, Tokens, WIDGET_RADIUS},
    commands::{Command, CommandFrame},
    constraint_trial::ConstraintTrial,
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    fonts, icons,
    model::{Action, Model, Notice},
    panels::{Focus, PanelState},
    ribbon::{self, explained},
    selection::Selection,
    shape_modes::{ShapeMode, ShapeModes},
    sketch_drag::{self, Moving},
    sketch_status::{self, SketchSummary},
    sketch_tools::{
        self, ActivityChange, BreakChange, ConstraintTool, ConstructionChange, SplitChange,
    },
    units::Units,
    widgets::{self, ToolButton},
};

pub const FINISH_LABEL: &str = "Finish sketch";
pub const ARC_LABEL: &str = "Arc";
pub const ARC_WAYS_LABEL: &str = "Ways to draw an arc";
pub const ARC_TOOLS: [Tool; 3] = [Tool::Arc, Tool::ThreePointArc, Tool::TangentArc];
pub const CURVE_LABEL: &str = "Curve";
pub const CURVE_WAYS_LABEL: &str = "Ways to draw a curve";
pub const CURVE_TOOLS: [Tool; 4] = [
    Tool::Spline,
    Tool::Ellipse,
    Tool::EllipticalArc,
    Tool::Conic,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupChoice {
    Tool(Tool),
    Way(ShapeMode),
}
const ARC_GROUP: ToolGroup = ToolGroup {
    tools: &ARC_TOOLS,
    label: ARC_LABEL,
    ways: ARC_WAYS_LABEL,
    id: "sketch-bar-arc",
};
const CURVE_GROUP: ToolGroup = ToolGroup {
    tools: &CURVE_TOOLS,
    label: CURVE_LABEL,
    ways: CURVE_WAYS_LABEL,
    id: "sketch-bar-curve",
};
const TOOL_GROUPS: [ToolGroup; 2] = [ARC_GROUP, CURVE_GROUP];

struct ToolGroup {
    tools: &'static [Tool],
    label: &'static str,
    ways: &'static str,
    id: &'static str,
}

impl ToolGroup {
    fn of(tool: Tool) -> Option<&'static ToolGroup> {
        TOOL_GROUPS.iter().find(|group| group.tools.contains(&tool))
    }

    fn leads_with(&self, tool: Tool) -> bool {
        self.tools.first() == Some(&tool)
    }
}
pub const OFF_RIBBON: [Tool; 6] = [
    Tool::Chamfer,
    Tool::Intersect,
    Tool::RectangularPattern,
    Tool::CircularPattern,
    Tool::TangentCircle,
    Tool::BlendCurve,
];
pub const OFF_RIBBON_CONSTRAINTS: [ConstraintTool; 1] = [ConstraintTool::Curvature];
pub const DELETE_LABEL: &str = "Delete";
pub const MOVE_LABEL: &str = "Move";
pub const SELECT_ALL_LABEL: &str = "Select all";
pub const CONSTRUCTION_LABEL: &str = "Construction";
pub const DISABLE_LABEL: &str = "Disable";
pub const ENABLE_LABEL: &str = "Enable";
const SELECT_KEY: &str = "Esc";
const FINISH_KEYS: &str = "Esc with nothing selected";
const NOTHING_TO_DISABLE: &str = "Select a constraint or a dimension to disable or enable it";
const DISABLE_HELP: &str = "A disabled constraint stays in the sketch but no longer holds; a disabled dimension shows the measured value instead";
const ENABLE_HELP: &str = "Make the selected constraints hold again";
pub const REFERENCE_ADDED: &str = "That geometry is already fully determined, so the dimension shows the measured value as a reference; enable it to make it drive instead";
const NOTHING_TO_DELETE: &str = "Select sketch geometry or constraints to delete them";
const DELETE_HELP: &str =
    "Delete the selected geometry and constraints, and the constraints on that geometry";
const MOVE_HELP: &str = "Move the selected geometry to a typed position, or by a typed offset";
const SELECT_ALL_HELP: &str = "Select all of the sketch's geometry";
const START_CONSTRUCTION: &str =
    "Draw construction geometry, which guides the sketch but makes no profile";
const STOP_CONSTRUCTION: &str = "Draw ordinary geometry again";
const MAKE_CONSTRUCTION: &str =
    "Make the selected curves construction geometry, which guides the sketch but makes no profile";
const MAKE_ORDINARY: &str = "Make the selected curves ordinary geometry again";

const RIBBON: &str = "sketch-ribbon";
const HEADER_TEXT_MIN_WIDTH: f32 = 160.0;
const HEADER_TEXT_MAX_WIDTH: f32 = 240.0;
const HEADER_GAP: f32 = SPACE_M;
const HEADER_LINE_GAP: f32 = SPACE_S;
const BADGE_SIDE: f32 = CONTROL_HEIGHT;
const FINISH_HEIGHT: f32 = CONTROL_HEIGHT;
const ACCENT_LINE_WIDTH: f32 = 2.0;
const BAR_MARGIN: Margin = Margin {
    top: ribbon::BAR_MARGIN.top + ACCENT_LINE_WIDTH as i8,
    ..ribbon::BAR_MARGIN
};
const GEOMETRIC_COLUMNS: usize = 6;
const DIMENSION_COLUMNS: usize = 3;

type Offer = (ConstraintTool, Result<Vec<Constraint>, String>);

#[derive(Debug, Clone, PartialEq)]
struct OfferBasis {
    feature: FeatureId,
    selected: Vec<EntityId>,
    revision: u64,
    evaluation: u64,
    sketches: u64,
    units: Units,
}

#[derive(Debug, Clone, Default)]
pub struct ConstraintOffers {
    current: Option<(OfferBasis, Arc<Vec<Offer>>)>,
    #[cfg(test)]
    computations: usize,
}

impl ConstraintOffers {
    fn refresh(
        &mut self,
        model: &Model,
        feature: &Feature,
        definition: &Sketch,
        shown: &Sketch,
        selected: &[EntityId],
    ) -> Arc<Vec<Offer>> {
        let basis = OfferBasis {
            feature: feature.id(),
            selected: selected.to_vec(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            sketches: model.display().sketches.generation(),
            units: model.units(),
        };
        let (_, offers) = match self.current.take() {
            Some((known, offers)) if known == basis => self.current.insert((known, offers)),
            Some(_) | None => {
                #[cfg(test)]
                {
                    self.computations += 1;
                }
                let relations = definition.relations();
                let offers = ConstraintTool::ALL
                    .into_iter()
                    .map(|tool| {
                        let candidates = tool
                            .candidates_among(definition, shown, selected, &relations)
                            .map(|constraints| sketch_tools::in_unit(constraints, basis.units));
                        (tool, candidates)
                    })
                    .collect();
                self.current.insert((basis, Arc::new(offers)))
            }
        };
        Arc::clone(offers)
    }

    #[cfg(test)]
    pub fn computations(&self) -> usize {
        self.computations
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    editing: &SketchEditing,
    selection: &Selection,
    commands: &mut CommandFrame<'_>,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let Some(active) = editing.active() else {
        return;
    };
    let Some(feature) = model.document().feature(active.feature) else {
        return;
    };
    let (Some(definition), Some(shown)) = (feature.kind.sketch(), model.displayed_sketch(feature))
    else {
        return;
    };
    let selected = sketch_tools::selected_entities(selection, feature.id());
    let offers = panels
        .constraint_offers
        .refresh(model, feature, definition, &shown, &selected);
    let deletable = Deletable {
        entities: selected
            .iter()
            .copied()
            .filter(|entity| !entity.is_reference())
            .collect(),
        constraints: sketch_tools::selected_constraints(selection, feature.id()),
    };
    let construction = ConstructionChange::of(definition, &selected);
    let split = SplitChange::of(&shown, &selected);
    let breaking = BreakChange::of(&shown, &selected);
    let activity = ActivityChange::of(
        definition,
        &sketch_tools::selected_constraints(selection, feature.id()),
    );
    let moving = Moving::offered(&shown, feature.id(), &selected, active.tool.draws())
        .map(|_| ())
        .or_else(|reason| {
            annotations::label_to_move(
                definition,
                &selected,
                &sketch_tools::selected_constraints(selection, feature.id()),
                active.tool.draws(),
            )
            .map(|_| ())
            .map_err(|_| reason)
        });
    let select_all = sketch_drag::can_select_all(&shown).map_err(str::to_owned);

    let tokens = appearance::tokens(ui);
    let mut bar = Bar {
        model,
        feature,
        active,
        modes: editing.modes(),
        offers: &offers,
        construction: construction.as_ref(),
        activity: activity.as_ref(),
        split: split.as_ref().map(|_| ()).map_err(String::clone),
        breaking: breaking.as_ref().map(|_| ()).map_err(String::clone),
        deletable: if deletable.is_empty() {
            Err(NOTHING_TO_DELETE.to_owned())
        } else {
            Ok(())
        },
        moving,
        select_all,
        commands,
        request: Request::default(),
    };
    let frame = Frame::side_top_panel(ui.style())
        .fill(tokens.accent_surface)
        .inner_margin(BAR_MARGIN);
    let panel = egui::Panel::top("sketch-toolbar")
        .frame(frame)
        .show(ui, |ui| bar.show(ui));
    let top = panel.response.rect.top() + ACCENT_LINE_WIDTH / 2.0;
    ui.painter().hline(
        panel.response.rect.x_range(),
        top,
        Stroke::new(ACCENT_LINE_WIDTH, tokens.accent_text),
    );
    let request = bar.request;

    if let Some(focus) = request.focus {
        panels.request_focus(focus);
    }
    if let Some(tool) = request.tool {
        actions.push(Action::Editing(EditingCommand::SetTool(tool)));
    }
    if let Some(mode) = request.mode {
        actions.push(Action::Editing(EditingCommand::SetMode(mode)));
    }
    if let Some((tool, constraints)) = request.constraints {
        let added = sketch_tools::add_constraints(model, feature.id(), tool, constraints);
        if tool.is_dimension()
            && let Some(constraint) = added.constraints.first()
            && !added.references.contains(constraint)
        {
            panels.request_focus(Focus::Dimension {
                feature: feature.id(),
                constraint: *constraint,
            });
        }
        if tool.is_dimension() {
            actions.push(Action::Apply(added.transaction));
        } else {
            actions.push(Action::Trial(ConstraintTrial {
                feature: feature.id(),
                added: added
                    .constraints
                    .iter()
                    .copied()
                    .filter(|constraint| !added.references.contains(constraint))
                    .collect(),
                transaction: added.transaction,
            }));
        }
        if !added.references.is_empty() {
            actions.push(Action::Inform(Notice::info(REFERENCE_ADDED)));
        }
    }
    if request.construction {
        actions.push(match &construction {
            Some(change) => Action::Apply(change.transaction(model, feature.id(), definition)),
            None => Action::Editing(EditingCommand::DrawConstruction(!active.construction)),
        });
    }
    if request.activity
        && let Some(change) = &activity
    {
        actions.push(Action::Apply(change.transaction(
            model,
            feature.id(),
            definition,
        )));
    }
    if request.split
        && let Ok(change) = &split
    {
        actions.push(match change.transaction(model, feature.id()) {
            Ok(transaction) => Action::Apply(transaction),
            Err(reason) => Action::Inform(Notice::warning(reason)),
        });
    }
    if request.breaking
        && let Ok(change) = &breaking
    {
        actions.push(match change.transaction(model, feature.id()) {
            Ok(transaction) => Action::Apply(transaction),
            Err(reason) => Action::Inform(Notice::warning(reason)),
        });
    }
    if request.delete && !deletable.is_empty() {
        let label = deletable.label(definition);
        actions.push(Action::Apply(sketch_tools::remove_items(
            model,
            feature.id(),
            label,
            deletable.entities,
            deletable.constraints,
        )));
    }
    if request.finish {
        actions.push(Action::Editing(EditingCommand::Finish));
    }
}

struct Deletable {
    entities: Vec<EntityId>,
    constraints: Vec<ConstraintId>,
}

impl Deletable {
    fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.constraints.is_empty()
    }

    fn label(&self, sketch: &Sketch) -> String {
        match (self.entities.as_slice(), self.constraints.as_slice()) {
            ([only], []) => format!("Delete {}", sketch.entity_label(*only)),
            ([], [only]) => format!("Delete {}", sketch.describe_constraint(*only)),
            (entities, constraints) => format!(
                "Delete {}",
                count(entities.len() + constraints.len(), "item", "items")
            ),
        }
    }
}

#[derive(Default)]
struct Request {
    focus: Option<Focus>,
    tool: Option<Tool>,
    mode: Option<ShapeMode>,
    constraints: Option<(ConstraintTool, Vec<Constraint>)>,
    construction: bool,
    activity: bool,
    split: bool,
    breaking: bool,
    delete: bool,
    finish: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Header,
    Select,
    Draw,
    Modify,
    Constrain,
    Dimension,
}

impl Group {
    const ALL: [Self; 6] = [
        Self::Header,
        Self::Select,
        Self::Draw,
        Self::Modify,
        Self::Constrain,
        Self::Dimension,
    ];

    fn caption(self) -> Option<&'static str> {
        match self {
            Self::Header => None,
            Self::Select => Some("Select"),
            Self::Draw => Some("Draw"),
            Self::Modify => Some("Modify"),
            Self::Constrain => Some("Constrain"),
            Self::Dimension => Some("Dimension"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Packing {
    header: f32,
    captions: bool,
}

struct Bar<'a, 'b> {
    model: &'a Model,
    feature: &'a Feature,
    active: ActiveSketch,
    modes: ShapeModes,
    offers: &'a [Offer],
    construction: Option<&'a ConstructionChange>,
    activity: Option<&'a ActivityChange>,
    split: Result<(), String>,
    breaking: Result<(), String>,
    deletable: Result<(), String>,
    moving: Result<(), String>,
    select_all: Result<(), String>,
    commands: &'a mut CommandFrame<'b>,
    request: Request,
}

impl Bar<'_, '_> {
    fn show(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = Vec2::splat(ribbon::TOOL_GAP);
        let full = ui.available_width();
        let band = widgets::tool_height(ui);
        let header = self.header_width(ui).min(full);
        let finish_id = Id::new("sketch-bar-finish");
        let widths: Vec<(Group, f32)> = ribbon::remembered_widths(ui, RIBBON, &Group::ALL)
            .into_iter()
            .map(|(group, width)| match group {
                Group::Header => (group, header),
                _ => (group, width),
            })
            .collect();
        let beside_finish = full - widgets::remembered_width(ui, finish_id) - ribbon::GROUP_SPACE;
        let finish_apart = beside_finish < header;
        if finish_apart {
            ui.allocate_ui_with_layout(
                vec2(full, FINISH_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| self.finish(ui, finish_id),
            );
            ui.add_space(ribbon::ROW_GAP);
        }
        let first = if finish_apart { full } else { beside_finish };
        let rows = ribbon::rows(&widths, first, full);
        let packing = Packing {
            header,
            captions: rows.len() == 1 && !finish_apart,
        };
        for (index, row) in rows.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(ribbon::ROW_GAP);
            }
            ui.horizontal_top(|ui| {
                ribbon::row(ui, RIBBON, &row, |ui, group| self.group(ui, group, packing));
                if index == 0 && !finish_apart {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), band),
                        Layout::right_to_left(Align::Center),
                        |ui| self.finish(ui, finish_id),
                    );
                }
            });
        }
    }

    fn group(&mut self, ui: &mut Ui, group: Group, packing: Packing) -> (Rect, f32) {
        let caption = group.caption().filter(|_| packing.captions);
        ribbon::captioned(ui, caption, |ui| self.content(ui, group, packing))
    }

    fn content(&mut self, ui: &mut Ui, group: Group, packing: Packing) -> f32 {
        match group {
            Group::Header => self.header(ui, packing),
            Group::Select => {
                let select = Tool::Select;
                self.tool_button(ui, select, select.label()).rect.width()
            }
            Group::Draw => self.drawing_tools(ui),
            Group::Modify => self.edit_buttons(ui),
            Group::Constrain => self.constraint_buttons(ui, false, GEOMETRIC_COLUMNS),
            Group::Dimension => self.dimension_buttons(ui),
        }
    }

    fn title(&self) -> String {
        format!("Editing {}", self.feature.name)
    }

    fn header_width(&self, ui: &Ui) -> f32 {
        let title = WidgetText::from(RichText::new(self.title()).text_style(TextStyle::Button))
            .into_galley(
                ui,
                Some(TextWrapMode::Extend),
                f32::INFINITY,
                TextStyle::Button,
            );
        let text = title
            .size()
            .x
            .clamp(HEADER_TEXT_MIN_WIDTH, HEADER_TEXT_MAX_WIDTH);
        BADGE_SIDE + HEADER_GAP + text
    }

    fn header(&mut self, ui: &mut Ui, packing: Packing) -> f32 {
        let tokens = appearance::tokens(ui);
        let width = packing.header;
        let height = ribbon::height(ui, packing.captions);
        let title = self.title();
        ui.horizontal_top(|ui| {
            ui.set_min_size(vec2(width, height));
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.x = HEADER_GAP;
            ui.vertical(|ui| {
                ui.add_space(((height - BADGE_SIDE) / 2.0).max(0.0));
                badge(ui, tokens);
            });
            let height_id = Id::new("sketch-bar-header-height");
            let known = widgets::remembered_width(ui, height_id);
            let content = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = HEADER_LINE_GAP;
                    ui.add_space(((height - known) / 2.0).max(0.0));
                    let top = ui.cursor().min.y;
                    ui.add(
                        Label::new(
                            RichText::new(&title)
                                .text_style(TextStyle::Button)
                                .color(tokens.text),
                        )
                        .truncate()
                        .selectable(false),
                    );
                    ui.horizontal_wrapped(|ui| {
                        let summary = SketchSummary::of(self.model.evaluation(), self.feature.id());
                        if let Some(focus) = sketch_status::show(ui, &summary) {
                            self.request.focus = Some(focus);
                        }
                    });
                    ui.min_rect().bottom() - top
                })
                .inner;
            widgets::remember_width(ui, height_id, content);
        });
        width
    }

    fn tool_button(&mut self, ui: &mut Ui, tool: Tool, label: &str) -> Response {
        let command = Command::SketchTool(tool);
        let invoked = self.commands.available(command);
        let keys = match (self.commands.keys(command), tool) {
            (Some(keys), _) => Some(keys),
            (None, Tool::Select) => Some(SELECT_KEY.to_owned()),
            (None, _) => None,
        };
        let mode = self.modes.of(tool);
        let description = mode.map_or(tool.description(), ShapeMode::description);
        let hover = match (keys, mode) {
            (Some(keys), Some(mode)) => format!(
                "{description} ({keys})\n{keys} again: {}",
                mode.next().label().to_lowercase()
            ),
            (Some(keys), None) => format!("{description} ({keys})"),
            (None, _) => description.to_owned(),
        };
        let active = self.active.tool == tool;
        let button = ToolButton::new(icons::tool(tool), label).selected(active);
        let response = ui.add(button).on_hover_text(hover);
        match mode.filter(|_| invoked && active) {
            Some(mode) => self.request.mode = Some(mode.next()),
            None if response.clicked() || invoked => self.request.tool = Some(tool),
            None => {}
        }
        response
    }

    fn off_ribbon_tool(&mut self, tool: Tool) {
        let invoked = self.commands.available(Command::SketchTool(tool));
        for mode in ShapeMode::of_tool(tool) {
            if self.commands.available(Command::ShapeMode(mode)) {
                self.request.mode = Some(mode);
            }
        }
        let active = self.active.tool == tool;
        match self.modes.of(tool).filter(|_| invoked && active) {
            Some(mode) => self.request.mode = Some(mode.next()),
            None if invoked => self.request.tool = Some(tool),
            None => {}
        }
    }

    fn mode_menu(&mut self, ui: &mut Ui, current: ShapeMode, button: Rect) {
        let tool = current.tool();
        for mode in ShapeMode::of_tool(tool) {
            if self.commands.available(Command::ShapeMode(mode)) {
                self.request.mode = Some(mode);
            }
        }
        let name = modes_label(tool);
        let id = Id::new(("sketch-bar-modes", tool));
        let selected = self.active.tool == tool;
        let response = widgets::corner_menu_button(ui, id, button, &name, selected);
        let commands = &*self.commands;
        let shown = Popup::menu(&response).show(|ui| {
            widgets::fitted_menu(ui, |ui| {
                let mut chosen = None;
                for mode in ShapeMode::of_tool(tool) {
                    let keys = commands.keys(Command::ShapeMode(mode));
                    let glyph = icons::shape_mode(mode);
                    if widgets::menu_choice(ui, glyph, mode.label(), keys, mode == current)
                        .clicked()
                    {
                        chosen = Some(mode);
                    }
                }
                chosen
            })
        });
        if let Some(mode) = shown.and_then(|shown| shown.inner) {
            self.request.mode = Some(mode);
        }
    }

    fn compact_tool_button(&mut self, ui: &mut Ui, tool: Tool) {
        let command = Command::SketchTool(tool);
        let invoked = self.commands.available(command);
        let button = ToolButton::new(icons::tool(tool), tool.label())
            .compact()
            .selected(self.active.tool == tool);
        let help = Ok(self.commands.with_keys(command, tool.description()));
        let response = explained(ui.add(button), tool.label(), &help);
        if response.clicked() || invoked {
            self.request.tool = Some(tool);
        }
    }

    fn drawing_tools(&mut self, ui: &mut Ui) -> f32 {
        ui.horizontal_wrapped(|ui| {
            Tool::ALL
                .into_iter()
                .filter(|tool| {
                    tool.draws() && ToolGroup::of(*tool).is_none_or(|group| group.leads_with(*tool))
                })
                .map(|tool| {
                    if let Some(group) = ToolGroup::of(tool) {
                        return self.group_button(ui, group);
                    }
                    let button = self.tool_button(ui, tool, tool.label()).rect;
                    if let Some(mode) = self.modes.of(tool) {
                        self.mode_menu(ui, mode, button);
                    }
                    button.width()
                })
                .reduce(|total, width| total + ribbon::TOOL_GAP + width)
                .unwrap_or_default()
        })
        .inner
    }

    fn group_button(&mut self, ui: &mut Ui, group: &ToolGroup) -> f32 {
        let remembered = Id::new(group.id);
        let first = group.tools.first().copied().unwrap_or(Tool::Arc);
        let current = if group.tools.contains(&self.active.tool) {
            ui.data_mut(|data| data.insert_temp(remembered, self.active.tool));
            self.active.tool
        } else {
            ui.data(|data| data.get_temp::<Tool>(remembered))
                .unwrap_or(first)
        };
        for tool in group.tools.iter().copied() {
            if tool != current && self.commands.available(Command::SketchTool(tool)) {
                self.request.tool = Some(tool);
            }
            for mode in ShapeMode::of_tool(tool) {
                if self.commands.available(Command::ShapeMode(mode)) {
                    self.request.mode = Some(mode);
                }
            }
        }
        let button = self.tool_button(ui, current, group.label).rect;
        let current_mode = self.modes.of(current);
        let id = Id::new(group.id).with("ways");
        let selected = group.tools.contains(&self.active.tool);
        let response = widgets::corner_menu_button(ui, id, button, group.ways, selected);
        let commands = &*self.commands;
        let shown = Popup::menu(&response).show(|ui| {
            widgets::fitted_menu(ui, |ui| {
                let mut chosen = None;
                for tool in group.tools.iter().copied() {
                    let keys = commands.keys(Command::SketchTool(tool));
                    let glyph = icons::tool(tool);
                    if widgets::menu_choice(ui, glyph, tool.label(), keys, tool == current)
                        .clicked()
                    {
                        chosen = Some(GroupChoice::Tool(tool));
                    }
                }
                if let Some(current_mode) = current_mode {
                    ui.separator();
                    for mode in ShapeMode::of_tool(current) {
                        let keys = commands.keys(Command::ShapeMode(mode));
                        let glyph = icons::shape_mode(mode);
                        if widgets::menu_choice(ui, glyph, mode.label(), keys, mode == current_mode)
                            .clicked()
                        {
                            chosen = Some(GroupChoice::Way(mode));
                        }
                    }
                }
                chosen
            })
        });
        match shown.and_then(|shown| shown.inner) {
            Some(GroupChoice::Tool(tool)) => {
                ui.data_mut(|data| data.insert_temp(remembered, tool));
                self.request.tool = Some(tool);
            }
            Some(GroupChoice::Way(mode)) => self.request.mode = Some(mode),
            None => {}
        }
        button.width()
    }

    fn edit_buttons(&mut self, ui: &mut Ui) -> f32 {
        if self.commands.invoke(Command::SplitCurve, &self.split) {
            self.request.split = true;
        }
        if self.commands.invoke(Command::BreakCurves, &self.breaking) {
            self.request.breaking = true;
        }
        for tool in OFF_RIBBON {
            self.off_ribbon_tool(tool);
        }
        let first = ui.horizontal_top(|ui| {
            self.construction_button(ui);
            self.compact_tool_button(ui, Tool::Trim);
            self.compact_tool_button(ui, Tool::Extend);
            self.compact_tool_button(ui, Tool::Fillet);
            self.compact_tool_button(ui, Tool::Offset);
        });
        let second = ui.horizontal_top(|ui| {
            self.compact_tool_button(ui, Tool::Mirror);
            self.compact_tool_button(ui, Tool::Project);
            let moving = self.moving.clone();
            if self.command_button(ui, Command::MoveGeometry, MOVE_LABEL, MOVE_HELP, &moving) {
                self.commands.trigger(Command::MoveGeometry);
            }
            let everything = self.select_all.clone();
            if self.command_button(
                ui,
                Command::SelectAll,
                SELECT_ALL_LABEL,
                SELECT_ALL_HELP,
                &everything,
            ) {
                self.commands.trigger(Command::SelectAll);
            }
            self.delete_button(ui);
        });
        first
            .response
            .rect
            .width()
            .max(second.response.rect.width())
    }

    fn activity_button(&mut self, ui: &mut Ui) {
        let availability = self
            .activity
            .map(|_| ())
            .ok_or_else(|| NOTHING_TO_DISABLE.to_owned());
        let invoked = self
            .commands
            .invoke(Command::ToggleConstraintActive, &availability);
        let enabling = self.activity.is_some_and(|change| change.active);
        let (label, help) = if enabling {
            (ENABLE_LABEL, ENABLE_HELP)
        } else {
            (DISABLE_LABEL, DISABLE_HELP)
        };
        let button =
            ToolButton::new(icons::command(Command::ToggleConstraintActive), label).compact();
        let response = ui.add_enabled(availability.is_ok(), button);
        let described = match &availability {
            Ok(()) => Ok(self
                .commands
                .with_keys(Command::ToggleConstraintActive, help)),
            Err(reason) => Err(self.commands.with_keys(
                Command::ToggleConstraintActive,
                &format!("{help}. {reason}"),
            )),
        };
        if explained(response, label, &described).clicked() || invoked {
            self.request.activity = true;
        }
    }

    fn construction_button(&mut self, ui: &mut Ui) {
        let invoked = self.commands.available(Command::Construction);
        let description = match self.construction {
            Some(change) if change.construction => MAKE_CONSTRUCTION,
            Some(_) => MAKE_ORDINARY,
            None if self.active.construction => STOP_CONSTRUCTION,
            None => START_CONSTRUCTION,
        };
        let button = ToolButton::new(icons::CONSTRUCTION, CONSTRUCTION_LABEL)
            .compact()
            .selected(self.active.construction);
        let help = Ok(self.commands.with_keys(Command::Construction, description));
        let response = explained(ui.add(button), CONSTRUCTION_LABEL, &help);
        if response.clicked() || invoked {
            self.request.construction = true;
        }
    }

    fn command_button(
        &mut self,
        ui: &mut Ui,
        command: Command,
        label: &str,
        help: &str,
        availability: &Result<(), String>,
    ) -> bool {
        let button = ToolButton::new(icons::command(command), label).compact();
        let response = ui.add_enabled(availability.is_ok(), button);
        let help = match availability {
            Ok(()) => Ok(self.commands.with_keys(command, help)),
            Err(reason) => Err(self
                .commands
                .with_keys(command, &format!("{help}. {reason}"))),
        };
        explained(response, label, &help).clicked()
    }

    fn delete_button(&mut self, ui: &mut Ui) {
        let invoked = self
            .commands
            .invoke(Command::DeleteSelection, &self.deletable);
        let enabled = self.deletable.is_ok();
        let button = ToolButton::new(icons::DELETE, DELETE_LABEL).compact();
        let response = ui.add_enabled(enabled, button);
        let help = if enabled {
            Ok(self
                .commands
                .with_keys(Command::DeleteSelection, DELETE_HELP))
        } else {
            Err(NOTHING_TO_DELETE.to_owned())
        };
        let response = explained(response, DELETE_LABEL, &help);
        if response.clicked() || invoked {
            self.request.delete = true;
        }
    }

    fn constraint_buttons(&mut self, ui: &mut Ui, dimensions: bool, columns: usize) -> f32 {
        let offers = self.offers;
        let (off_ribbon, offers): (Vec<&Offer>, Vec<&Offer>) = offers
            .iter()
            .filter(|(tool, _)| tool.is_dimension() == dimensions)
            .partition(|(tool, _)| OFF_RIBBON_CONSTRAINTS.contains(tool));
        for (tool, offer) in off_ribbon {
            if self.commands.invoke(Command::Constraint(*tool), offer)
                && let Ok(constraints) = offer
            {
                self.request.constraints = Some((*tool, constraints.clone()));
            }
        }
        offers
            .chunks(columns.max(1))
            .map(|row| {
                ui.horizontal_top(|ui| {
                    for (tool, offer) in row {
                        self.constraint_button(ui, *tool, offer);
                    }
                })
                .response
                .rect
                .width()
            })
            .fold(0.0, f32::max)
    }

    fn dimension_buttons(&mut self, ui: &mut Ui) -> f32 {
        let offers: Vec<&Offer> = self
            .offers
            .iter()
            .filter(|(tool, _)| tool.is_dimension())
            .collect();
        let (first, second) = offers.split_at(DIMENSION_COLUMNS.min(offers.len()));
        let top = ui.horizontal_top(|ui| {
            self.compact_tool_button(ui, Tool::Dimension);
            for (tool, offer) in first {
                self.constraint_button(ui, *tool, offer);
            }
        });
        let bottom = ui.horizontal_top(|ui| {
            for (tool, offer) in second {
                self.constraint_button(ui, *tool, offer);
            }
            self.activity_button(ui);
        });
        top.response.rect.width().max(bottom.response.rect.width())
    }

    fn constraint_button(
        &mut self,
        ui: &mut Ui,
        tool: ConstraintTool,
        offer: &Result<Vec<Constraint>, String>,
    ) {
        let command = Command::Constraint(tool);
        let invoked = self.commands.invoke(command, offer);
        let button = ToolButton::new(icons::constraint(tool), tool.label()).compact();
        let response = ui.add_enabled(offer.is_ok(), button);
        let help = match offer {
            Ok(_) => Ok(self.commands.with_keys(command, tool.description())),
            Err(reason) => Err(self
                .commands
                .with_keys(command, &format!("{}. {reason}", tool.description()))),
        };
        let response = explained(response, tool.label(), &help);
        if (response.clicked() || invoked)
            && let Ok(constraints) = offer
        {
            self.request.constraints = Some((tool, constraints.clone()));
        }
    }

    fn finish(&mut self, ui: &mut Ui, width_id: Id) {
        let invoked = self.commands.available(Command::FinishSketch);
        let keys = self
            .commands
            .keys(Command::FinishSketch)
            .unwrap_or_else(|| FINISH_KEYS.to_owned());
        let button =
            widgets::primary_icon_button(ui, icons::command(Command::FinishSketch), FINISH_LABEL)
                .min_size(vec2(0.0, FINISH_HEIGHT));
        let response = ui
            .add(widgets::Named::new(button, FINISH_LABEL))
            .on_hover_text(format!("Leave the sketch. Everything is kept. ({keys})"));
        let look = Command::LookAtSketch;
        let look_help = self.commands.with_keys(look, &look.title());
        let looking = widgets::icon_button(ui, icons::command(look), &look_help);
        let both = response.rect.union(looking.rect);
        widgets::remember_width(ui, width_id, both.width());
        if response.clicked() || invoked {
            self.request.finish = true;
        }
        if looking.clicked() {
            self.commands.trigger(look);
        }
    }
}

pub fn modes_label(tool: Tool) -> String {
    format!("Ways to draw a {}", tool.label().to_lowercase())
}

fn badge(ui: &mut Ui, tokens: &Tokens) {
    let (_, rect) = ui.allocate_space(Vec2::splat(BADGE_SIDE));
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(WIDGET_RADIUS),
        tokens.accent_subtle,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icons::SKETCH,
        FontId::new(ICON_SIZE, fonts::icons()),
        tokens.accent_text,
    );
}
