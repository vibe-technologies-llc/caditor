use std::sync::Arc;

use caditor_document::{Feature, FeatureId};
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{
    Align, Align2, CornerRadius, FontId, Id, Label, Layout, Margin, Popup, Rect, Response,
    RichText, Stroke, TextStyle, TextWrapMode, Ui, Vec2, WidgetText, vec2,
};

use crate::{
    annotations,
    appearance::{
        self, CONTROL_HEIGHT, ICON_SIZE, SPACE_M, SPACE_S, SPACE_XS, Tokens, WIDGET_RADIUS,
    },
    body_picks::{self, BodyPick, Projections, Staged},
    body_snap::BodySnaps,
    commands::{self, Command, CommandFrame},
    constraint_trial::ConstraintTrial,
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    fonts, icons,
    menu_bar::MenuEntries,
    model::{Action, Model, Notice},
    panels::{Focus, PanelState},
    queued,
    ribbon::{self, GroupMenu, GroupTools, explained},
    section::SectionCommand,
    selection::Selection,
    shape_modes::{ShapeMode, ShapeModes},
    sketch_drag::{self, Moving},
    sketch_status::{self, SketchSummary, StatusRequest},
    sketch_tools::{
        self, ActivityChange, BreakChange, ConstraintTool, ConstructionChange, RespaceChange,
        SplitChange,
    },
    toggles::ToggleStates,
    units::Units,
    widgets::{self, CornerMenu, ToolButton},
};

pub const FINISH_LABEL: &str = "Finish sketch";
pub const CANCEL_LABEL: &str = "Cancel sketch";
const CANCEL_HELP: &str = "Leave the sketch and take back every change made since it was entered";
pub const ARC_LABEL: &str = "Arc";
pub const ARC_WAYS_LABEL: &str = "Ways to draw an arc";
pub const ARC_TOOLS: [Tool; 3] = [Tool::Arc, Tool::ThreePointArc, Tool::TangentArc];
pub const CURVE_LABEL: &str = "Curve";
pub const CURVE_WAYS_LABEL: &str = "Ways to draw a curve";
pub const POLYGON_LABEL: &str = "Polygon";
pub const POLYGON_WAYS_LABEL: &str = "Ways to draw a polygon";
pub const POLYGON_TOOLS: [Tool; 2] = [Tool::Polygon, Tool::Gear];
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
const POLYGON_GROUP: ToolGroup = ToolGroup {
    tools: &POLYGON_TOOLS,
    label: POLYGON_LABEL,
    ways: POLYGON_WAYS_LABEL,
    id: "sketch-bar-polygon",
};
const TOOL_GROUPS: [ToolGroup; 3] = [ARC_GROUP, POLYGON_GROUP, CURVE_GROUP];

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
pub const OFF_RIBBON: [Tool; 5] = [
    Tool::Intersect,
    Tool::RectangularPattern,
    Tool::CircularPattern,
    Tool::TangentCircle,
    Tool::BlendCurve,
];
pub const OFF_RIBBON_CONSTRAINTS: [ConstraintTool; 1] = [ConstraintTool::Curvature];
pub const CURVE_FROM_GEOMETRY_TOOLS: Partners = Partners {
    host: Tool::Offset,
    others: &[Tool::TangentCircle, Tool::BlendCurve],
    name: "Draw from existing curves",
};
pub const COPYING_TOOLS: Partners = Partners {
    host: Tool::Mirror,
    others: &[Tool::RectangularPattern, Tool::CircularPattern],
    name: "Mirror or repeat geometry",
};
pub const MODEL_GEOMETRY_TOOLS: Partners = Partners {
    host: Tool::Project,
    others: &[Tool::Intersect],
    name: "Bring in model geometry",
};

pub struct Partners {
    host: Tool,
    others: &'static [Tool],
    pub name: &'static str,
}

impl Partners {
    pub fn tools(&self) -> Vec<Tool> {
        std::iter::once(self.host)
            .chain(self.others.iter().copied())
            .collect()
    }

    pub fn contents(&self) -> String {
        menu_contents(&self.tools(), Some(self.host))
    }
}

