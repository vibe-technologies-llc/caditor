use std::{borrow::Cow, collections::BTreeMap};

use caditor_document::{
    Annotation, AnnotationId, CalloutLine, CentreTarget, DetailParent, DetailScale, DetailView,
    DimensionDirection, DimensionMeasure, DimensionTolerance, Document, Drawing, DrawingDimension,
    DrawingId, DrawingScale, DrawingViewError, DrawnView, FIT_GRADES, FeatureId, FitLetter, IsoFit,
    MAX_DIMENSION_DECIMALS, MAX_DRAWING_NAME_CHARS, MAX_DRAWINGS, MAX_FIT_SIZE, MAX_TEXT_OFFSET,
    MIN_DETAIL_RADIUS, Projection, ScaleRatio, SectionPlace, SectionPlane, SectionView,
    SheetOrientation, SheetPlan, SheetSize, StandardView, TitleFields, Transaction, callout_lines,
    callout_texts, deviation_text, dimension_problem, lay_out_sheet,
};
use caditor_expression::{Dimension, Expression, format_decimals};
use caditor_geometry::Point2;
use egui::{Align, Area, Id, Key, Layout, Order, Rect, Sense, TextEdit, Ui, UiBuilder, vec2};
use thiserror::Error;

use crate::{
    appearance::{self, SPACE_M, SPACE_S},
    commands::{Command, CommandFrame, Reason},
    feature_tree::count,
    field::{self, ExpressionError},
    files::FileCommand,
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    sheet_canvas::{self, SheetCanvas, SheetContext},
    sheet_export::SheetJob,
    sheet_tools::{self, SheetTool},
    sheet_views::{DrawingViews, SheetViews},
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "Drawings";
pub const CLOSE: &str = "Close the drawings panel";
pub const NEW: &str = "New drawing";
pub const EXPORT: &str = "Export…";
pub const RENAME: &str = "Rename";
pub const DELETE: &str = "Delete";
pub const SHEET_TO_VIEW: &str = "Show the sheet in the view";
pub const SHEET_TO_PANEL: &str = "Show the sheet in this panel";
pub const LEAVE_SHEET_VIEW: &str = "Back to the 3D view";
pub const SHEET_IN_VIEW_NOTE: &str = "The sheet is shown in the view, where its views and detail \
                                      circles can be dragged.";
pub const LAYOUT: &str = "Layout";
pub const LAID_OUT: &str = "Every view stands where the drawing lays it out. Drag a view on the \
                            sheet to place it by hand; a top or side view moves only along its \
                            line with the front view.";
pub const NONE_YET: &str = "The model has no drawings yet. A drawing lays out views of the bodies \
                            on an ISO sheet with a title block, ready to print or send.";
pub const WORKING: &str = "Working out the views…";
pub const EVERY_BODY: &str = "Every shown body";
pub const USE_SELECTED: &str = "Use the selected bodies";
pub const USE_EVERY_BODY: &str = "Draw every shown body";
pub const HIDDEN_LINES: &str = "Hidden lines";
pub const FIT_SCALE: &str = "Fit the sheet";
pub const SECTION_VIEWS: &str = "Section views";
pub const ADD_SECTION: &str = "Add a section view";
pub const CUT_ON: &str = "Cut on";
pub const CUT_PLANE: &str = "Plane";
pub const THROUGH: &str = "Through";
pub const CENTRE: &str = "Centre";
pub const AT_OFFSET: &str = "Offset";
pub const DISTANCE: &str = "Distance";
pub const LOOK_BACK: &str = "Look the other way";
pub const NO_SECTIONS: &str = "A section view cuts the bodies along a plane seen as a line in \
                               one of the views, and shows the cut faces hatched.";
pub const DETAIL_VIEWS: &str = "Detail views";
pub const ADD_DETAIL: &str = "Add a detail view";
pub const ENLARGES: &str = "Enlarges";
pub const CENTRE_X: &str = "Centre across";
pub const CENTRE_Y: &str = "Centre up";
pub const RADIUS: &str = "Radius";
pub const DETAIL_SCALE: &str = "Scale";
pub const NO_DETAILS: &str = "A detail view enlarges a circle of one of the other views, drawn \
                              thin on that view and lettered, with the lines inside it clipped \
                              exactly to the circle.";
pub const ANNOTATIONS: &str = "Annotations";
pub const NO_ANNOTATIONS: &str = "A dimension measures edges of a view from the model, so it \
                                  follows every edit; a note writes text on the sheet, with a \
                                  leader to an edge if you like; centre marks and centrelines \
                                  mark circles and cylinders, and a callout reads a hole or a \
                                  thread. Add them with the buttons above the sheet.";
pub const TOLERANCE: &str = "Tolerance";
pub const NO_TOLERANCE: &str = "No tolerance";
pub const DEVIATION: &str = "Deviation";
pub const UPPER_DEVIATION: &str = "Upper deviation";
pub const LOWER_DEVIATION: &str = "Lower deviation";
pub const FIT_LETTER: &str = "Fit";
pub const FIT_GRADE: &str = "Grade";
pub const SHOW_DEVIATIONS: &str = "Show the deviations";
const DEFAULT_DEVIATION: f64 = 0.1;
pub const DECIMALS: &str = "Decimals";
pub const MEASURES: &str = "Measures";
pub const TEXT_OFFSET: &str = "Value along";
const PANEL: RightPanel = RightPanel { id: "drawings" };
const HEADER: PanelTitle<'static> = PanelTitle {
    glyph: icons::DRAWINGS,
    title: TITLE,
    close: CLOSE,
    help: Some(Page::Drawings),
};
const DEFAULT_DETAIL_RADIUS: f64 = 10.0;
const DETAIL_RADIUS_SHARE: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum SheetRefusal {
    #[error("The model has no drawings; New drawing makes one")]
    NoDrawing,
    #[error("The drawing's views are still being worked out")]
    Working,
    #[error("A model keeps at most {MAX_DRAWINGS} drawings")]
    TooMany,
    #[error("Click a view on the drawing sheet first to pick it")]
    NoViewPicked,
}

