use std::{borrow::Cow, collections::BTreeMap};

use caditor_document::{
    Document, Drawing, DrawingId, DrawingScale, DrawingViewError, FeatureId,
    MAX_DRAWING_NAME_CHARS, MAX_DRAWINGS, PlacedView, Projection, ScaleRatio, SheetMark,
    SheetOrientation, SheetPlan, SheetSize, StandardView, TextAnchor, TitleFields, Transaction,
    ViewLines, lay_out_sheet,
};
use caditor_geometry::Point2;
use caditor_kernel::{Curve2, ViewPiece, Visibility};
use egui::{Align2, FontId, Key, Pos2, Rect, Sense, Shape, Stroke, TextEdit, Ui, Vec2, vec2};
use thiserror::Error;

use crate::{
    appearance::{self, SPACE_M, SPACE_S},
    commands::{Command, CommandFrame, Reason},
    feature_tree::count,
    files::FileCommand,
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    sheet_export::SheetJob,
    sheet_views::{DrawingViews, SheetViews},
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "Drawings";
pub const CLOSE: &str = "Close the drawings panel";
pub const NEW: &str = "New drawing";
pub const EXPORT: &str = "Export…";
pub const RENAME: &str = "Rename";
pub const DELETE: &str = "Delete";
pub const ZOOM_IN: &str = "Zoom in";
pub const ZOOM_OUT: &str = "Zoom out";
pub const FIT: &str = "Fit the sheet";
pub const SHEET: &str = "Drawing sheet";
pub const NONE_YET: &str = "The model has no drawings yet. A drawing lays out views of the bodies \
                            on an ISO sheet with a title block, ready to print or send.";
pub const WORKING: &str = "Working out the views…";
pub const EVERY_BODY: &str = "Every shown body";
pub const USE_SELECTED: &str = "Use the selected bodies";
pub const USE_EVERY_BODY: &str = "Draw every shown body";
pub const HIDDEN_LINES: &str = "Hidden lines";
pub const FIT_SCALE: &str = "Fit the sheet";
const PANEL: RightPanel = RightPanel { id: "drawings" };
const HEADER: PanelTitle<'static> = PanelTitle {
    glyph: icons::DRAWINGS,
    title: TITLE,
    close: CLOSE,
    help: Some(Page::Drawings),
};
const CAP_HEIGHT: f32 = 0.727;
const SMALLEST_TEXT: f32 = 4.0;
const DESCENT: f32 = 0.21;
const CURVE_STEPS: usize = 32;
const ZOOM_STEP: f32 = 1.25;
const LEAST_ZOOM: f32 = 0.5;
const MOST_ZOOM: f32 = 64.0;
const PAN_STEP: f32 = 0.1;
const HIDDEN_DASH: f32 = 4.0;
const HIDDEN_GAP: f32 = 3.0;
const VISIBLE_WIDTH: f32 = 1.5;
const THIN_WIDTH: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SheetRefusal {
    #[error("The model has no drawings; New drawing makes one")]
    NoDrawing,
    #[error("The drawing's views are still being worked out")]
    Working,
    #[error("A model keeps at most {MAX_DRAWINGS} drawings")]
    TooMany,
}

impl Reason for SheetRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SheetCamera {
    zoom: f32,
    pan: Vec2,
}

impl Default for SheetCamera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl SheetCamera {
    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(LEAST_ZOOM, MOST_ZOOM);
    }
}

#[derive(Default)]
pub struct DrawingsTool {
    pub open: bool,
    pub chosen: Option<DrawingId>,
    views: DrawingViews,
    sheet: SheetViews,
    camera: SheetCamera,
    renaming: Option<String>,
    export_pending: bool,
}

