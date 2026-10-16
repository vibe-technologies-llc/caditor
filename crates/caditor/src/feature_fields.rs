use std::fmt::Display;

use caditor_document::{
    AxisReference, BodyOperation, Document, FeatureId, MAX_CONE_ANGLE, MAX_MATE_ANGLE_DEGREES,
    MAX_PATTERN_INSTANCES, MAX_PRISM_SIDES, MAX_RIB_DRAFT_DEGREES, MIN_PRISM_SIDES, ParameterOwner,
    Transaction, capitalized, describe_axis,
};
use caditor_expression::{Dimension, EvalError, Expression};
use caditor_kernel::{LEAST_COIL_TURNS, MAX_COIL_TAPER_DEGREES, MAX_COIL_TURNS, MAX_TAPER_DEGREES};
use egui::{Id, Label, RichText, TextWrapMode, Ui, WidgetText};

use crate::{
    appearance, completion, datum_tools,
    editing::EditingCommand,
    field::{self, Expected, ExpressionError},
    icons,
    model::{Action, Model, Notice},
    pattern_tools,
    reference_picking::{self, Picking, Slot},
    selection::Selection,
    visibility,
    widgets::{self, FIELD_WIDTH, Named, Tone},
};

pub const ABOVE_ZERO: &str = "Enter a value above zero";
pub const ZERO_OR_MORE: &str = "Enter a value of zero or more";
pub const ABOVE_ZERO_OR_REVERSE: &str =
    "Enter a value above zero. Use Reverse direction to go the other way";
pub const NOT_ZERO_OR_REVERSE: &str =
    "Enter a value other than zero; a negative one goes the other way";
pub const TURN: &str = "Enter an angle above 0° and up to 360°";
pub const TURNS_TOGETHER: &str =
    "Enter an angle above 0°; both angles together may turn up to 360°";
pub const WHOLE_COUNT: &str = "Enter a whole number of at least 1";
pub const REVERSE_DIRECTION: &str = "Reverse direction";
pub const NONE_CHOSEN: &str = "None chosen";
pub const MISSING_BODY: &str = "Missing body";
pub const MISSING_SKETCH: &str = "Missing sketch";
pub const USE_SELECTED: &str = "Use selected";
pub const CHOOSE_IN_VIEW: &str = "Choose in the view";
pub const STOP_CHOOSING: &str = "Stop choosing";
pub const KEEPS_CHOOSING: &str = "Keeps choosing until Esc";
const KEEPS_CHOOSING_HOVER: &str =
    "Every click adds or leaves out; choosing goes on until you press Esc or Stop choosing";
const STOP_LIST_HOVER: &str =
    "Stop choosing and close the feature, keeping its changes, as Esc does";
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const WHOLE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    Any,
    AboveZero,
    ZeroOrMore,
    AboveZeroOrReverse,
    FlipWhenNegative,
    Turn,
    TurnBeside(f64),
    Count,
    Sides,
    ConeAngle,
    ChamferAngle,
    Taper,
    MateAngle,
    CoilTurns,
    CoilTaper,
    RibDraft,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuleError {
    #[error("{}", ABOVE_ZERO)]
    AboveZero,
    #[error("{}", ZERO_OR_MORE)]
    ZeroOrMore,
    #[error("{}", ABOVE_ZERO_OR_REVERSE)]
    AboveZeroOrReverse,
    #[error("{}", NOT_ZERO_OR_REVERSE)]
    NotZeroOrReverse,
    #[error("{}", TURN)]
    Turn,
    #[error("{}", TURNS_TOGETHER)]
    TurnsTogether,
    #[error("{}", WHOLE_COUNT)]
    WholeCount,
    #[error("Enter a count of at most {most}")]
    CountAbove { most: u32 },
    #[error("Enter a whole number from {least} to {most}")]
    SidesOutside { least: u32, most: u32 },
    #[error("Enter an angle above 0° and up to {most}°")]
    ConeAngleOutside { most: f64 },
    #[error("Enter an angle above 0° and below {below}°")]
    ChamferAngleOutside { below: f64 },
    #[error("Enter an angle between -{limit}° and {limit}°; a positive taper draws the sides in")]
    TaperOutside { limit: f64 },
    #[error("Enter an angle from 0° to {most}°")]
    MateAngleOutside { most: f64 },
    #[error("Enter a number of turns from {least} to {most}")]
    CoilTurnsOutside { least: f64, most: f64 },
    #[error(
        "Enter an angle between -{limit}° and {limit}°; a positive taper widens the coil as it \
         rises"
    )]
    CoilTaperOutside { limit: f64 },
    #[error("Enter an angle from 0° up to below {below}°; the web widens by it toward the body")]
    RibDraftOutside { below: f64 },
}

