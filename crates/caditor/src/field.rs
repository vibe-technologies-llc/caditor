use std::{collections::BTreeSet, fmt::Display};

use caditor_document::{
    Document, Edit, EditError, FeatureId, Parameter, ParameterOwner, ParameterValues, Transaction,
};
use caditor_expression::{Dimension, EvalError, Expression, Naming, ParameterId, ParseError};
use caditor_sketch::{Constraint, ConstraintId, DimensionError};
use egui::{
    Align, Event, Id, Key, Margin, Rect, Response, Stroke, StrokeKind, TextEdit, Ui,
    text::{CCursor, CCursorRange},
    vec2,
};

use crate::{
    appearance::WIDGET_RADIUS,
    completion::{self, Candidate, PopupKey},
    stepping::{self, Direction, Refusal},
    units::{Units, attach_unit},
    widgets,
};

const ERROR_OUTLINE_WIDTH: f32 = 1.5;
const HORIZONTAL_MARGIN: i8 = 6;
const FIELD_MARGIN: Margin = Margin::symmetric(HORIZONTAL_MARGIN, 3);
const CARET_ROOM: f32 = 2.0;

pub fn text_margins() -> f32 {
    2.0 * f32::from(HORIZONTAL_MARGIN)
}

pub fn text_inset() -> f32 {
    text_margins() + CARET_ROOM
}

const SUGGESTING: &str = "suggesting";
const ASKED_FOCUS_KEY: &str = "asked-field-focus";
const ASKED_FOCUS_PASSES: u64 = 30;

#[derive(Debug, Clone, Copy)]
struct AskedFocus {
    field: Id,
    pass: u64,
}

pub fn ask_focus(ctx: &egui::Context, field: Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(Id::unique(ASKED_FOCUS_KEY), AskedFocus { field, pass }));
}

fn asked_focus(ctx: &egui::Context, field: Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<AskedFocus>(Id::unique(ASKED_FOCUS_KEY)))
        .is_some_and(|asked| asked.field == field && asked.pass + ASKED_FOCUS_PASSES > pass)
}

fn focus_arrived(ctx: &egui::Context, field: Id) {
    if asked_focus(ctx, field) {
        ctx.data_mut(|data| data.remove::<AskedFocus>(Id::unique(ASKED_FOCUS_KEY)));
    }
}

#[derive(Debug, Clone, Default)]
struct Suggesting {
    names: Vec<String>,
    rows: Vec<Rect>,
    selected: usize,
    navigated: bool,
    dismissed: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Draft {
    text: String,
    error: Option<String>,
    stored: String,
}

pub struct FieldResponse<T> {
    pub committed: Option<T>,
    pub error: Option<String>,
    pub edited: Option<String>,
    pub left: bool,
    pub response: Response,
}

pub fn busy<K: Ord>(ui: &Ui, fields: impl IntoIterator<Item = (K, Id)>) -> BTreeSet<K> {
    let focused = ui.memory(|memory| memory.focused());
    ui.data(|data| {
        fields
            .into_iter()
            .filter(|(_, id)| focused == Some(*id) || data.get_temp::<Draft>(*id).is_some())
            .map(|(key, _)| key)
            .collect()
    })
}

pub fn focused_value_field(ui: &Ui) -> Option<Id> {
    let focused = ui.memory(|memory| memory.focused())?;
    ui.data(|data| data.get_temp::<Suggesting>(focused.with(SUGGESTING)))
        .map(|_| focused)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    direction: Direction,
    big: bool,
}

fn take_step(ui: &Ui) -> Option<Step> {
    ui.input_mut(|input| {
        let mut taken = None;
        input.events.retain(|event| {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let direction = match key {
                Key::ArrowUp => Direction::Up,
                Key::ArrowDown => Direction::Down,
                _ => return true,
            };
            if modifiers.command || modifiers.alt {
                return true;
            }
            taken = Some(Step {
                direction,
                big: modifiers.shift,
            });
            false
        });
        taken
    })
}

pub fn commit_field<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    stored: &str,
    width: f32,
    focus: bool,
    validate: impl FnOnce(&str) -> Result<T, E>,
) -> FieldResponse<T> {
    field(ui, id, stored, width, focus, false, validate)
}

pub fn value_field<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    stored: &str,
    width: f32,
    focus: bool,
    validate: impl FnOnce(&str) -> Result<T, E>,
) -> FieldResponse<T> {
    field(ui, id, stored, width, focus, true, validate)
}