impl DrawingsTool {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.forget();
        }
    }

    pub fn forget(&mut self) {
        self.views.forget();
        self.sheet = SheetViews::default();
        self.renaming = None;
        self.export_pending = false;
    }

    fn current<'a>(&self, document: &'a Document) -> Option<&'a Drawing> {
        let drawings = &document.drawings().drawings;
        self.chosen
            .and_then(|id| document.drawing(id))
            .or_else(|| drawings.first())
    }

    pub fn refresh(&mut self, model: &Model) {
        let document = model.document();
        let Some(drawing) = self.current(document) else {
            self.chosen = None;
            self.views.forget();
            self.sheet = SheetViews::default();
            return;
        };
        if self.chosen != Some(drawing.id) {
            self.chosen = Some(drawing.id);
            self.camera = SheetCamera::default();
            self.renaming = None;
        }
        self.sheet = self.views.refresh(model, drawing);
    }

    pub fn is_ready(&self) -> bool {
        self.sheet.drawing.is_some() && self.sheet.is_ready()
    }

    pub fn plan(&self, model: &Model) -> Option<SheetPlan> {
        let document = model.document();
        let drawing = self.current(document)?;
        Some(plan_of(
            document,
            &model.display_name(),
            drawing,
            &self.sheet,
        ))
    }

    pub fn job(&self, model: &Model) -> Result<SheetJob, SheetRefusal> {
        let document = model.document();
        let drawing = self.current(document).ok_or(SheetRefusal::NoDrawing)?;
        if !self.sheet.is_ready() || self.sheet.drawing != Some(drawing.id) {
            return Err(SheetRefusal::Working);
        }
        let plan = plan_of(document, &model.display_name(), drawing, &self.sheet);
        let views = plan
            .views
            .iter()
            .filter_map(|placed| {
                let outcome = self.sheet.outcome(placed.view)?;
                Some((*placed, outcome))
            })
            .collect();
        Ok(SheetJob {
            name: drawing.name.clone(),
            plan,
            views,
            hidden_lines: drawing.hidden_lines,
        })
    }
}

pub fn plan_of(
    document: &Document,
    model_name: &str,
    drawing: &Drawing,
    sheet: &SheetViews,
) -> SheetPlan {
    let fields = TitleFields::of(document.properties(), model_name, drawing);
    let extents = sheet
        .finished()
        .filter_map(|(view, lines)| Some((view, lines.bounds?)))
        .collect::<BTreeMap<_, _>>();
    lay_out_sheet(drawing, &fields, &extents)
}

fn apply_or_say(
    built: Result<Transaction, caditor_document::EditError>,
    actions: &mut Vec<Action>,
) {
    match built {
        Ok(transaction) => actions.push(Action::Apply(transaction)),
        Err(error) => actions.push(Action::Inform(Notice::failure(format!(
            "The drawing was not changed: {error}."
        )))),
    }
}

fn change(
    model: &Model,
    id: DrawingId,
    label: &str,
    edit: impl FnOnce(&mut Drawing),
    actions: &mut Vec<Action>,
) {
    apply_or_say(model.document().changing_drawing(id, label, edit), actions);
}

pub fn new_drawing(
    model: &Model,
    tool: &mut DrawingsTool,
    bodies: &[FeatureId],
    actions: &mut Vec<Action>,
) {
    match model.document().adding_drawing(bodies.to_vec()) {
        Ok((transaction, id)) => {
            actions.push(Action::Apply(transaction));
            tool.open = true;
            tool.chosen = Some(id);
            tool.camera = SheetCamera::default();
        }
        Err(error) => actions.push(Action::Inform(Notice::failure(format!(
            "No drawing was made: {error}."
        )))),
    }
}

pub fn commands(
    model: &Model,
    tool: &mut DrawingsTool,
    bodies: &[FeatureId],
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    if commands.available(Command::Drawings) {
        tool.toggle();
    }
    let drawings = &model.document().drawings().drawings;
    let room = if drawings.len() >= MAX_DRAWINGS {
        Err(SheetRefusal::TooMany)
    } else {
        Ok(())
    };
    if commands.invoke(Command::NewDrawing, &room) {
        new_drawing(model, tool, bodies, actions);
    }
    let exportable = if drawings.is_empty() {
        Err(SheetRefusal::NoDrawing)
    } else {
        Ok(())
    };
    if commands.invoke(Command::ExportDrawingSheet, &exportable) {
        tool.open = true;
        tool.export_pending = true;
    }
}

pub fn export_when_ready(model: &Model, tool: &mut DrawingsTool, actions: &mut Vec<Action>) {
    if !tool.export_pending {
        return;
    }
    match tool.job(model) {
        Ok(job) => {
            tool.export_pending = false;
            actions.push(Action::File(FileCommand::ExportSheet(Box::new(job))));
        }
        Err(SheetRefusal::Working) => {}
        Err(refusal) => {
            tool.export_pending = false;
            actions.push(Action::Inform(Notice::failure(format!("{refusal}."))));
        }
    }
}