impl Rule {
    pub fn check(self, value: f64) -> Result<(), RuleError> {
        let fractional = (value - value.round()).abs() > WHOLE_TOLERANCE;
        match self {
            Self::Any => Ok(()),
            Self::AboveZero if value <= 0.0 => Err(RuleError::AboveZero),
            Self::ZeroOrMore if value < 0.0 => Err(RuleError::ZeroOrMore),
            Self::AboveZeroOrReverse if value <= 0.0 => Err(RuleError::AboveZeroOrReverse),
            Self::FlipWhenNegative if value == 0.0 => Err(RuleError::NotZeroOrReverse),
            Self::Turn if value <= 0.0 || value > FULL_TURN_DEGREES => Err(RuleError::Turn),
            Self::TurnBeside(other) if value <= 0.0 || value + other > FULL_TURN_DEGREES => {
                Err(RuleError::TurnsTogether)
            }
            Self::Count if fractional || value < 1.0 => Err(RuleError::WholeCount),
            Self::Count if value > f64::from(MAX_PATTERN_INSTANCES) => Err(RuleError::CountAbove {
                most: MAX_PATTERN_INSTANCES,
            }),
            Self::Sides
                if fractional
                    || value < f64::from(MIN_PRISM_SIDES)
                    || value > f64::from(MAX_PRISM_SIDES) =>
            {
                Err(RuleError::SidesOutside {
                    least: MIN_PRISM_SIDES,
                    most: MAX_PRISM_SIDES,
                })
            }
            Self::ConeAngle if value <= 0.0 || value > MAX_CONE_ANGLE => {
                Err(RuleError::ConeAngleOutside {
                    most: MAX_CONE_ANGLE,
                })
            }
            Self::ChamferAngle if value <= 0.0 || value >= HALF_TURN_DEGREES => {
                Err(RuleError::ChamferAngleOutside {
                    below: HALF_TURN_DEGREES,
                })
            }
            Self::Taper if value.abs() >= MAX_TAPER_DEGREES => Err(RuleError::TaperOutside {
                limit: MAX_TAPER_DEGREES,
            }),
            Self::MateAngle if !(0.0..=MAX_MATE_ANGLE_DEGREES).contains(&value) => {
                Err(RuleError::MateAngleOutside {
                    most: MAX_MATE_ANGLE_DEGREES,
                })
            }
            Self::CoilTurns if !(LEAST_COIL_TURNS..=MAX_COIL_TURNS).contains(&value) => {
                Err(RuleError::CoilTurnsOutside {
                    least: LEAST_COIL_TURNS,
                    most: MAX_COIL_TURNS,
                })
            }
            Self::CoilTaper if value.abs() >= MAX_COIL_TAPER_DEGREES => {
                Err(RuleError::CoilTaperOutside {
                    limit: MAX_COIL_TAPER_DEGREES,
                })
            }
            Self::RibDraft if !(0.0..MAX_RIB_DRAFT_DEGREES).contains(&value) => {
                Err(RuleError::RibDraftOutside {
                    below: MAX_RIB_DRAFT_DEGREES,
                })
            }
            Self::AboveZero
            | Self::ZeroOrMore
            | Self::AboveZeroOrReverse
            | Self::FlipWhenNegative
            | Self::Turn
            | Self::TurnBeside(_)
            | Self::Count
            | Self::Sides
            | Self::ConeAngle
            | Self::ChamferAngle
            | Self::Taper
            | Self::MateAngle
            | Self::CoilTurns
            | Self::CoilTaper
            | Self::RibDraft => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum QuantityError {
    #[error(transparent)]
    Expression(#[from] ExpressionError),
    #[error("{}", field::sentence(&.0.to_string()))]
    Unevaluable(EvalError),
    #[error(transparent)]
    Rule(#[from] RuleError),
}

pub fn parse_quantity(
    model: &Model,
    dimension: Dimension,
    rule: Rule,
    text: &str,
) -> Result<Expression, QuantityError> {
    let parameters = model.parameters();
    let parsed = field::parse_expression(
        model.document(),
        parameters,
        text,
        Expected {
            dimension: Some(dimension),
            non_negative: false,
        },
        model.units(),
    )?;
    let value = parameters
        .evaluate_expression(&parsed)
        .map_err(QuantityError::Unevaluable)?
        .value;
    rule.check(value)?;
    Ok(parsed)
}

pub struct Quantity<'a> {
    pub feature: FeatureId,
    pub id: Id,
    pub expression: &'a Expression,
    pub dimension: Dimension,
    pub rule: Rule,
}

pub struct Drafting {
    pub committed: Option<Transaction>,
    pub draft: Option<Option<Transaction>>,
}

impl Drafting {
    pub fn into_actions(self, feature: FeatureId) -> impl Iterator<Item = Action> {
        let preview = self.draft.map(|draft| Action::Preview { feature, draft });
        preview.into_iter().chain(self.committed.map(Action::Apply))
    }
}

pub fn expression_row_drafting<E: Display>(
    ui: &mut Ui,
    model: &Model,
    caption: &str,
    quantity: Quantity<'_>,
    change: impl Fn(Expression) -> Result<Transaction, E>,
) -> Drafting {
    let row = expression_row_with(ui, model, (caption, caption), quantity, change, |_| false);
    row.drafting
}

pub struct RemovableValue {
    pub drafting: Drafting,
    pub removed: bool,
    pub hovered: bool,
}

pub fn expression_row_removable<E: Display>(
    ui: &mut Ui,
    model: &Model,
    (caption, owned_as): (&str, &str),
    quantity: Quantity<'_>,
    change: impl Fn(Expression) -> Result<Transaction, E>,
    remove_hover: &str,
) -> RemovableValue {
    expression_row_with(ui, model, (caption, owned_as), quantity, change, |ui| {
        widgets::icon_button(ui, icons::REMOVE, remove_hover).clicked()
    })
}

fn expression_row_with<E: Display>(
    ui: &mut Ui,
    model: &Model,
    (caption, owned_as): (&str, &str),
    quantity: Quantity<'_>,
    change: impl Fn(Expression) -> Result<Transaction, E>,
    trailing: impl FnOnce(&mut Ui) -> bool,
) -> RemovableValue {
    widgets::caption(ui, caption);
    let document = model.document();
    let parameters = model.parameters();
    let unit = model.units();
    let owner = ParameterOwner::Feature {
        feature: quantity.feature,
        value: owned_as.to_owned(),
    };
    let parse = |text: &str| parse_quantity(model, quantity.dimension, quantity.rule, text);
    let named = field::NamedField {
        document,
        parameters,
        units: unit,
        dimension: Some(quantity.dimension),
        owner: owner.clone(),
        current: quantity.expression,
    };
    let validate = |text: &str| named.transaction(text, parse, &change);
    if let Some(held) = document.owned_parameter(&owner, quantity.expression) {
        completion::editing_parameter(ui, quantity.id, held.id());
    }
    let row = ui.horizontal_wrapped(|ui| {
        let field = field::value_field(
            ui,
            quantity.id,
            &field::value_text(document, &owner, quantity.expression),
            FIELD_WIDTH,
            false,
            validate,
        );
        let shown = field::shown_value(document, &owner, quantity.expression);
        if field.error.is_none()
            && let Some(preview) = field::value_preview(parameters, shown, unit)
        {
            ui.add(Label::new(widgets::muted(preview, ui)).wrap_mode(TextWrapMode::Extend));
        }
        let draft = match (field.committed.is_some(), field.left, field.edited) {
            (true, _, _) | (false, false, None) => None,
            (false, true, _) => Some(None),
            (false, false, Some(text)) => Some(validate(&text).ok()),
        };
        let removed = trailing(ui);
        (field.committed, field.error, draft, removed)
    });
    let hovered = row.response.contains_pointer();
    let (committed, error, draft, removed) = row.inner;
    ui.end_row();
    if let Some(error) = error {
        widgets::error_row(ui, &error);
    }
    if ui.memory(|memory| memory.has_focus(quantity.id)) {
        draft_failure_row(ui, model, quantity.feature);
    }
    RemovableValue {
        drafting: Drafting { committed, draft },
        removed,
        hovered,
    }
}

pub fn refusal(feature: &str, reason: &str) -> Action {
    Action::Inform(Notice::error(format!(
        "{feature} was not changed: {reason}"
    )))
}

pub fn applied<E: Display>(feature: &str, change: Result<Transaction, E>) -> Action {
    match change {
        Ok(transaction) => Action::Apply(transaction),
        Err(reason) => refusal(feature, &reason.to_string()),
    }
}

pub fn description_row(ui: &mut Ui, text: &str) {
    ui.label("");
    ui.add(Label::new(widgets::muted(text, ui)).wrap());
    ui.end_row();
}

pub fn missing(ui: &mut Ui, text: &str) {
    let warning = appearance::tokens(ui).warn;
    ui.horizontal_wrapped(|ui| {
        widgets::icon_label(ui, icons::WARNING, warning);
        ui.colored_label(warning, text);
    });
}

pub fn none_chosen(ui: &mut Ui) {
    ui.label(widgets::muted(NONE_CHOSEN, ui));
}

pub fn feature_name(document: &Document, feature: FeatureId) -> Option<&str> {
    document
        .feature(feature)
        .map(|feature| feature.name.as_str())
}

pub fn feature_row(ui: &mut Ui, document: &Document, caption: &str, feature: FeatureId) {
    widgets::caption(ui, caption);
    match document.body_label(feature) {
        Some(name) => {
            ui.label(name);
        }
        None => missing(ui, MISSING_BODY),
    }
    ui.end_row();
}

pub const POSITION_CAPTIONS: [&str; 2] = ["Position X", "Position Y"];

pub const WORLD: &str = "World";

pub struct FrameRow<'a> {
    pub feature: FeatureId,
    pub salt: &'a str,
    pub caption: &'a str,
    pub current: Option<FeatureId>,
}

pub fn frame_row<E: Display>(
    ui: &mut Ui,
    document: &Document,
    row: &FrameRow<'_>,
    change: impl Fn(Option<FeatureId>) -> Result<Transaction, E>,
) -> Option<Action> {
    let before = document.feature_index(row.feature).unwrap_or(usize::MAX);
    let frames = datum_tools::frames_before(document, before);
    if frames.is_empty() && row.current.is_none() {
        return None;
    }
    let name = |frame: FeatureId| {
        document.feature(frame).map_or_else(
            || "A deleted coordinate system".to_owned(),
            |feature| feature.name.clone(),
        )
    };
    let current = row.current.map_or_else(|| WORLD.to_owned(), name);
    widgets::caption(ui, row.caption);
    let chosen = combo(ui, Id::unique((row.salt, row.feature)), current, || {
        std::iter::once(None)
            .chain(frames.into_iter().map(Some))
            .map(|frame| Choice {
                label: frame.map_or_else(|| WORLD.to_owned(), name),
                selected: frame == row.current,
                change: change(frame).map(Action::Apply),
            })
            .collect()
    });
    ui.end_row();
    chosen
}

pub fn combo_text(ui: &mut Ui, name: Option<&str>, missing: &str) -> WidgetText {
    match name {
        Some(name) => name.into(),
        None => {
            let warning = appearance::tokens(ui).warn;
            widgets::icon_label(ui, icons::WARNING, warning);
            RichText::new(missing).color(warning).into()
        }
    }
}

pub struct Choice<T, E> {
    pub label: String,
    pub selected: bool,
    pub change: Result<T, E>,
}

pub fn combo<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    selected: impl Into<WidgetText>,
    choices: impl FnOnce() -> Vec<Choice<T, E>>,
) -> Option<T> {
    listing(ui, id, selected, choices).0
}

pub fn principal_combo<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    selected: impl Into<WidgetText>,
    choices: impl FnOnce() -> Vec<Choice<T, E>>,
) -> Option<T> {
    let (chosen, open) = listing(ui, id, selected, choices);
    if open {
        visibility::listing_principal(ui.ctx());
    }
    chosen
}

fn listing<T, E: Display>(
    ui: &mut Ui,
    id: Id,
    selected: impl Into<WidgetText>,
    choices: impl FnOnce() -> Vec<Choice<T, E>>,
) -> (Option<T>, bool) {
    let mut chosen = None;
    let shown = widgets::combo(id, selected).show(ui, |ui| {
        for choice in choices() {
            let enabled = choice.change.is_ok() || choice.selected;
            let button = widgets::button(choice.label.clone()).selected(choice.selected);
            let response = ui.add_enabled(
                enabled,
                Named::new(button, choice.label.as_str()).selected(choice.selected),
            );
            let response = match &choice.change {
                Err(reason) if !choice.selected => {
                    response.on_disabled_hover_text(reason.to_string())
                }
                Err(_) | Ok(_) => response,
            };
            if response.clicked()
                && !choice.selected
                && let Ok(action) = choice.change
            {
                chosen = Some(action);
            }
        }
    });
    (chosen, shown.inner.is_some())
}

pub fn operation_name(operation: BodyOperation) -> &'static str {
    match operation {
        BodyOperation::NewBody => "New body",
        BodyOperation::Add(_) => "Add to body",
        BodyOperation::Remove(_) => "Remove from body",
        BodyOperation::Intersect(_) => "Intersect with body",
    }
}

