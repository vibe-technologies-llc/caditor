use caditor_document::Rgb;
use egui::{
    Color32, CornerRadius, EventFilter, Id, Key, Painter, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, WidgetInfo, WidgetType,
    epaint::{Mesh, Vec2},
    pos2, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, SPACE_S, WIDGET_RADIUS},
    body_appearance::color32,
    field, widgets,
};

pub const HEX_HINT: &str = "Enter a colour as # and six hexadecimal digits, such as #4682b4";
pub const SQUARE_NAME: &str = "Saturation and brightness of the new colour";
pub const HUE_NAME: &str = "Hue of the new colour";
pub const CURRENT_CAPTION: &str = "Current";
pub const NEW_CAPTION: &str = "New";
pub const HEX_CAPTION: &str = "Hex colour";
const SQUARE_HEIGHT: f32 = 120.0;
const SQUARE_MAX_WIDTH: f32 = 240.0;
const STRIP_HEIGHT: f32 = 16.0;
const PREVIEW_SIDE: f32 = 22.0;
const FINE_STEP: f32 = 0.01;
const COARSE_STEP: f32 = 0.1;
const HUE_FINE_STEP: f32 = 1.0;
const HUE_COARSE_STEP: f32 = 15.0;
const HUE_TURN: f32 = 360.0;
const HUE_SECTORS: usize = 6;
const HANDLE_RADIUS: f32 = 5.0;
const HANDLE_RING: f32 = 2.0;
const STRIP_HANDLE_WIDTH: f32 = 6.0;
const GREY_SATURATION: f32 = 0.12;
const BLACK_VALUE: f32 = 0.12;
const DARK_VALUE: f32 = 0.45;
const LIGHT_SATURATION: f32 = 0.35;
const LIGHT_VALUE: f32 = 0.85;
const HUE_NAMES: [(f32, &str); 8] = [
    (15.0, "red"),
    (45.0, "orange"),
    (70.0, "yellow"),
    (160.0, "green"),
    (200.0, "teal"),
    (255.0, "blue"),
    (290.0, "purple"),
    (335.0, "pink"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}

impl Hsv {
    pub fn of(colour: Rgb) -> Self {
        let [red, green, blue] =
            [colour.red, colour.green, colour.blue].map(|channel| f32::from(channel) / 255.0);
        let high = red.max(green).max(blue);
        let low = red.min(green).min(blue);
        let spread = high - low;
        let sector = if spread == 0.0 {
            0.0
        } else if high == red {
            ((green - blue) / spread).rem_euclid(6.0)
        } else if high == green {
            (blue - red) / spread + 2.0
        } else {
            (red - green) / spread + 4.0
        };
        Self {
            hue: sector * 60.0,
            saturation: if high == 0.0 { 0.0 } else { spread / high },
            value: high,
        }
    }

    pub fn colour(self) -> Rgb {
        let chroma = self.value * self.saturation;
        let sector = self.hue.rem_euclid(HUE_TURN) / 60.0;
        let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
        let (red, green, blue) = match sector as u8 {
            0 => (chroma, second, 0.0),
            1 => (second, chroma, 0.0),
            2 => (0.0, chroma, second),
            3 => (0.0, second, chroma),
            4 => (second, 0.0, chroma),
            _ => (chroma, 0.0, second),
        };
        let floor = self.value - chroma;
        let channel = |part: f32| ((part + floor) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgb::new(channel(red), channel(green), channel(blue))
    }

    pub fn following(self, colour: Rgb) -> Self {
        let parsed = Self::of(colour);
        let grey = parsed.saturation == 0.0 || parsed.value == 0.0;
        Self {
            hue: if grey { self.hue } else { parsed.hue },
            ..parsed
        }
    }

    fn pure(hue: f32) -> Rgb {
        Self {
            hue,
            saturation: 1.0,
            value: 1.0,
        }
        .colour()
    }
}

pub fn name_of(colour: Rgb) -> String {
    let hsv = Hsv::of(colour);
    if hsv.value < BLACK_VALUE {
        return "black".to_owned();
    }
    if hsv.saturation < GREY_SATURATION {
        return match hsv.value {
            value if value > 0.92 => "white",
            value if value > 0.65 => "light grey",
            value if value > 0.3 => "grey",
            _ => "dark grey",
        }
        .to_owned();
    }
    let family = HUE_NAMES
        .iter()
        .find(|(limit, _)| hsv.hue < *limit)
        .map_or("red", |(_, name)| name);
    if hsv.value < DARK_VALUE {
        format!("dark {family}")
    } else if hsv.value > LIGHT_VALUE && hsv.saturation < LIGHT_SATURATION {
        format!("light {family}")
    } else {
        family.to_owned()
    }
}

pub fn describe(colour: Rgb) -> String {
    format!("{}, {}", name_of(colour), colour.hex())
}

#[derive(Debug, Clone, Copy)]
struct State {
    open: bool,
    hsv: Hsv,
    edited: bool,
    busy: bool,
    focus_pending: bool,
}

fn state(ui: &Ui, id: Id) -> Option<State> {
    ui.data(|data| data.get_temp::<State>(id))
}

pub fn is_open(ui: &Ui, id: Id) -> bool {
    state(ui, id).is_some_and(|state| state.open)
}

pub fn toggle(ui: &Ui, id: Id, from: Rgb) {
    let opened = !is_open(ui, id);
    let kept = state(ui, id).map_or_else(|| Hsv::of(from), |state| state.hsv);
    ui.data_mut(|data| {
        data.insert_temp(
            id,
            State {
                open: opened,
                hsv: kept.following(from),
                edited: false,
                busy: false,
                focus_pending: opened,
            },
        );
    });
}

pub fn working(ui: &Ui, id: Id) -> Option<Rgb> {
    state(ui, id)
        .filter(|state| state.open && state.busy)
        .map(|state| state.hsv.colour())
}

pub fn hex_id(id: Id) -> Id {
    id.with("hex")
}

pub fn square_id(id: Id) -> Id {
    id.with("square")
}

pub fn strip_id(id: Id) -> Id {
    id.with("strip")
}

fn gradient(painter: &Painter, rect: Rect, corners: [Color32; 4]) {
    let mut mesh = Mesh::default();
    let [top_left, top_right, bottom_right, bottom_left] = corners;
    mesh.colored_vertex(rect.left_top(), top_left);
    mesh.colored_vertex(rect.right_top(), top_right);
    mesh.colored_vertex(rect.right_bottom(), bottom_right);
    mesh.colored_vertex(rect.left_bottom(), bottom_left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

fn handle_rings(ui: &Ui, centre: egui::Pos2, fill: Color32) {
    let tokens = appearance::tokens(ui);
    let painter = ui.painter();
    painter.circle_filled(centre, HANDLE_RADIUS, fill);
    painter.circle_stroke(centre, HANDLE_RADIUS, Stroke::new(HANDLE_RING, tokens.text));
    painter.circle_stroke(
        centre,
        HANDLE_RADIUS + HANDLE_RING,
        Stroke::new(HANDLE_RING, tokens.panel),
    );
}

fn lock_arrows(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..EventFilter::default()
                },
            );
        });
    }
}

