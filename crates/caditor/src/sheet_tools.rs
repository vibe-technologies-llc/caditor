use caditor_document::{
    Annotation, AnnotationId, DEFAULT_DIMENSION_DECIMALS, DimensionMeasure, DimensionRefusal,
    Drawing, DrawingDimension, DrawingNote, DrawnLines, EdgePick, MAX_NOTE_CHARS, NoteLeader,
    SheetPlan, dimension_from_picks, is_measured_view, note_place, pick_edge, view_place,
};
use caditor_geometry::Point2;
use egui::{Id, Key, TextEdit, Ui};

use crate::{
    icons,
    model::{Action, Model, Notice},
    sheet_views::SheetViews,
    widgets,
};

pub const DIMENSION_TOOL: &str = "Add a dimension";
pub const NOTE_TOOL: &str = "Add a note";
pub const NOTE_FIELD: &str = "Note text";
pub const PLACE_NOTE: &str = "Place the note";
pub const STOP_ADDING: &str = "Stop adding";
pub const ADD_DIMENSION: &str = "Add dimension";
pub const REATTACH_DIMENSION: &str = "Re-attach dimension";
pub const ADD_NOTE: &str = "Add note";
pub const MOVE_DIMENSION: &str = "Move dimension";
pub const MOVE_NOTE: &str = "Move note";
pub const DELETE_DIMENSION: &str = "Delete dimension";
pub const DELETE_NOTE: &str = "Delete note";
pub const FIRST_EDGE: &str = "Click an edge of a view to dimension it: a straight edge, an arc or \
                              a circle.";
pub const SECOND_EDGE: &str = "Click a second edge of the same view, or the same edge again for \
                               its length, radius or diameter.";
pub const PLACE_DIMENSION: &str = "Click where the dimension line stands.";
pub const NOTE_PLACE: &str = "Click where the note stands on the sheet.";
pub const NOTE_TEXT: &str = "Type the note, click an edge to point a leader at it if you like, \
                             then Place the note.";
const NO_EDGE: &str = "There is no edge of a view there. Click on an edge's line.";

#[derive(Clone, Debug, PartialEq)]
pub enum SheetTool {
    Dimension {
        first: Option<EdgePick>,
        second: Option<EdgePick>,
        replacing: Option<AnnotationId>,
    },
    Note {
        at: Option<Point2>,
        text: String,
        leader: Option<EdgePick>,
        focus_pending: bool,
    },
}

impl SheetTool {
    pub fn dimension() -> Self {
        Self::Dimension {
            first: None,
            second: None,
            replacing: None,
        }
    }

    pub fn reattaching(id: AnnotationId) -> Self {
        Self::Dimension {
            first: None,
            second: None,
            replacing: Some(id),
        }
    }

    pub fn note() -> Self {
        Self::Note {
            at: None,
            text: String::new(),
            leader: None,
            focus_pending: false,
        }
    }

    pub fn is_dimension(&self) -> bool {
        matches!(self, Self::Dimension { .. })
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Self::Dimension { first: None, .. } => FIRST_EDGE,
            Self::Dimension { second: None, .. } => SECOND_EDGE,
            Self::Dimension { .. } => PLACE_DIMENSION,
            Self::Note { at: None, .. } => NOTE_PLACE,
            Self::Note { .. } => NOTE_TEXT,
        }
    }

    pub fn step_back(&mut self) -> bool {
        match self {
            Self::Dimension { second, .. } if second.is_some() => *second = None,
            Self::Dimension { first, .. } if first.is_some() => *first = None,
            Self::Note { leader, .. } if leader.is_some() => *leader = None,
            Self::Note { at, text, .. } if at.is_some() => {
                *at = None;
                text.clear();
            }
            Self::Dimension { .. } | Self::Note { .. } => return false,
        }
        true
    }

    pub fn picks(&self) -> Vec<EdgePick> {
        match self {
            Self::Dimension { first, second, .. } => first.iter().chain(second).copied().collect(),
            Self::Note { leader, .. } => leader.iter().copied().collect(),
        }
    }
}