impl Reason for SheetRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Default)]
pub struct DrawingsTool {
    pub open: bool,
    pub chosen: Option<DrawingId>,
    views: DrawingViews,
    sheet: SheetViews,
    canvas: SheetCanvas,
    in_view: bool,
    renaming: Option<String>,
    rename_focus_pending: bool,
    export_pending: bool,
    note_editing: Option<(AnnotationId, String)>,
    note_focus_pending: bool,
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
        self.note_editing = None;
        self.export_pending = false;
        self.in_view = false;
        self.canvas.reset();
    }

    pub fn sheet_in_view(&self) -> bool {
        self.open && self.in_view
    }

    pub fn show_sheet_in_view(&mut self, shown: bool) {
        if shown == self.sheet_in_view() {
            return;
        }
        self.open = true;
        self.in_view = shown;
        self.canvas.reset();
        if shown {
            self.canvas.focus();
        }
    }

    #[cfg(test)]
    pub fn canvas(&self) -> &SheetCanvas {
        &self.canvas
    }

    pub fn claim_delete(&mut self, ctx: &egui::Context) {
        if self.open {
            self.canvas.claim_delete(ctx);
        }
    }

    pub fn start_tool(&mut self, started: SheetTool) {
        if !self.open {
            self.show_sheet_in_view(true);
        }
        self.open = true;
        self.canvas.start_tool(started);
    }

    pub fn escape_sheet_view(&mut self) {
        if !self.canvas.let_go() {
            self.show_sheet_in_view(false);
        }
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
            self.in_view = false;
            return;
        };

        if self.chosen != Some(drawing.id) {
            self.chosen = Some(drawing.id);
            self.canvas.reset();
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
        Some(plan_of(model, drawing, &self.sheet))
    }

    pub fn job(&self, model: &Model) -> Result<SheetJob, SheetRefusal> {
        let document = model.document();
        let drawing = self.current(document).ok_or(SheetRefusal::NoDrawing)?;

        if !self.sheet.is_ready() || self.sheet.drawing != Some(drawing.id) {
            return Err(SheetRefusal::Working);
        }

        let plan = plan_of(model, drawing, &self.sheet);
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

pub fn plan_of(model: &Model, drawing: &Drawing, sheet: &SheetViews) -> SheetPlan {
    let document = model.document();
    let fields = TitleFields::of(document.properties(), &model.display_name(), drawing);
    let extents = sheet
        .finished()
        .filter_map(|(view, lines)| Some((view, lines.bounds?)))
        .collect::<BTreeMap<_, _>>();

    let mut plan = lay_out_sheet(drawing, &fields, &extents, &sheet.cuts());
    let callouts = callout_texts(document, model.evaluation(), drawing);

    plan.annotate(drawing, &sheet.finished().collect(), &callouts);
    plan
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
            tool.canvas.reset();
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
    if commands.invoke(Command::SheetInView, &exportable) {
        let shown = !tool.sheet_in_view();

        tool.show_sheet_in_view(shown);
    }
    if commands.invoke(Command::DrawingDimension, &exportable) {
        tool.start_tool(SheetTool::dimension());
    }
    if commands.invoke(Command::DrawingNote, &exportable) {
        tool.start_tool(SheetTool::note());
    }
    if commands.invoke(Command::DrawingCentreMark, &exportable) {
        tool.start_tool(SheetTool::Centre);
    }
    if commands.invoke(Command::DrawingCallout, &exportable) {
        tool.start_tool(SheetTool::callout());
    }

    let drawing = tool.current(model.document());
    let picked = tool
        .canvas
        .picked()
        .filter(|view| drawing.is_some_and(|drawing| drawing.has_view(*view)));

    let markable = match (drawing, picked) {
        (None, _) => Err(SheetRefusal::NoDrawing),
        (Some(_), None) => Err(SheetRefusal::NoViewPicked),
        (Some(_), Some(_)) => Ok(()),
    };

    if commands.invoke(Command::DrawingCentreMarks, &markable)
        && let (Some(drawing), Some(view)) = (drawing, picked)
    {
        sheet_tools::add_centre_marks(model, drawing, &tool.sheet, view, actions);
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
    let field_id = Id::unique(("drawing-name", id));

    let pointer_busy = ui.input(|input| input.pointer.any_down() || input.pointer.any_released());

    if tool.rename_focus_pending && !pointer_busy {
        tool.rename_focus_pending = false;
        field::select_all(ui.ctx(), field_id, name);
        ui.memory_mut(|memory| memory.request_focus(field_id));
    }
    ui.horizontal(|ui| {
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(name)
                    .id(field_id)
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
    if done && let Some(name) = tool.renaming.clone() {
        let renamed = model
            .document()
            .changing_drawing(id, "Rename drawing", |drawing| drawing.name = name);

        match renamed {
            Ok(transaction) => {
                tool.renaming = None;
                actions.push(Action::Apply(transaction));
            }
            Err(error) => {
                tool.rename_focus_pending = true;
                actions.push(Action::Inform(Notice::failure(format!(
                    "The drawing was not renamed: {error}."
                ))));
            }
        }
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

pub fn offset_field_id(drawing: DrawingId, letter: char) -> Id {
    Id::unique(("drawing-section-offset", drawing.raw(), letter))
}

fn offset_text(model: &Model, offset: &Expression) -> String {
    model.document().expression_text(offset)
}

fn offset_row(
    ui: &mut Ui,
    model: &Model,
    drawing: DrawingId,
    section: &SectionView,
    offset: &Expression,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let parameters = model.parameters();
    let units = model.units();
    let letter = section.letter;

    widgets::caption(ui, DISTANCE);

    let enter = |text: &str| -> Result<Expression, ExpressionError> {
        field::parse_expression(
            document,
            parameters,
            text,
            field::Expected {
                dimension: Some(Dimension::LENGTH),
                non_negative: false,
            },
            units,
        )
    };

    let field = field::value_field(
        ui,
        offset_field_id(drawing, letter),
        &offset_text(model, offset),
        widgets::FIELD_WIDTH,
        false,
        enter,
    );
    let problem = field.problem(ui, document, enter);

    field.response.on_hover_text(format!(
        "How far the plane stands from the origin along {}; units and parameters may be typed",
        section.plane.axis_name()
    ));
    ui.end_row();
    if let Some(error) = field.error {
        widgets::error_row(ui, &error);
    } else if let Some(problem) = problem {
        problem.row(ui);
    }
    if let Some(value) = field.committed {
        change(
            model,
            drawing,
            "Move section plane",
            |drawing| {
                if let Some(section) = drawing.section_mut(letter) {
                    section.place = SectionPlace::Offset(value);
                }
            },
            actions,
        );
    }
}

fn section_settings(
    ui: &mut Ui,
    model: &Model,
    drawing: DrawingId,
    section: &SectionView,
    cut_at: Option<f64>,
    actions: &mut Vec<Action>,
) {
    let letter = section.letter;
    let edit = |label: &str, actions: &mut Vec<Action>, apply: &dyn Fn(&mut SectionView)| {
        change(
            model,
            drawing,
            label,
            |drawing| {
                if let Some(section) = drawing.section_mut(letter) {
                    apply(section);
                }
            },
            actions,
        );
    };

    widgets::properties(ui, ("drawing-section", drawing.raw(), letter), |ui| {
        widgets::property(ui, CUT_ON, |ui| {
            let mut chosen = None;

            widgets::combo(
                ("drawing-section-parent", drawing.raw(), letter),
                format!("{} view", section.parent.name()),
            )
            .hover("The view the cutting line is drawn on")
            .show(ui, |ui| {
                for view in StandardView::SECTIONED {
                    let label = format!("{} view", view.name());
                    if widgets::menu_option(ui, view == section.parent, &label).clicked() {
                        chosen = Some(view);
                    }
                }
            });
            if let Some(parent) = chosen {
                edit("Change section parent view", actions, &|section| {
                    section.parent = parent;
                    if !section.plane.is_edge_on_in(parent)
                        && let Some(plane) = SectionPlane::edge_on_in(parent).next()
                    {
                        section.plane = plane;
                    }
                });
            }
        });
        widgets::property(ui, CUT_PLANE, |ui| {
            let planes: Vec<SectionPlane> = SectionPlane::edge_on_in(section.parent).collect();
            let hovers: Vec<String> = planes
                .iter()
                .map(|plane| format!("Cut square to {}", plane.axis_name()))
                .collect();

            let choices: Vec<(&str, &str)> = planes
                .iter()
                .zip(&hovers)
                .map(|(plane, hover)| (plane.name(), hover.as_str()))
                .collect();
            let selected = planes
                .iter()
                .position(|plane| *plane == section.plane)
                .unwrap_or(usize::MAX);
            if let Some(plane) = widgets::segmented(ui, &choices, selected)
                .and_then(|index| planes.get(index).copied())
            {
                edit("Change section plane", actions, &|section| {
                    section.plane = plane;
                });
            }
        });
        widgets::property(ui, THROUGH, |ui| {
            let choices = [
                (CENTRE, "Through the middle of the drawn bodies"),
                (AT_OFFSET, "At a distance from the origin"),
            ];
            let selected = match &section.place {
                SectionPlace::Centre => 0,
                SectionPlace::Offset(_) => 1,
            };
            match widgets::segmented(ui, &choices, selected) {
                Some(0) => edit("Cut through the centre", actions, &|section| {
                    section.place = SectionPlace::Centre;
                }),
                Some(1) => {
                    let start = model.units().measured(cut_at.unwrap_or(0.0));

                    edit("Move section plane", actions, &|section| {
                        section.place = SectionPlace::Offset(start.clone());
                    });
                }
                _ => {}
            }
        });
        if let SectionPlace::Offset(offset) = &section.place {
            offset_row(ui, model, drawing, section, offset, actions);
        }
    });

    let mut reversed = section.reversed;
    if widgets::checkbox(ui, &mut reversed, LOOK_BACK)
        .on_hover_text("Keep the other side of the plane and look at it from this side")
        .changed()
    {
        edit("Turn section view", actions, &|section| {
            section.reversed = reversed;
        });
    }
}

fn sections_rows(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    sheet: &SheetViews,
    actions: &mut Vec<Action>,
) {
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title(SECTION_VIEWS));
    if drawing.sections.is_empty() {
        ui.label(widgets::muted(NO_SECTIONS, ui));
    }

    let cuts = sheet.cuts();

    for section in &drawing.sections {
        let letter = section.letter;

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(ui, &format!("Section {}", section.label()), 1);

            let delete = format!("Delete section {}", section.label());
            if widgets::icon_button(ui, icons::DELETE, &delete).clicked() {
                change(
                    model,
                    drawing.id,
                    "Delete section view",
                    |drawing| {
                        drawing.sections.retain(|kept| kept.letter != letter);
                        drawing
                            .details
                            .retain(|kept| kept.parent != DetailParent::Section(letter));
                    },
                    actions,
                );
            }
        });

        let cut_at = cuts
            .get(&letter)
            .map(|cutting| cutting.origin().dot(section.plane.normal()));

        section_settings(ui, model, drawing.id, section, cut_at, actions);
    }
    ui.add_space(SPACE_S);

    let room = drawing.unused_letter().is_some();
    let add = small_enabled(ui, room, icons::ADD, ADD_SECTION)
        .on_hover_text(
            "Cut the bodies along a principal plane and lay the section out beside the views",
        )
        .on_disabled_hover_text("A drawing holds a section view for each letter at most");
    if add.clicked() {
        apply_or_say(
            model
                .document()
                .adding_section_view(drawing.id)
                .map(|(transaction, _)| transaction),
            actions,
        );
    }
}

pub fn detail_field_id(drawing: DrawingId, letter: char, field: &'static str) -> Id {
    Id::unique(("drawing-detail", drawing.raw(), letter, field))
}

fn length_row(
    ui: &mut Ui,
    model: &Model,
    id: Id,
    caption: &str,
    millimetres: f64,
    non_negative: bool,
    hover: &str,
) -> Option<f64> {
    let document = model.document();
    let parameters = model.parameters();
    let units = model.units();

    widgets::caption(ui, caption);

    let enter = |text: &str| -> Result<f64, ExpressionError> {
        let expression = field::parse_expression(
            document,
            parameters,
            text,
            field::Expected {
                dimension: Some(Dimension::LENGTH),
                non_negative,
            },
            units,
        )?;
        expression
            .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
            .map_err(ExpressionError::Unevaluable)
    };

    let field = field::value_field(
        ui,
        id,
        &model
            .document()
            .expression_text(&units.measured(millimetres)),
        widgets::FIELD_WIDTH,
        false,
        enter,
    );
    let problem = field.problem(ui, document, enter);

    field.response.on_hover_text(hover);
    ui.end_row();
    if let Some(error) = field.error {
        widgets::error_row(ui, &error);
    } else if let Some(problem) = problem {
        problem.row(ui);
    }
    field.committed
}

fn parent_label(parent: DetailParent) -> String {
    match parent {
        DetailParent::Standard(view) => format!("{} view", view.name()),
        DetailParent::Section(letter) => format!("Section {letter}\u{2013}{letter}"),
    }
}

fn detail_parents(drawing: &Drawing) -> Vec<DetailParent> {
    StandardView::ALL
        .into_iter()
        .filter(|view| drawing.views.contains(view))
        .map(DetailParent::Standard)
        .chain(
            drawing
                .sections
                .iter()
                .map(|section| DetailParent::Section(section.letter)),
        )
        .collect()
}

fn detail_settings(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    detail: &DetailView,
    actions: &mut Vec<Action>,
) {
    let letter = detail.letter;
    let id = drawing.id;

    let edit = |label: &str, actions: &mut Vec<Action>, apply: &dyn Fn(&mut DetailView)| {
        change(
            model,
            id,
            label,
            |drawing| {
                if let Some(detail) = drawing.detail_mut(letter) {
                    apply(detail);
                }
            },
            actions,
        );
    };

    widgets::properties(ui, ("drawing-detail", id.raw(), letter), |ui| {
        widgets::property(ui, ENLARGES, |ui| {
            let mut chosen = None;

            widgets::combo(
                ("drawing-detail-parent", id.raw(), letter),
                parent_label(detail.parent),
            )
            .hover("The view whose circle is enlarged")
            .show(ui, |ui| {
                for parent in detail_parents(drawing) {
                    if widgets::menu_option(ui, parent == detail.parent, &parent_label(parent))
                        .clicked()
                    {
                        chosen = Some(parent);
                    }
                }
            });
            if let Some(parent) = chosen {
                edit("Change detail parent view", actions, &|detail| {
                    detail.parent = parent;
                });
            }
        });

        let across = length_row(
            ui,
            model,
            detail_field_id(id, letter, "centre-across"),
            CENTRE_X,
            detail.centre.x,
            false,
            "Where the circle's centre stands across the view, in model lengths; units and \
             parameters may be typed",
        );

        if let Some(x) = across {
            edit("Move detail circle", actions, &|detail| {
                detail.centre.x = x;
            });
        }

        let up = length_row(
            ui,
            model,
            detail_field_id(id, letter, "centre-up"),
            CENTRE_Y,
            detail.centre.y,
            false,
            "Where the circle's centre stands up the view, in model lengths; units and \
             parameters may be typed",
        );

        if let Some(y) = up {
            edit("Move detail circle", actions, &|detail| {
                detail.centre.y = y;
            });
        }

        let radius = length_row(
            ui,
            model,
            detail_field_id(id, letter, "radius"),
            RADIUS,
            detail.radius,
            true,
            "The circle's radius, in model lengths; units and parameters may be typed",
        );

        if let Some(radius) = radius {
            edit("Resize detail circle", actions, &|detail| {
                detail.radius = radius;
            });
        }
        widgets::property(ui, DETAIL_SCALE, |ui| {
            let mut chosen = None;

            widgets::combo(
                ("drawing-detail-scale", id.raw(), letter),
                detail.scale.label(),
            )
            .hover("Sheet length to model length of the enlarged circle")
            .show(ui, |ui| {
                if widgets::menu_option(
                    ui,
                    detail.scale == DetailScale::Enlarged,
                    &DetailScale::Enlarged.label(),
                )
                .clicked()
                {
                    chosen = Some(DetailScale::Enlarged);
                }
                for ratio in ScaleRatio::PREFERRED {
                    let scale = DetailScale::Ratio(ratio);
                    if widgets::menu_option(ui, detail.scale == scale, &ratio.label()).clicked() {
                        chosen = Some(scale);
                    }
                }
            });
            if let Some(scale) = chosen {
                edit("Change detail scale", actions, &|detail| {
                    detail.scale = scale;
                });
            }
        });
    });
}

fn default_circle(sheet: &SheetViews, parent: DetailParent) -> (Point2, f64) {
    sheet
        .finished()
        .find(|(view, _)| *view == parent.drawn())
        .and_then(|(_, lines)| lines.bounds)
        .map_or((Point2::ZERO, DEFAULT_DETAIL_RADIUS), |bounds| {
            let size = bounds.size();
            (
                bounds.min(),
                (size.x.min(size.y) * DETAIL_RADIUS_SHARE).max(MIN_DETAIL_RADIUS),
            )
        })
}

fn details_rows(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    sheet: &SheetViews,
    actions: &mut Vec<Action>,
) {
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title(DETAIL_VIEWS));
    if drawing.details.is_empty() {
        ui.label(widgets::muted(NO_DETAILS, ui));
    }
    for detail in &drawing.details {
        let letter = detail.letter;

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(ui, &format!("Detail {letter}"), 1);

            let delete = format!("Delete detail {letter}");
            if widgets::icon_button(ui, icons::DELETE, &delete).clicked() {
                change(
                    model,
                    drawing.id,
                    "Delete detail view",
                    |drawing| drawing.details.retain(|kept| kept.letter != letter),
                    actions,
                );
            }
        });
        detail_settings(ui, model, drawing, detail, actions);
    }
    ui.add_space(SPACE_S);

    let room = drawing.unused_letter().is_some();
    let add = small_enabled(ui, room, icons::ADD, ADD_DETAIL)
        .on_hover_text("Enlarge a circle of one of the views and lay it out beside the views")
        .on_disabled_hover_text("A drawing holds a section or detail view for each letter at most");
    if add.clicked() {
        let parent = drawing.default_detail_parent();
        let (centre, radius) = default_circle(sheet, parent);

        apply_or_say(
            model
                .document()
                .adding_detail_view(drawing.id, parent, centre, radius)
                .map(|(transaction, _)| transaction),
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
        DrawingViewError::PlaneMisses => "its cutting plane passes through none of the drawn \
                                          bodies: cut through the centre, or choose an offset \
                                          that crosses them"
            .to_owned(),
        DrawingViewError::Offset(_) | DrawingViewError::OffsetOutOfRange => {
            format!("{error}; correct the Distance or the parameters it uses")
        }
        DrawingViewError::ParentFailed => {
            format!("{error}; correct that view first, or enlarge another")
        }
        DrawingViewError::ParentNotDrawn => {
            format!("{error}; show that view in the drawing, or enlarge another")
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
                "The {} could not be drawn: {}.",
                view.describe(),
                reason(error)
            ))
        });

        widgets::announced(ui, &response, true);
        ui.add_space(SPACE_S);
    }
    for letter in &plan.unmarked {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(format!(
                "The cutting line of section {letter}\u{2013}{letter} is not drawn, since the \
                 view it is cut on is not on the sheet. Show that view, or cut on another."
            ))
        });
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
                tool.canvas.reset();
                tool.renaming = None;
            }
            ui.horizontal_wrapped(|ui| {
                let room = document.drawings().drawings.len() < MAX_DRAWINGS;

                ui.label(widgets::strong(drawing.name.as_str()));
                if small_enabled(ui, room, icons::ADD, NEW)
                    .on_disabled_hover_text(SheetRefusal::TooMany.to_string())
                    .clicked()
                {
                    new_drawing(model, tool, context.selected, actions);
                }
                if small(ui, icons::RENAME, RENAME).clicked() {
                    tool.renaming = Some(drawing.name.clone());
                    tool.rename_focus_pending = true;
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
            sections_rows(ui, model, drawing, &tool.sheet, actions);
            details_rows(ui, model, drawing, &tool.sheet, actions);
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
            preview(ui, model, tool, drawing, plan, actions);
            layout_rows(ui, model, drawing, actions);
            annotation_rows(ui, model, tool, drawing, plan, actions);
        },
    );

    if export {
        tool.export_pending = true;
    }
    if panel.closed {
        tool.toggle();
    }
}

