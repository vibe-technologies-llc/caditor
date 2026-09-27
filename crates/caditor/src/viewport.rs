use std::time::Duration;

use caditor_document::Document;
use caditor_geometry::{Point3, Rotation3, Vector2, Vector3};
use caditor_render::{Camera, PickResult, Scene, View, Viewpoint, ViewportRect};
use egui::{Align2, Color32, FontId, Key, PointerButton, Rect, Response, Sense, vec2};

use crate::{
    scene::{self, BuiltScene, Highlight, PickTable},
    selection::{Pickable, Selection},
    view_cube::{self, CubeAction},
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_PX: f64 = 1.5;
const LABEL_MARGIN: f32 = 12.0;
const LABEL_COLOR: Color32 = Color32::from_rgb(225, 228, 235);
const HINT_COLOR: Color32 = Color32::from_rgba_premultiplied(120, 124, 132, 160);
const NAVIGATION_HINT: &str =
    "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom   F: fit";

#[derive(Debug, Clone, Copy, PartialEq)]
struct PointerHit {
    cursor: Vector2,
    position: Point3,
}

#[derive(Debug, Clone, PartialEq)]
struct PickKey {
    cursor: Vector2,
    view: View,
    scene: Scene,
}

pub struct ViewportRequest {
    pub view: View,
    pub rect: ViewportRect,
    pub pick_at: Option<Vector2>,
}

pub struct ViewportState {
    camera: Camera,
    selection: Selection,
    hovered: Option<Pickable>,
    pointer_hit: Option<PointerHit>,
    cursor: Option<Vector2>,
    rect: Option<Rect>,
    pixels_per_point: f32,
    drag_anchor: Option<Point3>,
    needs_initial_fit: bool,
    fit_requested: bool,
    picks_in_flight: Option<PickTable>,
    last_pick: Option<PickKey>,
}

impl ViewportState {
    pub fn new() -> Self {
        let initial = Viewpoint::looking_from(INITIAL_LOOK_FROM, Point3::ZERO, INITIAL_DISTANCE)
            .unwrap_or(Viewpoint {
                target: Point3::ZERO,
                orientation: Rotation3::IDENTITY,
                distance: INITIAL_DISTANCE,
            });
        Self {
            camera: Camera::new(initial),
            selection: Selection::default(),
            hovered: None,
            pointer_hit: None,
            cursor: None,
            rect: None,
            pixels_per_point: 1.0,
            drag_anchor: None,
            needs_initial_fit: true,
            fit_requested: false,
            picks_in_flight: None,
            last_pick: None,
        }
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn is_animating(&self) -> bool {
        self.camera.is_animating()
    }

    pub fn advance(&mut self, elapsed: Duration) {
        self.camera.advance(elapsed);
    }

    pub fn apply_pick(&mut self, result: &PickResult) {
        let Some(picks) = self.picks_in_flight.take() else {
            return;
        };
        if self.cursor.is_none() {
            return;
        }
        let best = picks.best_hit(result);
        self.hovered = best.map(|(pickable, _)| pickable);
        self.pointer_hit = best
            .map(|(_, hit)| hit)
            .or_else(|| result.hits.first().copied())
            .map(|hit| PointerHit {
                cursor: result.cursor,
                position: hit.position,
            });
    }

    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            self.rect = Some(rect);
            self.pixels_per_point = ui.ctx().pixels_per_point();
            let response = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());

            self.track_cursor(ui, &response, rect);
            self.navigate(ui, &response, rect);
            self.select(ui, &response);
            self.handle_keys(ui);
            self.decorate(ui, rect, document);
        });
    }

    pub fn build_scene(&mut self, document: &Document) -> BuiltScene {
        self.selection.retain_existing(document);
        let built = scene::build(
            document,
            &Highlight {
                selection: &self.selection,
                hovered: self.hovered,
            },
        );
        let Some(view) = self.view() else {
            return built;
        };
        if self.needs_initial_fit {
            self.camera = Camera::new(view.fitted(built.everything));
            self.needs_initial_fit = false;
        } else if self.fit_requested {
            let bounds = if self.selection.is_empty() {
                built.everything
            } else {
                built
                    .bounds_of(document, self.selection.iter())
                    .unwrap_or(built.everything)
            };
            self.camera.animate_to(view.fitted(bounds));
        }
        self.fit_requested = false;
        built
    }

    pub fn request(&mut self, built: &BuiltScene, can_pick: bool) -> Option<ViewportRequest> {
        let rect = self.rect?;
        let view = self.view()?;
        let pick_at = self.cursor.filter(|_| can_pick).and_then(|cursor| {
            let key = PickKey {
                cursor,
                view,
                scene: built.scene.clone(),
            };
            if self.last_pick.as_ref() == Some(&key) {
                return None;
            }
            self.last_pick = Some(key);
            self.picks_in_flight = Some(built.picks.clone());
            Some(cursor)
        });
        let scale = self.pixels_per_point;
        Some(ViewportRequest {
            view,
            rect: ViewportRect {
                x: rect.left() * scale,
                y: rect.top() * scale,
                width: rect.width() * scale,
                height: rect.height() * scale,
            },
            pick_at,
        })
    }

    pub fn pick_was_not_issued(&mut self) {
        self.last_pick = None;
        self.picks_in_flight = None;
    }

    fn view(&self) -> Option<View> {
        let size = self.rect?.size() * self.pixels_per_point;
        (size.x >= 1.0 && size.y >= 1.0)
            .then(|| self.camera.view(f64::from(size.x), f64::from(size.y)))
    }

    fn to_pixels(&self, points: egui::Vec2) -> Vector2 {
        Vector2::new(f64::from(points.x), f64::from(points.y)) * f64::from(self.pixels_per_point)
    }

    fn hit_under(&self, cursor: Vector2) -> Option<Point3> {
        self.pointer_hit
            .filter(|hit| hit.cursor.distance(cursor) <= HIT_CURSOR_TOLERANCE_PX)
            .map(|hit| hit.position)
    }

    fn track_cursor(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let inside = (response.hovered() || response.dragged())
            .then_some(pointer)
            .flatten()
            .filter(|position| rect.contains(*position));
        self.cursor = inside.map(|position| self.to_pixels(position - rect.min));
        if self.cursor.is_none() {
            self.hovered = None;
            self.pointer_hit = None;
            self.last_pick = None;
        }
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        let Some(view) = self.view() else {
            return;
        };
        if response.drag_started() {
            self.drag_anchor = self.cursor.and_then(|cursor| self.hit_under(cursor));
        }

        let shift = ui.input(|input| input.modifiers.shift);
        let drag = self.to_pixels(response.drag_delta());
        let orbiting = response.dragged_by(PointerButton::Secondary) && !shift;
        let panning = response.dragged_by(PointerButton::Middle)
            || (response.dragged_by(PointerButton::Secondary) && shift);
        if orbiting && drag != Vector2::ZERO {
            let pivot = self.drag_anchor.unwrap_or(view.viewpoint().target);
            self.camera.orbit(
                pivot,
                drag,
                f64::from(rect.height() * self.pixels_per_point),
            );
        } else if panning && drag != Vector2::ZERO {
            let depth = self
                .drag_anchor
                .map(|anchor| view.view_depth(anchor))
                .filter(|depth| *depth > view.near_plane())
                .unwrap_or(view.viewpoint().distance);
            self.camera.pan(drag, view.units_per_pixel_at(depth));
        }

        if !response.hovered() && !response.dragged() {
            return;
        }
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
        let factor = (-f64::from(scroll) * ZOOM_PER_SCROLL_POINT).exp() / f64::from(pinch);
        if (factor - 1.0).abs() < 1e-9 {
            return;
        }
        let Some(cursor) = self.cursor else {
            return;
        };
        let anchor = self
            .hit_under(cursor)
            .or_else(|| view.focal_point_under(cursor))
            .unwrap_or(view.viewpoint().target);
        self.camera.zoom(anchor, factor);
    }

    fn select(&mut self, ui: &egui::Ui, response: &Response) {
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        let toggle = ui.input(|input| input.modifiers.shift || input.modifiers.command);
        match (self.hovered, toggle) {
            (Some(pickable), true) => self.selection.toggle(pickable),
            (Some(pickable), false) => self.selection.replace_with(pickable),
            (None, false) => self.selection.clear(),
            (None, true) => {}
        }
    }

    fn handle_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (fit, clear) =
            ui.input(|input| (input.key_pressed(Key::F), input.key_pressed(Key::Escape)));
        if fit {
            self.fit_requested = true;
        }
        if clear {
            self.selection.clear();
        }
    }

    fn decorate(&mut self, ui: &mut egui::Ui, rect: Rect, document: &Document) {
        let orientation = self.camera.viewpoint().orientation;
        let fit_label = if self.selection.is_empty() {
            "Fit all"
        } else {
            "Fit selection"
        };
        match view_cube::show(ui, rect, orientation, fit_label) {
            Some(CubeAction::LookFrom(direction)) => {
                let destination = self.camera.destination();
                if let Some(viewpoint) =
                    Viewpoint::looking_from(direction, destination.target, destination.distance)
                {
                    self.camera.animate_to(viewpoint);
                }
            }
            Some(CubeAction::Fit) => self.fit_requested = true,
            None => {}
        }
        view_cube::show_axis_triad(ui, rect, orientation);

        let painter = ui.painter();
        if let Some(hovered) = self.hovered {
            painter.text(
                rect.left_top() + vec2(LABEL_MARGIN, LABEL_MARGIN),
                Align2::LEFT_TOP,
                hovered.describe(document),
                FontId::proportional(13.0),
                LABEL_COLOR,
            );
        }
        painter.text(
            rect.right_bottom() - vec2(LABEL_MARGIN, LABEL_MARGIN),
            Align2::RIGHT_BOTTOM,
            NAVIGATION_HINT,
            FontId::proportional(11.0),
            HINT_COLOR,
        );
    }
}