fn drawing_choice(ui: &mut Ui, document: &Document, current: &Drawing) -> Option<DrawingId> {
    let drawings = &document.drawings().drawings;
    if drawings.len() < 2 {
        return None;
    }
    let mut chosen = None;
    widgets::property(ui, "Drawing", |ui| {
        widgets::combo("drawing-choice", current.name.as_str())
            .hover("The drawing shown and changed below")
            .show(ui, |ui| {
                for drawing in drawings {
                    if widgets::menu_option(ui, drawing.id == current.id, &drawing.name).clicked() {
                        chosen = Some(drawing.id);
                    }
                }
            });
    });
    chosen
}

fn renaming_row(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    id: DrawingId,
    actions: &mut Vec<Action>,
) {
    let Some(name) = tool.renaming.as_mut() else {
        return;
    };
    let mut done = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(name)
                    .char_limit(MAX_DRAWING_NAME_CHARS)
                    .desired_width(ui.available_width() - 2.0 * ui.spacing().interact_size.y),
            )
        });
        let field = widgets::named(field, "Drawing name");
        let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        cancelled = widgets::escape_cancels(ui, &field);
        done = widgets::icon_button(ui, icons::DONE, "Rename the drawing").clicked() || enter;
        cancelled |= widgets::icon_button(ui, icons::CLOSE, "Keep the name (Esc)").clicked();
    });
    if cancelled {
        tool.renaming = None;
        return;
    }
    if done && let Some(name) = tool.renaming.take() {
        change(
            model,
            id,
            "Rename drawing",
            |drawing| drawing.name = name,
            actions,
        );
    }
}

fn small(ui: &mut Ui, glyph: &str, text: &str) -> egui::Response {
    let button = widgets::small_button(ui, glyph, text);
    ui.add(button)
}

fn small_enabled(ui: &mut Ui, enabled: bool, glyph: &str, text: &str) -> egui::Response {
    let button = widgets::small_button(ui, glyph, text);
    ui.add_enabled(enabled, button)
}

fn scale_label(scale: DrawingScale) -> String {
    match scale {
        DrawingScale::Fit => FIT_SCALE.to_owned(),
        DrawingScale::Ratio(ratio) => ratio.label(),
    }
}

fn settings(ui: &mut Ui, model: &Model, drawing: &Drawing, actions: &mut Vec<Action>) {
    let id = drawing.id;
    widgets::properties(ui, "drawing-settings", |ui| {
        widgets::property(ui, "Sheet", |ui| {
            let mut chosen = None;
            widgets::combo("drawing-size", drawing.size.name())
                .hover("The ISO 216 sheet the drawing is laid out on")
                .show(ui, |ui| {
                    for size in SheetSize::ALL {
                        if widgets::menu_option(ui, size == drawing.size, size.name()).clicked() {
                            chosen = Some(size);
                        }
                    }
                });
            if let Some(size) = chosen {
                change(
                    model,
                    id,
                    "Change sheet size",
                    |drawing| drawing.size = size,
                    actions,
                );
            }
        });
        widgets::property(ui, "Orientation", |ui| {
            let choices: Vec<(&str, &str)> = SheetOrientation::ALL
                .iter()
                .map(|orientation| (orientation.name(), "How the sheet is turned"))
                .collect();
            let selected = SheetOrientation::ALL
                .iter()
                .position(|orientation| *orientation == drawing.orientation)
                .unwrap_or_default();
            if let Some(orientation) = widgets::segmented(ui, &choices, selected)
                .and_then(|index| SheetOrientation::ALL.get(index).copied())
            {
                change(
                    model,
                    id,
                    "Turn the sheet",
                    |drawing| drawing.orientation = orientation,
                    actions,
                );
            }
        });
        widgets::property(ui, "Projection", |ui| {
            let choices: Vec<(&str, &str)> = Projection::ALL
                .iter()
                .map(|projection| {
                    (
                        projection.name(),
                        "Where the views stand round the front view",
                    )
                })
                .collect();
            let selected = Projection::ALL
                .iter()
                .position(|projection| *projection == drawing.projection)
                .unwrap_or_default();
            if let Some(projection) = widgets::segmented(ui, &choices, selected)
                .and_then(|index| Projection::ALL.get(index).copied())
            {
                change(
                    model,
                    id,
                    "Change projection",
                    |drawing| drawing.projection = projection,
                    actions,
                );
            }
        });
        widgets::property(ui, "Scale", |ui| {
            let mut chosen = None;
            widgets::combo("drawing-scale", scale_label(drawing.scale))
                .hover("Sheet length to model length; fitting takes the largest ISO 5455 scale the sheet holds")
                .show(ui, |ui| {
                    if widgets::menu_option(ui, drawing.scale == DrawingScale::Fit, FIT_SCALE)
                        .clicked()
                    {
                        chosen = Some(DrawingScale::Fit);
                    }
                    for ratio in ScaleRatio::PREFERRED {
                        let scale = DrawingScale::Ratio(ratio);
                        if widgets::menu_option(ui, drawing.scale == scale, &ratio.label())
                            .clicked()
                        {
                            chosen = Some(scale);
                        }
                    }
                });
            if let Some(scale) = chosen {
                change(
                    model,
                    id,
                    "Change drawing scale",
                    |drawing| drawing.scale = scale,
                    actions,
                );
            }
        });
    });
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title("Views"));
    ui.horizontal_wrapped(|ui| {
        for view in StandardView::ALL {
            let mut shown = drawing.views.contains(&view);
            if widgets::checkbox(ui, &mut shown, view.name()).changed() {
                change(
                    model,
                    id,
                    "Change drawing views",
                    |drawing| {
                        if shown {
                            drawing.views.insert(view);
                        } else {
                            drawing.views.remove(&view);
                        }
                    },
                    actions,
                );
            }
        }
    });
    let mut hidden = drawing.hidden_lines;
    if widgets::checkbox(ui, &mut hidden, HIDDEN_LINES)
        .on_hover_text("Draw edges behind the bodies dashed")
        .changed()
    {
        change(
            model,
            id,
            "Change hidden lines",
            |drawing| drawing.hidden_lines = hidden,
            actions,
        );
    }
}

