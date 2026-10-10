use std::sync::Arc;

use caditor_document::{DependencyGraph, Document, ParameterValues};
use caditor_expression::{Expression, ParameterId};
use egui::{
    Area, Context, CornerRadius, Frame, Id, Key, Order, Rect, Response, Sense, Stroke, Ui, Vec2,
    accesskit::{HasPopup, Role},
    text::{CCursor, CCursorRange},
    vec2,
};

use crate::{
    appearance::{self, FOCUS_WIDTH, SPACE_M, SPACE_XS, WIDGET_RADIUS},
    model::Model,
    units::Units,
};

pub const MAX_SHOWN: usize = 8;
pub const LIST_NAME: &str = "Parameter suggestions";

const COMPLETIONS_KEY: &str = "completion-candidates";
const OFFER_KEY: &str = "completion-offer";
const TAKEN_KEY: &str = "completion-taken";
const TARGET_KEY: &str = "completion-target";
const ROW_PADDING: Vec2 = vec2(SPACE_M, SPACE_XS);
const COLUMN_GAP: f32 = SPACE_M * 2.0;
const OFFER_LIFE: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: ParameterId,
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone)]
pub struct Completions {
    candidates: Arc<[Candidate]>,
    graph: Arc<DependencyGraph>,
}

#[derive(Clone)]
struct Built {
    revision: u64,
    evaluation: u64,
    units: Units,
    completions: Completions,
}

impl Completions {
    pub fn of(document: &Document, parameters: &ParameterValues, units: Units) -> Self {
        let mut candidates: Vec<Candidate> = document
            .parameters()
            .iter()
            .map(|parameter| Candidate {
                id: parameter.id(),
                name: parameter.name.clone(),
                value: match parameters.get(parameter.id()) {
                    Some(Ok(value)) => Some(units.show(*value)),
                    _ => None,
                },
            })
            .collect();
        candidates.sort_by_cached_key(|candidate| candidate.name.to_lowercase());
        Self {
            candidates: candidates.into(),
            graph: Arc::new(DependencyGraph::of(document)),
        }
    }

    pub fn forms_cycle(&self, target: ParameterId, candidate: ParameterId) -> bool {
        candidate == target
            || self
                .graph
                .cycle(target, &Expression::Parameter(candidate))
                .is_some()
    }

    pub fn matching(
        &self,
        typed: &str,
        editing: Option<ParameterId>,
        limit: usize,
    ) -> Vec<Candidate> {
        let wanted = typed.to_lowercase();
        let usable = |candidate: &&Candidate| {
            candidate.name != typed
                && editing.is_none_or(|target| !self.forms_cycle(target, candidate.id))
        };
        let starting = self
            .candidates
            .iter()
            .filter(|candidate| candidate.name.to_lowercase().starts_with(&wanted));
        let containing = self.candidates.iter().filter(|candidate| {
            let name = candidate.name.to_lowercase();
            !name.starts_with(&wanted) && name.contains(&wanted)
        });
        starting
            .chain(containing)
            .filter(usable)
            .take(limit)
            .cloned()
            .collect()
    }
}

pub fn publish(ctx: &Context, model: &Model) {
    if ctx.memory(|memory| memory.focused()).is_none() {
        return;
    }
    let key = Id::new(COMPLETIONS_KEY);
    let revision = model.revision();
    let evaluation = model.evaluation_generation();
    let units = model.units();
    let fresh = ctx.data(|data| {
        data.get_temp::<Built>(key).is_some_and(|built| {
            built.revision == revision && built.evaluation == evaluation && built.units == units
        })
    });
    if fresh {
        return;
    }
    let completions = Completions::of(model.document(), model.parameters(), units);
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            Built {
                revision,
                evaluation,
                units,
                completions,
            },
        );
    });
}

pub fn available(ctx: &Context) -> Option<Completions> {
    ctx.data(|data| {
        data.get_temp::<Built>(Id::new(COMPLETIONS_KEY))
            .map(|built| built.completions)
    })
}

#[derive(Clone, Copy, Default)]
struct Target(Option<ParameterId>);

pub fn editing_parameter(ui: &Ui, field: Id, parameter: ParameterId) {
    ui.data_mut(|data| data.insert_temp(field.with(TARGET_KEY), Target(Some(parameter))));
}

pub fn take_editing(ui: &Ui, field: Id) -> Option<ParameterId> {
    ui.data_mut(|data| data.remove_temp::<Target>(field.with(TARGET_KEY)))
        .and_then(|target| target.0)
}