pub struct OperationRows {
    pub feature: FeatureId,
    pub operation: BodyOperation,
    pub ids: (&'static str, &'static str),
}

pub fn body_operation_rows<E: Display>(
    ui: &mut Ui,
    model: &Model,
    rows: &OperationRows,
    change: impl Fn(BodyOperation) -> Result<Transaction, E>,
) -> Vec<Action> {
    let OperationRows {
        feature,
        operation,
        ids: (operation_id, body_id),
    } = *rows;
    let mut actions = Vec::new();
    let document = model.document();
    let bodies = document.bodies_before_in(feature, model.evaluation());
    let target = operation.target().or(bodies.last().copied());
    widgets::caption(ui, "Result");
    let candidates = [
        Some(BodyOperation::NewBody),
        target.map(BodyOperation::Add),
        target.map(BodyOperation::Remove),
        target.map(BodyOperation::Intersect),
    ];
    let chosen = combo(
        ui,
        Id::unique((operation_id, feature)),
        operation_name(operation),
        || {
            candidates
                .into_iter()
                .flatten()
                .map(|candidate| Choice {
                    label: operation_name(candidate).to_owned(),
                    selected: operation_name(candidate) == operation_name(operation),
                    change: change(candidate).map(Action::Apply),
                })
                .collect()
        },
    );
    ui.end_row();
    actions.extend(chosen);
    let Some(target) = operation.target() else {
        return actions;
    };
    widgets::caption(ui, "Body");
    let name = document.body_label(target);
    let chosen = ui
        .horizontal(|ui| {
            let text = combo_text(ui, name.as_deref(), MISSING_BODY);
            combo(ui, Id::unique((body_id, feature)), text, || {
                bodies
                    .iter()
                    .filter_map(|body| {
                        Some(Choice {
                            label: document.body_label(*body)?,
                            selected: *body == target,
                            change: change(operation.with_target(*body)).map(Action::Apply),
                        })
                    })
                    .collect()
            })
        })
        .inner;
    ui.end_row();
    actions.extend(chosen);
    actions
}

