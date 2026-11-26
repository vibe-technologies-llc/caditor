use std::{borrow::Cow, f64::consts::TAU};

use caditor_document::{
    Annotation, Drawing, DrawnView, EditError, LinePick, MAX_DETAIL_EXTENT, MIN_DETAIL_RADIUS,
    PlacedView, SheetAnnotation, SheetMark, SheetPlan, SheetText, TextAnchor, Transaction,
    ViewLines, ViewMovement, drawn_hatching, seen_line,
};
use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{Curve2, Interval, ViewPiece, Visibility};
use egui::{
    Align2, Color32, CursorIcon, Event, EventFilter, FontId, Key, Modifiers, Painter, Pos2, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, emath::Rot2, epaint::TextShape, vec2,
};

use crate::{
    appearance::{self, Tokens},
    drawings_panel::plan_of,
    icons,
    model::{Action, Model, Notice},
    sheet_tools::{
        self, CALLOUT_TOOL, CENTRE_MARKS, CENTRE_TOOL, Clicked, DELETE_CALLOUT, DELETE_CENTRE_MARK,
        DELETE_DIMENSION, DELETE_NOTE, DIMENSION_TOOL, MOVE_CALLOUT, MOVE_DIMENSION, MOVE_NOTE,
        NOTE_TOOL, SheetTool, ToolContext, ToolRow,
    },
    sheet_views::SheetViews,
    widgets,
};

pub const MOST_ZOOMED: &str = "The sheet is zoomed in as far as it goes";
pub const LEAST_ZOOMED: &str = "The sheet is zoomed out as far as it goes";
pub const ZOOM_READOUT_HOVER: &str =
    "How large the sheet is drawn, against fitting it to the space it has";
pub const ZOOM_IN: &str = "Zoom in";
pub const ZOOM_OUT: &str = "Zoom out";
pub const FIT: &str = "Fit the sheet";
pub const SHEET: &str = "Drawing sheet";
pub const RESET_POSITION: &str = "Reset position";
pub const NOT_PLACED: &str = "This view stands where the drawing lays it out";
pub const MOVE_VIEW: &str = "Move drawing view";
pub const RESET_VIEW: &str = "Reset view position";
pub const MOVE_CIRCLE: &str = "Move detail circle";
pub const RESIZE_CIRCLE: &str = "Resize detail circle";
pub const MOVE_LEFT: &str = "Move the picked view left";
pub const MOVE_RIGHT: &str = "Move the picked view right";
pub const MOVE_UP: &str = "Move the picked view up";
pub const MOVE_DOWN: &str = "Move the picked view down";
pub const DIMENSION_HOVER: &str = "Add a dimension: a length, distance, radius, diameter or angle \
                                   measured from the edges of a view";
pub const NOTE_HOVER: &str = "Add a note on the sheet, with a leader to an edge if you like";
pub const CENTRE_HOVER: &str = "Add a centre mark to a circle seen round, or a centreline to a \
                                cylinder seen from the side, one click each";
pub const CENTRE_MARKS_HOVER: &str = "Mark the centre of every circle seen round and the axis of \
                                      every cylinder seen from the side in the picked view";
pub const CALLOUT_HOVER: &str = "Call out a hole or a thread: its size, depth, counterbore or \
                                 countersink and thread, read from the feature so it follows \
                                 every edit";
pub const PICK_VIEW_FIRST: &str = "Click a view on the sheet to pick it first";
pub const CENTRE_STAYS: &str = "A centre mark stays on what it marks; it does not move";
pub const PICK_FIRST: &str = "Click a view, dimension or note on the sheet to pick it first";
pub const ONLY_UP_AND_DOWN: &str =
    "The top view moves only up and down, in line with the front view";