fn menu_contents(tools: &[Tool], ways_of: Option<Tool>) -> String {
    let named: Vec<&str> = tools.iter().map(|tool| tool.label()).collect();
    let listed = ribbon::listed(&named);
    let ways: Vec<&str> = ways_of
        .into_iter()
        .flat_map(ShapeMode::of_tool)
        .map(ShapeMode::label)
        .collect();
    if ways.is_empty() {
        listed
    } else {
        format!("{listed}. Ways: {}", ribbon::listed(&ways))
    }
}

fn ways_listed(tool: Tool) -> String {
    let ways: Vec<&str> = ShapeMode::of_tool(tool).map(ShapeMode::label).collect();
    ribbon::listed(&ways)
}
pub const DELETE_LABEL: &str = "Delete";
pub const MOVE_LABEL: &str = "Move";
pub const SELECT_ALL_LABEL: &str = "Select all";
pub const CONSTRUCTION_LABEL: &str = "Construction";
pub const DISABLE_LABEL: &str = "Disable";
pub const ENABLE_LABEL: &str = "Enable";
const SELECT_KEY: &str = "Esc once nothing is being drawn or picked";
const FINISH_KEYS: &str = "Esc with Select active and nothing selected";
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

const NOTHING_TO_UNDO: &str = "Nothing to undo";
const NOTHING_TO_REDO: &str = "Nothing to redo";

const RIBBON: &str = "sketch-ribbon";
const COMPACT_RIBBON: &str = "sketch-ribbon-compact";
const COMPACT_DRAW_ROWS: usize = 2;
const HEADER_TEXT_MIN_WIDTH: f32 = 160.0;
const HEADER_TEXT_MAX_WIDTH: f32 = 240.0;
const HEADER_GAP: f32 = SPACE_M;
const HEADER_LINE_GAP: f32 = SPACE_S;
const HEADER_COUNT_GAP: f32 = SPACE_XS;
const BADGE_SIDE: f32 = CONTROL_HEIGHT;
const FINISH_HEIGHT: f32 = CONTROL_HEIGHT;
const FINISH_MARGIN: f32 = SPACE_XS;
const ACCENT_LINE_WIDTH: f32 = 2.0;
const BAR_MARGIN: Margin = Margin {
    top: ribbon::BAR_MARGIN.top + ACCENT_LINE_WIDTH as i8,
    ..ribbon::BAR_MARGIN
};
const GEOMETRIC_COLUMNS: usize = 6;
const DIMENSION_COLUMNS: usize = 3;
const DRAW_MENU_HOST: Tool = Tool::Line;
const HISTORY_MENU: &[&[Command]] = &[&[Command::CancelSketch]];
const SELECT_MENU: &[&[Command]] = &[&[
    Command::SelectAll,
    Command::SelectFree,
    Command::SelectOpenEnds,
    Command::SelectRedundant,
]];
const DRAW_MENU: &[&[Command]] = &[
    &[
        Command::ChainArc,
        Command::ReverseArc,
        Command::MoreSides,
        Command::FewerSides,
    ],
    &[
        Command::TypeValue,
        Command::ToggleTypedDimensions,
        Command::DrawSymmetrically,
        Command::Section(SectionCommand::SliceSketch),
    ],
];
const MODIFY_MENU: &[&[Command]] = &[
    &[
        Command::SplitCurve,
        Command::BreakCurves,
        Command::RespaceFitSplines,
    ],
    &[
        Command::MoveGeometry,
        Command::RotateGeometry,
        Command::ScaleGeometry,
    ],
    &[
        Command::CopyGeometry,
        Command::CutGeometry,
        Command::PasteGeometry,
        Command::DeleteSelection,
    ],
    &[Command::ProjectSketch, Command::IntersectBody],
];
const CONSTRAIN_MENU: &[&[Command]] = &[&[Command::FindRelations, Command::CheckSketch]];
const DIMENSION_MENU: &[&[Command]] = &[
    &[Command::ToggleConstraintActive, Command::DimensionFromDatum],
    &[Command::ToggleFirstDimensionScales],
];

pub fn group_menu_name(caption: &str) -> String {
    format!("Sketch {} commands", caption.to_lowercase())
}

