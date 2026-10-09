use caditor_document::{
    Document, FeatureId, MAX_CONE_ANGLE, MAX_PATTERN_INSTANCES, MAX_PRISM_SIDES, MIN_PRISM_SIDES,
    ParameterOwner, Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::MAX_TAPER_DEGREES;
use egui::{ComboBox, Id, Label, RichText, TextWrapMode, Ui, WidgetText};

use crate::{
    appearance, datum_tools,
    editing::EditingCommand,
    field::{self, Expected},
    icons,
    model::{Action, Model, Notice},
    reference_picking::{self, Picking, Slot},
    selection::Selection,
    widgets::{self, FIELD_WIDTH, Named, Tone},
};

pub const ABOVE_ZERO: &str = "Enter a value above zero";
pub const ZERO_OR_MORE: &str = "Enter a value of zero or more";
pub const ABOVE_ZERO_OR_REVERSE: &str =
    "Enter a value above zero. Use Reverse direction to go the other way";
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
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const WHOLE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    Any,
    AboveZero,
    ZeroOrMore,
    AboveZeroOrReverse,
    Turn,
    TurnBeside(f64),
    Count,
    Sides,
    ConeAngle,
    ChamferAngle,
    Taper,
}

impl Rule {
    pub fn check(self, value: f64) -> Result<(), String> {
        let refusal = match self {
            Self::Any => None,
            Self::AboveZero => (value <= 0.0).then_some(ABOVE_ZERO),
            Self::ZeroOrMore => (value < 0.0).then_some(ZERO_OR_MORE),
            Self::AboveZeroOrReverse => (value <= 0.0).then_some(ABOVE_ZERO_OR_REVERSE),
            Self::Turn => (value <= 0.0 || value > FULL_TURN_DEGREES).then_some(TURN),
            Self::TurnBeside(other) => {
                (value <= 0.0 || value + other > FULL_TURN_DEGREES).then_some(TURNS_TOGETHER)
            }
            Self::Count if (value - value.round()).abs() > WHOLE_TOLERANCE || value < 1.0 => {
                Some(WHOLE_COUNT)
            }
            Self::Count if value > f64::from(MAX_PATTERN_INSTANCES) => {
                return Err(format!("Enter a count of at most {MAX_PATTERN_INSTANCES}"));
            }
            Self::Count => None,
            Self::Sides
                if (value - value.round()).abs() > WHOLE_TOLERANCE
                    || value < f64::from(MIN_PRISM_SIDES)
                    || value > f64::from(MAX_PRISM_SIDES) =>
            {
                return Err(format!(
                    "Enter a whole number from {MIN_PRISM_SIDES} to {MAX_PRISM_SIDES}"
                ));
            }
            Self::Sides => None,
            Self::ConeAngle if value <= 0.0 || value > MAX_CONE_ANGLE => {
                return Err(format!(
                    "Enter an angle above 0° and up to {MAX_CONE_ANGLE}°"
                ));
            }
            Self::ConeAngle => None,
            Self::ChamferAngle if value <= 0.0 || value >= HALF_TURN_DEGREES => {
                return Err(format!(
                    "Enter an angle above 0° and below {HALF_TURN_DEGREES}°"
                ));
            }
            Self::ChamferAngle => None,
            Self::Taper if value.abs() >= MAX_TAPER_DEGREES => {
                return Err(format!(
                    "Enter an angle between -{MAX_TAPER_DEGREES}° and {MAX_TAPER_DEGREES}°; a \
                     positive taper draws the sides in"
                ));
            }
            Self::Taper => None,
        };
        refusal.map_or(Ok(()), |refusal| Err(refusal.to_owned()))
    }
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

pub fn expression_row_drafting(
    ui: &mut Ui,
    model: &Model,
    caption: &str,
    quantity: Quantity<'_>,
    change: impl Fn(Expression) -> Result<Transaction, String>,
) -> Drafting {
    widgets::caption(ui, caption);
    let document = model.document();
    let parameters = model.parameters();
    let unit = model.units();
    let owner = ParameterOwner::Feature {
        feature: quantity.feature,
        value: caption.to_owned(),
    };
    let parse = |text: &str| {
        let parsed = field::parse_expression(
            document,
            parameters,
            text,
            Expected {
                dimension: Some(quantity.dimension),
                non_negative: false,
            },
            unit,
        )?;
        let value = parameters
            .evaluate_expression(&parsed)
            .map_err(|error| field::sentence(&error.to_string()))?
            .value;
        quantity.rule.check(value)?;
        Ok(parsed)
    };
    let named = field::NamedField {
        document,
        parameters,
        units: unit,
        dimension: Some(quantity.dimension),
        owner: owner.clone(),
        current: quantity.expression,
    };
    let validate = |text: &str| named.transaction(text, parse, &change);
    let (committed, error, draft) = ui
        .horizontal_wrapped(|ui| {
            let field = field::commit_field(
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
            (field.committed, field.error, draft)
        })
        .inner;
    ui.end_row();
    if let Some(error) = error {
        widgets::error_row(ui, &error);
    }
    if ui.memory(|memory| memory.has_focus(quantity.id)) {
        draft_failure_row(ui, model, quantity.feature);
    }
    Drafting { committed, draft }
}

pub fn refusal(feature: &str, reason: &str) -> Action {
    Action::Inform(Notice::error(format!(
        "{feature} was not changed: {reason}"
    )))
}

pub fn applied(feature: &str, change: Result<Transaction, String>) -> Action {
    match change {
        Ok(transaction) => Action::Apply(transaction),
        Err(reason) => refusal(feature, &reason),
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
    match feature_name(document, feature) {
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

pub fn frame_row(
    ui: &mut Ui,
    document: &Document,
    row: &FrameRow<'_>,
    change: impl Fn(Option<FeatureId>) -> Result<Transaction, String>,
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
    let chosen = combo(ui, Id::new((row.salt, row.feature)), current, || {
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

pub struct Choice {
    pub label: String,
    pub selected: bool,
    pub change: Result<Action, String>,
}

pub fn combo(
    ui: &mut Ui,
    id: Id,
    selected: impl Into<WidgetText>,
    choices: impl FnOnce() -> Vec<Choice>,
) -> Option<Action> {
    let mut chosen = None;
    let combo = ComboBox::from_id_salt(id)
        .selected_text(selected)
        .wrap_mode(TextWrapMode::Truncate)
        .show_ui(ui, |ui| {
            for choice in choices() {
                let enabled = choice.change.is_ok() || choice.selected;
                let button = widgets::button(choice.label.clone()).selected(choice.selected);
                let response = ui.add_enabled(
                    enabled,
                    Named::new(button, choice.label.as_str()).selected(choice.selected),
                );
                let response = match &choice.change {
                    Err(reason) if !choice.selected => response.on_disabled_hover_text(reason),
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
    widgets::tie_to_caption(ui, &combo.response);
    chosen
}

pub struct Segment<'a> {
    pub label: &'a str,
    pub hover: &'a str,
    pub change: Option<Result<Transaction, String>>,
}

pub fn segmented_row(
    ui: &mut Ui,
    caption: &str,
    feature: &str,
    segments: Vec<Segment<'_>>,
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
    let changed = ui.checkbox(&mut flipped, label).changed();
    ui.end_row();
    changed.then_some(flipped)
}

pub enum Shown {
    Named(String),
    NoneChosen,
}

pub struct Picker<'a> {
    pub feature: FeatureId,
    pub slot: Slot,
    pub selected: Result<Transaction, String>,
    pub hover: &'a str,
}

#[derive(Clone)]
struct OfferedChange {
    basis: (u64, u64, u64),
    change: Result<Transaction, String>,
}

pub fn offered_change(
    ctx: &egui::Context,
    model: &Model,
    selection: &Selection,
    reference: (FeatureId, Slot),
    compute: impl FnOnce() -> Result<Transaction, String>,
) -> Result<Transaction, String> {
    let id = Id::new(("offered-change", reference.0, reference.1));
    let basis = (
        selection.generation(),
        model.revision(),
        model.evaluation_generation(),
    );
    let known = ctx
        .data(|data| data.get_temp::<OfferedChange>(id))
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

pub fn reference_picker(ui: &mut Ui, model: &Model, picker: Picker<'_>, actions: &mut Vec<Action>) {
    let picking = reference_picking::current(ui.ctx())
        .filter(|picking| picking.is_for(picker.feature, picker.slot));
    if let Some(picking) = picking {
        let prompt = format!("{}.", reference_picking::prompt(model, picking));
        ui.label(widgets::muted(prompt, ui));
        let stop = widgets::small_button(ui, icons::CLOSE, STOP_CHOOSING);
        if ui.add(stop).clicked() {
            actions.push(Action::Editing(EditingCommand::StopPicking));
        }
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
            let button = widgets::small_button(ui, icons::CHOOSE_IN_VIEW, CHOOSE_IN_VIEW);
            let hover = reference_picking::prompt(model, picking);
            if ui.add(button).on_hover_text(hover).clicked() {
                actions.push(Action::Editing(EditingCommand::Pick(picking)));
            }
        }
    }
}

pub fn reference_row(
    ui: &mut Ui,
    model: &Model,
    caption: &str,
    shown: Shown,
    picker: Picker<'_>,
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
        ui.add(Label::new(format!("{DRAFT_FAILS} {}", error.reason)).wrap());
        ui.add(Label::new(widgets::muted(&error.remedy, ui)).wrap());
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
