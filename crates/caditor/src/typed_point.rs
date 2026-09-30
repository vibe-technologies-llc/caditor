use caditor_expression::Dimension;
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::MAX_LENGTH;
use egui::{
    Align2, Area, Event, Frame, Id, Key, Order, Pos2, RichText, TextEdit,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};

use crate::{
    field::{self, Expected},
    icons,
    model::Model,
    widgets,
};

pub const FIELD_LABEL: &str = "Place point";
pub const MOVE_LABEL: &str = "Move to";
pub const POINT_PLACEHOLDER: &str = "x, y  or  length < angle";
const FIELD_WIDTH: f32 = 180.0;
const RELATIVE_MARK: char = '@';
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
const FORMS: &str = "Type x, y such as 10, 20, or a length and an angle such as 25 < 30";

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
}

pub struct Typed {
    pub text: String,
}

fn starts_a_point(text: &str) -> bool {
    text.chars().next().is_some_and(|first| {
        first.is_ascii_digit() || matches!(first, '-' | '.' | '(' | RELATIVE_MARK)
    })
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

fn coordinates(text: &str) -> Vec<&str> {
    split_top_level(text, |character, _| SEPARATORS.contains(&character))
}

fn polar(text: &str) -> Vec<&str> {
    split_top_level(text, |character, next| {
        character == POLAR_MARK && next != Some('=')
    })
}

impl TypedPoint {
    pub fn is_open(&self) -> bool {
        self.text.is_some()
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn open(&mut self) {
        self.text = Some(String::new());
        self.error = None;
        self.focus_pending = true;
    }

    pub fn open_with(&mut self, text: String, error: String) {
        self.text = Some(text);
        self.error = Some(error);
        self.focus_pending = true;
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn open_from_typing(&mut self, ctx: &egui::Context) {
        if self.is_open() {
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
        if let Some(text) = typed {
            self.text = Some(text);
            self.error = None;
            self.focus_pending = true;
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
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
        let response = Area::new(id.with("area"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(anchor)
            .show(ctx, |ui| {
                Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            let field = ui.add(
                                TextEdit::singleline(text)
                                    .id(id)
                                    .desired_width(FIELD_WIDTH)
                                    .hint_text(placeholder),
                            );
                            ui.label(RichText::new(hint).weak());
                            field
                        })
                        .inner
                    })
                    .inner
            });
        if let Some(error) = &error {
            Area::new(id.with("error"))
                .order(Order::Foreground)
                .pivot(Align2::CENTER_TOP)
                .fixed_pos(response.response.rect.center_bottom())
                .show(ctx, |ui| {
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = ui.visuals().error_fg_color;
                            widgets::icon_label(ui, icons::FAILED, color);
                            ui.colored_label(color, error);
                        });
                    });
                });
        }
        let field = response.inner;
        if self.focus_pending {
            field.request_focus();
            self.focus_pending = false;
        }
        if field.changed() {
            self.error = None;
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
        if entered && !escaped {
            let typed = text.trim().to_owned();
            self.close();
            return Some(Typed { text: typed });
        }
        self.close();
        None
    }
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

fn value(model: &Model, text: &str, part: Part) -> Result<f64, String> {
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
        model.length_unit(),
    )
    .map_err(|error| format!("{label}: {error}"))?;
    let dimension = expected.dimension.unwrap_or(Dimension::NONE);
    expression
        .evaluate_as(dimension, &|id| model.parameters().value(id))
        .map_err(|error| format!("{label}: {}", field::sentence(&error.to_string())))
}

fn offset(model: &Model, text: &str, from: From) -> Result<Vector2, String> {
    let coordinates = coordinates(text);
    let polar = polar(text);
    match (coordinates.as_slice(), polar.as_slice()) {
        ([x, y], [_]) => Ok(Vector2::new(value(model, x, X)?, value(model, y, Y)?)),
        ([_], [length, angle]) => {
            let length = value(model, length, DISTANCE)?;
            let angle = value(model, angle, DIRECTION)?;
            Ok(Vector2::from_angle(angle.to_radians()) * length)
        }
        ([length], [_]) => {
            let length = value(model, length, DISTANCE)?;
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
            Ok(direction * length)
        }
        _ => Err(FORMS.to_owned()),
    }
}

pub fn parse(model: &Model, text: &str, from: From) -> Result<Point2, String> {
    let (relative, text) = match text.strip_prefix(RELATIVE_MARK) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let alone = coordinates(text).len() == 1 && polar(text).len() == 1;
    let offset = offset(model, text, from)?;
    let base = if relative || alone {
        from.last.unwrap_or(Point2::ZERO)
    } else {
        Point2::ZERO
    };
    let point = base + offset;
    if point.abs().max_element() > MAX_LENGTH {
        return Err(format!(
            "Keep the point within {} m of the sketch's origin",
            MAX_LENGTH / 1_000.0
        ));
    }
    Ok(point)
}

#[cfg(test)]
mod tests {
    use super::*;

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