fn menu_sections(group: Group) -> Vec<Vec<Command>> {
    let tools: Vec<Tool> = Tool::ALL
        .into_iter()
        .filter(|tool| match group {
            Group::Draw => tool.draws() || *tool == Tool::Gear,
            Group::Modify => {
                !tool.draws() && !matches!(tool, Tool::Select | Tool::Gear | Tool::Dimension)
            }
            _ => false,
        })
        .collect();
    let tool_commands = tools.iter().copied().map(Command::SketchTool);
    let (lead, rest): (Vec<Command>, &[&[Command]]) = match group {
        Group::Header => return Vec::new(),
        Group::History => (
            vec![
                Command::Undo,
                Command::Redo,
                Command::RepeatLast,
                Command::UndoHistory,
            ],
            HISTORY_MENU,
        ),
        Group::Select => (vec![Command::SketchTool(Tool::Select)], SELECT_MENU),
        Group::Draw => (tool_commands.collect(), DRAW_MENU),
        Group::Modify => (
            std::iter::once(Command::Construction)
                .chain(tool_commands)
                .collect(),
            MODIFY_MENU,
        ),
        Group::Constrain => (constraint_commands(false).collect(), CONSTRAIN_MENU),
        Group::Dimension => (
            std::iter::once(Command::SketchTool(Tool::Dimension))
                .chain(constraint_commands(true))
                .collect(),
            DIMENSION_MENU,
        ),
    };
    let ways = tools
        .iter()
        .map(|tool| ShapeMode::of_tool(*tool).map(Command::ShapeMode).collect())
        .filter(|ways: &Vec<Command>| !ways.is_empty());
    std::iter::once(lead)
        .chain(ways)
        .chain(rest.iter().map(|section| section.to_vec()))
        .collect()
}

fn constraint_commands(dimensions: bool) -> impl Iterator<Item = Command> {
    ConstraintTool::ALL
        .into_iter()
        .filter(move |tool| tool.is_dimension() == dimensions)
        .map(Command::Constraint)
}

type Offer = (ConstraintTool, Result<Vec<Constraint>, String>);

