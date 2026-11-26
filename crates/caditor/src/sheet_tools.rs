use caditor_document::{
    Annotation, AnnotationId, CentreMark, DEFAULT_DIMENSION_DECIMALS, DimensionMeasure,
    DimensionRefusal, Drawing, DrawingCallout, DrawingDimension, DrawingNote, DrawnLines,
    DrawnView, EdgePick, FeatureId, MAX_NOTE_CHARS, NoteLeader, SheetPlan, callout_feature,
    dimension_from_picks, is_measured_view, measured_lines_of, new_centres, note_place,
    pick_centre, pick_edge, view_place,
};
use caditor_geometry::Point2;
use egui::{Id, Key, KeyboardShortcut, Modifiers, Response, TextEdit, Ui};

use crate::{
    icons,
    model::{Action, Model, Notice},
    sheet_views::SheetViews,
    widgets,
};

pub const DIMENSION_TOOL: &str = "Add a dimension";
pub const NOTE_TOOL: &str = "Add a note";
pub const CENTRE_TOOL: &str = "Add a centre mark or centreline";
pub const CENTRE_MARKS: &str = "Add centre marks to this view";
pub const CALLOUT_TOOL: &str = "Add a callout to a hole";
pub const ADD_CENTRE_MARK: &str = "Add centre mark";
pub const ADD_CENTRE_MARKS: &str = "Add centre marks";
pub const ADD_CALLOUT: &str = "Add callout";
pub const MOVE_CALLOUT: &str = "Move callout";
pub const DELETE_CENTRE_MARK: &str = "Delete centre mark";
pub const DELETE_CALLOUT: &str = "Delete callout";
pub const CENTRE_PICK: &str = "Click a circle seen round to mark its centre, or the outline of a \
                               cylinder seen from the side for its centreline.";
pub const CALLOUT_EDGE: &str = "Click an edge of a hole or of a threaded face to call it out.";
pub const CALLOUT_PLACE: &str = "Click where the callout's text stands.";
const NOT_ROUND: &str = "That is neither a circle seen round nor the outline of a cylinder seen \
                         from the side. Click one of those.";
const ALREADY_MARKED: &str = "That already has its centre marked.";
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
pub const NOTE_TEXT: &str = "Type the note (Shift+Enter starts a new line), click an edge to point \
                             a leader at it if you like, then Place the note or press Enter.";