fn number(id: AnnotationId) -> u64 {
    id.raw().saturating_add(1)
}

pub fn text_offset_field_id(drawing: DrawingId, id: AnnotationId) -> Id {
    Id::unique(("drawing-dimension-offset", drawing.raw(), id.raw()))
}

pub fn tolerance_field_id(drawing: DrawingId, id: AnnotationId, field: &'static str) -> Id {
    Id::unique((
        "drawing-dimension-tolerance",
        drawing.raw(),
        id.raw(),
        field,
    ))
}

pub fn note_field_id(drawing: DrawingId, id: AnnotationId) -> Id {
    Id::unique(("drawing-note-text", drawing.raw(), id.raw()))
}

fn decimals_label(decimals: u8) -> String {
    match decimals {
        0 => "Whole numbers".to_owned(),
        1 => "1 decimal".to_owned(),
        many => format!("{many} decimals"),
    }
}

fn change_dimension(
    model: &Model,
    drawing: DrawingId,
    id: AnnotationId,
    label: &str,
    apply: impl FnOnce(&mut DrawingDimension),
    actions: &mut Vec<Action>,
) {
    change(
        model,
        drawing,
        label,
        |drawing| {
            if let Some(dimension) = drawing.dimension_mut(id) {
                apply(dimension);
            }
        },
        actions,
    );
}