pub struct Segment<'a, E> {
    pub label: &'a str,
    pub hover: &'a str,
    pub change: Option<Result<Transaction, E>>,
}

pub fn segmented_row<E: Display>(
    ui: &mut Ui,
    caption: &str,
    feature: &str,
    segments: Vec<Segment<'_, E>>,
) -> Option<Action> {
    widgets::caption(ui, caption);
    let labels: Vec<(&str, &str)> = segments
        .iter()
        .map(|segment| (segment.label, segment.hover))
        .collect();
    let current = segments
        .iter()
        .position(|segment| segment.change.is_none())
        .unwrap_or(0);
    let chosen = widgets::segmented(ui, &labels, current);
    ui.end_row();
    let change = segments.into_iter().nth(chosen?)?.change?;
    Some(applied(feature, change))
}

pub fn reverse_row(ui: &mut Ui, label: &str, reversed: bool) -> Option<bool> {
    ui.label("");
    let mut flipped = reversed;
    let changed = widgets::checkbox(ui, &mut flipped, label).changed();
    ui.end_row();
    changed.then_some(flipped)
}

pub enum Shown {
    Named(String),
    NoneChosen,
}

pub struct Picker<'a, E> {
    pub feature: FeatureId,
    pub slot: Slot,
    pub selected: Result<Transaction, E>,
    pub hover: &'a str,
}