fn arrows(ui: &Ui, response: &Response) -> Vec2 {
    if !response.has_focus() {
        return Vec2::ZERO;
    }
    ui.input(|input| {
        let pressed = |key: Key| f32::from(u8::from(input.key_pressed(key)));
        let sign = vec2(
            pressed(Key::ArrowRight) - pressed(Key::ArrowLeft),
            pressed(Key::ArrowUp) - pressed(Key::ArrowDown),
        );
        let step = if input.modifiers.shift {
            COARSE_STEP
        } else {
            FINE_STEP
        };
        sign * step
    })
}

fn hue_arrows(ui: &Ui, response: &Response) -> f32 {
    if !response.has_focus() {
        return 0.0;
    }
    ui.input(|input| {
        let pressed = |key: Key| f32::from(u8::from(input.key_pressed(key)));
        let sign = pressed(Key::ArrowRight) + pressed(Key::ArrowUp)
            - pressed(Key::ArrowLeft)
            - pressed(Key::ArrowDown);
        let step = if input.modifiers.shift {
            HUE_COARSE_STEP
        } else {
            HUE_FINE_STEP
        };
        sign * step
    })
}

fn pointer_inside(response: &Response, rect: Rect) -> Option<Vec2> {
    let active =
        response.is_pointer_button_down_on() || response.clicked() || response.drag_stopped();
    let pointer = response.interact_pointer_pos().filter(|_| active)?;
    Some(vec2(
        ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
        ((pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
    ))
}

fn square(ui: &mut Ui, id: Id, width: f32, state: &mut State) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, SQUARE_HEIGHT), Sense::hover());
    let response = ui.interact(rect, square_id(id), Sense::click_and_drag());
    lock_arrows(ui, &response);
    if state.focus_pending {
        response.request_focus();
        response.scroll_to_me(Some(egui::Align::Center));
        state.focus_pending = false;
    }
    if let Some(inside) = pointer_inside(&response, rect) {
        state.hsv.saturation = inside.x;
        state.hsv.value = 1.0 - inside.y;
    }
    let nudge = arrows(ui, &response);
    if nudge != Vec2::ZERO {
        state.hsv.saturation = (state.hsv.saturation + nudge.x).clamp(0.0, 1.0);
        state.hsv.value = (state.hsv.value + nudge.y).clamp(0.0, 1.0);
        state.edited = true;
    }
    let name = format!(
        "saturation {}%, brightness {}%",
        (state.hsv.saturation * 100.0).round(),
        (state.hsv.value * 100.0).round()
    );
    response.widget_info(|| WidgetInfo {
        current_text_value: Some(name.clone()),
        ..WidgetInfo::labeled(WidgetType::Other, true, SQUARE_NAME)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hue = color32(Hsv::pure(state.hsv.hue));
        gradient(painter, rect, [Color32::WHITE, hue, hue, Color32::WHITE]);
        gradient(
            painter,
            rect,
            [
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
                Color32::BLACK,
                Color32::BLACK,
            ],
        );
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(BORDER_WIDTH, appearance::tokens(ui).field_border),
            StrokeKind::Inside,
        );
        let centre = pos2(
            rect.left() + state.hsv.saturation * rect.width(),
            rect.top() + (1.0 - state.hsv.value) * rect.height(),
        );
        handle_rings(ui, centre, color32(state.hsv.colour()));
        if response.has_focus() {
            widgets::paint_focus_ring(ui, rect);
        }
    }
    response
}

