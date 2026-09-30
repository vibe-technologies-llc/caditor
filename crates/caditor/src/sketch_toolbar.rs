use std::sync::Arc;

use caditor_document::Feature;
use caditor_sketch::{Constraint, ConstraintId, EntityId, Sketch};
use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Galley, Id, Label, Layout, Margin, Popup, Rect,
    Response, RichText, Stroke, TextStyle, TextWrapMode, Ui, Vec2, WidgetText, pos2, vec2,
};

use crate::{
    appearance::{self, ICON_SIZE, Tokens, WIDGET_RADIUS},
    commands::{Command, CommandFrame},
    editing::{ActiveSketch, EditingCommand, SketchEditing, Tool},
    feature_tree::count,
    fonts, icons,
    model::{Action, Model},
    panels::{Focus, PanelState},
    selection::Selection,
    shape_modes::{ShapeMode, ShapeModes},
    sketch_drag::{self, Moving},
    sketch_status::{self, SketchSummary},
    sketch_tools::{self, ConstraintTool, ConstructionChange},
    widgets::{self, ToolButton},
};

pub const FINISH_LABEL: &str = "Finish sketch";
pub const DELETE_LABEL: &str = "Delete";
pub const MOVE_LABEL: &str = "Move";
pub const SELECT_ALL_LABEL: &str = "Select all";
pub const CONSTRUCTION_LABEL: &str = "Construction";
const SELECT_KEY: &str = "Esc";
const FINISH_KEYS: &str = "Esc with nothing selected";
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

const TOOL_GAP: f32 = widgets::COMPACT_TOOL_GAP;
const DIVIDER_SPACE: f32 = 12.0;
const GROUP_SPACE: f32 = DIVIDER_SPACE + TOOL_GAP;
const CAPTION_GAP: f32 = 2.0;
const HEADER_WIDTH: f32 = 220.0;
const HEADER_GAP: f32 = 10.0;
const HEADER_LINE_GAP: f32 = 4.0;
const BADGE_SIDE: f32 = 32.0;
const FINISH_HEIGHT: f32 = 32.0;
const ACCENT_LINE_WIDTH: f32 = 2.0;
const BAR_MARGIN: Margin = Margin {
    left: 8,
    right: 8,
    top: 7,
    bottom: 4,
};
const WIDTH_TOLERANCE: f32 = 0.5;
const GEOMETRIC_COLUMNS: usize = 6;
const DIMENSION_COLUMNS: usize = 3;

type Offer = (ConstraintTool, Result<Vec<Constraint>, String>);

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
    let offers: Vec<Offer> = ConstraintTool::ALL
        .into_iter()
        .map(|tool| {
            let candidates = tool
                .candidates(definition, &shown, &selected)
                .map(|constraints| sketch_tools::in_unit(constraints, model.length_unit()));
            (tool, candidates)
        })
        .collect();
    let deletable = Deletable {
        entities: selected
            .iter()
            .copied()
            .filter(|entity| !entity.is_reference())
            .collect(),
        constraints: sketch_tools::selected_constraints(selection, feature.id()),
    };
    let construction = ConstructionChange::of(definition, &selected);
    let moving = Moving::offered(&shown, feature.id(), &selected, active.tool.draws()).map(|_| ());
    let select_all = sketch_drag::can_select_all(&shown).map_err(str::to_owned);

    let tokens = appearance::tokens(ui);
    let mut bar = Bar {
        model,
        feature,
        active,
        modes: editing.modes(),
        offers: &offers,
        construction: construction.as_ref(),
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
        {
            panels.request_focus(Focus::Dimension {
                feature: feature.id(),
                constraint: *constraint,
            });
        }
        actions.push(Action::Apply(added.transaction));
    }
    if request.construction {
        actions.push(match &construction {
            Some(change) => Action::Apply(change.transaction(model, feature.id(), definition)),
            None => Action::Editing(EditingCommand::DrawConstruction(!active.construction)),
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
            Self::Header | Self::Select => None,
            Self::Draw => Some("Draw"),
            Self::Modify => Some("Modify"),
            Self::Constrain => Some("Constrain"),
            Self::Dimension => Some("Dimension"),
        }
    }

    fn width_id(self) -> Id {
        Id::new(("sketch-bar-group", self))
    }
}