fn bodies_row(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    selected: &[FeatureId],
    sheet: &SheetViews,
    actions: &mut Vec<Action>,
) {
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title("Bodies"));
    let shown = if drawing.bodies.is_empty() {
        EVERY_BODY.to_owned()
    } else {
        format!("{} chosen", count(drawing.bodies.len(), "body", "bodies"))
    };
    ui.label(shown);
    ui.horizontal_wrapped(|ui| {
        let usable = !selected.is_empty() && selected != drawing.bodies.as_slice();
        let pick = small_enabled(ui, usable, icons::SELECT, USE_SELECTED)
            .on_hover_text("Draw only the bodies selected in the view or the tree")
            .on_disabled_hover_text("Select bodies in the view or the tree to draw only those");
        if pick.clicked() {
            let bodies = selected.to_vec();
            change(
                model,
                drawing.id,
                "Choose drawn bodies",
                |drawing| drawing.bodies = bodies,
                actions,
            );
        }
        if !drawing.bodies.is_empty() && small(ui, icons::RESET, USE_EVERY_BODY).clicked() {
            change(
                model,
                drawing.id,
                "Choose drawn bodies",
                |drawing| drawing.bodies.clear(),
                actions,
            );
        }
    });
    if !sheet.missing.is_empty() {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "{} chosen for this drawing no longer in the model or not built, so not drawn. \
                 Choose the bodies again.",
                count(sheet.missing.len(), "body is", "bodies are")
            ))
        });
    }
}

fn reason(error: &DrawingViewError) -> String {
    match error {
        DrawingViewError::NoBodies => {
            "there is no body to draw: show a body, or choose bodies that are built".to_owned()
        }
        other => format!("{other}; change the model slightly or leave this view out"),
    }
}

fn status(ui: &mut Ui, sheet: &SheetViews, plan: &SheetPlan) {
    let working = sheet.working();
    if working > 0 {
        let total = sheet.views.len();
        widgets::progress(WORKING)
            .count(total - working, total)
            .wrapping()
            .show(ui);
        ui.add_space(SPACE_S);
    }
    for (view, error) in sheet.failed() {
        let response = widgets::callout(ui, Tone::Error, |ui| {
            ui.label(format!(
                "The {} view could not be drawn: {}.",
                view.name().to_lowercase(),
                reason(error)
            ))
        });
        widgets::announced(ui, &response, true);
        ui.add_space(SPACE_S);
    }
    if sheet.is_ready() && !plan.fits && !plan.views.is_empty() {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "At {} the views run past the sheet. Choose a smaller scale or a larger sheet.",
                plan.scale.label()
            ))
        });
        ui.add_space(SPACE_S);
    }
}