fn measure_choice(
    ui: &mut Ui,
    model: &Model,
    drawing: DrawingId,
    dimension: &DrawingDimension,
    actions: &mut Vec<Action>,
) {
    let id = dimension.id;

    match dimension.measure {
        DimensionMeasure::Linear {
            from,
            to,
            direction,
        } => {
            widgets::property(ui, MEASURES, |ui| {
                let choices: Vec<(&str, &str)> = DimensionDirection::ALL
                    .iter()
                    .map(|direction| (direction.name(), "Which way the distance is measured"))
                    .collect();
                let selected = DimensionDirection::ALL
                    .iter()
                    .position(|chosen| *chosen == direction)
                    .unwrap_or_default();

                if let Some(chosen) = widgets::segmented(ui, &choices, selected)
                    .and_then(|index| DimensionDirection::ALL.get(index).copied())
                {
                    change_dimension(
                        model,
                        drawing,
                        id,
                        "Change dimension direction",
                        |dimension| {
                            dimension.measure = DimensionMeasure::Linear {
                                from,
                                to,
                                direction: chosen,
                            };
                        },
                        actions,
                    );
                }
            });
        }
        DimensionMeasure::Radius(edge) | DimensionMeasure::Diameter(edge) => {
            widgets::property(ui, MEASURES, |ui| {
                let choices = [
                    ("Radius", "Measure the radius, written R"),
                    ("Diameter", "Measure the diameter, written \u{2300}"),
                ];
                let selected =
                    usize::from(matches!(dimension.measure, DimensionMeasure::Diameter(_)));

                if let Some(chosen) = widgets::segmented(ui, &choices, selected) {
                    change_dimension(
                        model,
                        drawing,
                        id,
                        "Change dimension kind",
                        |dimension| {
                            dimension.measure = if chosen == 1 {
                                DimensionMeasure::Diameter(edge)
                            } else {
                                DimensionMeasure::Radius(edge)
                            };
                        },
                        actions,
                    );
                }
            });
        }
        DimensionMeasure::Parallel { .. }
        | DimensionMeasure::Angle { .. }
        | DimensionMeasure::CylinderDiameter(_) => {}
    }
}