const NOTE_ROWS: usize = 2;
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
    Centre,
    Callout {
        pick: Option<(EdgePick, FeatureId)>,
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

    pub fn callout() -> Self {
        Self::Callout { pick: None }
    }

    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Self::Dimension { first: None, .. } => FIRST_EDGE,
            Self::Dimension { second: None, .. } => SECOND_EDGE,
            Self::Dimension { .. } => PLACE_DIMENSION,
            Self::Note { at: None, .. } => NOTE_PLACE,
            Self::Note { .. } => NOTE_TEXT,
            Self::Centre => CENTRE_PICK,
            Self::Callout { pick: None } => CALLOUT_EDGE,
            Self::Callout { .. } => CALLOUT_PLACE,
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
            Self::Callout { pick } if pick.is_some() => *pick = None,
            Self::Dimension { .. } | Self::Note { .. } | Self::Centre | Self::Callout { .. } => {
                return false;
            }
        }
        true
    }

    pub fn picks(&self) -> Vec<EdgePick> {
        match self {
            Self::Dimension { first, second, .. } => first.iter().chain(second).copied().collect(),
            Self::Note { leader, .. } => leader.iter().copied().collect(),
            Self::Callout { pick } => pick.iter().map(|(pick, _)| *pick).collect(),
            Self::Centre => Vec::new(),
        }
    }

    pub fn places(&self) -> bool {
        matches!(
            self,
            Self::Dimension {
                first: Some(_),
                second: Some(_),
                ..
            } | Self::Callout { pick: Some(_) }
        )
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
    let kept = replacing.and_then(|id| context.drawing.dimension(id));
    Some(DrawingDimension {
        id,
        view: first.view,
        measure,
        place,
        decimals: kept.map_or(DEFAULT_DIMENSION_DECIMALS, |kept| kept.decimals),
        text_offset: 0.0,
        tolerance: kept
            .and_then(|kept| kept.tolerance)
            .filter(|tolerance| tolerance.is_usable_on(&measure)),
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
                kept.tolerance = draft.tolerance;
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
        SheetTool::Centre => return place_centre(context, point, reach, actions),
        SheetTool::Callout { pick: Some(_) } => {
            return place_callout(tool, context, point, actions);
        }
        SheetTool::Callout { pick } => match context.pick(point, reach) {
            None => say(actions, NO_EDGE.to_owned()),
            Some(found) => {
                let model = context.model;

                match callout_feature(
                    model.document(),
                    model.evaluation(),
                    context.drawing,
                    found.edge,
                ) {
                    Ok(feature) => *pick = Some((found, feature)),
                    Err(refusal) => say(actions, format!("That cannot be called out: {refusal}.")),
                }
            }
        },
    }
    Clicked::Kept
}

fn place_centre(
    context: &ToolContext<'_>,
    point: Point2,
    reach: f64,
    actions: &mut Vec<Action>,
) -> Clicked {
    let say = |actions: &mut Vec<Action>, text: &str| {
        actions.push(Action::Inform(Notice::info(text)));
    };
    let Some((view, target)) = pick_centre(
        context.drawing,
        context.plan,
        &context.lines(),
        point,
        reach,
    ) else {
        say(actions, NOT_ROUND);
        return Clicked::Kept;
    };

    if context.drawing.has_centre(view, target) {
        say(actions, ALREADY_MARKED);
        return Clicked::Kept;
    }

    let id = AnnotationId::from_raw(context.drawing.first_free_annotation());
    let built =
        context
            .model
            .document()
            .changing_drawing(context.drawing.id, ADD_CENTRE_MARK, |drawing| {
                let id = drawing.new_annotation_id();

                drawing.centres.push(CentreMark { id, view, target });
            });

    if apply_or_say(built, actions) {
        Clicked::Placed(Annotation::Centre(id))
    } else {
        Clicked::Kept
    }
}

pub fn draft_callout(
    tool: &SheetTool,
    context: &ToolContext<'_>,
    pointer: Point2,
) -> Option<DrawingCallout> {
    let SheetTool::Callout {
        pick: Some((pick, feature)),
    } = tool
    else {
        return None;
    };
    Some(DrawingCallout {
        id: AnnotationId::from_raw(context.drawing.first_free_annotation()),
        view: pick.view,
        feature: *feature,
        edge: pick.edge,
        at: note_place(context.plan, pointer),
    })
}

fn place_callout(
    tool: &mut SheetTool,
    context: &ToolContext<'_>,
    point: Point2,
    actions: &mut Vec<Action>,
) -> Clicked {
    let Some(draft) = draft_callout(tool, context, point) else {
        return Clicked::Kept;
    };
    let built =
        context
            .model
            .document()
            .changing_drawing(context.drawing.id, ADD_CALLOUT, |drawing| {
                let id = drawing.new_annotation_id();

                drawing.callouts.push(DrawingCallout { id, ..draft });
            });

    if !apply_or_say(built, actions) {
        return Clicked::Kept;
    }
    *tool = SheetTool::callout();
    Clicked::Placed(Annotation::Callout(draft.id))
}

pub fn add_centre_marks(
    model: &Model,
    drawing: &Drawing,
    views: &SheetViews,
    view: DrawnView,
    actions: &mut Vec<Action>,
) {
    let lines: DrawnLines<'_> = views.finished().collect();
    let Some(measured) = lines.get(&measured_lines_of(drawing, view)) else {
        actions.push(Action::Inform(Notice::info(format!(
            "The {} is still being worked out; add its centre marks once it is drawn.",
            view.describe()
        ))));
        return;
    };
    let targets = new_centres(drawing, view, measured);

    if targets.is_empty() {
        actions.push(Action::Inform(Notice::info(format!(
            "There is nothing more to mark in the {}: every circle seen round and every \
             cylinder seen from the side already has its centre marked, or it has none.",
            view.describe()
        ))));
        return;
    }

    let built = model
        .document()
        .changing_drawing(drawing.id, ADD_CENTRE_MARKS, |changed| {
            for target in targets {
                let id = changed.new_annotation_id();

                changed.centres.push(CentreMark { id, view, target });
            }
        });

    apply_or_say(built, actions);
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

pub fn note_text_edit(ui: &mut Ui, id: Id, text: &mut String, name: &str) -> (Response, bool) {
    let newline = KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter);

    let field = widgets::text_field(ui, |ui| {
        ui.add(
            TextEdit::multiline(text)
                .id(id)
                .return_key(newline)
                .desired_rows(NOTE_ROWS)
                .char_limit(MAX_NOTE_CHARS)
                .desired_width(ui.available_width() - 2.0 * ui.spacing().interact_size.y),
        )
    });
    let field = widgets::named(field, name);

    let entered = (field.has_focus() || field.lost_focus())
        && ui.input(|input| input.key_pressed(Key::Enter) && !input.modifiers.shift);

    (field, entered)
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
            let (field, entered) = note_text_edit(ui, field_id, text, NOTE_FIELD);

            placed = entered;
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
