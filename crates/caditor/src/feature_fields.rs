use caditor_document::{Document, FeatureId, MAX_PATTERN_INSTANCES, Transaction};
use caditor_expression::{Dimension, Expression};
use egui::{ComboBox, Id, Label, RichText, Ui, WidgetText};

use crate::{
    appearance,
    editing::EditingCommand,
    field::{self, Expected},
    icons,
    model::{Action, Model, Notice},
    reference_picking::{self, Picking, Slot},
    widgets::{self, FIELD_WIDTH, Named, Tone},
};

pub const ABOVE_ZERO: &str = "Enter a value above zero";
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
const WHOLE_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    Any,
    AboveZero,
    AboveZeroOrReverse,
    Turn,
    TurnBeside(f64),
    Count,
}

impl Rule {
    pub fn check(self, value: f64) -> Result<(), String> {
        let refusal = match self {
            Self::Any => None,
            Self::AboveZero => (value <= 0.0).then_some(ABOVE_ZERO),
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
        };
        refusal.map_or(Ok(()), |refusal| Err(refusal.to_owned()))
    }
}

pub struct Quantity<'a> {
    pub id: Id,
    pub expression: &'a Expression,
    pub dimension: Dimension,
    pub rule: Rule,
}

pub fn expression_row(
    ui: &mut Ui,
    model: &Model,
    caption: &str,
    quantity: Quantity<'_>,
    change: impl FnOnce(Expression) -> Result<Transaction, String>,
) -> Option<Transaction> {
    widgets::caption(ui, caption);
    let document = model.document();
    let parameters = model.parameters();
    let unit = model.units();
    let (committed, error) = ui
        .horizontal(|ui| {
            let field = field::commit_field(
                ui,
                quantity.id,
                &document.expression_text(quantity.expression),
                FIELD_WIDTH,
                false,
                |text| {
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
                    change(parsed)
                },
            );
            if field.error.is_none()
                && let Some(preview) = field::value_preview(parameters, quantity.expression, unit)
            {
                ui.label(widgets::muted(preview, ui));
            }
            (field.committed, field.error)
        })
        .inner;
    ui.end_row();
    if let Some(error) = error {
        widgets::error_row(ui, &error);
    }
    committed
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

pub fn info_callout(ui: &mut Ui, text: &str) {
    widgets::callout(ui, Tone::Info, |ui| {
        ui.add(Label::new(text).wrap());
    });
}

pub fn choosing_list(ui: &Ui, opened: bool) -> bool {
    opened && reference_picking::current(ui.ctx()).is_none()
}