fn tolerance_choices(
    dimension: &DrawingDimension,
) -> Vec<(&'static str, Option<DimensionTolerance>)> {
    let fit = IsoFit::new(FitLetter::HoleH, 7).map(|fit| DimensionTolerance::Fit {
        fit,
        deviations: false,
    });
    let mut choices = vec![
        (NO_TOLERANCE, None),
        (
            "Plus or minus",
            Some(DimensionTolerance::Symmetric(DEFAULT_DEVIATION)),
        ),
        (
            "Deviations",
            Some(DimensionTolerance::Deviations {
                upper: DEFAULT_DEVIATION,
                lower: 0.0,
            }),
        ),
        (
            "Limits",
            Some(DimensionTolerance::Limits {
                upper: DEFAULT_DEVIATION,
                lower: -DEFAULT_DEVIATION,
            }),
        ),
    ];

    if !matches!(dimension.measure, DimensionMeasure::Angle { .. }) {
        choices.push(("ISO 286 fit", fit));
    }
    choices
}

fn same_kind(left: Option<DimensionTolerance>, right: Option<DimensionTolerance>) -> bool {
    let kind = |tolerance: Option<DimensionTolerance>| {
        tolerance.map(|tolerance| std::mem::discriminant(&tolerance))
    };
    kind(left) == kind(right)
}

fn change_tolerance(
    model: &Model,
    drawing: DrawingId,
    id: AnnotationId,
    tolerance: Option<DimensionTolerance>,
    actions: &mut Vec<Action>,
) {
    change_dimension(
        model,
        drawing,
        id,
        "Change dimension tolerance",
        |dimension| dimension.tolerance = tolerance,
        actions,
    );
}