#[derive(Clone)]
struct Offer {
    text: String,
    pass: u64,
}

pub fn offer_insertion(ctx: &Context, text: String) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(Id::new(OFFER_KEY), Offer { text, pass }));
}

pub fn offered(ctx: &Context) -> Option<String> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<Offer>(Id::new(OFFER_KEY)))
        .filter(|offer| offer.pass + OFFER_LIFE > pass)
        .map(|offer| offer.text)
}

#[derive(Clone, Copy, Default)]
struct Taken;

pub fn mark_taken(ctx: &Context) {
    ctx.data_mut(|data| data.insert_temp(Id::new(TAKEN_KEY), Taken));
}

pub fn take_click(ctx: &Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<Taken>(Id::new(TAKEN_KEY)))
        .is_some()
}

pub fn forget_taken_click(ctx: &Context) {
    if ctx.input(|input| input.pointer.any_released()) {
        ctx.data_mut(|data| data.remove::<Taken>(Id::new(TAKEN_KEY)));
    }
}

pub fn dimension_text(document: &Document, expression: &Expression) -> String {
    let text = document.expression_text(expression);
    match expression {
        Expression::Parameter(_)
        | Expression::Number(_)
        | Expression::Measure(..)
        | Expression::Constant(_)
        | Expression::Call(..) => text,
        Expression::WithUnit(inner, ..) if matches!(**inner, Expression::Number(_)) => text,
        _ => format!("({text})"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

pub fn token_at(text: &str, caret: usize) -> Option<Token> {
    let characters: Vec<char> = text.chars().collect();
    let caret = caret.min(characters.len());
    if characters
        .get(caret)
        .copied()
        .is_some_and(is_name_character)
    {
        return None;
    }
    let start = characters
        .get(..caret)?
        .iter()
        .rposition(|character| !is_name_character(*character))
        .map_or(0, |at| at + 1);
    let token: String = characters.get(start..caret)?.iter().collect();
    let first = token.chars().next()?;
    if first.is_numeric() {
        return None;
    }
    let before: String = characters.get(..start)?.iter().collect();
    let after: String = characters.get(caret..)?.iter().collect();
    let names_a_value = before.trim().is_empty()
        && after.trim_start().starts_with('=')
        && !after.trim_start().starts_with("==");
    if names_a_value {
        return None;
    }
    Some(Token {
        start,
        end: caret,
        text: token,
    })
}

pub fn replaced(text: &str, start: usize, end: usize, with: &str) -> (String, usize) {
    let characters: Vec<char> = text.chars().collect();
    let end = end.min(characters.len());
    let start = start.min(end);
    let mut result: String = characters.iter().take(start).collect();
    result.push_str(with);
    let caret = result.chars().count();
    result.extend(characters.iter().skip(end));
    (result, caret)
}

pub fn inserted(text: &str, start: usize, end: usize, with: &str) -> (String, usize) {
    let characters: Vec<char> = text.chars().collect();
    let before = start
        .checked_sub(1)
        .and_then(|at| characters.get(at))
        .copied();
    let after = characters.get(end).copied();
    let lead = before.is_some_and(|c| is_name_character(c) || c == '.' || c == ')');
    let trail = after.is_some_and(|c| is_name_character(c) || c == '(');
    let spaced = format!(
        "{}{with}{}",
        if lead { " " } else { "" },
        if trail { " " } else { "" }
    );
    let (result, caret) = replaced(text, start, end, &spaced);
    (result, if trail { caret - 1 } else { caret })
}

pub fn selection_of(ctx: &Context, id: Id, text: &str) -> (usize, usize) {
    let length = text.chars().count();
    egui::TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map_or((length, length), |range| {
            let primary: usize = range.primary.index.into();
            let secondary: usize = range.secondary.index.into();
            let first = primary.min(secondary);
            let last = primary.max(secondary);
            (first.min(length), last.min(length))
        })
}

pub fn place_caret(ctx: &Context, id: Id, caret: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    egui::TextEdit::store_state(ctx, id, state);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKey {
    Previous,
    Next,
    Accept,
    Enter,
    Close,
}

pub fn take_key(ui: &Ui, navigated: bool) -> Option<PopupKey> {
    ui.input_mut(|input| {
        let mut taken = None;
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            if modifiers.command || modifiers.alt {
                return true;
            }
            let pressed = match key {
                Key::ArrowUp => PopupKey::Previous,
                Key::ArrowDown => PopupKey::Next,
                Key::Tab if !modifiers.shift => PopupKey::Accept,
                Key::Enter if navigated => PopupKey::Enter,
                Key::Escape => PopupKey::Close,
                _ => return true,
            };
            taken = Some(pressed);
            false
        });
        taken
    })
}

pub fn filter(open: bool) -> egui::EventFilter {
    egui::EventFilter {
        horizontal_arrows: true,
        vertical_arrows: true,
        tab: open,
        escape: open,
    }
}

pub fn row_id(field: Id, index: usize) -> Id {
    field.with(("suggestion", index))
}

pub fn list(ui: &Ui, field: &Response, shown: &[Candidate], selected: usize) -> Vec<Rect> {
    let tokens = appearance::tokens(ui);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let names: Vec<_> = shown
        .iter()
        .map(|candidate| {
            ui.painter()
                .layout_no_wrap(candidate.name.clone(), font.clone(), tokens.text)
        })
        .collect();
    let values: Vec<_> = shown
        .iter()
        .map(|candidate| {
            ui.painter().layout_no_wrap(
                candidate.value.clone().unwrap_or_default(),
                font.clone(),
                tokens.text_muted,
            )
        })
        .collect();
    let name_width = names
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0, f32::max);
    let value_width = values
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0, f32::max);
    let row_height = names
        .iter()
        .chain(&values)
        .map(|galley| galley.size().y)
        .fold(0.0, f32::max)
        + 2.0 * ROW_PADDING.y;
    let width =
        (name_width + COLUMN_GAP + value_width + 2.0 * ROW_PADDING.x).max(field.rect.width());
    let mut rows = Vec::new();
    Area::new(field.id.with("suggestions"))
        .order(Order::Foreground)
        .fixed_pos(field.rect.left_bottom() + vec2(0.0, SPACE_XS))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            Frame::menu(ui.style()).show(ui, |ui| {
                ui.scope(|ui| {
                    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                        node.set_role(Role::ListBox);
                        node.set_label(LIST_NAME);
                    });
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, candidate) in shown.iter().enumerate() {
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                        let id = row_id(field.id, index);
                        let response = ui.interact(rect, id, Sense::hover());
                        let chosen = index == selected;
                        ui.ctx().accesskit_node_builder(id, |node| {
                            node.set_role(Role::ListBoxOption);
                            node.set_label(match &candidate.value {
                                Some(value) => format!("{}, {value}", candidate.name),
                                None => candidate.name.clone(),
                            });
                            node.set_bounds(egui::accesskit::Rect {
                                x0: rect.min.x.into(),
                                y0: rect.min.y.into(),
                                x1: rect.max.x.into(),
                                y1: rect.max.y.into(),
                            });
                            node.set_selected(chosen);
                            node.set_position_in_set(index + 1);
                            node.set_size_of_set(shown.len());
                        });
                        let fill = if chosen {
                            tokens.accent_subtle
                        } else if response.hovered() {
                            tokens.hover
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        let painter = ui.painter();
                        painter.rect_filled(rect, CornerRadius::same(WIDGET_RADIUS), fill);
                        if chosen {
                            painter.rect_stroke(
                                rect,
                                CornerRadius::same(WIDGET_RADIUS),
                                Stroke::new(FOCUS_WIDTH, tokens.focus),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let at = rect.left_center() + vec2(ROW_PADDING.x, 0.0);
                        if let Some(name) = names.get(index) {
                            painter.galley(
                                at - vec2(0.0, name.size().y / 2.0),
                                name.clone(),
                                tokens.text,
                            );
                        }
                        if let Some(value) = values.get(index) {
                            let colour = if chosen || response.hovered() {
                                tokens.text
                            } else {
                                tokens.text_muted
                            };
                            painter.galley(
                                egui::pos2(
                                    rect.right() - ROW_PADDING.x - value.size().x,
                                    rect.center().y - value.size().y / 2.0,
                                ),
                                value.clone(),
                                colour,
                            );
                        }
                        rows.push(rect);
                    }
                });
            });
        });
    ui.ctx().accesskit_node_builder(field.id, |node| {
        node.set_has_popup(HasPopup::Listbox);
        node.set_expanded(true);
        node.set_active_descendant(row_id(field.id, selected).accesskit_id());
    });
    rows
}