pub struct ToolContext<'a> {
    pub model: &'a Model,
    pub drawing: &'a Drawing,
    pub plan: &'a SheetPlan,
    pub views: &'a SheetViews,
}

impl ToolContext<'_> {
    fn lines(&self) -> DrawnLines<'_> {
        self.views.finished().collect()
    }

    fn pick(&self, point: Point2, reach: f64) -> Option<EdgePick> {
        pick_edge(
            self.plan,
            &self.lines(),
            self.drawing.hidden_lines,
            point,
            reach,
        )
    }

    fn measure(
        &self,
        first: EdgePick,
        second: EdgePick,
        point: Point2,
    ) -> Result<(DimensionMeasure, Point2), DimensionRefusal> {
        let place = self
            .plan
            .placed(first.view)
            .map_or(point, |placed| view_place(placed, point));

        dimension_from_picks(self.drawing, &self.lines(), first, second, place)
            .map(|measure| (measure, place))
    }
}

fn apply_or_say(
    built: Result<caditor_document::Transaction, caditor_document::EditError>,
    actions: &mut Vec<Action>,
) -> bool {
    match built {
        Ok(transaction) => {
            actions.push(Action::Apply(transaction));
            true
        }
        Err(error) => {
            actions.push(Action::Inform(Notice::failure(format!(
                "The drawing was not changed: {error}."
            ))));
            false
        }
    }
}

pub fn draft_dimension(
    tool: &SheetTool,
    context: &ToolContext<'_>,
    pointer: Point2,
) -> Option<DrawingDimension> {
    let SheetTool::Dimension {
        first: Some(first),
        second: Some(second),
        replacing,
    } = tool
    else {
        return None;
    };

    let (measure, place) = context.measure(*first, *second, pointer).ok()?;
    let id = replacing
        .unwrap_or_else(|| AnnotationId::from_raw(context.drawing.first_free_annotation()));
    let decimals = replacing
        .and_then(|id| context.drawing.dimension(id))
        .map_or(DEFAULT_DIMENSION_DECIMALS, |kept| kept.decimals);
    Some(DrawingDimension {
        id,
        view: first.view,
        measure,
        place,
        decimals,
        text_offset: 0.0,
    })
}

pub fn with_draft(drawing: &mut Drawing, dimension: DrawingDimension) {
    match drawing.dimension_mut(dimension.id) {
        Some(kept) => *kept = dimension,
        None => drawing.dimensions.push(dimension),
    }
}

pub enum Clicked {
    Kept,
    Placed(Annotation),
    Done,
}

fn place_dimension(
    tool: &mut SheetTool,
    context: &ToolContext<'_>,
    point: Point2,
    actions: &mut Vec<Action>,
) -> Clicked {
    let Some(draft) = draft_dimension(tool, context, point) else {
        return Clicked::Kept;
    };
    let replacing = matches!(
        tool,
        SheetTool::Dimension {
            replacing: Some(_),
            ..
        }
    );
    let label = if replacing {
        REATTACH_DIMENSION
    } else {
        ADD_DIMENSION
    };

    let built = context
        .model
        .document()
        .changing_drawing(context.drawing.id, label, |drawing| {
            if let Some(kept) = drawing.dimension_mut(draft.id) {
                kept.view = draft.view;
                kept.measure = draft.measure;
                kept.place = draft.place;
            } else {
                let id = drawing.new_annotation_id();

                drawing.dimensions.push(DrawingDimension { id, ..draft });
            }
        });

    if !apply_or_say(built, actions) {
        return Clicked::Kept;
    }
    if replacing {
        return Clicked::Done;
    }
    *tool = SheetTool::dimension();
    Clicked::Placed(Annotation::Dimension(draft.id))
}