fn strip(ui: &mut Ui, id: Id, width: f32, state: &mut State) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, STRIP_HEIGHT), Sense::hover());
    let response = ui.interact(rect, strip_id(id), Sense::click_and_drag());
    lock_arrows(ui, &response);
    if let Some(inside) = pointer_inside(&response, rect) {
        state.hsv.hue = inside.x * HUE_TURN;
    }
    let nudge = hue_arrows(ui, &response);
    if nudge != 0.0 {
        state.hsv.hue = (state.hsv.hue + nudge).clamp(0.0, HUE_TURN);
        state.edited = true;
    }
    let name = format!("hue {} degrees", state.hsv.hue.round());
    response.widget_info(|| WidgetInfo {
        current_text_value: Some(name.clone()),
        ..WidgetInfo::slider(true, f64::from(state.hsv.hue), HUE_NAME)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let sector_width = rect.width() / HUE_SECTORS as f32;
        for sector in 0..HUE_SECTORS {
            let left = rect.left() + sector as f32 * sector_width;
            let part = Rect::from_min_max(
                pos2(left, rect.top()),
                pos2(left + sector_width, rect.bottom()),
            );
            let start = color32(Hsv::pure(sector as f32 * 60.0));
            let end = color32(Hsv::pure((sector + 1) as f32 * 60.0));
            gradient(painter, part, [start, end, end, start]);
        }
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(BORDER_WIDTH, appearance::tokens(ui).field_border),
            StrokeKind::Inside,
        );
        let x = rect.left() + state.hsv.hue / HUE_TURN * rect.width();
        let thumb = Rect::from_center_size(
            pos2(x, rect.center().y),
            vec2(STRIP_HANDLE_WIDTH, rect.height() + HANDLE_RING * 2.0),
        );
        let tokens = appearance::tokens(ui);
        let radius = CornerRadius::same(WIDGET_RADIUS / 2);
        painter.rect_filled(thumb, radius, color32(Hsv::pure(state.hsv.hue)));
        painter.rect_stroke(
            thumb,
            radius,
            Stroke::new(HANDLE_RING, tokens.text),
            StrokeKind::Inside,
        );
        painter.rect_stroke(
            thumb,
            radius,
            Stroke::new(HANDLE_RING, tokens.panel),
            StrokeKind::Outside,
        );
        if response.has_focus() {
            widgets::paint_focus_ring(ui, rect);
        }
    }
    response
}

