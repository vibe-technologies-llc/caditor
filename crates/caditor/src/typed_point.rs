use caditor_expression::Dimension;
use caditor_geometry::Point2;
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
const FIELD_WIDTH: f32 = 180.0;
const RELATIVE_MARK: char = '@';
const SEPARATORS: [char; 2] = [',', ';'];
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};

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
    text.chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit() || matches!(first, '-' | '.' | RELATIVE_MARK))
}

impl TypedPoint {
    pub fn is_open(&self) -> bool {
        self.text.is_some()
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn open_with(&mut self, text: String, error: String) {
        self.text = Some(text);
        self.error = Some(error);
        self.focus_pending = true;
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

    pub fn show(&mut self, ctx: &egui::Context, anchor: Pos2, hint: &str) -> Option<Typed> {
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
                            ui.label(FIELD_LABEL);
                            let field = ui.add(
                                TextEdit::singleline(text)
                                    .id(id)
                                    .desired_width(FIELD_WIDTH)
                                    .hint_text("x, y"),
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

pub fn parse(model: &Model, text: &str, last: Option<Point2>) -> Result<Point2, String> {
    let (relative, text) = match text.strip_prefix(RELATIVE_MARK) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut parts = text.split(SEPARATORS);
    let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err("Type two lengths separated by a comma, such as 10, 20".to_owned());
    };
    let coordinate = |part: &str, name: &str| {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("The {name} coordinate is missing"));
        }
        let expression = field::parse_expression(
            model.document(),
            model.parameters(),
            part,
            LENGTH,
            model.length_unit(),
        )
        .map_err(|error| format!("{name}: {error}"))?;
        model
            .parameters()
            .evaluate_expression(&expression)
            .map(|quantity| quantity.value)
            .map_err(|error| format!("{name}: {}", field::sentence(&error.to_string())))
    };
    let offset = Point2::new(coordinate(x, "x")?, coordinate(y, "y")?);
    if relative {
        Ok(last.unwrap_or(Point2::ZERO) + offset)
    } else {
        Ok(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_starts_with_a_digit_a_sign_a_decimal_point_or_the_relative_mark() {
        for text in ["1", "-2", ".5", "@3, 4"] {
            assert!(starts_a_point(text), "{text}");
        }
        for text in ["l", " ", "", "x"] {
            assert!(!starts_a_point(text), "{text}");
        }
    }
}