pub fn clicked(
    tool: &mut SheetTool,
    context: &ToolContext<'_>,
    point: Point2,
    reach: f64,
    actions: &mut Vec<Action>,
) -> Clicked {
    let say = |actions: &mut Vec<Action>, text: String| {
        actions.push(Action::Inform(Notice::info(text)));
    };

    match tool {
        SheetTool::Dimension {
            first: Some(_),
            second: Some(_),
            ..
        } => return place_dimension(tool, context, point, actions),
        SheetTool::Dimension {
            first: first @ None,
            ..
        } => match context.pick(point, reach) {
            None => say(actions, NO_EDGE.to_owned()),
            Some(pick) if !is_measured_view(context.drawing, pick.view) => say(
                actions,
                "The isometric view is not drawn true to size, so nothing is measured in it. \
                 Dimension another view."
                    .to_owned(),
            ),
            Some(pick) => *first = Some(pick),
        },
        SheetTool::Dimension {
            first: Some(first),
            second,
            ..
        } => match context.pick(point, reach) {
            None => say(actions, NO_EDGE.to_owned()),
            Some(pick) => match context.measure(*first, pick, point) {
                Ok(_) => *second = Some(pick),
                Err(refusal) => say(actions, format!("That cannot be dimensioned: {refusal}.")),
            },
        },
        SheetTool::Note {
            at: at @ None,
            focus_pending,
            ..
        } => {
            *at = Some(point);
            *focus_pending = true;
        }
        SheetTool::Note { leader, at, .. } => match context.pick(point, reach) {
            Some(pick) => *leader = Some(pick),
            None => *at = Some(point),
        },
    }
    Clicked::Kept
}

fn place_note(
    tool: &SheetTool,
    context: &ToolContext<'_>,
    actions: &mut Vec<Action>,
) -> Option<Annotation> {
    let SheetTool::Note {
        at: Some(at),
        text,
        leader,
        ..
    } = tool
    else {
        return None;
    };
    let id = AnnotationId::from_raw(context.drawing.first_free_annotation());

    let note = DrawingNote {
        id,
        text: text.clone(),
        at: note_place(context.plan, *at),
        leader: leader.map(|pick| NoteLeader {
            view: pick.view,
            edge: pick.edge,
        }),
    };
    let built =
        context
            .model
            .document()
            .changing_drawing(context.drawing.id, ADD_NOTE, |drawing| {
                let id = drawing.new_annotation_id();

                drawing.notes.push(DrawingNote { id, ..note });
            });
    apply_or_say(built, actions).then_some(Annotation::Note(id))
}

pub fn note_field_id() -> Id {
    Id::unique("drawing note text")
}

pub enum ToolRow {
    Kept,
    Stopped,
    Placed(Annotation),
}

pub fn tool_row(
    ui: &mut Ui,
    tool: &mut SheetTool,
    context: &ToolContext<'_>,
    actions: &mut Vec<Action>,
) -> ToolRow {
    let mut outcome = ToolRow::Kept;

    ui.label(widgets::muted(tool.hint(), ui));

    if let SheetTool::Note {
        at: Some(_),
        text,
        focus_pending,
        ..
    } = tool
    {
        let field_id = note_field_id();
        let mut placed = false;

        if std::mem::take(focus_pending) {
            ui.memory_mut(|memory| memory.request_focus(field_id));
        }
        ui.horizontal(|ui| {
            let field = widgets::text_field(ui, |ui| {
                ui.add(
                    TextEdit::singleline(text)
                        .id(field_id)
                        .char_limit(MAX_NOTE_CHARS)
                        .desired_width(ui.available_width() - 2.0 * ui.spacing().interact_size.y),
                )
            });
            let field = widgets::named(field, NOTE_FIELD);

            placed = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            if widgets::escape_cancels(ui, &field) {
                outcome = ToolRow::Stopped;
            }
            placed |= widgets::icon_button(ui, icons::DONE, PLACE_NOTE).clicked();
        });
        if placed && let Some(annotation) = place_note(tool, context, actions) {
            return ToolRow::Placed(annotation);
        }
    }
    ui.horizontal_wrapped(|ui| {
        let made = widgets::small_button(ui, icons::CLOSE, STOP_ADDING);
        let stop = ui.add(made);
        if stop.on_hover_text("Stop adding (Esc)").clicked() {
            outcome = ToolRow::Stopped;
        }
    });
    outcome
}