#[cfg(test)]
mod tests {
    use caditor_render::{PickHit, PickId};
    use egui::pos2;

    use super::*;

    const ORIGIN_PICK_INDEX: usize = 6;

    fn state_with_cursor() -> ViewportState {
        let mut state = ViewportState::new();
        state.rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)));
        state.cursor = Some(Vector2::new(120.0, 80.0));
        state
    }

    #[test]
    fn picks_once_per_unchanged_state_and_retries_when_not_issued() {
        let document = Document::default();
        let mut state = state_with_cursor();
        let built = state.build_scene(&document);

        let first = state.request(&built, true).unwrap();
        assert_eq!(first.pick_at, Some(Vector2::new(120.0, 80.0)));
        assert_eq!(first.rect.width, 400.0);
        assert_eq!(state.request(&built, true).unwrap().pick_at, None);

        state.pick_was_not_issued();
        assert!(state.request(&built, false).unwrap().pick_at.is_none());
        assert!(state.request(&built, true).unwrap().pick_at.is_some());
    }

    #[test]
    fn a_pick_result_sets_the_hovered_item_and_the_hit_under_the_cursor() {
        let document = Document::default();
        let mut state = state_with_cursor();
        let built = state.build_scene(&document);
        state.request(&built, true);

        let cursor = Vector2::new(120.0, 80.0);
        state.apply_pick(&PickResult {
            cursor,
            hits: vec![PickHit {
                id: PickId::from_index(ORIGIN_PICK_INDEX).unwrap(),
                offset_px: 2.0,
                position: Point3::ZERO,
            }],
        });

        assert_eq!(state.hovered, Some(Pickable::Origin));
        assert_eq!(state.hit_under(cursor), Some(Point3::ZERO));
        assert_eq!(state.hit_under(cursor + Vector2::new(5.0, 0.0)), None);
    }

    #[test]
    fn the_first_scene_fits_the_camera_and_later_fits_animate() {
        let document = Document::default();
        let mut state = state_with_cursor();
        let initial = state.camera.viewpoint();
        state.build_scene(&document);
        assert_ne!(state.camera.viewpoint(), initial);
        assert!(!state.is_animating());

        state.fit_requested = true;
        state
            .selection
            .replace_with(Pickable::Axis(crate::selection::Axis::X));
        state.build_scene(&document);
        assert!(state.is_animating());
        assert!(!state.fit_requested);
    }
}