#[derive(Debug, Clone, PartialEq)]
struct OfferBasis {
    feature: FeatureId,
    selected: Vec<EntityId>,
    bodies: Vec<BodyPick>,
    snaps: u64,
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

fn offers_among(
    definition: &Sketch,
    shown: &Sketch,
    picks: &[EntityId],
    units: Units,
) -> Vec<Offer> {
    let relations = definition.relations();
    ConstraintTool::ALL
        .into_iter()
        .map(|tool| {
            let candidates = tool
                .candidates_among(definition, shown, picks, &relations)
                .map(|constraints| sketch_tools::in_unit(constraints, units));
            (tool, candidates)
        })
        .collect()
}

impl ConstraintOffers {
    fn refresh(
        &mut self,
        model: &Model,
        (feature, definition, shown): (&Feature, &Sketch, &Sketch),
        (selected, bodies): (&[EntityId], &[BodyPick]),
        snaps: &BodySnaps,
    ) -> Arc<Vec<Offer>> {
        let basis = OfferBasis {
            feature: feature.id(),
            selected: selected.to_vec(),
            bodies: bodies.to_vec(),
            snaps: snaps.generation(),
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
                let offers = if bodies.is_empty() {
                    offers_among(definition, shown, selected, basis.units)
                } else {
                    match Staged::new(model, feature.id(), snaps, selected, bodies) {
                        Ok(staged) => offers_among(
                            &staged.definition,
                            &staged.shown,
                            &staged.picks,
                            basis.units,
                        ),
                        Err(unstaged) => ConstraintTool::ALL
                            .into_iter()
                            .map(|tool| (tool, Err(unstaged.to_string())))
                            .collect(),
                    }
                };
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

pub struct BarContext<'a> {
    pub selection: &'a Selection,
    pub snaps: &'a BodySnaps,
    pub last_offers: &'a [commands::Offer],
    pub toggles: ToggleStates,
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    editing: &SketchEditing,
    context: &BarContext<'_>,
    commands: &mut CommandFrame<'_>,
    panels: &mut PanelState,
    actions: &mut Vec<Action>,
) -> Vec<Command> {
    let BarContext {
        selection, snaps, ..
    } = *context;
    let Some(active) = editing.active() else {
        return Vec::new();
    };
    let Some(feature) = model.document().feature(active.feature) else {
        return Vec::new();
    };
    let (Some(definition), Some(shown)) = (feature.kind.sketch(), model.displayed_sketch(feature))
    else {
        return Vec::new();
    };
    let selected = sketch_tools::selected_entities(selection, feature.id());
    let bodies = body_picks::selected(selection, feature.id());
    let offers = panels.constraint_offers.refresh(
        model,
        (feature, definition, &shown),
        (&selected, &bodies),
        snaps,
    );
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
    let respacing = RespaceChange::of(&shown, &selected);
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
    let cancel = editing
        .sketch_cancelling(model)
        .map(|_| ())
        .map_err(|refusal| refusal.to_string());

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
        respacing: respacing.as_ref().map(|_| ()).map_err(String::clone),
        deletable: if deletable.is_empty() {
            Err(NOTHING_TO_DELETE.to_owned())
        } else {
            Ok(())
        },
        moving,
        select_all,
        cancel,
        commands,
        last_offers: context.last_offers,
        toggles: context.toggles,
        request: Request::default(),
        compact: false,
    };
    let frame = ribbon::bar_frame(ui.style())
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
    if let Some((tool, _)) = request.constraints
        && !tool.is_dimension()
        && model.is_checking_constraints()
    {
        let command = Command::Constraint(tool);
        actions.push(queued::refused(
            command,
            &command.title(),
            queued::CONSTRAINT_CHECK,
        ));
    } else if let Some((tool, constraints)) = request.constraints {
        let projections = if bodies.is_empty() {
            Projections::default()
        } else {
            Staged::new(model, feature.id(), snaps, &selected, &bodies)
                .map(|staged| staged.projections)
                .unwrap_or_default()
        };
        let added =
            sketch_tools::add_constraints(model, feature.id(), tool, constraints, &projections);
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
    if request.respacing
        && let Ok(change) = &respacing
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
    if request.cancel {
        actions.push(Action::Editing(EditingCommand::CancelSketch));
    }
    actions.extend(request.history);
    request.chosen
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Beside {
    InRow,
    Stacked,
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
    respacing: bool,
    delete: bool,
    finish: bool,
    cancel: bool,
    history: Option<Action>,
    chosen: Vec<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Header,
    History,
    Select,
    Draw,
    Modify,
    Constrain,
    Dimension,
}

impl Group {
    const ALL: [Self; 7] = [
        Self::Header,
        Self::History,
        Self::Select,
        Self::Draw,
        Self::Modify,
        Self::Constrain,
        Self::Dimension,
    ];

    fn caption(self) -> Option<&'static str> {
        match self {
            Self::Header => None,
            Self::History => Some("History"),
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
    respacing: Result<(), String>,
    deletable: Result<(), String>,
    moving: Result<(), String>,
    select_all: Result<(), String>,
    cancel: Result<(), String>,
    commands: &'a mut CommandFrame<'b>,
    last_offers: &'a [commands::Offer],
    toggles: ToggleStates,
    request: Request,
    compact: bool,
}

struct Arrangement {
    ribbon: &'static str,
    header: f32,
    rows: Vec<Vec<Group>>,
    finish_apart: bool,
}

impl Arrangement {
    fn bands(&self) -> usize {
        self.rows.len() + usize::from(self.finish_apart)
    }
}

impl Bar<'_, '_> {
    fn arrange(&self, ui: &Ui, compact: bool, full: f32, finish: f32) -> Arrangement {
        let ribbon = if compact { COMPACT_RIBBON } else { RIBBON };
        let header = self.header_width(ui, compact).min(full);
        let widths: Vec<(Group, f32)> = ribbon::remembered_widths(ui, ribbon, &Group::ALL)
            .into_iter()
            .map(|(group, width)| match group {
                Group::Header => (group, header),
                _ => (group, width),
            })
            .collect();
        let beside_finish = full - finish - ribbon::GROUP_SPACE;
        let finish_apart = beside_finish < header;
        let first = if finish_apart { full } else { beside_finish };
        Arrangement {
            ribbon,
            header,
            rows: ribbon::rows(&widths, first, full),
            finish_apart,
        }
    }

    fn show(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = Vec2::splat(ribbon::TOOL_GAP);
        let full = ui.available_width();
        let finish_id = Id::new("sketch-bar-finish");
        let finish = widgets::remembered_width(ui, finish_id);
        let labelled = self.arrange(ui, false, full, finish);
        self.compact = ribbon::labelled_too_tall(ui, labelled.bands());
        let Arrangement {
            ribbon: name,
            header,
            rows,
            finish_apart,
        } = if self.compact {
            self.arrange(ui, true, full, finish)
        } else {
            labelled
        };
        if finish_apart {
            ui.allocate_ui_with_layout(
                vec2(full, FINISH_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| self.finish(ui, finish_id, Beside::InRow),
            );
            ui.add_space(ribbon::ROW_GAP);
        }
        let packing = Packing {
            header,
            captions: !self.compact && rows.len() == 1 && !finish_apart,
        };
        for (index, row) in rows.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(ribbon::ROW_GAP);
            }
            ui.horizontal_top(|ui| {
                ribbon::row(ui, name, &row, |ui, group| self.group(ui, group, packing));
                if index == 0 && !finish_apart {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), ribbon::height(ui, packing.captions)),
                        Layout::right_to_left(Align::Center),
                        |ui| self.finish(ui, finish_id, Beside::Stacked),
                    );
                }
            });
        }
    }

