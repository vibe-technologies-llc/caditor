use egui::{Align, Layout, Rect, Sense, Sides, Ui, UiBuilder, pos2, vec2};

use crate::{
    appearance::{SPACE_L, SPACE_M},
    canvas,
    commands::Command,
    icons, widgets,
};

pub const TITLE: &str = "Start the model";
pub const TEXT: &str = "Sketch on a plane and extrude it, or begin with a box or an imported file.";
pub const START_WITH_SKETCH: &str = "Start with a sketch";
pub const START_WITH_BOX: &str = "Start with a box";
pub const START_WITH_IMPORT: &str = "Import a file…";
pub const DONT_SHOW_AGAIN: &str = "Don't show this again";
pub const CLOSE: &str = "Close the start card for this model";
pub const NOT_EMPTY: &str = "The start card is only for an empty model";
pub const ALREADY_SHOWN: &str = "The start card is already shown";
const DONT_SHOW_AGAIN_HOVER: &str = "Show the start card brings it back for one model, and \
                                     Preferences › General › Tips for every empty model";
const WIDTH: f32 = 400.0;
const MIN_WIDTH: f32 = 300.0;
const TOP_MARGIN: f32 = 16.0;
const BUTTON_HEIGHT: f32 = 30.0;
const BUTTON_GAP: f32 = SPACE_M;
const SECTION_GAP: f32 = SPACE_L;

pub struct Start {
    pub command: Command,
    pub label: &'static str,
    pub hover: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartCard {
    enabled: bool,
    closed_in: Option<u64>,
    brought_back_in: Option<u64>,
    dont_show_again: bool,
    stop_asked: bool,
    drawn_height: u32,
}

impl Default for StartCard {
    fn default() -> Self {
        Self {
            enabled: true,
            closed_in: None,
            brought_back_in: None,
            dont_show_again: false,
            stop_asked: false,
            drawn_height: 0,
        }
    }
}

impl StartCard {
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.closed_in = None;
        }
        self.enabled = enabled;
    }

    pub fn shown_in(&self, session: u64) -> bool {
        self.closed_in != Some(session) && (self.enabled || self.brought_back_in == Some(session))
    }

    pub fn bring_back(&mut self, session: u64) {
        self.closed_in = None;
        self.brought_back_in = Some(session);
    }

    pub fn take_stop_asked(&mut self) -> bool {
        std::mem::take(&mut self.stop_asked)
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        view: Rect,
        cube: Rect,
        session: u64,
        starts: &[Start],
    ) -> Shown {
        let placed = placement(view, cube, self.drawn_height as f32);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("start card")
                .max_rect(placed.area)
                .layout(Layout::top_down(Align::Min))
                .sense(Sense::click_and_drag()),
        );
        child.set_clip_rect(view);
        let frame = widgets::dialog_frame(ui.ctx());
        let inner_width = placed.area.width() - frame.total_margin().sum().x;
        let mut chosen = None;
        let mut closed = false;
        let area = frame
            .show(&mut child, |ui| {
                ui.set_width(inner_width);
                closed = header(ui);
                if !placed.under_cube {
                    ui.add_space(SPACE_M);
                    widgets::balanced_label(ui, widgets::muted(TEXT, ui));
                }
                ui.add_space(SECTION_GAP);
                chosen = start_buttons(ui, starts);
                ui.add_space(SECTION_GAP);
                widgets::checkbox(ui, &mut self.dont_show_again, DONT_SHOW_AGAIN)
                    .on_hover_text(DONT_SHOW_AGAIN_HOVER);
            })
            .response
            .rect;
        let drawn_height = area.height().ceil() as u32;
        if drawn_height != self.drawn_height {
            self.drawn_height = drawn_height;
            ui.ctx().request_repaint();
        }
        if closed || chosen.is_some() {
            self.finish(session, closed);
        }
        Shown { area, chosen }
    }

    fn finish(&mut self, session: u64, closed: bool) {
        if closed {
            self.closed_in = Some(session);
            self.brought_back_in = None;
        }
        self.stop_asked = std::mem::take(&mut self.dont_show_again);
    }
}

pub struct Shown {
    pub area: Rect,
    pub chosen: Option<Command>,
}

fn header(ui: &mut Ui) -> bool {
    Sides::new()
        .show(
            ui,
            |ui| ui.heading(TITLE),
            |ui| widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked(),
        )
        .1
}

fn start_buttons(ui: &mut Ui, starts: &[Start]) -> Option<Command> {
    let width = ui.available_width();
    let mut chosen = None;
    let mut starts = starts.iter();
    if let Some(first) = starts.next() {
        let primary = widgets::primary_icon_button(ui, icons::command(first.command), first.label)
            .min_size(vec2(width, BUTTON_HEIGHT));
        let clicked = centred(ui, width, |ui| {
            ui.add(primary).on_hover_text(&first.hover).clicked()
        });
        if clicked {
            chosen = Some(first.command);
        }
    }
    let rest: Vec<&Start> = starts.collect();
    let count = rest.len().max(1) as f32;
    let share = (width - BUTTON_GAP * (count - 1.0)) / count;
    let side_by_side = rest
        .iter()
        .all(|start| widgets::icon_text_button_width(ui, start.label) <= share);
    let button_width = if side_by_side { share } else { width };
    ui.add_space(BUTTON_GAP - ui.spacing().item_spacing.y);
    let mut secondary = |ui: &mut Ui, start: &Start| {
        let button = widgets::sized_icon_button(
            ui,
            icons::command(start.command),
            start.label,
            vec2(button_width, BUTTON_HEIGHT),
        );
        if centred(ui, button_width, |ui| {
            ui.add(button).on_hover_text(&start.hover).clicked()
        }) {
            chosen = Some(start.command);
        }
    };
    if side_by_side {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            for start in &rest {
                secondary(ui, start);
            }
        });
    } else {
        for start in &rest {
            secondary(ui, start);
        }
    }
    chosen
}