fn field<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    stored: &str,
    width: f32,
    focus: bool,
    value: bool,
    validate: impl FnOnce(&str) -> Result<T, E>,
) -> FieldResponse<T> {
    let focus = focus || asked_focus(ui.ctx(), id);
    let editing = ui.memory(|memory| memory.has_focus(id));
    let editing_parameter = completion::take_editing(ui, id);
    let suggesting_key = id.with(SUGGESTING);
    let mut draft = ui
        .data(|data| data.get_temp::<Draft>(id))
        .filter(|draft| editing || draft.stored == stored);
    let mut text = draft
        .as_ref()
        .map_or_else(|| stored.to_owned(), |draft| draft.text.clone());
    let mut suggesting = if value && editing {
        ui.data(|data| data.get_temp::<Suggesting>(suggesting_key))
            .unwrap_or_default()
    } else {
        Suggesting::default()
    };
    let popup_open = !suggesting.names.is_empty();
    let mut completed = Completed::default();
    if value && editing {
        completed = complete(ui, id, &mut text, &mut suggesting);
        completed.held |= pointer_holds(ui, id, completed.held);
    }
    let mut stepped = completed.replaced;
    let mut refusal: Option<Refusal> = None;
    if value
        && editing
        && let Some(requested) = take_step(ui)
    {
        match stepping::step(&text, requested.direction, requested.big) {
            Ok(next) => {
                select_all(ui.ctx(), id, &next);
                text = next;
                stepped = true;
            }
            Err(reason) => refusal = Some(reason),
        }
    }
    let response = widgets::text_field(ui, |ui| {
        ui.add(
            TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(width)
                .margin(FIELD_MARGIN)
                .min_size(vec2(width, 0.0))
                .event_filter(completion::filter(popup_open)),
        )
    });
    widgets::tie_to_caption(ui, &response);
    if focus {
        response.request_focus();
        response.scroll_to_me(Some(Align::Center));
    }
    if response.has_focus() {
        focus_arrived(ui.ctx(), id);
    }
    if completed.held {
        response.request_focus();
    }
    let focused = response.has_focus() || completed.held;
    if arrived(ui, id, focused) && text == stored {
        select_all(ui.ctx(), id, &text);
    }
    if value && focused {
        suggest(
            ui,
            &response,
            &text,
            completed.caret,
            editing_parameter,
            &mut suggesting,
        );
        ui.data_mut(|data| data.insert_temp(suggesting_key, suggesting));
    } else {
        ui.data_mut(|data| data.remove::<Suggesting>(suggesting_key));
    }
    let changed = response.changed() || stepped;
    let edited = changed.then(|| text.trim().to_owned());
    if changed {
        draft = Some(Draft {
            text: text.clone(),
            error: None,
            stored: stored.to_owned(),
        });
    }
    if let Some(reason) = refusal {
        draft = Some(Draft {
            text: text.clone(),
            error: Some(reason.message().to_owned()),
            stored: stored.to_owned(),
        });
    }

    let left = response.lost_focus() && !completed.held;
    let mut committed = None;
    if left {
        let reverted = ui.input(|input| input.key_pressed(Key::Escape));
        if reverted || text.trim() == stored {
            draft = None;
        } else {
            match validate(text.trim()) {
                Ok(value) => {
                    committed = Some(value);
                    draft = None;
                }
                Err(error) => {
                    draft = Some(Draft {
                        text,
                        error: Some(error.to_string()),
                        stored: stored.to_owned(),
                    });
                }
            }
        }
    }

    let error = draft.as_ref().and_then(|draft| draft.error.clone());
    if error.is_some() {
        ui.painter().rect_stroke(
            response.rect,
            f32::from(WIDGET_RADIUS),
            Stroke::new(ERROR_OUTLINE_WIDTH, ui.visuals().error_fg_color),
            StrokeKind::Outside,
        );
    }
    ui.data_mut(|data| match draft {
        Some(draft) => {
            data.insert_temp(id, draft);
        }
        None => data.remove::<Draft>(id),
    });
    FieldResponse {
        committed,
        error,
        edited,
        left,
        response,
    }
}

#[derive(Debug, Clone, Copy)]
struct Holding;

fn pointer_holds(ui: &Ui, id: Id, started: bool) -> bool {
    let key = id.with("holding");
    if started {
        ui.data_mut(|data| data.insert_temp(key, Holding));
        return true;
    }
    let holding = ui.data(|data| data.get_temp::<Holding>(key).is_some());
    let (down, released) =
        ui.input(|input| (input.pointer.any_down(), input.pointer.any_released()));
    if holding && !down && !released {
        ui.data_mut(|data| data.remove::<Holding>(key));
    }
    holding
}