    fn group(&mut self, ui: &mut Ui, group: Group, packing: Packing) -> (Rect, f32) {
        let Some(caption) = group.caption() else {
            return ribbon::captioned(ui, None, packing.captions, |ui| {
                self.tools(ui, group, packing).width
            });
        };
        let title = (!self.compact).then(|| ribbon::caption_galley(ui, caption));
        let name = group_menu_name(caption);
        let contents =
            format!("Every {caption} command of the sketch, the ones without a button included");
        let menu = GroupMenu {
            id: Id::new(("sketch-ribbon-group-menu", group)),
            name: &name,
            contents: &contents,
        };
        let (shown, trigger) = ribbon::menu_group(ui, title, packing.captions, &menu, |ui| {
            self.tools(ui, group, packing)
        });
        if let Some(trigger) = trigger {
            let entries = MenuEntries::new(self.last_offers, self.toggles, self.commands.keymap());
            let chosen = ribbon::group_menu(&trigger, entries, |ui, entries| {
                self.group_menu(ui, entries, group);
            });
            let active = Command::SketchTool(self.active.tool);
            self.request
                .chosen
                .extend(chosen.into_iter().filter(|command| *command != active));
        }
        shown
    }

    fn tools(&mut self, ui: &mut Ui, group: Group, packing: Packing) -> GroupTools {
        let (width, host, compact_host) = match group {
            Group::Header => {
                let width = self.header(ui, packing);
                (width, ui.min_rect(), true)
            }
            Group::History => {
                let (width, host) = self.history_buttons(ui);
                (width, host, true)
            }
            Group::Select => {
                let select = Tool::Select;
                let host = self.tool_button(ui, select, select.label()).rect;
                (host.width(), host, self.compact)
            }
            Group::Draw => {
                let (width, host) = self.drawing_tools(ui);
                (width, host, self.compact)
            }
            Group::Modify => {
                let (width, host) = self.edit_buttons(ui);
                (width, host, true)
            }
            Group::Constrain => {
                let (width, host) = self.constraint_buttons(ui, false, GEOMETRIC_COLUMNS);
                (width, host, true)
            }
            Group::Dimension => {
                let (width, host) = self.dimension_buttons(ui);
                (width, host, true)
            }
        };
        GroupTools {
            width,
            host,
            compact_host,
        }
    }

    fn group_menu(&self, ui: &mut Ui, entries: &mut MenuEntries<'_>, group: Group) {
        ribbon::menu_sections(
            ui,
            entries,
            &menu_sections(group),
            |ui, entries, command| {
                self.menu_entry(ui, entries, command);
            },
        );
    }

    fn menu_entry(&self, ui: &mut Ui, entries: &mut MenuEntries<'_>, command: Command) {
        if entries.history_item(ui, command, self.model) {
            return;
        }
        let chosen = match command {
            Command::SketchTool(tool) => Some(self.active.tool == tool),
            Command::ShapeMode(mode) => Some(self.modes.of(mode.tool()) == Some(mode)),
            Command::Construction => Some(self.active.construction),
            _ => self.toggles.shown(command).map(|shown| shown.is_on()),
        };
        match chosen {
            Some(chosen) => entries.marked(ui, command, chosen),
            None => entries.item(ui, command),
        }
    }

    fn title(&self) -> String {
        format!("Editing {}", self.feature.name)
    }