fn sampled(piece: &ViewPiece) -> Vec<Point2> {
    let steps = match piece.curve {
        Curve2::Line(_) => 1,
        _ => CURVE_STEPS,
    };
    (0..=steps)
        .map(|step| {
            let along = step as f64 / steps as f64;
            let at = piece.range.start() + (piece.range.end() - piece.range.start()) * along;
            piece.curve.point(at)
        })
        .collect()
}

struct Mapping {
    rect: Rect,
    centre: Point2,
    scale: f32,
}

impl Mapping {
    fn screen(&self, point: Point2) -> Pos2 {
        let offset = point - self.centre;
        self.rect.center()
            + vec2(
                offset.x as f32 * self.scale,
                -(offset.y as f32) * self.scale,
            )
    }
}

fn view_lines(
    painter: &egui::Painter,
    mapping: &Mapping,
    placed: &PlacedView,
    lines: &ViewLines,
    hidden_lines: bool,
    tokens: &appearance::Tokens,
) {
    for piece in lines.drawn_pieces(hidden_lines) {
        let points: Vec<Pos2> = sampled(piece)
            .into_iter()
            .map(|point| mapping.screen(placed.on_sheet(point)))
            .collect();
        match piece.visibility {
            Visibility::Visible => {
                painter.add(Shape::line(points, Stroke::new(VISIBLE_WIDTH, tokens.text)));
            }
            _ => {
                painter.extend(Shape::dashed_line(
                    &points,
                    Stroke::new(THIN_WIDTH, tokens.text_muted),
                    HIDDEN_DASH,
                    HIDDEN_GAP,
                ));
            }
        }
    }
}

fn sheet_canvas(ui: &mut Ui, tool: &mut DrawingsTool, plan: &SheetPlan, hidden_lines: bool) {
    let tokens = appearance::tokens(ui);
    ui.horizontal_wrapped(|ui| {
        if small(ui, icons::ZOOM_IN, ZOOM_IN).clicked() {
            tool.camera.zoom_by(ZOOM_STEP);
        }
        if small(ui, icons::ZOOM_OUT, ZOOM_OUT).clicked() {
            tool.camera.zoom_by(1.0 / ZOOM_STEP);
        }
        if small(ui, icons::FIT_SHEET, FIT).clicked() {
            tool.camera = SheetCamera::default();
        }
    });
    ui.add_space(SPACE_S);
    let width = ui.available_width().max(1.0);
    let height = width * (plan.height / plan.width.max(1.0)) as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let response = widgets::named(response, SHEET).on_hover_text(
        "Drag to move the sheet, pinch or Ctrl and scroll to zoom; with the sheet focused, the \
         arrow keys move it",
    );
    let fit = width / plan.width.max(1.0) as f32;
    if response.hovered() {
        let zoom = ui.input(|input| input.zoom_delta());
        if (zoom - 1.0).abs() > f32::EPSILON {
            tool.camera.zoom_by(zoom);
        }
    }
    let scale = fit * tool.camera.zoom;
    if response.dragged() {
        let moved = response.drag_delta() / scale;
        tool.camera.pan += vec2(-moved.x, moved.y);
    }
    if response.has_focus() {
        let step = PAN_STEP * plan.width as f32 / tool.camera.zoom;
        let keys = [
            (Key::ArrowLeft, vec2(-step, 0.0)),
            (Key::ArrowRight, vec2(step, 0.0)),
            (Key::ArrowUp, vec2(0.0, step)),
            (Key::ArrowDown, vec2(0.0, -step)),
        ];
        for (key, by) in keys {
            if ui.input(|input| input.key_pressed(key)) {
                tool.camera.pan += by;
            }
        }
    }
    let mapping = Mapping {
        rect,
        centre: Point2::new(
            plan.width / 2.0 + f64::from(tool.camera.pan.x),
            plan.height / 2.0 + f64::from(tool.camera.pan.y),
        ),
        scale,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.sunken);
    let paper = Rect::from_two_pos(
        mapping.screen(Point2::new(0.0, 0.0)),
        mapping.screen(Point2::new(plan.width, plan.height)),
    );
    painter.rect_filled(paper, 0.0, tokens.raised);
    painter.rect_stroke(
        paper,
        0.0,
        Stroke::new(THIN_WIDTH, tokens.border_strong),
        egui::StrokeKind::Inside,
    );
    for mark in &plan.marks {
        match mark {
            SheetMark::Line(from, to) => {
                painter.line_segment(
                    [mapping.screen(*from), mapping.screen(*to)],
                    Stroke::new(THIN_WIDTH, tokens.text),
                );
            }
            SheetMark::Circle { center, radius } => {
                painter.circle_stroke(
                    mapping.screen(*center),
                    *radius as f32 * scale,
                    Stroke::new(THIN_WIDTH, tokens.text),
                );
            }
        }
    }
    for text in &plan.texts {
        let size = text.height as f32 * scale / CAP_HEIGHT;
        if size < SMALLEST_TEXT {
            continue;
        }
        let at = mapping.screen(text.at);
        let (at, align) = match text.anchor {
            TextAnchor::Start => (at + vec2(0.0, size * DESCENT), Align2::LEFT_BOTTOM),
            TextAnchor::Middle => (at, Align2::CENTER_CENTER),
        };
        painter.text(
            at,
            align,
            &text.content,
            FontId::proportional(size),
            tokens.text,
        );
    }
    for placed in &plan.views {
        if let Some(lines) = tool
            .sheet
            .finished()
            .find(|(view, _)| *view == placed.view)
            .map(|(_, lines)| lines)
        {
            view_lines(&painter, &mapping, placed, lines, hidden_lines, &tokens);
        }
    }
    if response.has_focus() {
        widgets::paint_focus_ring(ui, rect);
    }
}