#[derive(Debug, Default)]
struct Completed {
    replaced: bool,
    held: bool,
    caret: Option<usize>,
}

fn complete(ui: &Ui, id: Id, text: &mut String, suggesting: &mut Suggesting) -> Completed {
    let ctx = ui.ctx();
    let mut completed = Completed::default();
    let pressed = ui.input(|input| {
        input
            .pointer
            .primary_pressed()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    if let Some(position) = pressed {
        if let Some(row) = suggesting
            .rows
            .iter()
            .position(|rect| rect.contains(position))
        {
            accept(ctx, id, text, suggesting, row, &mut completed);
            completed.held = true;
            return completed;
        }
        if let Some(offered) = completion::offered(ctx) {
            let (start, end) = completion::selection_of(ctx, id, text);
            let (next, caret) = completion::inserted(text, start, end, &offered);
            *text = next;
            completion::place_caret(ctx, id, caret);
            completion::mark_taken(ctx);
            *suggesting = Suggesting::default();
            completed.replaced = true;
            completed.held = true;
            completed.caret = Some(caret);
            return completed;
        }
    }
    let count = suggesting.names.len();
    if count == 0 {
        return completed;
    }
    match completion::take_key(ui, suggesting.navigated) {
        Some(PopupKey::Next) => {
            suggesting.selected = (suggesting.selected + 1) % count;
            suggesting.navigated = true;
        }
        Some(PopupKey::Previous) => {
            suggesting.selected = (suggesting.selected + count - 1) % count;
            suggesting.navigated = true;
        }
        Some(PopupKey::Accept | PopupKey::Enter) => {
            let row = suggesting.selected;
            accept(ctx, id, text, suggesting, row, &mut completed);
        }
        Some(PopupKey::Close) => {
            let (_, caret) = completion::selection_of(ctx, id, text);
            suggesting.dismissed = completion::token_at(text, caret).map(|token| token.text);
            suggesting.names.clear();
            suggesting.rows.clear();
            suggesting.navigated = false;
        }
        None => {}
    }
    completed
}

fn accept(
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    suggesting: &mut Suggesting,
    row: usize,
    completed: &mut Completed,
) {
    let Some(name) = suggesting.names.get(row).cloned() else {
        return;
    };
    let (_, caret) = completion::selection_of(ctx, id, text);
    if let Some(token) = completion::token_at(text, caret) {
        let (next, placed) = completion::replaced(text, token.start, token.end, &name);
        *text = next;
        completion::place_caret(ctx, id, placed);
        completed.replaced = true;
        completed.caret = Some(placed);
    }
    suggesting.dismissed = Some(name);
    suggesting.names.clear();
    suggesting.rows.clear();
    suggesting.selected = 0;
    suggesting.navigated = false;
}

fn suggest(
    ui: &Ui,
    response: &Response,
    text: &str,
    caret: Option<usize>,
    editing_parameter: Option<ParameterId>,
    suggesting: &mut Suggesting,
) {
    let ctx = ui.ctx();
    let (first, last) = completion::selection_of(ctx, response.id, text);
    let caret = caret.unwrap_or(last);
    let token = (caret == last && first == last)
        .then(|| completion::token_at(text, caret))
        .flatten();
    let token_text = token.map(|token| token.text);
    let blocked = suggesting.dismissed.is_some() && suggesting.dismissed == token_text;
    if !blocked {
        suggesting.dismissed = None;
    }
    let shown: Vec<Candidate> = match (&token_text, completion::available(ctx)) {
        (Some(typed), Some(completions)) if !blocked => {
            completions.matching(typed, editing_parameter, completion::MAX_SHOWN)
        }
        _ => Vec::new(),
    };
    let names: Vec<String> = shown
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect();
    if names != suggesting.names {
        suggesting.selected = 0;
        suggesting.navigated = false;
    }
    suggesting.selected = suggesting.selected.min(names.len().saturating_sub(1));
    suggesting.names = names;
    suggesting.rows = if shown.is_empty() {
        Vec::new()
    } else {
        completion::list(ui, response, &shown, suggesting.selected)
    };
}

#[derive(Debug, Clone, Copy)]
struct HeldFocus;

fn arrived(ui: &Ui, id: Id, focused: bool) -> bool {
    let key = id.with("held-focus");
    let held = ui.data(|data| data.get_temp::<HeldFocus>(key).is_some());
    match (focused, held) {
        (true, false) => ui.data_mut(|data| {
            data.insert_temp(key, HeldFocus);
        }),
        (false, true) => ui.data_mut(|data| data.remove::<HeldFocus>(key)),
        (true, true) | (false, false) => {}
    }
    focused && !held
}

pub fn select_all(context: &egui::Context, id: Id, text: &str) {
    let mut state = TextEdit::load_state(context, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(text.chars().count()),
    )));
    TextEdit::store_state(context, id, state);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    pub dimension: Option<Dimension>,
    pub non_negative: bool,
}