    fn history_buttons(&mut self, ui: &mut Ui) -> (f32, Rect) {
        let steps = [
            (
                Command::Undo,
                self.model.undo_label(),
                Action::Undo,
                NOTHING_TO_UNDO,
            ),
            (
                Command::Redo,
                self.model.redo_label(),
                Action::Redo,
                NOTHING_TO_REDO,
            ),
        ];
        steps
            .into_iter()
            .map(|(command, step, action, idle)| {
                let verb = command.title();
                let help = match step {
                    Some(step) => Ok(self.commands.with_keys(command, &format!("{verb} {step}"))),
                    None => Err(idle.to_owned()),
                };
                let button = ToolButton::new(icons::command(command), &verb).compact();
                let response = explained(ui.add_enabled(help.is_ok(), button), &verb, &help);
                if response.clicked() {
                    self.request.history = Some(action);
                }
                response.rect
            })
            .fold((0.0, Rect::NOTHING), |(width, _), rect| {
                (width.max(rect.width()), rect)
            })
    }

    fn header_width(&self, ui: &Ui, compact: bool) -> f32 {
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
            .clamp(HEADER_TEXT_MIN_WIDTH, HEADER_TEXT_MAX_WIDTH)
            .max(sketch_status::widest_status(ui));
        if compact {
            text
        } else {
            BADGE_SIDE + HEADER_GAP + text
        }
    }

