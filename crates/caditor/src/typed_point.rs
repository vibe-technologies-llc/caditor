use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::MAX_LENGTH;
use egui::{
    Align, Align2, Area, Event, Frame, Id, Key, Label, Margin, Order, Pos2, Rect, RichText, Sense,
    TextEdit,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

use crate::{
    appearance, canvas,
    field::{self, Expected},
    icons,
    model::Model,
    sketch_drag::Transform,
    widgets,
};

pub const FIELD_LABEL: &str = "Place point";
pub const MOVE_LABEL: &str = "Move to";
pub const POINT_PLACEHOLDER: &str = "x, y  or  length < angle";
pub const WAITING_HINT: &str = "Still waiting: click the field or type to carry on   Esc: clear it";
const FIELD_WIDTH: f32 = 180.0;
const FRAME_MARGIN: Margin = Margin::symmetric(8, 6);
const HINT_GAP: f32 = 6.0;
const RELATIVE_MARK: char = '@';
const VALUE_MARK: char = '=';
const SEPARATORS: [char; 2] = [',', ';'];
const POLAR_MARK: char = '<';
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};
const ANGLE: Expected = Expected {
    dimension: Some(Dimension::ANGLE),
    non_negative: false,
};
const SIDES_WORDS: [&str; 2] = ["sides", "side"];
const RHO_WORD: &str = "rho";
const RHO: Part = Part {
    label: "rho",
    noun: "rho",
    expected: Expected {
        dimension: Some(Dimension::NONE),
        non_negative: true,
    },
};
const FULL_TURN_DEGREES: f64 = 360.0;
const FORMS: &str = "Type x, y such as 10, 20, or a length and an angle such as 25 < 30";
const LENGTH_LOCK_NEEDS_A_POINT: &str = "A length held with < sets the distance from the last placed point: place a point first, \
     or type length < angle";