impl Expected {
    pub const ANYTHING: Self = Self {
        dimension: None,
        non_negative: false,
    };
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExpressionError {
    #[error(transparent)]
    Unreadable(#[from] ParseError),
    #[error("{}", sentence(&.0.to_string()))]
    Unevaluable(EvalError),
    #[error("{}", sentence(&.0.to_string()))]
    WrongDimension(EvalError),
    #[error("The value cannot be negative")]
    Negative,
}

pub fn parse_expression(
    document: &Document,
    parameters: &ParameterValues,
    text: &str,
    expected: Expected,
    unit: impl Into<Units>,
) -> Result<Expression, ExpressionError> {
    let unit = unit.into();
    let expression = document.parse(text)?;
    let value = parameters
        .evaluate_expression(&expression)
        .map_err(ExpressionError::Unevaluable)?;
    if let Some(dimension) = expected.dimension {
        expression
            .evaluate_as(dimension, &|id| parameters.value(id))
            .map_err(ExpressionError::WrongDimension)?;
    }
    if expected.non_negative && value.value < 0.0 {
        return Err(ExpressionError::Negative);
    }
    Ok(
        match unit.attach_plain(expected.dimension, value.dimension) {
            Some(plain) => attach_unit(expression, plain),
            None => expression,
        },
    )
}

pub fn parameter_expression(
    document: &Document,
    parameters: &ParameterValues,
    text: &str,
    current: Option<Dimension>,
    unit: impl Into<Units>,
) -> Result<Expression, ExpressionError> {
    let unit = unit.into();
    let expression = parse_expression(document, parameters, text, Expected::ANYTHING, unit)?;
    let plain = parameters
        .evaluate_expression(&expression)
        .is_ok_and(|value| value.dimension.is_plain());
    Ok(match current {
        Some(Dimension::LENGTH) if plain => unit.attach(expression),
        Some(Dimension::ANGLE) if plain => unit.angle.attach(expression),
        _ => expression,
    })
}

pub fn value_text(document: &Document, owner: &ParameterOwner, expression: &Expression) -> String {
    match document.owned_parameter(owner, expression) {
        Some(parameter) => format!(
            "{} = {}",
            parameter.name,
            document.expression_text(&parameter.expression)
        ),
        None => document.expression_text(expression),
    }
}

fn referenced_parameter<'a>(
    document: &'a Document,
    expression: &Expression,
) -> Option<&'a Parameter> {
    match expression {
        Expression::Parameter(id) => document.parameter(*id),
        _ => None,
    }
}

pub fn driving_parameter<'a>(
    document: &'a Document,
    owner: &ParameterOwner,
    expression: &Expression,
) -> Option<&'a Parameter> {
    document
        .owned_parameter(owner, expression)
        .or_else(|| referenced_parameter(document, expression))
}

pub fn driven_text(document: &Document, owner: &ParameterOwner, expression: &Expression) -> String {
    match driving_parameter(document, owner, expression) {
        Some(parameter) => format!(
            "{} = {}",
            parameter.name,
            document.expression_text(&parameter.expression)
        ),
        None => document.expression_text(expression),
    }
}

pub fn shown_value<'a>(
    document: &'a Document,
    owner: &ParameterOwner,
    expression: &'a Expression,
) -> &'a Expression {
    document
        .owned_parameter(owner, expression)
        .map_or(expression, |parameter| &parameter.expression)
}

pub fn copied_value(
    document: &Document,
    feature: FeatureId,
    expression: &Expression,
) -> Expression {
    let owned = referenced_parameter(document, expression).filter(|parameter| {
        matches!(
            &parameter.owner,
            Some(ParameterOwner::Feature { feature: owner, .. }) if *owner == feature
        )
    });
    owned.map_or_else(
        || expression.clone(),
        |parameter| parameter.expression.clone(),
    )
}