fn deviation_row(
    ui: &mut Ui,
    model: &Model,
    (drawing, id): (DrawingId, AnnotationId),
    (field, caption): (&'static str, &str),
    (value, non_negative): (f64, bool),
) -> Option<f64> {
    length_row(
        ui,
        model,
        tolerance_field_id(drawing, id, field),
        caption,
        value,
        non_negative,
        "How far the size may be from the value, in model lengths; units may be typed",
    )
}

fn fit_rows(
    ui: &mut Ui,
    model: &Model,
    (drawing, id): (DrawingId, AnnotationId),
    (fit, deviations): (IsoFit, bool),
    actions: &mut Vec<Action>,
) {
    let refit = |fit: IsoFit, deviations: bool| Some(DimensionTolerance::Fit { fit, deviations });
    let mut chosen = None;

    widgets::property(ui, FIT_LETTER, |ui| {
        widgets::combo(
            ("drawing-dimension-fit", drawing.raw(), id.raw()),
            fit.letter().code(),
        )
        .hover("The ISO 286 fundamental deviation: a capital letter for a hole, a small one for a shaft")
        .show(ui, |ui| {
            for letter in FitLetter::ALL {
                if widgets::menu_option(ui, letter == fit.letter(), letter.describe()).clicked() {
                    chosen = IsoFit::new(letter, fit.grade());
                }
            }
        });
    });
    widgets::property(ui, FIT_GRADE, |ui| {
        widgets::combo(
            ("drawing-dimension-grade", drawing.raw(), id.raw()),
            format!("IT{}", fit.grade()),
        )
        .hover("The ISO 286 tolerance grade: the smaller the number, the closer the tolerance")
        .show(ui, |ui| {
            for grade in FIT_GRADES {
                if widgets::menu_option(ui, grade == fit.grade(), &format!("IT{grade}")).clicked() {
                    chosen = IsoFit::new(fit.letter(), grade);
                }
            }
        });
    });
    if let Some(fit) = chosen {
        change_tolerance(model, drawing, id, refit(fit, deviations), actions);
    }
    widgets::property(ui, "", |ui| {
        let mut shown = deviations;

        if widgets::checkbox(ui, &mut shown, SHOW_DEVIATIONS)
            .on_hover_text("Write the deviations from ISO 286's table after the fit")
            .changed()
        {
            change_tolerance(model, drawing, id, refit(fit, shown), actions);
        }
    });
}

fn tolerance_rows(
    ui: &mut Ui,
    model: &Model,
    drawing: DrawingId,
    dimension: &DrawingDimension,
    actions: &mut Vec<Action>,
) {
    let id = dimension.id;
    let choices = tolerance_choices(dimension);
    let current = choices
        .iter()
        .find(|(_, tolerance)| same_kind(*tolerance, dimension.tolerance))
        .map_or(NO_TOLERANCE, |(name, _)| name);
    let mut chosen = None;

    widgets::property(ui, TOLERANCE, |ui| {
        widgets::combo(
            ("drawing-dimension-tolerance", drawing.raw(), id.raw()),
            current,
        )
        .hover("How far the made size may stray from the value")
        .show(ui, |ui| {
            for (name, tolerance) in &choices {
                if widgets::menu_option(ui, *name == current, name).clicked()
                    && !same_kind(*tolerance, dimension.tolerance)
                {
                    chosen = Some(*tolerance);
                }
            }
        });
    });
    if let Some(tolerance) = chosen {
        change_tolerance(model, drawing, id, tolerance, actions);
    }

    let at = (drawing, id);

    match dimension.tolerance {
        None => {}
        Some(DimensionTolerance::Symmetric(deviation)) => {
            if let Some(changed) =
                deviation_row(ui, model, at, ("plus-minus", DEVIATION), (deviation, true))
            {
                change_tolerance(
                    model,
                    drawing,
                    id,
                    Some(DimensionTolerance::Symmetric(changed)),
                    actions,
                );
            }
        }
        Some(
            tolerance @ (DimensionTolerance::Deviations { upper, lower }
            | DimensionTolerance::Limits { upper, lower }),
        ) => {
            let limits = matches!(tolerance, DimensionTolerance::Limits { .. });
            let made = |upper: f64, lower: f64| {
                Some(if limits {
                    DimensionTolerance::Limits { upper, lower }
                } else {
                    DimensionTolerance::Deviations { upper, lower }
                })
            };

            if let Some(changed) =
                deviation_row(ui, model, at, ("upper", UPPER_DEVIATION), (upper, false))
            {
                change_tolerance(model, drawing, id, made(changed, lower), actions);
            }
            if let Some(changed) =
                deviation_row(ui, model, at, ("lower", LOWER_DEVIATION), (lower, false))
            {
                change_tolerance(model, drawing, id, made(upper, changed), actions);
            }
        }
        Some(DimensionTolerance::Fit { fit, deviations }) => {
            fit_rows(ui, model, at, (fit, deviations), actions);
        }
    }
}

fn fit_note(ui: &mut Ui, dimension: &DrawingDimension, measured: Option<f64>) {
    let (Some(DimensionTolerance::Fit { fit, .. }), Some(size)) = (dimension.tolerance, measured)
    else {
        return;
    };
    let code = fit.code();
    let size_text = format_decimals(size, usize::from(MAX_DIMENSION_DECIMALS));

    match fit.deviations(size) {
        Some((upper, lower)) => {
            ui.label(widgets::muted(
                format!(
                    "{code} at {size_text} mm runs from {} to {} mm.",
                    deviation_text(upper, ""),
                    deviation_text(lower, "")
                ),
                ui,
            ));
        }
        None => {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.label(format!(
                    "ISO 286's table runs to {MAX_FIT_SIZE} mm, so {code} has no deviations for \
                     {size_text} mm; the drawing writes the fit alone."
                ))
            });
        }
    }
}

fn dimension_settings(
    ui: &mut Ui,
    model: &Model,
    drawing: DrawingId,
    dimension: &DrawingDimension,
    actions: &mut Vec<Action>,
) {
    let id = dimension.id;

    widgets::properties(ui, ("drawing-dimension", drawing.raw(), id.raw()), |ui| {
        measure_choice(ui, model, drawing, dimension, actions);
        if !matches!(dimension.measure, DimensionMeasure::Angle { .. }) {
            tolerance_rows(ui, model, drawing, dimension, actions);
        }
        widgets::property(ui, DECIMALS, |ui| {
            let mut chosen = None;

            widgets::combo(
                ("drawing-dimension-decimals", drawing.raw(), id.raw()),
                decimals_label(dimension.decimals),
            )
            .hover("The most decimals the value shows; trailing zeros are left off")
            .show(ui, |ui| {
                for decimals in 0..=MAX_DIMENSION_DECIMALS {
                    if widgets::menu_option(
                        ui,
                        decimals == dimension.decimals,
                        &decimals_label(decimals),
                    )
                    .clicked()
                    {
                        chosen = Some(decimals);
                    }
                }
            });
            if let Some(decimals) = chosen {
                change_dimension(
                    model,
                    drawing,
                    id,
                    "Change dimension decimals",
                    |dimension| dimension.decimals = decimals,
                    actions,
                );
            }
        });

        let offset = length_row(
            ui,
            model,
            text_offset_field_id(drawing, id),
            TEXT_OFFSET,
            dimension.text_offset,
            false,
            "How far the value stands from the middle of its line, in sheet lengths; units may \
             be typed",
        );

        if let Some(offset) = offset {
            change_dimension(
                model,
                drawing,
                id,
                "Move dimension value",
                |dimension| {
                    dimension.text_offset = offset.clamp(-MAX_TEXT_OFFSET, MAX_TEXT_OFFSET);
                },
                actions,
            );
        }
    });
}