fn preview(ui: &mut Ui, caption: &str, colour: Rgb) {
    let side = PREVIEW_SIDE.max(ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    let label = format!("{caption} colour, {}", describe(colour));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, &label));
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(WIDGET_RADIUS);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, color32(colour));
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(BORDER_WIDTH, appearance::tokens(ui).field_border),
            StrokeKind::Inside,
        );
    }
    ui.label(widgets::muted(caption, ui));
}

pub fn show(ui: &mut Ui, id: Id, current: Rgb) -> Option<Rgb> {
    let mut state = state(ui, id).filter(|state| state.open)?;
    if !state.busy && !state.edited && state.hsv.colour() != current {
        state.hsv = state.hsv.following(current);
    }
    let width = ui.available_width().min(SQUARE_MAX_WIDTH);
    ui.add_space(SPACE_S);
    let square = square(ui, id, width, &mut state);
    ui.add_space(SPACE_S);
    let strip = strip(ui, id, width, &mut state);
    ui.add_space(SPACE_S);
    ui.horizontal(|ui| {
        preview(ui, CURRENT_CAPTION, current);
        preview(ui, NEW_CAPTION, state.hsv.colour());
    });
    ui.add_space(SPACE_S);

    let settled = square.clicked()
        || square.drag_stopped()
        || strip.clicked()
        || strip.drag_stopped()
        || (state.edited && (square.lost_focus() || strip.lost_focus()));
    let mut chosen = settled.then(|| state.hsv.colour());
    if settled {
        state.edited = false;
    }

    let typed = widgets::properties(ui, ("custom-colour", id), |ui| {
        widgets::caption(ui, HEX_CAPTION);
        let field = field::commit_field(
            ui,
            hex_id(id),
            &state.hsv.colour().hex(),
            widgets::FIELD_WIDTH,
            false,
            |text| Rgb::from_hex(text).ok_or_else(|| HEX_HINT.to_owned()),
        );
        ui.end_row();
        if let Some(error) = field.error {
            widgets::error_row(ui, &error);
        }
        field.committed
    });
    if let Some(colour) = typed {
        state.hsv = state.hsv.following(colour);
        chosen = Some(colour);
    }

    state.busy = square.is_pointer_button_down_on() || strip.is_pointer_button_down_on();
    ui.data_mut(|data| data.insert_temp(id, state));
    chosen.filter(|colour| *colour != current)
}

#[cfg(test)]
mod tests {
    use caditor_document::Rgb;

    use super::{Hsv, describe, name_of};

    #[test]
    fn a_colour_survives_the_trip_through_hue_saturation_and_brightness() {
        for red in (0..=255).step_by(15) {
            for green in (0..=255).step_by(15) {
                for blue in (0..=255).step_by(15) {
                    let colour = Rgb::new(red, green, blue);

                    assert_eq!(Hsv::of(colour).colour(), colour);
                }
            }
        }
    }

    #[test]
    fn a_grey_keeps_the_hue_it_was_chosen_with() {
        let blue = Hsv::of(Rgb::new(40, 80, 200));
        let grey = blue.following(Rgb::new(120, 120, 120));
        let black = blue.following(Rgb::new(0, 0, 0));

        assert_eq!(grey.hue, blue.hue);
        assert_eq!(grey.saturation, 0.0);
        assert_eq!(black.hue, blue.hue);
        assert_eq!(black.value, 0.0);
    }

    #[test]
    fn colours_are_named_by_hue_and_lightness() {
        assert_eq!(name_of(Rgb::new(0, 0, 0)), "black");
        assert_eq!(name_of(Rgb::new(255, 255, 255)), "white");
        assert_eq!(name_of(Rgb::new(128, 128, 128)), "grey");
        assert_eq!(name_of(Rgb::new(200, 40, 40)), "red");
        assert_eq!(name_of(Rgb::new(15, 30, 90)), "dark blue");
        assert_eq!(name_of(Rgb::new(190, 220, 255)), "light blue");
        assert_eq!(describe(Rgb::new(70, 130, 180)), "blue, #4682b4");
    }
}