#[derive(Clone)]
struct OfferedChange<E> {
    basis: (u64, u64, u64),
    change: Result<Transaction, E>,
}

pub fn offered_change<E: Clone + Send + Sync + 'static>(
    ctx: &egui::Context,
    model: &Model,
    selection: &Selection,
    reference: (FeatureId, Slot),
    compute: impl FnOnce() -> Result<Transaction, E>,
) -> Result<Transaction, E> {
    let id = Id::unique(("offered-change", reference.0, reference.1));
    let basis = (
        selection.generation(),
        model.revision(),
        model.evaluation_generation(),
    );
    let known = ctx
        .data(|data| data.get_temp::<OfferedChange<E>>(id))
        .filter(|known| known.basis == basis);
    if let Some(known) = known {
        return known.change;
    }
    let change = compute();
    ctx.data_mut(|data| {
        data.insert_temp(
            id,
            OfferedChange {
                basis,
                change: change.clone(),
            },
        );
    });
    change
}

pub fn choosing_prompt(
    ui: &mut Ui,
    model: &Model,
    (feature, slot): (FeatureId, Slot),
    stop: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(picking) =
        reference_picking::current(ui.ctx()).filter(|picking| picking.is_for(feature, slot))
    else {
        return false;
    };
    let prompt = format!("{}.", reference_picking::prompt(model, picking));
    if choosing_row(ui, &prompt, slot.keeps_choosing(), (stop, None)) {
        actions.push(Action::Editing(EditingCommand::StopPicking));
    }
    true
}