fn dimension_rows(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    for dimension in &drawing.dimensions {
        let annotation = Annotation::Dimension(dimension.id);
        let laid = plan.annotation(annotation);
        let number = number(dimension.id);
        let measured = dimension_problem(drawing, dimension, &tool.sheet.finished().collect());
        let problem = laid
            .and_then(|laid| laid.problem)
            .or_else(|| measured.and_then(Result::err));
        let size = measured
            .and_then(Result::ok)
            .map(|geometry| geometry.value());
        let value = laid
            .filter(|laid| laid.problem.is_none())
            .map_or_else(String::new, |laid| format!(" {}", laid.content()));

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(
                ui,
                &format!(
                    "Dimension {number}:{value} {} in the {}",
                    dimension.measure.name().to_lowercase(),
                    dimension.view.describe()
                ),
                2,
            );
            if widgets::icon_button(ui, icons::EDIT, &format!("Re-attach dimension {number}"))
                .on_hover_text("Pick the edges it measures again, keeping where it stands")
                .clicked()
            {
                tool.start_tool(SheetTool::reattaching(dimension.id));
            }
            if widgets::icon_button(ui, icons::DELETE, &format!("Delete dimension {number}"))
                .clicked()
            {
                sheet_canvas::delete_annotation(model, drawing, annotation, actions);
            }
        });
        if let Some(problem) = problem {
            let response = widgets::callout(ui, Tone::Error, |ui| {
                ui.label(format!(
                    "Dimension {number} lost what it measures: {problem}. Re-attach it to edges \
                     or outlines of a view, or delete it."
                ))
            });

            widgets::announced(ui, &response, false);
        }
        dimension_settings(ui, model, drawing.id, dimension, actions);
        fit_note(ui, dimension, size);
    }
}

fn centre_rows(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    for centre in &drawing.centres {
        let annotation = Annotation::Centre(centre.id);
        let number = number(centre.id);
        let (kind, what) = match centre.target {
            CentreTarget::Circle(_) => ("Centre mark", "a circle"),
            CentreTarget::Cylinder(_) => ("Centreline", "a cylinder"),
        };

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(
                ui,
                &format!(
                    "{kind} {number} of {what} in the {}",
                    centre.view.describe()
                ),
                1,
            );
            if widgets::icon_button(
                ui,
                icons::DELETE,
                &format!("Delete {} {number}", kind.to_lowercase()),
            )
            .clicked()
            {
                sheet_canvas::delete_annotation(model, drawing, annotation, actions);
            }
        });
        if let Some(problem) = plan.annotation(annotation).and_then(|laid| laid.problem) {
            let response = widgets::callout(ui, Tone::Error, |ui| {
                ui.label(format!(
                    "{kind} {number} lost what it marks: {problem}. Delete it, then add centre \
                     marks to the view again."
                ))
            });

            widgets::announced(ui, &response, false);
        }
    }
}

fn callout_rows(
    ui: &mut Ui,
    model: &Model,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    let document = model.document();

    for callout in &drawing.callouts {
        let annotation = Annotation::Callout(callout.id);
        let number = number(callout.id);
        let source = document.feature(callout.feature).map_or_else(
            || "a deleted feature".to_owned(),
            |found| found.name.clone(),
        );
        let read = callout_lines(
            document,
            &model.evaluation().parameters,
            callout.feature,
            None,
        )
        .map(|lines| {
            lines
                .iter()
                .map(CalloutLine::spoken)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(
                ui,
                &format!(
                    "Callout {number} of {source} in the {}",
                    callout.view.describe()
                ),
                1,
            )
            .on_hover_text(read);
            if widgets::icon_button(ui, icons::DELETE, &format!("Delete callout {number}"))
                .clicked()
            {
                sheet_canvas::delete_annotation(model, drawing, annotation, actions);
            }
        });
        if let Some(problem) = plan.annotation(annotation).and_then(|laid| laid.problem) {
            let response = widgets::callout(ui, Tone::Error, |ui| {
                ui.label(format!(
                    "Callout {number} lost what it reads: {problem}. Delete it and call the hole \
                     out again."
                ))
            });

            widgets::announced(ui, &response, false);
        }
    }
}

fn note_editing_row(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    drawing: DrawingId,
    actions: &mut Vec<Action>,
) {
    let Some((id, text)) = tool.note_editing.as_mut() else {
        return;
    };
    let id = *id;
    let field_id = note_field_id(drawing, id);
    let pointer_busy = ui.input(|input| input.pointer.any_down() || input.pointer.any_released());
    let mut done = false;
    let mut cancelled = false;

    if tool.note_focus_pending && !pointer_busy {
        tool.note_focus_pending = false;
        field::select_all(ui.ctx(), field_id, text);
        ui.memory_mut(|memory| memory.request_focus(field_id));
    }
    ui.horizontal(|ui| {
        let (field, enter) = sheet_tools::note_text_edit(
            ui,
            field_id,
            text,
            &format!("Text of note {}", number(id)),
        );

        cancelled = widgets::escape_cancels(ui, &field);
        done = widgets::icon_button(ui, icons::DONE, "Keep the note's new text").clicked() || enter;
        cancelled |=
            widgets::icon_button(ui, icons::CLOSE, "Keep the note as it was (Esc)").clicked();
    });
    if cancelled {
        tool.note_editing = None;
        return;
    }
    if done && let Some((_, text)) = tool.note_editing.clone() {
        let changed = model
            .document()
            .changing_drawing(drawing, "Change note text", |drawing| {
                if let Some(note) = drawing.note_mut(id) {
                    note.text = text;
                }
            });

        match changed {
            Ok(transaction) => {
                tool.note_editing = None;
                actions.push(Action::Apply(transaction));
            }
            Err(error) => {
                tool.note_focus_pending = true;
                actions.push(Action::Inform(Notice::failure(format!(
                    "The note was not changed: {error}."
                ))));
            }
        }
    }
}