fn rows(widths: &[(Group, f32)], first: f32, rest: f32) -> Vec<Vec<Group>> {
    let mut rows = Vec::new();
    let mut row: Vec<Group> = Vec::new();
    let mut used = 0.0;
    for &(group, width) in widths {
        let room = if rows.is_empty() { first } else { rest };
        let needed = used + GROUP_SPACE + width;
        if row.is_empty() {
            used = width;
        } else if needed <= room + WIDTH_TOLERANCE {
            used = needed;
        } else {
            rows.push(std::mem::take(&mut row));
            used = width;
        }
        row.push(group);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

struct Bar<'a, 'b> {
    model: &'a Model,
    feature: &'a Feature,
    active: ActiveSketch,
    modes: ShapeModes,
    offers: &'a [Offer],
    construction: Option<&'a ConstructionChange>,
    deletable: Result<(), String>,
    moving: Result<(), String>,
    select_all: Result<(), String>,
    commands: &'a mut CommandFrame<'b>,
    request: Request,
}

impl Bar<'_, '_> {
    fn show(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing = Vec2::splat(TOOL_GAP);
        let full = ui.available_width();
        let band = widgets::tool_height(ui);
        let header = HEADER_WIDTH.min(full);
        let finish_id = Id::new("sketch-bar-finish");
        let widths: Vec<(Group, f32)> = Group::ALL
            .into_iter()
            .map(|group| match group {
                Group::Header => (group, header),
                _ => (group, widgets::remembered_width(ui, group.width_id())),
            })
            .collect();
        let beside_finish = full - widgets::remembered_width(ui, finish_id) - GROUP_SPACE;
        let finish_apart = beside_finish < header;
        if finish_apart {
            ui.allocate_ui_with_layout(
                vec2(full, FINISH_HEIGHT),
                Layout::right_to_left(Align::Center),
                |ui| self.finish(ui, finish_id),
            );
        }
        let first = if finish_apart { full } else { beside_finish };
        for (index, row) in rows(&widths, first, full).into_iter().enumerate() {
            ui.horizontal_top(|ui| {
                let mut previous: Option<f32> = None;
                for group in row {
                    if previous.is_some() {
                        ui.add_space(DIVIDER_SPACE);
                    }
                    let (rect, natural) = self.group(ui, group, band);
                    if let Some(right) = previous {
                        let divider = ui.visuals().widgets.noninteractive.bg_stroke;
                        ui.painter()
                            .vline((right + rect.left()) / 2.0, rect.y_range(), divider);
                    }
                    previous = Some(rect.right());
                    if group != Group::Header {
                        widgets::remember_width(ui, group.width_id(), natural);
                    }
                }
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

    fn group(&mut self, ui: &mut Ui, group: Group, band: f32) -> (Rect, f32) {
        let caption = group.caption().map(|text| caption_galley(ui, text));
        let shown = ui.vertical(|ui| {
            let content = match group {
                Group::Header => self.header(ui, band),
                Group::Select => self.tool_button(ui, Tool::Select).rect.width(),
                Group::Draw => self.drawing_tools(ui),
                Group::Modify => self.edit_buttons(ui),
                Group::Constrain => self.constraint_buttons(ui, false, GEOMETRIC_COLUMNS),
                Group::Dimension => self.constraint_buttons(ui, true, DIMENSION_COLUMNS),
            };
            let height = ui.text_style_height(&TextStyle::Small);
            let top = ui.min_rect().bottom() + CAPTION_GAP;
            match caption {
                Some(galley) => {
                    let width = ui.min_rect().width().max(galley.size().x);
                    let rect =
                        Rect::from_min_size(pos2(ui.min_rect().left(), top), vec2(width, height));
                    let natural = content.max(galley.size().x);
                    ui.put(rect, Label::new(galley).selectable(false));
                    natural
                }
                None => {
                    ui.allocate_space(vec2(0.0, height));
                    content
                }
            }
        });
        (shown.response.rect, shown.inner)
    }

    fn header(&mut self, ui: &mut Ui, band: f32) -> f32 {
        let tokens = appearance::tokens(ui);
        let width = HEADER_WIDTH.min(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.set_min_size(vec2(width, band));
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.x = HEADER_GAP;
            ui.vertical(|ui| {
                ui.add_space(((band - BADGE_SIDE) / 2.0).max(0.0));
                badge(ui, tokens);
            });
            let height_id = Id::new("sketch-bar-header-height");
            let known = widgets::remembered_width(ui, height_id);
            let content = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = HEADER_LINE_GAP;
                    ui.add_space(((band - known) / 2.0).max(0.0));
                    let top = ui.cursor().min.y;
                    ui.add(
                        Label::new(
                            RichText::new(format!("Editing {}", self.feature.name))
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

    fn tool_button(&mut self, ui: &mut Ui, tool: Tool) -> Response {
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
        let button = ToolButton::new(icons::tool(tool), tool.label()).selected(active);
        let response = ui.add(button).on_hover_text(hover);
        match mode.filter(|_| invoked && active) {
            Some(mode) => self.request.mode = Some(mode.next()),
            None if response.clicked() || invoked => self.request.tool = Some(tool),
            None => {}
        }
        response
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
            let mut chosen = None;
            for mode in ShapeMode::of_tool(tool) {
                let keys = commands.keys(Command::ShapeMode(mode));
                let glyph = icons::shape_mode(mode);
                if widgets::menu_choice(ui, glyph, mode.label(), keys, mode == current).clicked() {
                    chosen = Some(mode);
                }
            }
            chosen
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
                .filter(|tool| tool.draws())
                .map(|tool| {
                    let button = self.tool_button(ui, tool).rect;
                    if let Some(mode) = self.modes.of(tool) {
                        self.mode_menu(ui, mode, button);
                    }
                    button.width()
                })
                .reduce(|total, width| total + TOOL_GAP + width)
                .unwrap_or_default()
        })
        .inner
    }

    fn edit_buttons(&mut self, ui: &mut Ui) -> f32 {
        let first = ui.horizontal_top(|ui| {
            self.construction_button(ui);
            self.compact_tool_button(ui, Tool::Trim);
            self.compact_tool_button(ui, Tool::Extend);
            self.compact_tool_button(ui, Tool::Fillet);
        });
        let second = ui.horizontal_top(|ui| {
            self.compact_tool_button(ui, Tool::Offset);
            self.compact_tool_button(ui, Tool::Mirror);
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
        let offers: Vec<&Offer> = self
            .offers
            .iter()
            .filter(|(tool, _)| tool.is_dimension() == dimensions)
            .collect();
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
        widgets::remember_width(ui, width_id, response.rect.width());
        if response.clicked() || invoked {
            self.request.finish = true;
        }
    }
}

pub fn modes_label(tool: Tool) -> String {
    format!("Ways to draw a {}", tool.label().to_lowercase())
}

fn explained(response: Response, title: &str, help: &Result<String, String>) -> Response {
    let tip = |ui: &mut Ui, text: &str| {
        ui.strong(title);
        ui.label(text);
    };
    match help {
        Ok(text) => response.on_hover_ui(|ui| tip(ui, text)),
        Err(text) => response.on_disabled_hover_ui(|ui| tip(ui, text)),
    }
}

fn caption_galley(ui: &Ui, text: &str) -> Arc<Galley> {
    let muted = appearance::tokens(ui).text_muted;
    WidgetText::from(
        RichText::new(text)
            .text_style(TextStyle::Small)
            .color(muted),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Small,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTHS: [(Group, f32); 4] = [
        (Group::Header, 200.0),
        (Group::Select, 50.0),
        (Group::Draw, 500.0),
        (Group::Modify, 50.0),
    ];

    #[test]
    fn groups_fill_each_row_before_wrapping_and_the_first_row_leaves_room_for_finish() {
        assert_eq!(
            rows(&WIDTHS, 1000.0, 1000.0),
            vec![vec![
                Group::Header,
                Group::Select,
                Group::Draw,
                Group::Modify
            ]]
        );
        assert_eq!(
            rows(&WIDTHS, 700.0, 900.0),
            vec![
                vec![Group::Header, Group::Select],
                vec![Group::Draw, Group::Modify]
            ]
        );
        assert_eq!(
            rows(&WIDTHS, 300.0, 300.0),
            vec![
                vec![Group::Header, Group::Select],
                vec![Group::Draw],
                vec![Group::Modify]
            ]
        );
        assert_eq!(
            rows(&WIDTHS, 100.0, 100.0),
            vec![
                vec![Group::Header],
                vec![Group::Select],
                vec![Group::Draw],
                vec![Group::Modify]
            ]
        );
    }
}