fn choosing_row(
    ui: &mut Ui,
    prompt: &str,
    keeps_choosing: bool,
    (stop, stop_hover): (&str, Option<&str>),
) -> bool {
    ui.add(Label::new(widgets::muted(prompt, ui)).wrap());
    ui.horizontal_wrapped(|ui| {
        if keeps_choosing {
            widgets::pill(ui, Tone::Info, KEEPS_CHOOSING).on_hover_text(KEEPS_CHOOSING_HOVER);
        }
        let button = widgets::small_button(ui, icons::CLOSE, stop);
        let button = ui.add(button);
        match stop_hover {
            Some(hover) => button.on_hover_text(hover),
            None => button,
        }
        .clicked()
    })
    .inner
}

pub fn reference_picker<E>(
    ui: &mut Ui,
    model: &Model,
    picker: Picker<'_, E>,
    actions: &mut Vec<Action>,
) {
    let reference = (picker.feature, picker.slot);
    if choosing_prompt(ui, model, reference, STOP_CHOOSING, actions) {
        return;
    }
    match picker.selected {
        Ok(transaction) => {
            let button = widgets::small_button(ui, icons::USE_SELECTED, USE_SELECTED);
            if ui.add(button).on_hover_text(picker.hover).clicked() {
                actions.push(Action::Apply(transaction));
            }
        }
        Err(_) => {
            let picking = Picking::new(picker.feature, picker.slot);
            let hover = reference_picking::prompt(model, picking);
            if widgets::choose_in_view(ui, &hover) {
                actions.push(Action::Editing(EditingCommand::Pick(picking)));
            }
        }
    }
}

pub enum ListChoosing {
    Start,
    Stop,
}