pub const ONLY_ACROSS: &str = "A side view moves only across, in line with the front view";
pub const SHEET_HOVER: &str = "Drag a view to move it (a top or side view only along its line \
                               with the front), a detail circle to move it or its rim to resize \
                               it, or the paper to move the sheet; pinch or Ctrl and scroll to \
                               zoom. With the sheet focused, a click picks a view, dimension or \
                               note, the arrow keys move what is picked (Shift for larger steps) \
                               or else the sheet, Delete removes a picked dimension or note, and \
                               Escape lets it go";
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
const MARKED_WIDTH: f32 = 2.0;
const OUTLINE_GAP: f32 = 4.0;
const OUTLINE_DASH: f32 = 5.0;
const OUTLINE_SPACE: f32 = 3.0;
const CHAIN_DASHES: [f32; 2] = [12.0, 0.5];
const CHAIN_GAPS: [f32; 2] = [3.0, 3.0];
const GRIP_REACH: f32 = 6.0;
const NUDGE: f64 = 1.0;
const FAR_NUDGE: f64 = 10.0;
const BROKEN_DASH: f32 = 3.0;
const BROKEN_GAP: f32 = 2.0;
const PICKED_EDGE_WIDTH: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetCamera {
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

    fn zoom_about(&mut self, factor: f32, from_centre: Vec2, fit: f32) {
        let before = fit * self.zoom;

        self.zoom_by(factor);

        let after = fit * self.zoom;
        let kept = from_centre * (1.0 / before - 1.0 / after);

        self.pan += vec2(kept.x, -kept.y);
    }

    fn can_zoom_in(&self) -> bool {
        self.zoom < MOST_ZOOM
    }

    fn can_zoom_out(&self) -> bool {
        self.zoom > LEAST_ZOOM
    }

    fn percent(&self) -> String {
        format!("{:.0}%", self.zoom * 100.0)
    }

    fn mapping(&self, rect: Rect, plan: &SheetPlan, scale: f32) -> Mapping {
        Mapping {
            rect,
            centre: Point2::new(
                plan.width / 2.0 + f64::from(self.pan.x),
                plan.height / 2.0 + f64::from(self.pan.y),
            ),
            scale,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

    fn sheet(&self, position: Pos2) -> Point2 {
        let offset = (position - self.rect.center()) / self.scale;
        self.centre + Vector2::new(f64::from(offset.x), -f64::from(offset.y))
    }

    fn reach(&self) -> f64 {
        f64::from(GRIP_REACH / self.scale)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    View(DrawnView),
    Centre(char),
    Rim(char),
    Annotation(Annotation),
}

impl Grip {
    fn label(self) -> &'static str {
        match self {
            Self::View(_) => MOVE_VIEW,
            Self::Centre(_) => MOVE_CIRCLE,
            Self::Rim(_) => RESIZE_CIRCLE,
            Self::Annotation(Annotation::Dimension(_)) => MOVE_DIMENSION,
            Self::Annotation(Annotation::Note(_)) => MOVE_NOTE,
            Self::Annotation(Annotation::Callout(_)) => MOVE_CALLOUT,
            Self::Annotation(Annotation::Centre(_)) => CENTRE_STAYS,
        }
    }

    fn moves(self) -> bool {
        !matches!(self, Self::Annotation(Annotation::Centre(_)))
    }

    fn cursor(self, held: bool) -> CursorIcon {
        match (self, held) {
            (Self::Annotation(Annotation::Centre(_)), _) => CursorIcon::PointingHand,
            (Self::Rim(_), _) => CursorIcon::ResizeNwSe,
            (_, true) => CursorIcon::Grabbing,
            (_, false) => CursorIcon::Grab,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SheetDrag {
    grip: Grip,
    from: Point2,
    to: Point2,
}

#[derive(Debug, Default)]
pub struct SheetCanvas {
    camera: SheetCamera,
    picked: Option<DrawnView>,
    picked_annotation: Option<Annotation>,
    tool: Option<SheetTool>,
    drag: Option<SheetDrag>,
    menu_view: Option<DrawnView>,
    mapping: Option<Mapping>,
    focus_pending: bool,
    id: Option<egui::Id>,
    delete_pending: bool,
}

impl SheetCanvas {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn focus(&mut self) {
        self.focus_pending = true;
    }

    pub fn picked(&self) -> Option<DrawnView> {
        self.picked
    }

    #[cfg(test)]
    pub fn picked_annotation(&self) -> Option<Annotation> {
        self.picked_annotation
    }

    #[cfg(test)]
    pub fn tool(&self) -> Option<&SheetTool> {
        self.tool.as_ref()
    }

    pub fn start_tool(&mut self, tool: SheetTool) {
        self.tool = Some(tool);
        self.picked = None;
        self.picked_annotation = None;
        self.focus_pending = true;
    }

    fn pick_annotation(&mut self, annotation: Option<Annotation>) {
        self.picked_annotation = annotation;
        if annotation.is_some() {
            self.picked = None;
        }
    }

    pub fn let_go(&mut self) -> bool {
        if let Some(tool) = self.tool.as_mut() {
            if !tool.step_back() {
                self.tool = None;
            }
            return true;
        }
        self.picked_annotation.take().is_some() || self.picked.take().is_some()
    }

    pub fn claim_delete(&mut self, ctx: &egui::Context) {
        let focused = self
            .id
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));

        if !focused || self.picked_annotation.is_none() || self.tool.is_some() {
            return;
        }
        self.delete_pending |= ctx.input_mut(|input| {
            input.consume_key(Modifiers::NONE, Key::Delete)
                || input.consume_key(Modifiers::NONE, Key::Backspace)
        });
    }

    fn holds_escape(&self) -> bool {
        self.tool.is_some() || self.picked_annotation.is_some() || self.picked.is_some()
    }

    #[cfg(test)]
    pub fn screen_of(&self, point: Point2) -> Option<Pos2> {
        self.mapping.map(|mapping| mapping.screen(point))
    }
}

pub struct SheetContext<'a> {
    pub model: &'a Model,
    pub drawing: &'a Drawing,
    pub plan: &'a SheetPlan,
    pub views: &'a SheetViews,
}

fn detail_circle(drawing: &Drawing, plan: &SheetPlan, letter: char) -> Option<(Point2, f64)> {
    let detail = drawing.detail(letter)?;

    plan.placed(DrawnView::Detail(letter))?;

    let parent = plan.placed(detail.parent.drawn())?;
    Some((parent.on_sheet(detail.centre), detail.radius * parent.scale))
}

fn view_at(plan: &SheetPlan, point: Point2, reach: f64) -> Option<DrawnView> {
    plan.views
        .iter()
        .filter(|placed| placed.extent.expanded(reach).contains(point))
        .min_by(|left, right| {
            let area = |placed: &&PlacedView| {
                let size = placed.extent.size();
                size.x * size.y
            };
            area(left).total_cmp(&area(right))
        })
        .map(|placed| placed.view)
}

fn grip_at(drawing: &Drawing, plan: &SheetPlan, point: Point2, reach: f64) -> Option<Grip> {
    if let Some(annotation) = plan.annotation_at(point, reach) {
        return Some(Grip::Annotation(annotation));
    }

    let circles: Vec<(char, Point2, f64)> = drawing
        .details
        .iter()
        .filter_map(|detail| {
            let (centre, radius) = detail_circle(drawing, plan, detail.letter)?;
            Some((detail.letter, centre, radius))
        })
        .collect();

    let rim = circles
        .iter()
        .find(|(_, centre, radius)| ((point - *centre).length() - radius).abs() <= reach)
        .map(|(letter, ..)| Grip::Rim(*letter));
    let inside = || {
        circles
            .iter()
            .find(|(_, centre, radius)| (point - *centre).length() < *radius)
            .map(|(letter, ..)| Grip::Centre(*letter))
    };
    rim.or_else(inside)
        .or_else(|| view_at(plan, point, reach).map(Grip::View))
}

fn dragged(drag: SheetDrag, plan: &SheetPlan, drawing: &mut Drawing) {
    let letter = match drag.grip {
        Grip::View(view) => {
            plan.moved(drawing, view, drag.to - drag.from);
            return;
        }
        Grip::Annotation(annotation) => {
            plan.moved_annotation(drawing, annotation, drag.to - drag.from);
            return;
        }
        Grip::Centre(letter) | Grip::Rim(letter) => letter,
    };
    let Some(parent) = drawing
        .detail(letter)
        .and_then(|detail| plan.placed(detail.parent.drawn()))
        .copied()
    else {
        return;
    };
    let Some(detail) = drawing.detail_mut(letter) else {
        return;
    };

    let centre = parent.on_sheet(detail.centre);
    let largest = Point2::splat(MAX_DETAIL_EXTENT);

    match drag.grip {
        Grip::Rim(_) => {
            let grown = (drag.to - centre).length() - (drag.from - centre).length();

            detail.radius =
                (detail.radius + grown / parent.scale).clamp(MIN_DETAIL_RADIUS, MAX_DETAIL_EXTENT);
        }
        Grip::Centre(_) | Grip::View(_) | Grip::Annotation(_) => {
            let moved = detail.centre + (drag.to - drag.from) / parent.scale;

            detail.centre = moved.clamp(-largest, largest);
        }
    }
}

fn apply_or_say(built: Result<Transaction, EditError>, actions: &mut Vec<Action>) {
    match built {
        Ok(transaction) => actions.push(Action::Apply(transaction)),
        Err(error) => actions.push(Action::Inform(Notice::failure(format!(
            "The drawing was not changed: {error}."
        )))),
    }
}

pub fn reset_position(
    model: &Model,
    drawing: &Drawing,
    view: DrawnView,
    actions: &mut Vec<Action>,
) {
    apply_or_say(
        model
            .document()
            .changing_drawing(drawing.id, RESET_VIEW, |changed| {
                changed.positions.remove(&view);
            }),
        actions,
    );
}

fn annotation_moved_by(
    context: &SheetContext<'_>,
    annotation: Annotation,
    by: Vector2,
    actions: &mut Vec<Action>,
) {
    let label = Grip::Annotation(annotation).label();

    apply_or_say(
        context
            .model
            .document()
            .changing_drawing(context.drawing.id, label, |changed| {
                context.plan.moved_annotation(changed, annotation, by);
            }),
        actions,
    );
}

pub fn delete_annotation(
    model: &Model,
    drawing: &Drawing,
    annotation: Annotation,
    actions: &mut Vec<Action>,
) {
    let label = match annotation {
        Annotation::Dimension(_) => DELETE_DIMENSION,
        Annotation::Note(_) => DELETE_NOTE,
        Annotation::Centre(_) => DELETE_CENTRE_MARK,
        Annotation::Callout(_) => DELETE_CALLOUT,
    };

    apply_or_say(
        model
            .document()
            .changing_drawing(drawing.id, label, |changed| {
                changed.remove_annotation(annotation);
            }),
        actions,
    );
}

fn moved_by(context: &SheetContext<'_>, view: DrawnView, by: Vector2, actions: &mut Vec<Action>) {
    apply_or_say(
        context
            .model
            .document()
            .changing_drawing(context.drawing.id, MOVE_VIEW, |changed| {
                context.plan.moved(changed, view, by);
            }),
        actions,
    );
}

fn dropped(context: &SheetContext<'_>, drag: SheetDrag, actions: &mut Vec<Action>) {
    if (drag.to - drag.from).length() <= f64::EPSILON {
        return;
    }
    apply_or_say(
        context.model.document().changing_drawing(
            context.drawing.id,
            drag.grip.label(),
            |changed| {
                dragged(drag, context.plan, changed);
            },
        ),
        actions,
    );
}

fn move_buttons(
    ui: &mut Ui,
    canvas: &SheetCanvas,
    context: &SheetContext<'_>,
    actions: &mut Vec<Action>,
) {
    let picked = canvas
        .picked
        .filter(|view| context.plan.placed(*view).is_some());
    let annotation = canvas
        .picked_annotation
        .filter(|annotation| context.plan.annotation(*annotation).is_some());

    let fixed = annotation.is_some_and(|annotation| !Grip::Annotation(annotation).moves());
    let movement = match annotation {
        Some(_) => Some(ViewMovement::Free),
        None => picked.map(|view| context.plan.movement(view)),
    };
    let buttons = [
        (icons::MOVE_LEFT, MOVE_LEFT, Vector2::new(-FAR_NUDGE, 0.0)),
        (icons::MOVE_RIGHT, MOVE_RIGHT, Vector2::new(FAR_NUDGE, 0.0)),
        (icons::MOVE_UP, MOVE_UP, Vector2::new(0.0, FAR_NUDGE)),
        (icons::MOVE_DOWN, MOVE_DOWN, Vector2::new(0.0, -FAR_NUDGE)),
    ];

    for (glyph, name, by) in buttons {
        let refusal = match movement {
            _ if fixed => Some(CENTRE_STAYS),
            None => Some(PICK_FIRST),
            Some(ViewMovement::Up) if by.x != 0.0 => Some(ONLY_UP_AND_DOWN),
            Some(ViewMovement::Across) if by.y != 0.0 => Some(ONLY_ACROSS),
            Some(_) => None,
        };
        let clicked = ui
            .add_enabled_ui(refusal.is_none(), |ui| {
                widgets::icon_button(ui, glyph, name)
            })
            .inner
            .on_disabled_hover_text(refusal.unwrap_or_default())
            .clicked();

        if clicked {
            match (annotation, picked) {
                (Some(annotation), _) => annotation_moved_by(context, annotation, by, actions),
                (None, Some(view)) => moved_by(context, view, by, actions),
                (None, None) => {}
            }
        }
    }
}

pub fn toolbar(
    ui: &mut Ui,
    canvas: &mut SheetCanvas,
    context: &SheetContext<'_>,
    actions: &mut Vec<Action>,
) {
    move_buttons(ui, canvas, context, actions);

    let tools = [
        (
            icons::SHEET_DIMENSION,
            DIMENSION_TOOL,
            DIMENSION_HOVER,
            SheetTool::dimension(),
        ),
        (icons::SHEET_NOTE, NOTE_TOOL, NOTE_HOVER, SheetTool::note()),
        (
            icons::SHEET_CENTRE_MARK,
            CENTRE_TOOL,
            CENTRE_HOVER,
            SheetTool::Centre,
        ),
        (
            icons::SHEET_CALLOUT,
            CALLOUT_TOOL,
            CALLOUT_HOVER,
            SheetTool::callout(),
        ),
    ];

    for (glyph, name, hover, started) in tools {
        let active = canvas
            .tool
            .as_ref()
            .is_some_and(|tool| tool.same_kind(&started));

        if widgets::icon_button_for(ui, glyph, hover, name).clicked() {
            if active {
                canvas.tool = None;
            } else {
                canvas.start_tool(started);
            }
        }
    }

    let picked = canvas
        .picked
        .filter(|view| context.plan.placed(*view).is_some());
    let marked = ui
        .add_enabled_ui(picked.is_some(), |ui| {
            widgets::icon_button_for(
                ui,
                icons::SHEET_CENTRE_MARKS,
                CENTRE_MARKS_HOVER,
                CENTRE_MARKS,
            )
        })
        .inner
        .on_disabled_hover_text(PICK_VIEW_FIRST)
        .clicked();

    if marked && let Some(view) = picked {
        sheet_tools::add_centre_marks(context.model, context.drawing, context.views, view, actions);
    }

    let camera = &mut canvas.camera;

    if small_enabled(ui, camera.can_zoom_in(), icons::ZOOM_IN, ZOOM_IN)
        .on_disabled_hover_text(MOST_ZOOMED)
        .clicked()
    {
        camera.zoom_by(ZOOM_STEP);
    }
    if small_enabled(ui, camera.can_zoom_out(), icons::ZOOM_OUT, ZOOM_OUT)
        .on_disabled_hover_text(LEAST_ZOOMED)
        .clicked()
    {
        camera.zoom_by(1.0 / ZOOM_STEP);
    }
    if small_enabled(ui, true, icons::FIT_SHEET, FIT).clicked() {
        *camera = SheetCamera::default();
    }
    ui.label(widgets::muted(camera.percent(), ui))
        .on_hover_text(ZOOM_READOUT_HOVER);
}

fn small_enabled(ui: &mut Ui, enabled: bool, glyph: &str, text: &str) -> Response {
    let button = widgets::small_button(ui, glyph, text);
    ui.add_enabled(enabled, button)
}

fn sampled(piece: &ViewPiece) -> Vec<Point2> {
    sampled_range(&piece.curve, piece.range)
}

fn sampled_range(curve: &Curve2, range: Interval) -> Vec<Point2> {
    let steps = match curve {
        Curve2::Line(_) => 1,
        _ => CURVE_STEPS,
    };
    (0..=steps)
        .map(|step| {
            let along = step as f64 / steps as f64;
            let at = range.start() + (range.end() - range.start()) * along;
            curve.point(at)
        })
        .collect()
}

fn arc_points(center: Point2, radius: f64, start: f64, end: f64) -> Vec<Point2> {
    (0..=CURVE_STEPS)
        .map(|step| {
            let angle = start + (end - start) * step as f64 / CURVE_STEPS as f64;
            center + Vector2::from_angle(angle) * radius
        })
        .collect()
}

struct Paint<'a> {
    painter: &'a Painter,
    mapping: &'a Mapping,
    tokens: &'a Tokens,
}

impl Paint<'_> {
    fn view_lines(&self, placed: &PlacedView, lines: &ViewLines, hidden_lines: bool) {
        let (mapping, tokens) = (self.mapping, self.tokens);

        for (from, to) in drawn_hatching(lines, placed) {
            self.painter.line_segment(
                [mapping.screen(from), mapping.screen(to)],
                Stroke::new(THIN_WIDTH, tokens.text_muted),
            );
        }
        for piece in lines.drawn_pieces(hidden_lines) {
            let points: Vec<Pos2> = sampled(piece)
                .into_iter()
                .map(|point| mapping.screen(placed.on_sheet(point)))
                .collect();
            match piece.visibility {
                Visibility::Visible => {
                    self.painter
                        .add(Shape::line(points, Stroke::new(VISIBLE_WIDTH, tokens.text)));
                }
                _ => {
                    self.painter.extend(Shape::dashed_line(
                        &points,
                        Stroke::new(THIN_WIDTH, tokens.text_muted),
                        HIDDEN_DASH,
                        HIDDEN_GAP,
                    ));
                }
            }
        }
    }

    fn plan(&self, plan: &SheetPlan, views: &SheetViews, hidden_lines: bool) {
        let (painter, mapping, tokens) = (self.painter, self.mapping, self.tokens);
        let scale = mapping.scale;

        painter.rect_filled(mapping.rect, 0.0, tokens.sunken);

        let paper = Rect::from_two_pos(
            mapping.screen(Point2::new(0.0, 0.0)),
            mapping.screen(Point2::new(plan.width, plan.height)),
        );

        painter.rect_filled(paper, 0.0, tokens.raised);
        painter.rect_stroke(
            paper,
            0.0,
            Stroke::new(THIN_WIDTH, tokens.border_strong),
            StrokeKind::Inside,
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
                SheetMark::Arc { .. } => {
                    painter.add(Shape::line(
                        self.mark_points(mark),
                        Stroke::new(THIN_WIDTH, tokens.text),
                    ));
                }
            }
        }
        for text in &plan.texts {
            self.text(text, tokens.text);
        }
        for line in &plan.cutting {
            let chain = [mapping.screen(line.chain.0), mapping.screen(line.chain.1)];
            let dashes = CHAIN_DASHES.map(|length| (length * scale).max(1.0));
            let gaps = CHAIN_GAPS.map(|length| (length * scale).max(1.0));

            painter.extend(Shape::dashed_line_with_offset(
                &chain,
                Stroke::new(THIN_WIDTH, tokens.text),
                &dashes,
                &gaps,
                0.0,
            ));
            for (from, to) in line.ends {
                painter.line_segment(
                    [mapping.screen(from), mapping.screen(to)],
                    Stroke::new(VISIBLE_WIDTH, tokens.text),
                );
            }
        }
        for placed in &plan.views {
            if let Some((_, lines)) = views.finished().find(|(view, _)| *view == placed.view) {
                self.view_lines(placed, lines, hidden_lines);
            }
        }
        for laid in &plan.annotations {
            self.annotation(laid);
        }
    }

    fn text(&self, text: &SheetText, colour: Color32) {
        let size = text.height as f32 * self.mapping.scale / CAP_HEIGHT;

        if size < SMALLEST_TEXT {
            return;
        }

        let at = self.mapping.screen(text.at);

        if text.angle != 0.0 {
            let turn = -(text.angle as f32);
            let galley = self.painter.layout_no_wrap(
                text.content.clone(),
                FontId::proportional(size),
                colour,
            );

            let corner = at - Rot2::from_angle(turn) * (galley.size() / 2.0);

            self.painter
                .add(TextShape::new(corner, galley, colour).with_angle(turn));
            return;
        }

        let (at, align) = match text.anchor {
            TextAnchor::Start => (at + vec2(0.0, size * DESCENT), Align2::LEFT_BOTTOM),
            TextAnchor::Middle => (at, Align2::CENTER_CENTER),
        };

        self.painter
            .text(at, align, &text.content, FontId::proportional(size), colour);
    }

    fn mark_points(&self, mark: &SheetMark) -> Vec<Pos2> {
        let points = match *mark {
            SheetMark::Line(from, to) => vec![from, to],
            SheetMark::Circle { center, radius } => arc_points(center, radius, 0.0, TAU),
            SheetMark::Arc {
                center,
                radius,
                start,
                end,
            } => arc_points(center, radius, start, end),
        };
        points
            .into_iter()
            .map(|point| self.mapping.screen(point))
            .collect()
    }

    fn chained(&self, points: &[Pos2], stroke: Stroke) {
        let scale = self.mapping.scale;
        let dashes = CHAIN_DASHES.map(|length| (length * scale).max(1.0));
        let gaps = CHAIN_GAPS.map(|length| (length * scale).max(1.0));

        self.painter.extend(Shape::dashed_line_with_offset(
            points, stroke, &dashes, &gaps, 0.0,
        ));
    }

    fn annotation(&self, laid: &SheetAnnotation) {
        let lost = laid.problem.is_some();
        let colour = if lost {
            self.tokens.error
        } else {
            self.tokens.text
        };
        let stroke = Stroke::new(THIN_WIDTH, colour);
        let centre = matches!(laid.annotation, Annotation::Centre(_));

        for mark in &laid.marks {
            let points = self.mark_points(mark);

            if lost {
                self.painter
                    .extend(Shape::dashed_line(&points, stroke, BROKEN_DASH, BROKEN_GAP));
            } else if centre {
                self.chained(&points, stroke);
            } else {
                self.painter.add(Shape::line(points, stroke));
            }
        }
        for text in &laid.texts {
            self.text(text, colour);
        }
    }

    fn annotation_outline(&self, laid: &SheetAnnotation, picked: bool) {
        let Some(bounds) = laid.bounds() else {
            return;
        };

        self.outline_rect(
            Rect::from_two_pos(
                self.mapping.screen(bounds.min()),
                self.mapping.screen(bounds.max()),
            ),
            picked,
        );
    }

    fn picked_edge(&self, pick: &LinePick, plan: &SheetPlan, views: &SheetViews) {
        let Some(placed) = plan.placed(pick.view) else {
            return;
        };
        let Some((_, lines)) = views.finished().find(|(view, _)| *view == pick.view) else {
            return;
        };

        for seen in seen_line(lines, pick.line) {
            let points: Vec<Pos2> = sampled_range(&seen.curve, seen.range)
                .into_iter()
                .map(|point| self.mapping.screen(placed.on_sheet(point)))
                .collect();

            self.painter.add(Shape::line(
                points,
                Stroke::new(PICKED_EDGE_WIDTH, self.tokens.accent_text),
            ));
        }
    }

    fn outline(&self, placed: &PlacedView, picked: bool) {
        self.outline_rect(
            Rect::from_two_pos(
                self.mapping.screen(placed.extent.min()),
                self.mapping.screen(placed.extent.max()),
            ),
            picked,
        );
    }

    fn outline_rect(&self, rect: Rect, picked: bool) {
        let rect = rect.expand(OUTLINE_GAP);
        let stroke = Stroke::new(
            if picked { MARKED_WIDTH } else { THIN_WIDTH },
            self.tokens.accent_text,
        );

        if picked {
            self.painter
                .rect_stroke(rect, 0.0, stroke, StrokeKind::Outside);
        } else {
            let corners = [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
                rect.left_top(),
            ];

            self.painter.extend(Shape::dashed_line(
                &corners,
                stroke,
                OUTLINE_DASH,
                OUTLINE_SPACE,
            ));
        }
    }

    fn circle(&self, centre: Point2, radius: f64) {
        self.painter.circle_stroke(
            self.mapping.screen(centre),
            radius as f32 * self.mapping.scale,
            Stroke::new(MARKED_WIDTH, self.tokens.accent_text),
        );
    }

    fn grip(&self, grip: Grip, drawing: &Drawing, plan: &SheetPlan, picked: Option<DrawnView>) {
        match grip {
            Grip::View(view) => {
                if let Some(placed) = plan.placed(view)
                    && picked != Some(view)
                {
                    self.outline(placed, false);
                }
            }
            Grip::Centre(letter) | Grip::Rim(letter) => {
                if let Some((centre, radius)) = detail_circle(drawing, plan, letter) {
                    self.circle(centre, radius);
                }
            }
            Grip::Annotation(annotation) => {
                if let Some(laid) = plan.annotation(annotation) {
                    self.annotation_outline(laid, false);
                }
            }
        }
    }
}

