use std::time::Duration;

use caditor_document::{Document, Evaluation, FeatureId};
use caditor_geometry::{Plane, Point2, Point3, Rotation3, Vector2, Vector3};
use caditor_render::{Camera, PickResult, Scene, View, Viewpoint, ViewportRect};
use caditor_sketch::ConstraintId;
use egui::{Align2, Color32, FontId, Key, PointerButton, Rect, Response, Sense, vec2};

use crate::{
    annotations::{Annotations, Surface},
    blend_tools,
    bodies::{self, BodyMeshes},
    drawing::Drawing,
    editing::{self, EditingCommand, SketchEditing, Tool},
    model::{Action, Model},
    scene::{self, BuiltScene, EditedSketch, Highlight, PickTable, Sources},
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    snap::{Pointer, Screen},
    solid_tools,
    view_cube::{self, CubeAction},
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_PX: f64 = 1.5;
const LABEL_MARGIN: f32 = 12.0;
const LABEL_COLOR: Color32 = Color32::from_rgb(225, 228, 235);
const HINT_COLOR: Color32 = Color32::from_rgba_premultiplied(120, 124, 132, 160);
const PROMPT_COLOR: Color32 = Color32::from_rgb(255, 214, 120);
const PROMPT_MARGIN: f32 = 16.0;
const NAVIGATION_HINT: &str =
    "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom   F: fit";
pub const CHOOSE_PLANE_PROMPT: &str = "Click a plane or a flat face to sketch on";
const CHOOSE_PLANE_HINT: &str = "Esc: cancel";
const CHOOSE_REGIONS_PROMPT: &str = "Click regions of the sketch to include or leave them out";
const CHOOSE_REGIONS_HINT: &str = "Esc: done";
const CHOOSE_EDGES_PROMPT: &str = "Click edges to add them or leave them out";
const SNAP_LABEL_OFFSET: egui::Vec2 = vec2(14.0, 10.0);
const SNAP_LABEL_COLOR: Color32 = Color32::from_rgb(80, 226, 236);

struct SketchScreen {
    view: View,
    plane: Plane,
    pixels_per_point: f64,
}

impl Screen for SketchScreen {
    fn to_screen(&self, point: Point2) -> Option<Vector2> {
        self.view
            .project(self.plane.to_world(point))
            .map(|pixel| pixel / self.pixels_per_point)
    }
}

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
    edited: Option<FeatureId>,
    face_edited_sketch: bool,
    sketch_cursor: Option<Point2>,
    drawing: Drawing,
    annotations: Annotations,
    bodies: BodyMeshes,
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
            edited: None,
            face_edited_sketch: false,
            sketch_cursor: None,
            drawing: Drawing::default(),
            annotations: Annotations::default(),
            bodies: BodyMeshes::default(),
        }
    }

    pub fn edit_dimension(&mut self, feature: FeatureId, constraint: ConstraintId) {
        self.annotations.request_field(feature, constraint);
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    #[cfg(test)]
    pub fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    pub fn is_drawing(&self) -> bool {
        self.drawing.in_progress()
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

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &Model,
        editing: &SketchEditing,
        keys_free: bool,
        actions: &mut Vec<Action>,
    ) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            self.rect = Some(rect);
            self.pixels_per_point = ui.ctx().pixels_per_point();
            let response = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());

            self.track_cursor(ui, &response, rect);
            self.track_sketch_cursor(model, editing);
            self.track_drawing(model, editing);
            self.navigate(ui, &response, rect);
            self.click(ui, &response, model, editing, actions);
            if keys_free {
                self.handle_keys(ui, model, editing, actions);
            }
            self.annotate(ui, rect, model, editing, actions);
            self.decorate(ui, rect, model, editing);
        });
    }

    pub fn build_scene(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        editing: &SketchEditing,
    ) -> BuiltScene {
        let edited = editing.feature();
        let context = editing.context();
        if edited != self.edited {
            self.edited = edited;
            self.face_edited_sketch = edited.is_some();
            self.last_pick = None;
        }
        self.bodies.update(evaluation);
        self.bodies.update_open(evaluation, context.solid);
        self.selection
            .retain_available(document, evaluation, context);
        self.hovered = self
            .hovered
            .filter(|hovered| hovered.is_available(document, evaluation, context));
        let hovered: Vec<Pickable> = if self.drawing.is_active() {
            edited
                .zip(self.drawing.snap_entity())
                .map(|(feature, entity)| Pickable::SketchEntity { feature, entity })
                .into_iter()
                .collect()
        } else if let Some(annotation) = self.annotations.hovered() {
            annotation.constrained_entities(document)
        } else {
            self.hovered.into_iter().collect()
        };
        let sources = Sources {
            document,
            evaluation,
            bodies: &self.bodies,
        };
        let mut built = scene::build(
            &sources,
            &Highlight {
                selection: &self.selection,
                hovered: &hovered,
            },
            context,
        );
        if let Some(sketch) = built.edited {
            scene::add_preview(&mut built.scene, sketch.plane, &self.drawing.preview());
        }
        let Some(view) = self.view() else {
            return built;
        };
        if self.needs_initial_fit {
            self.camera = Camera::new(view.fitted(built.everything));
            self.needs_initial_fit = false;
        } else if self.face_edited_sketch {
            if let Some(sketch) = built.edited {
                self.camera.animate_to(facing(&view, &sketch));
            }
            self.face_edited_sketch = false;
        } else if self.fit_requested {
            let everything = built.fit_all();
            let bounds = if self.selection.is_empty() {
                everything
            } else {
                built
                    .bounds_of(&sources, self.selection.iter())
                    .unwrap_or(everything)
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

    #[cfg(test)]
    pub fn screen_position(&self, plane: Plane, point: Point2) -> Option<egui::Pos2> {
        let pixel = self.view()?.project(plane.to_world(point))? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn hover_through_pick(&mut self, built: &BuiltScene, pickable: Pickable) {
        let (Some(cursor), Some(id)) = (self.cursor, built.picks.id_of(pickable)) else {
            return;
        };
        self.picks_in_flight = Some(built.picks.clone());
        self.apply_pick(&PickResult {
            cursor,
            hits: vec![caditor_render::PickHit {
                id,
                offset_px: 0.0,
                position: Point3::ZERO,
            }],
        });
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

    fn track_sketch_cursor(&mut self, model: &Model, editing: &SketchEditing) {
        let plane = editing
            .feature()
            .and_then(|feature| scene::sketch_plane(model.document(), model.evaluation(), feature));
        self.sketch_cursor =
            plane
                .zip(self.cursor)
                .zip(self.view())
                .and_then(|((plane, cursor), view)| {
                    let ray = view.ray_through(cursor)?;
                    let distance = ray.intersect_plane(&plane)?;
                    Some(plane.to_local(ray.at(distance)))
                });
    }

    fn track_drawing(&mut self, model: &Model, editing: &SketchEditing) {
        let displayed = editing
            .feature()
            .and_then(|feature| model.document().feature(feature))
            .and_then(|feature| scene::displayed_sketch(model.evaluation(), feature));
        self.drawing.sync(editing.active(), displayed.as_deref());
        let scale = f64::from(self.pixels_per_point);
        let pointer = self
            .cursor
            .zip(self.sketch_cursor)
            .map(|(cursor, sketch)| Pointer {
                screen: cursor / scale,
                sketch,
            });
        match (displayed, self.view()) {
            (Some(sketch), Some(view)) => {
                let screen = SketchScreen {
                    view,
                    plane: sketch.plane(),
                    pixels_per_point: scale,
                };
                self.drawing.hover(&sketch, &screen, pointer);
            }
            _ => self.drawing.leave(),
        }
    }

    fn click(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        if response.double_clicked() && editing.feature().is_none() {
            let command = match self.hovered {
                Some(Pickable::SketchEntity { feature, .. }) => {
                    Some(EditingCommand::Enter(feature))
                }
                Some(Pickable::Face { body, face }) => model
                    .evaluation()
                    .body(body)
                    .and_then(|solid| bodies::face_origin(solid, face))
                    .map(|origin| EditingCommand::OpenSolid(bodies::origin_feature(origin))),
                _ => None,
            };
            if let Some(command) = command {
                actions.push(Action::Editing(command));
                return;
            }
        }
        if !response.clicked_by(PointerButton::Primary) {
            return;
        }
        if let Some(Pickable::Region { feature, region }) = self.hovered {
            if let Some(transaction) = solid_tools::toggle_region(model, feature, region) {
                actions.push(Action::Apply(transaction));
            }
            return;
        }
        if let Some(Pickable::BlendEdge { feature, edge }) = self.hovered {
            if let Some(transaction) = blend_tools::toggle_edge(model, feature, edge) {
                actions.push(Action::Apply(transaction));
            }
            return;
        }
        if editing.is_choosing_plane() {
            let command = match self.hovered {
                Some(Pickable::Plane(plane)) => Some(EditingCommand::NewSketch(Some(plane))),
                Some(pickable) => FaceChoice::of(pickable)
                    .filter(|face| sketch_placement::is_flat(model, *face))
                    .map(EditingCommand::NewSketchOnFace),
                None => None,
            };
            if let Some(command) = command {
                actions.push(Action::Editing(command));
            }
            return;
        }
        if let Some(active) = editing.active()
            && active.tool.draws()
        {
            if let Some(transaction) = self.drawing.click(model) {
                actions.push(Action::Apply(transaction));
            }
            return;
        }
        self.select(ui);
    }

    fn select(&mut self, ui: &egui::Ui) {
        let toggle = ui.input(|input| input.modifiers.shift || input.modifiers.command);
        match (self.hovered, toggle) {
            (Some(pickable), true) => self.selection.toggle(pickable),
            (Some(pickable), false) => self.selection.replace_with(pickable),
            (None, false) => self.selection.clear(),
            (None, true) => {}
        }
    }

    fn handle_keys(
        &mut self,
        ui: &egui::Ui,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let (fit, escape, finish, back) = ui.input(|input| {
            (
                input.key_pressed(Key::F),
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Backspace),
            )
        });
        if fit {
            self.fit_requested = true;
        }
        if escape {
            self.escape(editing, actions);
        }
        if finish && let Some(transaction) = self.drawing.finish(model) {
            actions.push(Action::Apply(transaction));
        }
        if back && self.drawing.in_progress() {
            self.drawing.remove_last();
        }
    }

    fn escape(&mut self, editing: &SketchEditing, actions: &mut Vec<Action>) {
        let active = editing.active();
        if editing.is_choosing_plane() {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        } else if self.drawing.in_progress() {
            self.drawing.cancel();
        } else if let Some(active) = active
            && active.tool != Tool::Select
        {
            actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
        } else if !self.selection.is_empty() {
            self.selection.clear();
        } else if active.is_some() {
            actions.push(Action::Editing(EditingCommand::Finish));
        } else if editing.solid().is_some() {
            actions.push(Action::Editing(EditingCommand::CloseSolid));
        }
    }

    fn annotate(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let edited = editing.feature().and_then(|feature| {
            let plane = scene::sketch_plane(model.document(), model.evaluation(), feature)?;
            Some((feature, plane))
        });
        let (Some((feature, plane)), Some(view)) = (edited, self.view()) else {
            self.annotations.clear();
            return;
        };
        let screen = SketchScreen {
            view,
            plane,
            pixels_per_point: f64::from(self.pixels_per_point),
        };
        let surface = Surface {
            rect,
            screen: &screen,
            feature,
            interactive: !self.drawing.is_active(),
        };
        self.annotations
            .show(ui, model, &surface, &mut self.selection, actions);
    }

    fn decorate(&mut self, ui: &mut egui::Ui, rect: Rect, model: &Model, editing: &SketchEditing) {
        let document = model.document();
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
        let hovered = self.annotations.hovered().or(self.hovered);
        if let Some(hovered) = hovered.filter(|_| !self.drawing.is_active()) {
            painter.text(
                rect.left_top() + vec2(LABEL_MARGIN, LABEL_MARGIN),
                Align2::LEFT_TOP,
                hovered.describe(document, model.evaluation()),
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
        let prompt = if editing.is_choosing_plane() {
            Some((CHOOSE_PLANE_PROMPT, CHOOSE_PLANE_HINT))
        } else if let Some(feature) = editing.solid() {
            let blend = document
                .feature(feature)
                .is_some_and(|feature| feature.kind.blend().is_some());
            let prompt = if blend {
                CHOOSE_EDGES_PROMPT
            } else {
                CHOOSE_REGIONS_PROMPT
            };
            Some((prompt, CHOOSE_REGIONS_HINT))
        } else {
            self.drawing
                .prompt()
                .map(|prompt| (prompt.text, prompt.keys))
        };
        if let Some((text, keys)) = prompt {
            let prompt = painter.text(
                rect.center_top() + vec2(0.0, PROMPT_MARGIN),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(16.0),
                PROMPT_COLOR,
            );
            painter.text(
                prompt.center_bottom() + vec2(0.0, LABEL_MARGIN / 2.0),
                Align2::CENTER_TOP,
                keys,
                FontId::proportional(11.0),
                HINT_COLOR,
            );
        }
        let snap_label = editing
            .feature()
            .and_then(|feature| editing::edited_sketch(document, feature))
            .and_then(|sketch| self.drawing.snap_label(sketch));
        if let (Some(label), Some(cursor)) = (snap_label, self.cursor) {
            let position = rect.min
                + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
            painter.text(
                position + SNAP_LABEL_OFFSET,
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.0),
                SNAP_LABEL_COLOR,
            );
        }
        if let Some(position) = self.sketch_cursor {
            painter.text(
                rect.left_bottom() + vec2(LABEL_MARGIN, -LABEL_MARGIN),
                Align2::LEFT_BOTTOM,
                format!("x {:.2} mm   y {:.2} mm", position.x, position.y),
                FontId::monospace(11.0),
                LABEL_COLOR,
            );
        }
    }
}

fn facing(view: &View, sketch: &EditedSketch) -> Viewpoint {
    let size = view.size();
    let facing = Viewpoint::facing(
        &sketch.plane,
        sketch.bounds.center(),
        view.viewpoint().distance,
    );
    View::new(facing, size.x, size.y).fitted(sketch.bounds)
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
        let built = state.build_scene(&document, &Evaluation::default(), &SketchEditing::default());

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
        let built = state.build_scene(&document, &Evaluation::default(), &SketchEditing::default());
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
    fn entering_a_sketch_turns_the_camera_to_face_it_and_keeps_only_its_selection() {
        let mut document = Document::default();
        let mut sketch = caditor_sketch::Sketch::new(caditor_geometry::Plane::XZ);
        let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(30.0, 20.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Side", caditor_document::FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let evaluation = Evaluation::default();
        let mut state = state_with_cursor();
        state.build_scene(&document, &evaluation, &SketchEditing::default());
        let entity = Pickable::SketchEntity {
            feature,
            entity: line,
        };
        state.selection.toggle(Pickable::Origin);
        state.selection.toggle(entity);

        let editing = SketchEditing::editing(feature);
        let built = state.build_scene(&document, &evaluation, &editing);
        assert!(state.is_animating());
        assert_eq!(state.selection.iter().collect::<Vec<_>>(), vec![entity]);
        state.advance(Duration::from_secs(1));
        let viewpoint = state.camera.viewpoint();
        assert!(viewpoint.forward().distance(Vector3::Y) < 1e-9);
        assert!(viewpoint.up().distance(Vector3::Z) < 1e-9);
        let view = state.view().unwrap();
        for corner in built.edited.unwrap().bounds.corners() {
            let pixel = view.project(corner).unwrap();
            assert!(pixel.x >= 0.0 && pixel.x <= view.size().x);
            assert!(pixel.y >= 0.0 && pixel.y <= view.size().y);
        }

        state.build_scene(&document, &evaluation, &editing);
        assert!(!state.is_animating());
        let reference = Pickable::SketchEntity {
            feature,
            entity: caditor_sketch::EntityId::ORIGIN,
        };
        state.selection.toggle(reference);
        state.build_scene(&document, &evaluation, &editing);
        assert!(state.selection.contains(reference));
        state.build_scene(&document, &evaluation, &SketchEditing::default());
        assert_eq!(state.selection.iter().collect::<Vec<_>>(), vec![entity]);
    }

    #[test]
    fn the_first_scene_fits_the_camera_and_later_fits_animate() {
        let document = Document::default();
        let mut state = state_with_cursor();
        let initial = state.camera.viewpoint();
        state.build_scene(&document, &Evaluation::default(), &SketchEditing::default());
        assert_ne!(state.camera.viewpoint(), initial);
        assert!(!state.is_animating());

        state.fit_requested = true;
        state
            .selection
            .replace_with(Pickable::Axis(crate::selection::Axis::X));
        state.build_scene(&document, &Evaluation::default(), &SketchEditing::default());
        assert!(state.is_animating());
        assert!(!state.fit_requested);
    }
}