    fn header(&mut self, ui: &mut Ui, packing: Packing) -> f32 {
        let tokens = appearance::tokens(ui);
        let width = packing.header;
        let height = ribbon::height(ui, packing.captions);
        let title = self.title();
        let summary = SketchSummary::of(self.model.evaluation(), self.feature.id());
        let mut requested = None;
        ui.horizontal_top(|ui| {
            ui.set_min_size(vec2(width, height));
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.x = HEADER_GAP;
            if !self.compact {
                ui.vertical(|ui| {
                    ui.add_space(((height - BADGE_SIDE) / 2.0).max(0.0));
                    badge(ui, &tokens);
                });
            }
            let height_id = Id::new("sketch-bar-header-height");
            let known = widgets::remembered_width(ui, height_id);
            let content = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = HEADER_LINE_GAP;
                    ui.add_space(((height - known) / 2.0).max(0.0));
                    let top = ui.cursor().min.y;
                    let line = widgets::count_pill_height(ui)
                        .max(ui.text_style_height(&TextStyle::Button));
                    let counts = ui
                        .allocate_ui_with_layout(
                            vec2(ui.available_width(), line),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                ui.set_min_height(line);
                                ui.spacing_mut().item_spacing.x = HEADER_COUNT_GAP;
                                let counted =
                                    sketch_status::counts_from_the_right(ui, &summary, true);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(
                                        Label::new(
                                            RichText::new(&title)
                                                .text_style(TextStyle::Button)
                                                .color(tokens.text),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                });
                                counted
                            },
                        )
                        .inner;
                    let status = sketch_status::status_pill(ui, &summary, true);
                    requested = status.or(counts);
                    ui.min_rect().bottom() - top
                })
                .inner;
            widgets::remember_width(ui, height_id, content);
        });
        match requested {
            Some(StatusRequest::ShowProblem(focus)) => self.request.focus = Some(focus),
            Some(StatusRequest::Run(command)) => self.commands.trigger(command),
            None => {}
        }
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
        let response = if self.compact {
            explained(ui.add(button.compact()), label, &Ok(hover))
        } else {
            ui.add(button).on_hover_text(hover)
        };
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
        let contents = ways_listed(tool);
        let menu = CornerMenu {
            id: Id::new(("sketch-bar-modes", tool)),
            name: &name,
            contents: &contents,
            on_selected: self.active.tool == tool,
        };
        let response = self.corner_menu_button(ui, button, &menu);
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

    fn corner_menu_button(&self, ui: &mut Ui, host: Rect, menu: &CornerMenu<'_>) -> Response {
        if self.compact {
            widgets::compact_corner_menu_button(ui, host, menu)
        } else {
            widgets::corner_menu_button(ui, host, menu)
        }
    }

    fn compact_tool_button(&mut self, ui: &mut Ui, tool: Tool) -> Rect {
        let command = Command::SketchTool(tool);
        let invoked = self.commands.available(command);
        let active = self.active.tool == tool;
        let button = ToolButton::new(icons::tool(tool), tool.label())
            .compact()
            .selected(active);
        let mode = self.modes.of(tool);
        let description = mode.map_or(tool.description(), ShapeMode::description);
        let help = Ok(self.commands.with_keys(command, description));
        let response = explained(ui.add(button), tool.label(), &help);
        for way in ShapeMode::of_tool(tool) {
            if self.commands.available(Command::ShapeMode(way)) {
                self.request.mode = Some(way);
            }
        }
        match mode.filter(|_| invoked && active) {
            Some(mode) => self.request.mode = Some(mode.next()),
            None if response.clicked() || invoked => self.request.tool = Some(tool),
            None => {}
        }
        response.rect
    }

    fn partnered_tool_button(&mut self, ui: &mut Ui, partners: &Partners) {
        let button = self.compact_tool_button(ui, partners.host);
        let tools = partners.tools();
        let contents = partners.contents();
        let menu = CornerMenu {
            id: Id::new(("sketch-bar-partners", partners.host)),
            name: partners.name,
            contents: &contents,
            on_selected: tools.contains(&self.active.tool),
        };
        let response = widgets::compact_corner_menu_button(ui, button, &menu);
        let commands = &*self.commands;
        let active = self.active.tool;
        let host_mode = self.modes.of(partners.host);
        let shown = Popup::menu(&response).show(|ui| {
            widgets::fitted_menu(ui, |ui| {
                let mut chosen = None;
                for tool in tools.iter().copied() {
                    let keys = commands.keys(Command::SketchTool(tool));
                    if widgets::menu_choice(
                        ui,
                        icons::tool(tool),
                        tool.label(),
                        keys,
                        tool == active,
                    )
                    .clicked()
                    {
                        chosen = Some(GroupChoice::Tool(tool));
                    }
                }
                if let Some(current) = host_mode {
                    ui.separator();
                    for mode in ShapeMode::of_tool(partners.host) {
                        let keys = commands.keys(Command::ShapeMode(mode));
                        let glyph = icons::shape_mode(mode);
                        if widgets::menu_choice(ui, glyph, mode.label(), keys, mode == current)
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
            Some(GroupChoice::Tool(tool)) => self.request.tool = Some(tool),
            Some(GroupChoice::Way(mode)) => self.request.mode = Some(mode),
            None => {}
        }
    }

    fn drawing_tools(&mut self, ui: &mut Ui) -> (f32, Rect) {
        let shown: Vec<Tool> = Tool::ALL
            .into_iter()
            .filter(|tool| {
                tool.draws() && ToolGroup::of(*tool).is_none_or(|group| group.leads_with(*tool))
            })
            .collect();
        let rows = if self.compact { COMPACT_DRAW_ROWS } else { 1 };
        let per_row = shown.len().div_ceil(rows).max(1);
        let mut host = Rect::NOTHING;
        let width = shown
            .chunks(per_row)
            .map(|row| {
                ui.horizontal_wrapped(|ui| {
                    row.iter()
                        .map(|tool| {
                            let button = self.drawing_button(ui, *tool);
                            if *tool == DRAW_MENU_HOST {
                                host = button;
                            }
                            button.width()
                        })
                        .reduce(|total, width| total + ribbon::TOOL_GAP + width)
                        .unwrap_or_default()
                })
                .inner
            })
            .fold(0.0, f32::max);
        (width, host)
    }

    fn drawing_button(&mut self, ui: &mut Ui, tool: Tool) -> Rect {
        if let Some(group) = ToolGroup::of(tool) {
            return self.group_button(ui, group);
        }
        let button = self.tool_button(ui, tool, tool.label()).rect;
        if let Some(mode) = self.modes.of(tool) {
            self.mode_menu(ui, mode, button);
        }
        button
    }

    fn group_button(&mut self, ui: &mut Ui, group: &ToolGroup) -> Rect {
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
        let contents = menu_contents(group.tools, Some(current));
        let menu = CornerMenu {
            id: Id::new(group.id).with("ways"),
            name: group.ways,
            contents: &contents,
            on_selected: group.tools.contains(&self.active.tool),
        };
        let response = self.corner_menu_button(ui, button, &menu);
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
        button
    }

    fn edit_buttons(&mut self, ui: &mut Ui) -> (f32, Rect) {
        if self.commands.invoke(Command::SplitCurve, &self.split) {
            self.request.split = true;
        }
        if self.commands.invoke(Command::BreakCurves, &self.breaking) {
            self.request.breaking = true;
        }
        if self
            .commands
            .invoke(Command::RespaceFitSplines, &self.respacing)
        {
            self.request.respacing = true;
        }
        for tool in OFF_RIBBON {
            self.off_ribbon_tool(tool);
        }
        let first = ui.horizontal_top(|ui| {
            self.construction_button(ui);
            self.compact_tool_button(ui, Tool::Trim);
            self.compact_tool_button(ui, Tool::Extend);
            self.compact_tool_button(ui, Tool::Fillet);
            self.compact_tool_button(ui, Tool::Chamfer);
            self.partnered_tool_button(ui, &CURVE_FROM_GEOMETRY_TOOLS);
        });
        let second = ui.horizontal_top(|ui| {
            self.partnered_tool_button(ui, &COPYING_TOOLS);
            self.partnered_tool_button(ui, &MODEL_GEOMETRY_TOOLS);
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
            self.delete_button(ui)
        });
        let width = first
            .response
            .rect
            .width()
            .max(second.response.rect.width());
        (width, second.inner)
    }

    fn activity_button(&mut self, ui: &mut Ui) -> Rect {
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
        let response = explained(response, label, &described);
        if response.clicked() || invoked {
            self.request.activity = true;
        }
        response.rect
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

    fn delete_button(&mut self, ui: &mut Ui) -> Rect {
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
        response.rect
    }

    fn constraint_buttons(&mut self, ui: &mut Ui, dimensions: bool, columns: usize) -> (f32, Rect) {
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
        let mut host = Rect::NOTHING;
        let width = offers
            .chunks(columns.max(1))
            .map(|row| {
                ui.horizontal_top(|ui| {
                    for (tool, offer) in row {
                        host = self.constraint_button(ui, *tool, offer);
                    }
                })
                .response
                .rect
                .width()
            })
            .fold(0.0, f32::max);
        (width, host)
    }

    fn dimension_buttons(&mut self, ui: &mut Ui) -> (f32, Rect) {
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
            self.activity_button(ui)
        });
        let width = top.response.rect.width().max(bottom.response.rect.width());
        (width, bottom.inner)
    }

    fn constraint_button(
        &mut self,
        ui: &mut Ui,
        tool: ConstraintTool,
        offer: &Result<Vec<Constraint>, String>,
    ) -> Rect {
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
        response.rect
    }

    fn finish(&mut self, ui: &mut Ui, width_id: Id, beside: Beside) {
        ui.add_space(FINISH_MARGIN);
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
        let beside = match beside {
            Beside::InRow => {
                let cancelling = self.cancel_button(ui);
                cancelling.rect.union(self.look_button(ui).rect)
            }
            Beside::Stacked => {
                let column = vec2(widgets::compact_tool_side(ui), widgets::tool_height(ui));
                ui.allocate_ui_with_layout(column, Layout::top_down(Align::Center), |ui| {
                    let cancelling = self.cancel_button(ui);
                    cancelling.rect.union(self.look_button(ui).rect)
                })
                .inner
            }
        };
        let all = response.rect.union(beside);
        widgets::remember_width(ui, width_id, all.width() + FINISH_MARGIN);
        if response.clicked() || invoked {
            self.request.finish = true;
        }
    }

    fn look_button(&mut self, ui: &mut Ui) -> Response {
        let look = Command::LookAtSketch;
        let look_help = self.commands.with_keys(look, &look.title());
        let looking = widgets::framed_icon_button(ui, icons::command(look), &look_help);
        if looking.clicked() {
            self.commands.trigger(look);
        }
        looking
    }

    fn cancel_button(&mut self, ui: &mut Ui) -> Response {
        let command = Command::CancelSketch;
        let invoked = self.commands.invoke(command, &self.cancel);
        let button = ToolButton::new(icons::command(command), CANCEL_LABEL).compact();
        let response = ui.add_enabled(self.cancel.is_ok(), button);
        let help = match &self.cancel {
            Ok(()) => Ok(self.commands.with_keys(command, CANCEL_HELP)),
            Err(reason) => Err(format!("{CANCEL_HELP}. Not now: {reason}")),
        };
        let response = explained(response, CANCEL_LABEL, &help);
        if response.clicked() || invoked {
            self.request.cancel = true;
        }
        response
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