fn steered(ui: &mut Ui, canvas: &mut SheetCanvas, response: &Response, plan: &SheetPlan) {
    if !response.has_focus() {
        return;
    }
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            response.id,
            EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: canvas.holds_escape(),
                ..EventFilter::default()
            },
        );
    });
    if canvas.holds_escape()
        && !ui.ctx().text_edit_focused()
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        canvas.let_go();
    }
    if canvas.picked.is_some() || canvas.picked_annotation.is_some() {
        return;
    }

    let step = PAN_STEP * plan.width as f32 / canvas.camera.zoom;
    let keys = [
        (Key::ArrowLeft, vec2(-step, 0.0)),
        (Key::ArrowRight, vec2(step, 0.0)),
        (Key::ArrowUp, vec2(0.0, step)),
        (Key::ArrowDown, vec2(0.0, -step)),
    ];
    for (key, by) in keys {
        if ui.input(|input| input.key_pressed(key)) {
            canvas.camera.pan += by;
        }
    }
}

fn nudged(ui: &Ui, response: &Response) -> Option<Vector2> {
    if !response.has_focus() {
        return None;
    }

    let directions = [
        (Key::ArrowLeft, Vector2::new(-1.0, 0.0)),
        (Key::ArrowRight, Vector2::new(1.0, 0.0)),
        (Key::ArrowUp, Vector2::new(0.0, 1.0)),
        (Key::ArrowDown, Vector2::new(0.0, -1.0)),
    ];

    let by: Vector2 = ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let (_, direction) = directions.iter().find(|(arrow, _)| arrow == key)?;
                    let step = if modifiers.shift { FAR_NUDGE } else { NUDGE };
                    Some(*direction * step)
                }
                _ => None,
            })
            .sum()
    });
    (by != Vector2::ZERO).then_some(by)
}