fn note_rows(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    for note in &drawing.notes {
        let annotation = Annotation::Note(note.id);
        let number = number(note.id);
        let buttons = if note.leader.is_some() { 3 } else { 2 };

        ui.add_space(SPACE_S);
        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(
                ui,
                &format!(
                    "Note {number}: \u{201c}{}\u{201d}",
                    note.text.replace('\n', " / ")
                ),
                buttons,
            );
            if widgets::icon_button(
                ui,
                icons::RENAME,
                &format!("Change the text of note {number}"),
            )
            .clicked()
            {
                tool.note_editing = Some((note.id, note.text.clone()));
                tool.note_focus_pending = true;
            }
            if note.leader.is_some()
                && widgets::icon_button(
                    ui,
                    icons::REMOVE,
                    &format!("Remove the leader of note {number}"),
                )
                .clicked()
            {
                change(
                    model,
                    drawing.id,
                    "Remove note leader",
                    |drawing| {
                        if let Some(note) = drawing.note_mut(note.id) {
                            note.leader = None;
                        }
                    },
                    actions,
                );
            }
            if widgets::icon_button(ui, icons::DELETE, &format!("Delete note {number}")).clicked() {
                sheet_canvas::delete_annotation(model, drawing, annotation, actions);
            }
        });
        if tool
            .note_editing
            .as_ref()
            .is_some_and(|(editing, _)| *editing == note.id)
        {
            note_editing_row(ui, model, tool, drawing.id, actions);
        }
        if let Some(problem) = plan.annotation(annotation).and_then(|laid| laid.problem) {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.label(format!(
                    "The leader of note {number} points at nothing: {problem}. Remove the leader, \
                     or delete the note and place it again."
                ))
            });
        }
    }
}

fn annotation_rows(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title(ANNOTATIONS));
    if drawing.annotation_count() == 0 {
        ui.label(widgets::muted(NO_ANNOTATIONS, ui));
    }
    dimension_rows(ui, model, tool, drawing, plan, actions);
    note_rows(ui, model, tool, drawing, plan, actions);
    centre_rows(ui, model, drawing, plan, actions);
    callout_rows(ui, model, drawing, plan, actions);
}

fn position_label(view: DrawnView) -> String {
    match view {
        DrawnView::Standard(standard) => format!("{} view", standard.name()),
        DrawnView::Section(letter) => format!("Section {letter}\u{2013}{letter}"),
        DrawnView::Detail(letter) => format!("Detail {letter}"),
    }
}

fn layout_rows(ui: &mut Ui, model: &Model, drawing: &Drawing, actions: &mut Vec<Action>) {
    ui.add_space(SPACE_S);
    ui.label(widgets::section_title(LAYOUT));
    if drawing.positions.is_empty() {
        ui.label(widgets::muted(LAID_OUT, ui));
        return;
    }
    for view in drawing.positions.keys() {
        let label = position_label(*view);

        ui.horizontal(|ui| {
            widgets::label_before_icon_buttons(ui, &format!("{label}, placed by hand"), 1);

            let reset = format!("Reset the position of {}", view.describe());
            if widgets::icon_button(ui, icons::RESET, &reset).clicked() {
                sheet_canvas::reset_position(model, drawing, *view, actions);
            }
        });
    }
}

fn preview(
    ui: &mut Ui,
    model: &Model,
    tool: &mut DrawingsTool,
    drawing: &Drawing,
    plan: &SheetPlan,
    actions: &mut Vec<Action>,
) {
    if tool.in_view {
        ui.label(SHEET_IN_VIEW_NOTE);
        ui.horizontal_wrapped(|ui| {
            if small(ui, icons::SHEET_IN_PANEL, SHEET_TO_PANEL).clicked() {
                tool.show_sheet_in_view(false);
            }
        });
        return;
    }

    let context = SheetContext {
        model,
        drawing,
        plan,
        views: &tool.sheet,
    };
    let mut in_view = false;

    ui.horizontal_wrapped(|ui| {
        sheet_canvas::toolbar(ui, &mut tool.canvas, &context, actions);
        in_view = small(ui, icons::SHEET_IN_VIEW, SHEET_TO_VIEW)
            .on_hover_text("Show the sheet across the view, the panel beside it")
            .clicked();
    });
    sheet_canvas::tool_row(ui, &mut tool.canvas, &context, actions);
    ui.add_space(SPACE_S);

    let width = ui.available_width().max(1.0);
    let height = width * (plan.height / plan.width.max(1.0)) as f32;

    sheet_canvas::canvas(ui, &mut tool.canvas, &context, vec2(width, height), actions);
    if in_view {
        tool.show_sheet_in_view(true);
    }
}

pub fn sheet_view_id() -> Id {
    Id::unique("drawing sheet in the view")
}

pub fn show_in_view(
    ctx: &egui::Context,
    area: Rect,
    model: &Model,
    tool: &mut DrawingsTool,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let (Some(drawing), Some(plan)) = (tool.current(document).cloned(), tool.plan(model)) else {
        tool.in_view = false;
        return;
    };
    let tokens = appearance::tokens_of(ctx);

    Area::new(sheet_view_id())
        .order(Order::Middle)
        .fixed_pos(area.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_clip_rect(area);
            ui.allocate_rect(area, Sense::hover());
            ui.painter().rect_filled(area, 0.0, tokens.panel);

            let mut page = ui.new_child(
                UiBuilder::new()
                    .max_rect(area.shrink(SPACE_S))
                    .layout(Layout::top_down(Align::Min)),
            );

            let context = SheetContext {
                model,
                drawing: &drawing,
                plan: &plan,
                views: &tool.sheet,
            };
            let mut leave = false;

            page.horizontal_wrapped(|ui| {
                leave = small(ui, icons::BACK, LEAVE_SHEET_VIEW)
                    .on_hover_text("Show the 3D view again (Esc)")
                    .clicked();
                ui.label(widgets::strong(drawing.name.as_str()));
                sheet_canvas::toolbar(ui, &mut tool.canvas, &context, actions);
            });
            sheet_canvas::tool_row(&mut page, &mut tool.canvas, &context, actions);
            page.add_space(SPACE_S);

            let size = page.available_size();

            sheet_canvas::canvas(&mut page, &mut tool.canvas, &context, size, actions);
            if leave {
                tool.show_sheet_in_view(false);
            }
        });
}