fn centred<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        vec2(width, BUTTON_HEIGHT),
        Layout::top_down(Align::Center),
        add,
    )
    .inner
}

struct Placed {
    area: Rect,
    under_cube: bool,
}

fn placement(view: Rect, cube: Rect, height: f32) -> Placed {
    let room = (view.width() - 2.0 * canvas::MARGIN).max(0.0);
    let width = WIDTH.min(room);
    let top = view.top() + TOP_MARGIN;
    let cube_at_top = cube.center().y <= view.center().y;
    let reaches_cube = cube_at_top || top + height > cube.top() - canvas::MARGIN;
    let mut band_left = view.left() + canvas::MARGIN;
    let mut band_right = view.right() - canvas::MARGIN;
    if reaches_cube {
        if cube.center().x < view.center().x {
            band_left = cube.right() + canvas::MARGIN;
        } else {
            band_right = cube.left() - canvas::MARGIN;
        }
    }
    let bottom = if reaches_cube {
        view.bottom()
    } else {
        cube.top() - canvas::MARGIN
    };
    let centred_left = view.center().x - width / 2.0;
    let at = |left: f32, top: f32, width: f32, under_cube: bool| Placed {
        area: Rect::from_min_max(pos2(left, top), pos2(left + width, bottom.max(top))),
        under_cube,
    };
    if centred_left >= band_left && centred_left + width <= band_right {
        return at(centred_left, top, width, false);
    }
    let band = band_right - band_left;
    if band >= MIN_WIDTH {
        let width = width.min(band);
        let left = (view.center().x - width / 2.0)
            .min(band_right - width)
            .max(band_left);
        return at(left, top, width, false);
    }
    if cube_at_top {
        return at(centred_left, cube.bottom() + canvas::MARGIN, width, true);
    }
    at(centred_left, top, width, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_cube::{Corner, CubePlacement, CubeSize};

    const CUBE_SIDE: f32 = 130.0;
    const CARD_HEIGHT: f32 = 260.0;

    fn cube_of(view: Rect) -> Rect {
        Rect::from_min_size(
            pos2(view.right() - CUBE_SIDE, view.top()),
            vec2(CUBE_SIDE, CUBE_SIDE),
        )
    }

    #[test]
    fn a_wide_view_centres_the_card_at_the_top() {
        let view = Rect::from_min_size(pos2(300.0, 100.0), vec2(1100.0, 800.0));

        let placed = placement(view, cube_of(view), 0.0).area;

        assert_eq!(placed.center().x, view.center().x);
        assert_eq!(placed.top(), view.top() + TOP_MARGIN);
        assert_eq!(placed.width(), WIDTH);
    }

    #[test]
    fn a_view_too_narrow_to_centre_moves_the_card_left_of_the_cube() {
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(560.0, 400.0));
        let cube = cube_of(view);

        let placed = placement(view, cube, CARD_HEIGHT).area;

        assert!(placed.right() <= cube.left() - canvas::MARGIN);
        assert!(placed.left() >= view.left() + canvas::MARGIN);
        assert_eq!(placed.top(), view.top() + TOP_MARGIN);
        assert!(placed.width() >= MIN_WIDTH);
    }

    #[test]
    fn a_view_without_room_beside_the_cube_puts_the_card_under_it() {
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0));
        let cube = cube_of(view);

        let placed = placement(view, cube, CARD_HEIGHT).area;

        assert!(placed.top() >= cube.bottom());
        assert!(view.contains_rect(placed));
        assert_eq!(placed.center().x, view.center().x);
    }

    #[test]
    fn the_card_keeps_clear_of_the_view_cube_in_every_corner_and_size() {
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(560.0, 400.0));
        for size in [CubeSize::Small, CubeSize::Standard, CubeSize::Large] {
            for corner in Corner::ALL {
                let cube = CubePlacement { size, corner }.area(view);

                let placed = placement(view, cube, CARD_HEIGHT).area;

                assert!(!placed.intersects(cube), "{size:?} {corner:?}");
                assert!(view.contains_rect(placed), "{size:?} {corner:?}");
                assert!(placed.width() >= MIN_WIDTH, "{size:?} {corner:?}");
            }
        }
    }

    #[test]
    fn closing_hides_the_card_for_that_model_only() {
        let mut card = StartCard::default();

        card.finish(1, true);

        assert!(!card.shown_in(1));
        assert!(card.shown_in(2));
        assert!(!card.take_stop_asked());
    }

    #[test]
    fn closing_with_dont_show_again_asks_to_stop_and_the_preference_brings_it_back() {
        let mut card = StartCard {
            dont_show_again: true,
            ..StartCard::default()
        };

        card.finish(1, true);
        card.set_enabled(false);

        assert!(card.take_stop_asked());
        assert!(!card.take_stop_asked());
        assert!(!card.shown_in(1));
        assert!(!card.shown_in(2));

        card.set_enabled(true);

        assert!(card.shown_in(1));
        assert!(card.shown_in(2));
    }

    #[test]
    fn bringing_the_card_back_shows_it_on_that_model_while_turned_off() {
        let mut card = StartCard::default();
        card.set_enabled(false);

        card.bring_back(3);

        assert!(card.shown_in(3));
        assert!(!card.shown_in(4));
    }
}