fn tool_context<'a>(context: &SheetContext<'a>) -> ToolContext<'a> {
    ToolContext {
        model: context.model,
        drawing: context.drawing,
        plan: context.plan,
        views: context.views,
    }
}

pub fn tool_row(
    ui: &mut Ui,
    canvas: &mut SheetCanvas,
    context: &SheetContext<'_>,
    actions: &mut Vec<Action>,
) {
    let Some(tool) = canvas.tool.as_mut() else {
        return;
    };

    match sheet_tools::tool_row(ui, tool, &tool_context(context), actions) {
        ToolRow::Kept => {}
        ToolRow::Stopped => canvas.tool = None,
        ToolRow::Placed(annotation) => {
            canvas.tool = None;
            canvas.pick_annotation(Some(annotation));
            canvas.focus_pending = true;
        }
    }
}

fn shown_with_tool(
    canvas: &SheetCanvas,
    context: &SheetContext<'_>,
    pointer: Option<Point2>,
) -> Option<Drawing> {
    let tool = canvas.tool.as_ref()?;
    let mut shown = context.drawing.clone();

    match tool {
        SheetTool::Dimension { .. } => {
            let draft = sheet_tools::draft_dimension(tool, &tool_context(context), pointer?)?;

            sheet_tools::with_draft(&mut shown, draft);
        }
        SheetTool::Note {
            at: Some(at),
            text,
            leader,
            ..
        } => {
            let id = caditor_document::AnnotationId::from_raw(shown.first_free_annotation());
            let shown_text = if text.trim().is_empty() {
                sheet_tools::NOTE_FIELD.to_owned()
            } else {
                text.clone()
            };

            shown.notes.push(caditor_document::DrawingNote {
                id,
                text: caditor_document::note_text(&shown_text),
                at: caditor_document::note_place(context.plan, *at),
                leader: leader.map(|pick| caditor_document::NoteLeader {
                    view: pick.view,
                    edge: pick.edge,
                }),
            });
        }
        SheetTool::Callout { .. } => {
            let draft = sheet_tools::draft_callout(tool, &tool_context(context), pointer?)?;

            shown.callouts.push(draft);
        }
        SheetTool::Note { .. } | SheetTool::Centre => return None,
    }
    Some(shown)
}