pub struct NamedField<'a> {
    pub document: &'a Document,
    pub parameters: &'a ParameterValues,
    pub units: Units,
    pub dimension: Option<Dimension>,
    pub owner: ParameterOwner,
    pub current: &'a Expression,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CheckError {
    #[error(transparent)]
    Rejected(#[from] EditError),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum NamedEditError<P, H> {
    #[error("{0}")]
    Unparsed(P),
    #[error("{0}")]
    Unheld(H),
    #[error(transparent)]
    Rejected(CheckError),
    #[error("This value cannot be given a name here")]
    NotNameable,
    #[error(
        "{parameter} is the reading of {measurement}, taken again on every recompute; change \
         the model to change it"
    )]
    MeasuredReading {
        parameter: String,
        measurement: String,
    },
}

impl NamedField<'_> {
    pub fn transaction<P, H>(
        &self,
        text: &str,
        parse: impl Fn(&str) -> Result<Expression, P>,
        hold: impl FnOnce(Expression) -> Result<Transaction, H>,
    ) -> Result<Transaction, NamedEditError<P, H>> {
        let document = self.document;
        let held = document.owned_parameter(&self.owner, self.current);
        let parsed = |text: &str| parse(text).map_err(NamedEditError::Unparsed);
        match (Naming::split(text), held) {
            (None, None) => hold(parsed(text)?).map_err(NamedEditError::Unheld),
            (None, Some(parameter)) => {
                let unnamed = hold(parsed(text)?).map_err(NamedEditError::Unheld)?;
                checked(document, document.releasing(unnamed, [parameter.id()]))
                    .map_err(NamedEditError::Rejected)
            }
            (Some(naming), Some(parameter)) => self.edit_parameter(parameter, &naming, parse),
            (Some(naming), None) => {
                let expression = self.with_unit(parsed(naming.expression)?);
                let marker =
                    Expression::Negate(Box::new(Expression::Negate(Box::new(expression.clone()))));
                let holding = hold(marker.clone()).map_err(NamedEditError::Unheld)?;
                let mut transaction = document.transaction(format!("Name {}", naming.name));
                let id =
                    transaction.add_owned_parameter(naming.name, expression, self.owner.clone());
                let (holding, count) = holding.substituting(&marker, &Expression::Parameter(id));
                if count == 0 {
                    return Err(NamedEditError::NotNameable);
                }
                for edit in holding.edits() {
                    transaction.edit(edit.clone());
                }
                checked(document, transaction.finish()).map_err(NamedEditError::Rejected)
            }
        }
    }

    pub fn through_reference<P, H>(
        &self,
        text: &str,
        parse: impl Fn(&str) -> Result<Expression, P>,
    ) -> Option<Result<Transaction, NamedEditError<P, H>>> {
        let document = self.document;
        if document
            .owned_parameter(&self.owner, self.current)
            .is_some()
        {
            return None;
        }
        let parameter = referenced_parameter(document, self.current)?;
        let naming = Naming::split(text)?;
        if let Some(measurement) = document.measurement_of(parameter.id()) {
            return Some(Err(NamedEditError::MeasuredReading {
                parameter: parameter.name.clone(),
                measurement: measurement.name.clone(),
            }));
        }
        Some(self.edit_parameter(parameter, &naming, parse))
    }

    fn edit_parameter<P, H>(
        &self,
        parameter: &Parameter,
        naming: &Naming<'_>,
        parse: impl Fn(&str) -> Result<Expression, P>,
    ) -> Result<Transaction, NamedEditError<P, H>> {
        let expression =
            self.with_unit(parse(naming.expression).map_err(NamedEditError::Unparsed)?);
        let id = parameter.id();
        let renaming = (naming.name != parameter.name).then(|| Edit::RenameParameter {
            id,
            name: naming.name.to_owned(),
        });
        let changing = (expression != parameter.expression)
            .then_some(Edit::SetParameterExpression { id, expression });
        checked(
            self.document,
            Transaction::new(
                format!("Edit {}", naming.name),
                renaming.into_iter().chain(changing).collect(),
            ),
        )
        .map_err(NamedEditError::Rejected)
    }

    fn with_unit(&self, expression: Expression) -> Expression {
        let plain = self
            .parameters
            .evaluate_expression(&expression)
            .is_ok_and(|value| value.dimension.is_plain());
        match self.dimension {
            Some(Dimension::LENGTH) if plain => self.units.attach(expression),
            Some(Dimension::ANGLE) if plain => self.units.angle.attach(expression),
            _ => expression,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionTarget {
    pub feature: FeatureId,
    pub constraint: ConstraintId,
}

impl DimensionTarget {
    pub fn owner(self) -> ParameterOwner {
        ParameterOwner::Dimension {
            sketch: self.feature,
            constraint: self.constraint,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    Replaced,
    Followed,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DimensionValueError {
    #[error(transparent)]
    Expression(#[from] ExpressionError),
    #[error("{}", sentence(&.0.to_string()))]
    Unevaluable(EvalError),
    #[error(
        "A {} cannot be negative. It keeps the side the points are drawn on, so enter its size \
         alone.",
        .kind.to_lowercase()
    )]
    NegativeOffset { kind: &'static str },
    #[error("{}", sentence(&.0.to_string()))]
    Refused(DimensionError),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DimensionEditError {
    #[error("The sketch no longer exists")]
    SketchGone { sketch: FeatureId },
    #[error("The dimension no longer exists")]
    DimensionGone {
        sketch: FeatureId,
        constraint: ConstraintId,
    },
    #[error(transparent)]
    Named(NamedEditError<DimensionValueError, CheckError>),
}

impl From<NamedEditError<DimensionValueError, CheckError>> for DimensionEditError {
    fn from(error: NamedEditError<DimensionValueError, CheckError>) -> Self {
        Self::Named(error)
    }
}

pub fn dimension_transaction(
    document: &Document,
    parameters: &ParameterValues,
    target: DimensionTarget,
    text: &str,
    unit: impl Into<Units>,
) -> Result<Transaction, DimensionEditError> {
    dimension_edit(
        document,
        parameters,
        (target, Reference::Replaced),
        text,
        unit,
    )
}

pub fn dimension_edit(
    document: &Document,
    parameters: &ParameterValues,
    (target, reference): (DimensionTarget, Reference),
    text: &str,
    unit: impl Into<Units>,
) -> Result<Transaction, DimensionEditError> {
    let unit = unit.into();
    let owner = document
        .feature(target.feature)
        .ok_or(DimensionEditError::SketchGone {
            sketch: target.feature,
        })?;
    let dimension_gone = DimensionEditError::DimensionGone {
        sketch: target.feature,
        constraint: target.constraint,
    };
    let definition = owner
        .kind
        .sketch()
        .and_then(|sketch| sketch.constraint(target.constraint))
        .ok_or_else(|| dimension_gone.clone())?;
    let offset = matches!(
        definition,
        Constraint::HorizontalDistance { .. } | Constraint::VerticalDistance { .. }
    );
    let expected = Expected {
        dimension: definition.dimension_kind(),
        non_negative: !offset && !matches!(definition, Constraint::Angle { .. }),
    };
    let current = definition.dimension().ok_or(dimension_gone)?;
    let parse = |text: &str| -> Result<Expression, DimensionValueError> {
        let value = parse_expression(document, parameters, text, expected, unit)?;
        let quantity = parameters
            .evaluate_expression(&value)
            .map_err(DimensionValueError::Unevaluable)?;
        definition
            .check_dimension_value(quantity.value)
            .map_err(|error| match error {
                DimensionError::Negative if offset => DimensionValueError::NegativeOffset {
                    kind: definition.kind_name(),
                },
                other => DimensionValueError::Refused(other),
            })?;
        Ok(value)
    };
    let hold = |value| {
        checked::<CheckError>(
            document,
            Transaction::single(
                format!("Edit dimension in {}", owner.name),
                Edit::SetDimension {
                    feature: target.feature,
                    constraint: target.constraint,
                    value,
                },
            ),
        )
    };
    let field = NamedField {
        document,
        parameters,
        units: unit,
        dimension: definition.dimension_kind(),
        owner: target.owner(),
        current,
    };
    if reference == Reference::Followed
        && let Some(edited) = field.through_reference(text, parse)
    {
        return Ok(edited?);
    }
    Ok(field.transaction(text, parse, hold)?)
}

pub fn checked<E: From<CheckError>>(
    document: &Document,
    transaction: Transaction,
) -> Result<Transaction, E> {
    document
        .check(&transaction)
        .map(|()| transaction)
        .map_err(|error| CheckError::from(error).into())
}

pub fn value_preview(
    parameters: &ParameterValues,
    expression: &Expression,
    unit: impl Into<Units>,
) -> Option<String> {
    let unit = unit.into();
    if expression.is_literal() {
        return None;
    }
    parameters
        .evaluate_expression(expression)
        .ok()
        .map(|value| format!("= {}", unit.show(value)))
}

pub fn sentence(clause: &str) -> String {
    let mut characters = clause.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use caditor_expression::Unit;

    use super::*;
    use crate::units::{AngleUnit, LengthUnit};

    fn document() -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Parameters");
        transaction.add_parameter("width", Expression::Measure(40.0, Unit::Millimetre));
        transaction.add_parameter("zero", Expression::Number(0.0));
        document.apply(transaction.finish()).unwrap();
        document
    }

    fn parse(text: &str, expected: Expected) -> Result<Expression, String> {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        parse_expression(
            &document,
            &parameters,
            text,
            expected,
            LengthUnit::Millimetre,
        )
        .map_err(|error| error.to_string())
    }

    const LENGTH: Expected = Expected {
        dimension: Some(Dimension::LENGTH),
        non_negative: true,
    };

    #[test]
    fn field_input_is_parsed_evaluated_and_checked() {
        assert!(parse("width / 2", LENGTH).is_ok());
        assert!(parse("12", LENGTH).is_ok());
        assert_eq!(
            parse("width * width", LENGTH),
            Err("It gives an area, but a length is needed".to_owned())
        );
        assert_eq!(
            parse("-width", LENGTH),
            Err("The value cannot be negative".to_owned())
        );
        assert_eq!(
            parse("width / zero", Expected::ANYTHING),
            Err("It divides by zero".to_owned())
        );
        assert_eq!(
            parse("wdth", Expected::ANYTHING),
            Err("There is no parameter named 'wdth'".to_owned())
        );
        assert!(parse("-width * width", Expected::ANYTHING).is_ok());
    }

    #[test]
    fn a_plain_parameter_value_keeps_the_kind_of_the_parameter() {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        let text = |input: &str, current: Option<Dimension>| {
            parameter_expression(
                &document,
                &parameters,
                input,
                current,
                LengthUnit::Centimetre,
            )
            .map(|expression| document.expression_text(&expression))
        };
        let length = Some(Dimension::LENGTH);
        let angle = Some(Dimension::ANGLE);
        assert_eq!(text("12", length).unwrap(), "12 cm");
        assert_eq!(text("-2", length).unwrap(), "-2 cm");
        assert_eq!(text("2 * 3", length).unwrap(), "(2 * 3) cm");
        assert_eq!(text("width / 2", length).unwrap(), "width / 2");
        assert_eq!(text("30", angle).unwrap(), "30 deg");
        assert_eq!(text("5 mm", angle).unwrap(), "5 mm");
        assert_eq!(text("4", Some(Dimension::NONE)).unwrap(), "4");
        assert_eq!(text("4", None).unwrap(), "4");
    }

    #[test]
    fn plain_numbers_typed_for_an_angle_take_the_chosen_angle_unit() {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        let radians = Units {
            angle: AngleUnit::Radian,
            ..Units::default()
        };
        let angle = Expected {
            dimension: Some(Dimension::ANGLE),
            non_negative: false,
        };
        let text = |input: &str, unit: Units| {
            parse_expression(&document, &parameters, input, angle, unit)
                .map(|expression| document.expression_text(&expression))
        };

        assert_eq!(text("1.5", radians).unwrap(), "1.5 rad");
        assert_eq!(text("1.5", Units::default()).unwrap(), "1.5");
        assert_eq!(text("45 deg", radians).unwrap(), "45 deg");
        assert!(text("2 * 3", radians).is_err());
        assert_eq!(
            parameter_expression(&document, &parameters, "2", Some(Dimension::ANGLE), radians)
                .map(|expression| document.expression_text(&expression))
                .unwrap(),
            "2 rad"
        );
        assert_eq!(
            value_preview(&parameters, &Expression::Number(1.0), radians),
            None
        );
    }

    #[test]
    fn a_dimension_value_must_suit_its_constraint() {
        let mut document = document();
        let mut sketch = caditor_sketch::Sketch::new(caditor_geometry::Plane::XY);
        let circle = sketch.add_circle(caditor_geometry::Point2::ZERO, 5.0);
        let radius = sketch
            .add_constraint(Constraint::Radius {
                entity: circle,
                value: Expression::Measure(5.0, Unit::Millimetre),
            })
            .unwrap();
        let mut transaction = document.transaction("Sketch");
        let feature = transaction.add_feature("Holes", caditor_document::FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let parameters = ParameterValues::evaluate(&document);
        let target = DimensionTarget {
            feature,
            constraint: radius,
        };
        let edit = |text| {
            dimension_transaction(&document, &parameters, target, text, LengthUnit::Millimetre)
                .map_err(|error| error.to_string())
        };

        assert_eq!(
            edit("width - 40 mm").err(),
            Some("A radius must be greater than zero".to_owned())
        );
        assert_eq!(
            edit("-2 mm").err(),
            Some("The value cannot be negative".to_owned())
        );
        assert_eq!(
            edit("5 deg").err(),
            Some("It gives an angle, but a length is needed".to_owned())
        );
        assert!(edit("width / 8").is_ok());
    }

    #[test]
    fn a_dimension_named_in_its_field_holds_a_model_parameter() {
        let mut document = document();
        let mut sketch = caditor_sketch::Sketch::new(caditor_geometry::Plane::XY);
        let circle = sketch.add_circle(caditor_geometry::Point2::ZERO, 5.0);
        let radius = sketch
            .add_constraint(Constraint::Radius {
                entity: circle,
                value: Expression::Measure(5.0, Unit::Millimetre),
            })
            .unwrap();
        let mut transaction = document.transaction("Sketch");
        let feature = transaction.add_feature("Holes", caditor_document::FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let target = DimensionTarget {
            feature,
            constraint: radius,
        };
        let edit = |document: &Document, text| {
            let parameters = ParameterValues::evaluate(document);
            dimension_transaction(document, &parameters, target, text, LengthUnit::Millimetre)
                .map_err(|error| error.to_string())
        };
        let held = |document: &Document| {
            let value = document
                .feature(feature)
                .and_then(|feature| feature.kind.sketch())
                .and_then(|sketch| sketch.constraint(radius))
                .and_then(Constraint::dimension)
                .unwrap()
                .clone();
            value_text(document, &target.owner(), &value)
        };

        assert_eq!(
            edit(&document, "bore = -1 mm").err(),
            Some("The value cannot be negative".to_owned())
        );
        assert_eq!(
            edit(&document, "width = 3 mm").err(),
            Some("There is already a parameter named 'width'".to_owned())
        );
        assert_eq!(
            edit(&document, "sin = 3 mm").err(),
            Some("'sin' is a function, so it cannot be used as a name".to_owned())
        );

        document
            .apply(edit(&document, "bore = 6").unwrap())
            .unwrap();

        assert_eq!(held(&document), "bore = 6 mm");

        document
            .apply(edit(&document, "hole = width / 8").unwrap())
            .unwrap();

        assert_eq!(held(&document), "hole = width / 8");
        assert!(document.parameter_named("bore").is_none());

        document.apply(edit(&document, "4 mm").unwrap()).unwrap();

        assert_eq!(held(&document), "4 mm");
        assert!(document.parameter_named("hole").is_none());
    }

    #[test]
    fn a_negative_horizontal_or_vertical_distance_is_refused_with_its_reason() {
        let mut document = document();
        let mut sketch = caditor_sketch::Sketch::new(caditor_geometry::Plane::XY);
        let from = sketch.add_point(caditor_geometry::Point2::ZERO);
        let to = sketch.add_point(caditor_geometry::Point2::X);
        let across = sketch
            .add_constraint(Constraint::HorizontalDistance {
                from,
                to,
                value: Expression::Measure(1.0, Unit::Millimetre),
            })
            .unwrap();
        let mut transaction = document.transaction("Sketch");
        let feature = transaction.add_feature("Holes", caditor_document::FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let parameters = ParameterValues::evaluate(&document);
        let target = DimensionTarget {
            feature,
            constraint: across,
        };
        let edit = |text| {
            dimension_transaction(&document, &parameters, target, text, LengthUnit::Millimetre)
                .map_err(|error| error.to_string())
        };

        assert_eq!(
            edit("-2 mm").err(),
            Some(
                "A horizontal distance cannot be negative. It keeps the side the points are \
                 drawn on, so enter its size alone."
                    .to_owned()
            )
        );
        assert_eq!(edit("-width").err(), edit("-2 mm").err());
        assert!(edit("width / 8").is_ok());
    }

    #[test]
    fn previews_show_computed_values_only_for_non_literals() {
        let document = document();
        let parameters = ParameterValues::evaluate(&document);
        let preview = |text: &str| {
            value_preview(
                &parameters,
                &document.parse(text).unwrap(),
                LengthUnit::Millimetre,
            )
        };
        assert_eq!(preview("width / 4"), Some("= 10 mm".to_owned()));
        assert_eq!(preview("2 cm"), None);
        assert_eq!(preview("width / zero"), None);
    }
}