pub fn list_in_view(ui: &mut Ui, opened: bool, prompt: &str, hover: &str) -> Option<ListChoosing> {
    if choosing_list(ui, opened) {
        choosing_row(ui, prompt, true, (STOP_CHOOSING, Some(STOP_LIST_HOVER)))
            .then_some(ListChoosing::Stop)
    } else {
        widgets::choose_in_view(ui, hover).then_some(ListChoosing::Start)
    }
}

pub const ALREADY_CHOSEN: &str = "It is already the one chosen";

pub struct AxisRow<'a, E> {
    pub caption: &'a str,
    pub shown: Option<String>,
    pub picker: Picker<'a, E>,
}

pub fn axis_row<E, L: Display>(
    ui: &mut Ui,
    model: &Model,
    row: AxisRow<'_, E>,
    listed: Option<&dyn Fn() -> Vec<Choice<Action, L>>>,
    actions: &mut Vec<Action>,
) {
    widgets::caption(ui, row.caption);
    let id = Id::unique(("axis-row", row.caption, row.picker.feature));
    ui.vertical(|ui| {
        match (listed, row.shown) {
            (Some(listed), shown) => {
                let shown: WidgetText = match shown {
                    Some(text) => text.into(),
                    None => widgets::muted(NONE_CHOSEN, ui).into(),
                };
                actions.extend(principal_combo(ui, id, shown, listed));
            }
            (None, Some(text)) => {
                ui.add(Label::new(text).wrap());
            }
            (None, None) => none_chosen(ui),
        }
        reference_picker(ui, model, row.picker, actions);
    });
    ui.end_row();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChoiceRefusal {
    #[error("{}", ALREADY_CHOSEN)]
    AlreadyChosen,
}

pub fn model_axis_choices<E: From<ChoiceRefusal>>(
    model: &Model,
    feature: FeatureId,
    current: Option<&AxisReference>,
    change: &dyn Fn(AxisReference) -> Result<Transaction, E>,
) -> Vec<Choice<Action, E>> {
    let document = model.document();
    let listed = pattern_tools::listed_axes(model, feature);
    let shown_too = current.filter(|current| !listed.contains(current)).cloned();
    listed
        .into_iter()
        .chain(shown_too)
        .map(|axis| {
            let selected = current == Some(&axis);
            Choice {
                label: capitalized(&describe_axis(document, &axis)),
                selected,
                change: if selected {
                    Err(ChoiceRefusal::AlreadyChosen.into())
                } else {
                    change(axis).map(Action::Apply)
                },
            }
        })
        .collect()
}

pub fn reference_row<E>(
    ui: &mut Ui,
    model: &Model,
    caption: &str,
    shown: Shown,
    picker: Picker<'_, E>,
    remove: Option<&str>,
    actions: &mut Vec<Action>,
) -> bool {
    widgets::caption(ui, caption);
    let removed = ui
        .vertical(|ui| {
            let removed = match (shown, remove) {
                (Shown::Named(text), Some(hover)) => {
                    widgets::removable_row(ui, RichText::new(text), hover)
                }
                (Shown::Named(text), None) => {
                    ui.add(Label::new(text).wrap());
                    false
                }
                (Shown::NoneChosen, _) => {
                    none_chosen(ui);
                    false
                }
            };
            reference_picker(ui, model, picker, actions);
            removed
        })
        .inner;
    ui.end_row();
    removed
}

fn draft_failure_row(ui: &mut Ui, model: &Model, feature: FeatureId) {
    let Some(error) = model.draft_failure(feature) else {
        return;
    };
    ui.label("");
    widgets::callout(ui, Tone::Error, |ui| {
        ui.add(Label::new(format!("{DRAFT_FAILS} {}", error.reason())).wrap());
        ui.add(Label::new(widgets::muted(error.remedy(), ui)).wrap());
    });
    ui.end_row();
}

pub const DRAFT_FAILS: &str = "With the value being typed:";

pub fn info_callout(ui: &mut Ui, text: &str) {
    widgets::callout(ui, Tone::Info, |ui| {
        ui.add(Label::new(text).wrap());
    });
}

pub fn choosing_list(ui: &Ui, opened: bool) -> bool {
    opened && reference_picking::current(ui.ctx()).is_none()
}