fn menu(
    response: &Response,
    canvas: &mut SheetCanvas,
    context: &SheetContext<'_>,
    actions: &mut Vec<Action>,
) {
    response.context_menu(|ui| {
        widgets::fitted_menu(ui, |ui| {
            if let Some(view) = canvas.menu_view {
                let placed = context.drawing.position(view).is_some();
                let reset = ui
                    .add_enabled_ui(placed, |ui| {
                        widgets::menu_item(ui, icons::RESET, RESET_POSITION, None)
                    })
                    .inner
                    .on_disabled_hover_text(NOT_PLACED);

                if reset.clicked() {
                    reset_position(context.model, context.drawing, view, actions);
                    ui.close();
                }
            }
            if widgets::menu_item(ui, icons::FIT_SHEET, FIT, None).clicked() {
                canvas.camera = SheetCamera::default();
                ui.close();
            }
        });
    });
}

pub fn canvas(
    ui: &mut Ui,
    canvas: &mut SheetCanvas,
    context: &SheetContext<'_>,
    size: Vec2,
    actions: &mut Vec<Action>,
) {
    let plan = context.plan;
    let tokens = appearance::tokens(ui);

    if canvas
        .picked
        .is_some_and(|view| plan.placed(view).is_none())
    {
        canvas.picked = None;
    }
    if canvas
        .picked_annotation
        .is_some_and(|annotation| !context.drawing.has_annotation(annotation))
    {
        canvas.picked_annotation = None;
    }

    let (rect, response) =
        ui.allocate_exact_size(size.max(Vec2::splat(1.0)), Sense::click_and_drag());
    let response = widgets::named(response, SHEET).on_hover_text(SHEET_HOVER);

    if std::mem::take(&mut canvas.focus_pending) {
        response.request_focus();
    }

    let fit = (rect.width() / plan.width.max(1.0) as f32)
        .min(rect.height() / plan.height.max(1.0) as f32);

    if response.hovered() {
        let zoom = ui.input(|input| input.zoom_delta());
        let from_centre = response
            .hover_pos()
            .map_or(Vec2::ZERO, |pointer| pointer - rect.center());

        if (zoom - 1.0).abs() > f32::EPSILON {
            canvas.camera.zoom_about(zoom, from_centre, fit);
        }
    }

    let scale = fit * canvas.camera.zoom;
    let mapping = canvas.camera.mapping(rect, plan, scale);
    let placing = canvas.tool.as_ref().is_some_and(SheetTool::places);

    if response.drag_started() {
        let from = ui
            .input(|input| input.pointer.press_origin())
            .map(|pressed| mapping.sheet(pressed));

        canvas.drag = from.filter(|_| !placing).and_then(|from| {
            let grip = grip_at(context.drawing, plan, from, mapping.reach())
                .filter(|grip| grip.moves())?;
            Some(SheetDrag {
                grip,
                from,
                to: from,
            })
        });
        match canvas.drag.map(|drag| drag.grip) {
            Some(Grip::View(view)) => {
                canvas.picked = Some(view);
                canvas.picked_annotation = None;
            }
            Some(Grip::Annotation(annotation)) => canvas.pick_annotation(Some(annotation)),
            _ => {}
        }
        response.request_focus();
    }
    if response.dragged() && !placing {
        match (canvas.drag.as_mut(), response.interact_pointer_pos()) {
            (Some(drag), Some(pointer)) => drag.to = mapping.sheet(pointer),
            (Some(_), None) => {}
            (None, _) => {
                let moved = response.drag_delta() / scale;

                canvas.camera.pan += vec2(-moved.x, moved.y);
            }
        }
    }
    if response.drag_stopped() {
        if let Some(drag) = canvas.drag.take() {
            dropped(context, drag, actions);
        } else if placing && let Some(pointer) = response.interact_pointer_pos() {
            tool_clicked(
                canvas,
                context,
                mapping.sheet(pointer),
                mapping.reach(),
                actions,
            );
        }
    }
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let point = mapping.sheet(pointer);

        if canvas.tool.is_some() {
            tool_clicked(canvas, context, point, mapping.reach(), actions);
        } else if let Some(annotation) = plan.annotation_at(point, mapping.reach()) {
            canvas.pick_annotation(Some(annotation));
        } else {
            canvas.picked_annotation = None;
            canvas.picked = view_at(plan, point, mapping.reach());
        }
        response.request_focus();
    }
    if response.secondary_clicked() {
        canvas.menu_view = response
            .interact_pointer_pos()
            .and_then(|pointer| view_at(plan, mapping.sheet(pointer), mapping.reach()));
    }
    if let Some(by) = nudged(ui, &response) {
        match (canvas.picked_annotation, canvas.picked) {
            (Some(annotation), _) if !Grip::Annotation(annotation).moves() => {}
            (Some(annotation), _) => annotation_moved_by(context, annotation, by, actions),
            (None, Some(view)) => moved_by(context, view, by, actions),
            (None, None) => {}
        }
    }
    canvas.id = Some(response.id);
    if std::mem::take(&mut canvas.delete_pending)
        && let Some(annotation) = canvas.picked_annotation.take()
    {
        delete_annotation(context.model, context.drawing, annotation, actions);
    }
    steered(ui, canvas, &response, plan);

    let mapping = canvas.camera.mapping(rect, plan, scale);
    let pointer = response.hover_pos().map(|pointer| mapping.sheet(pointer));
    let shown_drawing = match canvas.drag {
        Some(drag) => {
            let mut moved = context.drawing.clone();

            dragged(drag, plan, &mut moved);
            Cow::Owned(moved)
        }
        None => shown_with_tool(canvas, context, pointer)
            .map_or(Cow::Borrowed(context.drawing), Cow::Owned),
    };
    let shown_plan = match shown_drawing {
        Cow::Owned(_) => Cow::Owned(plan_of(context.model, &shown_drawing, context.views)),
        Cow::Borrowed(_) => Cow::Borrowed(plan),
    };
    let painter = ui.painter_at(rect);

    let paint = Paint {
        painter: &painter,
        mapping: &mapping,
        tokens: &tokens,
    };

    paint.plan(&shown_plan, context.views, context.drawing.hidden_lines);

    if let Some(placed) = canvas.picked.and_then(|view| shown_plan.placed(view)) {
        paint.outline(placed, true);
    }
    if let Some(laid) = canvas
        .picked_annotation
        .and_then(|annotation| shown_plan.annotation(annotation))
    {
        paint.annotation_outline(laid, true);
    }

    if let Some(tool) = canvas.tool.as_ref() {
        let hovered = pointer.and_then(|pointer| {
            let lines = context.views.finished().collect();
            let hidden = context.drawing.hidden_lines;

            if tool.picks_outlines() {
                caditor_document::pick_line(plan, &lines, hidden, pointer, mapping.reach())
            } else {
                caditor_document::pick_edge(plan, &lines, hidden, pointer, mapping.reach())
                    .map(LinePick::from)
            }
        });

        for pick in tool.picks().iter().chain(hovered.iter()) {
            paint.picked_edge(pick, plan, context.views);
        }
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    } else {
        let hovered = match canvas.drag {
            Some(drag) => Some(drag.grip),
            None => pointer
                .and_then(|pointer| grip_at(&shown_drawing, &shown_plan, pointer, mapping.reach())),
        };

        if let Some(grip) = hovered {
            paint.grip(grip, &shown_drawing, &shown_plan, canvas.picked);
            ui.ctx().set_cursor_icon(grip.cursor(canvas.drag.is_some()));
        }
    }
    menu(&response, canvas, context, actions);
    if response.has_focus() {
        widgets::paint_focus_ring(ui, rect);
    }
    canvas.mapping = Some(mapping);
}

fn tool_clicked(
    canvas: &mut SheetCanvas,
    context: &SheetContext<'_>,
    point: Point2,
    reach: f64,
    actions: &mut Vec<Action>,
) {
    let Some(tool) = canvas.tool.as_mut() else {
        return;
    };

    match sheet_tools::clicked(tool, &tool_context(context), point, reach, actions) {
        Clicked::Kept => {}
        Clicked::Placed(annotation) => canvas.picked_annotation = Some(annotation),
        Clicked::Done => canvas.tool = None,
    }
}