pub struct DrawingsContext<'a> {
    pub model: &'a Model,
    pub selected: &'a [FeatureId],
}

pub fn show(
    ui: &mut Ui,
    context: &DrawingsContext<'_>,
    tool: &mut DrawingsTool,
    room: f32,
    actions: &mut Vec<Action>,
) {
    let model = context.model;
    let document = model.document();
    let current = tool.current(document).cloned();
    let plan = tool.plan(model);
    let mut export = false;
    let panel = widgets::side_panel(
        ui,
        room,
        PANEL,
        &HEADER,
        |_| {},
        |ui| {
            let (Some(drawing), Some(plan)) = (current.as_ref(), plan.as_ref()) else {
                widgets::empty_state(ui, icons::DRAWINGS, NONE_YET, |ui| {
                    if small(ui, icons::ADD, NEW).clicked() {
                        new_drawing(model, tool, context.selected, actions);
                    }
                });
                return;
            };
            let mut chosen = None;
            widgets::properties(ui, "drawing-choice-grid", |ui| {
                chosen = drawing_choice(ui, document, drawing);
            });
            if let Some(id) = chosen {
                tool.chosen = Some(id);
                tool.camera = SheetCamera::default();
                tool.renaming = None;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(widgets::strong(drawing.name.as_str()));
                if small(ui, icons::ADD, NEW).clicked() {
                    new_drawing(model, tool, context.selected, actions);
                }
                if small(ui, icons::RENAME, RENAME).clicked() {
                    tool.renaming = Some(drawing.name.clone());
                }
                if small(ui, icons::DELETE, DELETE)
                    .on_hover_text("Delete this drawing; Undo brings it back")
                    .clicked()
                {
                    apply_or_say(document.deleting_drawing(drawing.id), actions);
                }
            });
            renaming_row(ui, model, tool, drawing.id, actions);
            ui.add_space(SPACE_S);
            settings(ui, model, drawing, actions);
            bodies_row(ui, model, drawing, context.selected, &tool.sheet, actions);
            ui.add_space(SPACE_M);
            status(ui, &tool.sheet, plan);
            let ready = tool.is_ready();
            export = ui
                .add_enabled(ready, widgets::primary_button(ui, EXPORT))
                .on_hover_text("Export the sheet as PDF, SVG or DXF")
                .on_disabled_hover_text(SheetRefusal::Working.to_string())
                .clicked();
            ui.add_space(SPACE_M);
            sheet_canvas(ui, tool, plan, drawing.hidden_lines);
        },
    );
    if export {
        tool.export_pending = true;
    }
    if panel.closed {
        tool.toggle();
    }
}