const LENGTH_LOCK_ABOVE_ZERO: &str = "A held length must be above zero";
const HEADING_NEEDS_A_POINT: &str = "An angle alone sets the direction from the last placed point: place a point first, or type \
     length < angle";

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct From {
    pub last: Option<Point2>,
    pub toward: Option<Point2>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypedPoint {
    text: Option<String>,
    error: Option<String>,
    focus_pending: bool,
    waiting: bool,
}

pub struct Typed {
    pub text: String,
    pub entered: String,
}

impl Typed {
    fn of(entered: &str) -> Self {
        let entered = entered.trim().to_owned();
        Self {
            text: value_text(&entered).to_owned(),
            entered,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub degrees: f64,
    pub dimension: Option<TypedDimension>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LengthLock {
    pub length: f64,
    pub dimension: Option<TypedDimension>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measured {
    Across,
    Up,
    Length,
    Angle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedDimension {
    pub measured: Measured,
    pub from: Option<Point2>,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub position: Point2,
    pub dimensions: Vec<TypedDimension>,
}

struct TypedValue {
    measured: Measured,
    expression: Expression,
    value: f64,
}

impl TypedValue {
    fn dimension(self, from: Option<Point2>) -> Option<TypedDimension> {
        let value = match self.measured {
            Measured::Angle if self.value > 0.0 && self.value < FULL_TURN_DEGREES => {
                self.expression
            }
            Measured::Angle => return None,
            Measured::Across | Measured::Up | Measured::Length if self.value == 0.0 => {
                return None;
            }
            Measured::Across | Measured::Up | Measured::Length => {
                magnitude(self.expression, self.value)
            }
        };
        Some(TypedDimension {
            measured: self.measured,
            from,
            value,
        })
    }
}

fn magnitude(expression: Expression, value: f64) -> Expression {
    if value >= 0.0 {
        return expression;
    }
    match expression {
        Expression::Negate(inner) => *inner,
        other => Expression::Negate(Box::new(other)),
    }
}

pub fn sides(text: &str) -> Option<usize> {
    let text = text.trim().to_lowercase();
    let count = SIDES_WORDS
        .iter()
        .find_map(|word| text.strip_suffix(word))?
        .trim_end();
    let digits = !count.is_empty() && count.chars().all(|digit| digit.is_ascii_digit());
    digits.then(|| count.parse().unwrap_or(usize::MAX))
}

pub fn rho(model: &Model, text: &str) -> Option<Result<(f64, Expression), String>> {
    let trimmed = text.trim();
    let split = trimmed.len().checked_sub(RHO_WORD.len())?;
    let (value, word) = (trimmed.get(..split)?, trimmed.get(split..)?);
    if !word.eq_ignore_ascii_case(RHO_WORD) {
        return None;
    }
    Some(
        typed_value(model, value, RHO, Measured::Angle)
            .map(|typed| (typed.value, typed.expression)),
    )
}

fn starts_a_point(text: &str) -> bool {
    text.chars().next().is_some_and(|first| {
        first.is_ascii_digit()
            || matches!(
                first,
                '-' | '.' | '(' | RELATIVE_MARK | VALUE_MARK | POLAR_MARK
            )
    })
}

pub fn value_text(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix(VALUE_MARK).map_or(text, str::trim_start)
}

fn resumed(kept: &str, typed: &str) -> String {
    let typed = match kept.is_empty() {
        true => typed,
        false => typed.strip_prefix(VALUE_MARK).unwrap_or(typed),
    };
    format!("{kept}{typed}")
}

fn split_top_level(text: &str, separates: impl Fn(char, Option<char>) -> bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        let next = characters.peek().map(|(_, next)| *next);
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            separator if depth == 0 && separates(separator, next) => {
                parts.push(text.get(start..index).unwrap_or_default());
                start = index + separator.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(text.get(start..).unwrap_or_default());
    parts
}

pub(crate) fn coordinates(text: &str) -> Vec<&str> {
    split_top_level(text, |character, _| SEPARATORS.contains(&character))
}

pub(crate) fn polar(text: &str) -> Vec<&str> {
    split_top_level(text, |character, next| {
        character == POLAR_MARK && next != Some('=')
    })
}

impl TypedPoint {
    pub fn is_open(&self) -> bool {
        self.text.is_some()
    }

    pub fn is_waiting(&self) -> bool {
        self.is_open() && self.waiting
    }

    pub fn is_typing(&self) -> bool {
        self.is_open() && !self.waiting
    }

    pub fn open_or_resume(&mut self) {
        if self.is_waiting() {
            self.waiting = false;
            self.focus_pending = true;
        } else {
            self.open();
        }
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn open(&mut self) {
        self.text = Some(String::new());
        self.error = None;
        self.focus_pending = true;
        self.waiting = false;
    }

    pub fn open_with(&mut self, text: String, error: String) {
        self.text = Some(text);
        self.error = Some(error);
        self.focus_pending = true;
        self.waiting = false;
    }

    pub fn typing_text(&self) -> Option<&str> {
        self.text
            .as_deref()
            .filter(|_| !self.waiting)
            .map(value_text)
    }

    pub fn open_from_typing(&mut self, ctx: &egui::Context) {
        if self.is_typing() {
            return;
        }
        let typed = ctx.input_mut(|input| {
            let first = input.events.iter().find_map(|event| match event {
                Event::Text(text) => Some(text.clone()),
                _ => None,
            })?;
            if !starts_a_point(&first) {
                return None;
            }
            let mut text = String::new();
            input.events.retain(|event| match event {
                Event::Text(typed) => {
                    text.push_str(typed);
                    false
                }
                _ => true,
            });
            Some(text)
        });
        if let Some(typed) = typed {
            let kept = self.text.take().unwrap_or_default();
            self.text = Some(resumed(&kept, &typed));
            self.error = None;
            self.focus_pending = true;
            self.waiting = false;
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        bounds: Rect,
        anchor: Pos2,
        label: &str,
        hint: &str,
        placeholder: &str,
    ) -> Option<Typed> {
        let text = self.text.as_mut()?;
        let id = Id::new("typed-point");
        if self.focus_pending {
            let mut state = TextEditState::load(ctx, id).unwrap_or_default();
            let end = CCursor::new(text.chars().count());
            state.cursor.set_char_range(Some(CCursorRange::one(end)));
            state.store(ctx, id);
        }
        let error = self.error.clone();
        let waiting = self.waiting && !self.focus_pending;
        let (label_color, hint) = match waiting {
            true => (canvas::chrome(ctx).muted, WAITING_HINT),
            false => (canvas::chrome(ctx).text, hint),
        };
        let inner_width = (bounds.width() - FRAME_MARGIN.sum().x).max(FIELD_WIDTH);
        let response = Area::new(id.with("area"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(anchor)
            .constrain_to(bounds)
            .show(ctx, |ui| {
                canvas_frame(ctx).show(ui, |ui| {
                    ui.set_max_width(inner_width);
                    let field = ui
                        .horizontal(|ui| {
                            canvas_text(ui, label, label_color);
                            widgets::text_field(ui, |ui| {
                                let field = TextEdit::singleline(text)
                                    .id(id)
                                    .desired_width(FIELD_WIDTH)
                                    .hint_text(placeholder);
                                ui.add(match waiting {
                                    true => field.text_color(appearance::tokens(ui).text_muted),
                                    false => field,
                                })
                            })
                        })
                        .inner;
                    ui.add_space(HINT_GAP);
                    let hints = canvas::Hints::bare(ui.painter(), hint, inner_width, Align::Min);
                    let (rect, _) = ui.allocate_exact_size(hints.size(), Sense::hover());
                    hints.paint(ui.painter(), rect.min);
                    field
                })
            });
        if let Some(error) = &error {
            Area::new(id.with("error"))
                .order(Order::Foreground)
                .pivot(Align2::CENTER_TOP)
                .fixed_pos(response.response.rect.center_bottom())
                .constrain_to(bounds)
                .show(ctx, |ui| {
                    canvas_frame(ctx).show(ui, |ui| {
                        ui.set_max_width(inner_width);
                        ui.horizontal(|ui| {
                            let failed = canvas::chrome(ui.ctx()).error;
                            widgets::icon_label(ui, icons::FAILED, failed);
                            canvas_text(ui, error, failed);
                        });
                    });
                });
        }
        let field = response.inner.inner;
        if self.focus_pending {
            field.request_focus();
            self.focus_pending = false;
        }
        if field.changed() {
            self.error = None;
        }
        if field.has_focus() {
            self.waiting = false;
        }
        if !field.lost_focus() {
            return None;
        }
        let (entered, escaped) = ctx.input(|input| {
            (
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Escape),
            )
        });
        if escaped {
            self.close();
            return None;
        }
        if entered {
            let typed = Typed::of(text);
            self.close();
            return Some(typed);
        }
        self.waiting = true;
        None
    }
}

fn canvas_frame(ctx: &egui::Context) -> Frame {
    Frame::new()
        .fill(canvas::chrome(ctx).panel)
        .corner_radius(canvas::RADIUS)
        .inner_margin(FRAME_MARGIN)
}

fn canvas_text(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add(Label::new(RichText::new(text).font(canvas::body()).color(color)).selectable(false));
}

struct Part {
    label: &'static str,
    noun: &'static str,
    expected: Expected,
}

const X: Part = Part {
    label: "x",
    noun: "x coordinate",
    expected: LENGTH,
};
const Y: Part = Part {
    label: "y",
    noun: "y coordinate",
    expected: LENGTH,
};
const DISTANCE: Part = Part {
    label: "length",
    noun: "length",
    expected: LENGTH,
};
const DIRECTION: Part = Part {
    label: "angle",
    noun: "angle",
    expected: ANGLE,
};

fn typed_value(
    model: &Model,
    text: &str,
    part: Part,
    measured: Measured,
) -> Result<TypedValue, String> {
    let Part {
        label,
        noun,
        expected,
    } = part;
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("The {noun} is missing"));
    }
    let expression = field::parse_expression(
        model.document(),
        model.parameters(),
        text,
        expected,
        model.units(),
    )
    .map_err(|error| format!("{label}: {error}"))?;
    let dimension = expected.dimension.unwrap_or(Dimension::NONE);
    let value = expression
        .evaluate_as(dimension, &|id| model.parameters().value(id))
        .map_err(|error| format!("{label}: {}", field::sentence(&error.to_string())))?;
    Ok(TypedValue {
        measured,
        expression,
        value,
    })
}

fn offset(model: &Model, text: &str, from: From) -> Result<(Vector2, Vec<TypedValue>), String> {
    let coordinates = coordinates(text);
    let polar = polar(text);
    match (coordinates.as_slice(), polar.as_slice()) {
        ([x, y], [_]) => {
            let x = typed_value(model, x, X, Measured::Across)?;
            let y = typed_value(model, y, Y, Measured::Up)?;
            Ok((Vector2::new(x.value, y.value), vec![x, y]))
        }
        ([_], [length, angle]) => {
            let length = typed_value(model, length, DISTANCE, Measured::Length)?;
            let angle = typed_value(model, angle, DIRECTION, Measured::Angle)?;
            let offset = Vector2::from_angle(angle.value.to_radians()) * length.value;
            let typed = if length.value < 0.0 {
                vec![length]
            } else {
                vec![length, angle]
            };
            Ok((offset, typed))
        }
        ([length], [_]) => {
            let length = typed_value(model, length, DISTANCE, Measured::Length)?;
            let Some(last) = from.last else {
                return Err(format!(
                    "{FORMS}; a length alone needs a placed point to measure from"
                ));
            };
            let direction = from
                .toward
                .and_then(|toward| (toward - last).try_normalize())
                .ok_or_else(|| {
                    "Point the way the length should go, or type length < angle".to_owned()
                })?;
            Ok((direction * length.value, vec![length]))
        }
        _ => Err(FORMS.to_owned()),
    }
}

pub struct TransformField {
    pub label: &'static str,
    pub hint: &'static str,
    pub placeholder: &'static str,
}

pub const ROTATE_FIELD: TransformField = TransformField {
    label: "Rotate by",
    hint: "Degrees unless a unit is named, counter-clockwise   Enter: rotate   Esc: cancel",
    placeholder: "angle",
};
pub const SCALE_FIELD: TransformField = TransformField {
    label: "Scale by",
    hint: "A factor above zero, such as 2 or 0.5   Enter: scale   Esc: cancel",
    placeholder: "factor",
};
const FACTOR: Expected = Expected {
    dimension: Some(Dimension::NONE),
    non_negative: false,
};

pub fn transform_field(transform: Transform) -> TransformField {
    match transform {
        Transform::Rotate => ROTATE_FIELD,
        Transform::Scale => SCALE_FIELD,
    }
}

pub fn parse_transform(model: &Model, text: &str, transform: Transform) -> Result<f64, String> {
    let part = match transform {
        Transform::Rotate => DIRECTION,
        Transform::Scale => Part {
            label: "factor",
            noun: "factor",
            expected: FACTOR,
        },
    };
    typed_value(model, text, part, Measured::Angle).map(|typed| typed.value)
}

pub fn parse(model: &Model, text: &str, from: From) -> Result<Point2, String> {
    parse_placed(model, text, from).map(|placed| placed.position)
}

pub fn heading(model: &Model, text: &str, from: From) -> Option<Result<Heading, String>> {
    let text = text.strip_prefix(RELATIVE_MARK).unwrap_or(text);
    let [length, angle] = polar(text)[..] else {
        return None;
    };
    if !length.trim().is_empty() || coordinates(text).len() != 1 {
        return None;
    }
    Some(
        typed_value(model, angle, DIRECTION, Measured::Angle).and_then(|angle| {
            let last = from.last.ok_or_else(|| HEADING_NEEDS_A_POINT.to_owned())?;
            Ok(Heading {
                degrees: angle.value,
                dimension: angle.dimension(Some(last)),
            })
        }),
    )
}

pub fn length_lock(model: &Model, text: &str, from: From) -> Option<Result<LengthLock, String>> {
    let text = text.strip_prefix(RELATIVE_MARK).unwrap_or(text);
    let [length, angle] = polar(text)[..] else {
        return None;
    };
    if length.trim().is_empty() || !angle.trim().is_empty() || coordinates(text).len() != 1 {
        return None;
    }
    Some(
        typed_value(model, length, DISTANCE, Measured::Length).and_then(|length| {
            let last = from
                .last
                .ok_or_else(|| LENGTH_LOCK_NEEDS_A_POINT.to_owned())?;
            if length.value <= 0.0 {
                return Err(LENGTH_LOCK_ABOVE_ZERO.to_owned());
            }
            if length.value > MAX_LENGTH {
                return Err(format!("Keep the length within {} m", MAX_LENGTH / 1_000.0));
            }
            Ok(LengthLock {
                length: length.value,
                dimension: length.dimension(Some(last)),
            })
        }),
    )
}

pub fn parse_placed(model: &Model, text: &str, from: From) -> Result<Placed, String> {
    let (relative, text) = match text.strip_prefix(RELATIVE_MARK) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let alone = coordinates(text).len() == 1 && polar(text).len() == 1;
    let (offset, typed) = offset(model, text, from)?;
    let measured_from = if relative || alone { from.last } else { None };
    let point = measured_from.unwrap_or(Point2::ZERO) + offset;
    if point.abs().max_element() > MAX_LENGTH {
        return Err(format!(
            "Keep the point within {} m of the sketch's origin",
            MAX_LENGTH / 1_000.0
        ));
    }
    Ok(Placed {
        position: point,
        dimensions: typed
            .into_iter()
            .filter_map(|typed| typed.dimension(measured_from))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use caditor_document::Document;
    use caditor_file::StorageConfig;

    use super::*;
    use crate::model::Services;

    fn empty_model() -> Model {
        Model::new(
            Document::default(),
            Services {
                make_waker: Box::new(|| Box::new(|| {})),
                storage: StorageConfig::default(),
                panic_flush: Arc::default(),
            },
        )
    }

    #[test]
    fn a_point_starts_with_a_digit_a_sign_a_decimal_point_or_the_relative_mark() {
        for text in ["1", "-2", ".5", "@3, 4", "(1 + 2), 3"] {
            assert!(starts_a_point(text), "{text}");
        }
        for text in ["l", " ", "", "x"] {
            assert!(!starts_a_point(text), "{text}");
        }
    }

    #[test]
    fn an_equals_sign_opens_the_field_and_is_left_out_of_the_value() {
        for text in ["=width, 20", "=t", "<30", "@<30"] {
            assert!(starts_a_point(text), "{text}");
        }
        assert_eq!(value_text("=width, 20"), "width, 20");
        assert_eq!(value_text(" = t "), "t");
        assert_eq!(value_text("10, 20"), "10, 20");
        assert_eq!(value_text("a = b"), "a = b");
    }

    #[test]
    fn typing_into_a_waiting_field_carries_on_from_the_kept_text() {
        assert_eq!(resumed("10, 2", "0"), "10, 20");
        assert_eq!(resumed("10, ", "=w"), "10, w");
        assert_eq!(resumed("", "=w"), "=w");
    }

    #[test]
    fn an_angle_alone_is_a_heading_from_the_last_point_and_nothing_else_is() {
        let model = empty_model();
        let last = Point2::new(5.0, 5.0);
        let from = From {
            last: Some(last),
            toward: None,
        };

        let absolute = heading(&model, "<30", from).and_then(Result::ok);
        let relative = heading(&model, "@ < 30", from).and_then(Result::ok);
        let without_point = heading(&model, "<30", From::default());

        assert_eq!(absolute.as_ref().map(|found| found.degrees), Some(30.0));
        assert_eq!(
            absolute
                .and_then(|heading| heading.dimension)
                .map(|dimension| (dimension.measured, dimension.from)),
            Some((Measured::Angle, Some(last)))
        );
        assert_eq!(relative.map(|found| found.degrees), Some(30.0));
        assert_eq!(without_point, Some(Err(HEADING_NEEDS_A_POINT.to_owned())));
        for text in ["10 < 30", "10, 20", "5", "1, 2 < 3"] {
            assert!(heading(&model, text, from).is_none(), "{text}");
        }
    }

    #[test]
    fn a_length_followed_by_a_less_than_sign_is_a_held_length_and_nothing_else_is() {
        let model = empty_model();
        let last = Point2::new(5.0, 5.0);
        let from = From {
            last: Some(last),
            toward: None,
        };

        let held = length_lock(&model, "12 <", from).and_then(Result::ok);
        let relative = length_lock(&model, "@12<", from).and_then(Result::ok);
        let without_point = length_lock(&model, "12<", From::default());
        let zero = length_lock(&model, "0<", from);

        assert_eq!(held.as_ref().map(|found| found.length), Some(12.0));
        assert_eq!(
            held.and_then(|lock| lock.dimension)
                .map(|dimension| (dimension.measured, dimension.from)),
            Some((Measured::Length, Some(last)))
        );
        assert_eq!(relative.map(|found| found.length), Some(12.0));
        assert_eq!(
            without_point,
            Some(Err(LENGTH_LOCK_NEEDS_A_POINT.to_owned()))
        );
        assert_eq!(zero, Some(Err(LENGTH_LOCK_ABOVE_ZERO.to_owned())));
        for text in ["<30", "10 < 30", "10, 20", "5", "1, 2 <"] {
            assert!(length_lock(&model, text, from).is_none(), "{text}");
        }
    }

    #[test]
    fn a_count_followed_by_sides_is_a_side_count_and_nothing_else_is() {
        assert_eq!(sides("6 sides"), Some(6));
        assert_eq!(sides(" 12Sides "), Some(12));
        assert_eq!(sides("1 side"), Some(1));
        assert_eq!(sides("99999999999999999999999 sides"), Some(usize::MAX));
        for text in ["6", "sides", "-6 sides", "6.5 sides", "2 + 4 sides", "6, 5"] {
            assert_eq!(sides(text), None, "{text}");
        }
    }

    #[test]
    fn only_top_level_commas_separate_the_coordinates() {
        assert_eq!(coordinates("max(w, 10), 5"), ["max(w, 10)", " 5"]);
        assert_eq!(coordinates("1; 2"), ["1", " 2"]);
        assert_eq!(coordinates("min(a, b)"), ["min(a, b)"]);
        assert_eq!(coordinates("1, 2, 3").len(), 3);
    }

    #[test]
    fn only_a_top_level_less_than_sign_separates_a_length_from_its_angle() {
        assert_eq!(polar("25 < 30"), ["25 ", " 30"]);
        assert_eq!(polar("if(a < b, 1, 2) < 45"), ["if(a < b, 1, 2) ", " 45"]);
        assert_eq!(polar("a <= b"), ["a <= b"]);
        assert_eq!(polar("10, 20"), ["10, 20"]);
    }
}
