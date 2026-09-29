use std::time::Duration;

use caditor_document::{Document, Evaluation, FeatureId, FeatureKind};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Rotation3, Vector2, Vector3};
use caditor_render::{Camera, PickResult, Scene, View, Viewpoint, ViewportRect};
use caditor_sketch::ConstraintId;
use egui::{Align2, FontId, Key, PointerButton, Rect, Response, Sense, vec2};

use crate::{
    annotations::{Annotations, Surface},
    blend_tools,
    bodies::{self, BodyMeshes, BodyMeshing},
    canvas,
    commands::{CameraMove, Command, CommandFrame, StandardView},
    datum_tools,
    drawing::Drawing,
    editing::{self, EditingCommand, SketchEditing, Tool},
    model::{Action, Model, Notice, RecomputeStatus},
    preferences::{Navigation, PreferenceChange, PreferencesCommand},
    scene::{self, BuiltScene, EditedSketch, Highlight, PickTable, Sources},
    selection::{Pickable, Selection},
    shell_tools,
    sketch_placement::FaceChoice,
    snap::{Pointer, Screen},
    solid_tools,
    typed_point::{self, TypedPoint},
    view_cube::{self, CubeAction},
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_POINTS: f64 = 1.5;
const LABEL_MARGIN: f32 = 12.0;
const PROMPT_MARGIN: f32 = 16.0;
const NAVIGATION_HINT: &str =
    "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom";
pub const CHOOSE_PLANE_PROMPT: &str = "Click a plane or a flat face to sketch on";
const CHOOSE_PLANE_HINT: &str = "Esc: cancel";
const CHOOSE_REGIONS_PROMPT: &str = "Click regions of the sketch to include or leave them out";
const CHOOSE_REGIONS_HINT: &str = "Esc: done";
const CHOOSE_EDGES_PROMPT: &str = "Click edges to add them or leave them out";
const CHOOSE_FACES_PROMPT: &str = "Click flat faces to open them or close them again";
const CHOOSE_REFERENCES_PROMPT: &str =
    "Select planes, faces, axes or edges, then use them from the feature's panel";
const SNAP_LABEL_OFFSET: egui::Vec2 = vec2(14.0, 10.0);
const KEYBOARD_ORBIT_FRACTION: f64 = 1.0 / 12.0;
const KEYBOARD_PAN_FRACTION: f64 = 0.1;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.25;
const TYPED_POINT_OFFSET: f32 = 64.0;
const TYPE_POINT_HINT: &str = "Type x, y for an exact point";
const TYPED_POINT_HINT: &str = "@ for relative   Enter: place   Esc: cancel";
const NOTHING_TO_HIGHLIGHT: &str = "Nothing in the view can be picked";

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

#[derive(Debug, Clone, Copy, PartialEq)]
struct PickSource {
    cursor: Vector2,
    view: View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Click {
    double: bool,
    primary: bool,
    toggle: bool,
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
    pub pixels_per_point: f32,
}

pub struct ViewportState {
    camera: Camera,
    selection: Selection,
    hovered: Option<Pickable>,
    hover_source: Option<PickSource>,
    pending_click: Option<Click>,
    pointer_hit: Option<PointerHit>,
    cursor: Option<Vector2>,
    rect: Option<Rect>,
    pixels_per_point: f32,
    drag_anchor: Option<Point3>,
    needs_initial_fit: bool,
    fit_requested: bool,
    picks_in_flight: Option<(PickTable, View)>,
    last_pick: Option<PickKey>,
    edited: Option<FeatureId>,
    face_edited_sketch: bool,
    sketch_cursor: Option<Point2>,
    drawing: Drawing,
    annotations: Annotations,
    bodies: BodyMeshes,
    navigation: Navigation,
    keyboard_highlight: Option<Pickable>,
    highlightable: Vec<Pickable>,
    typed_point: TypedPoint,
    hovered_in_tree: Option<Pickable>,
    session: u64,
    fit_when_computed: bool,
    scene_bounds: Option<Aabb>,
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
            hover_source: None,
            pending_click: None,
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
            navigation: Navigation::default(),
            keyboard_highlight: None,
            highlightable: Vec::new(),
            typed_point: TypedPoint::default(),
            hovered_in_tree: None,
            session: 0,
            fit_when_computed: false,
            scene_bounds: None,
        }
    }

    pub fn select_only(&mut self, pickable: Pickable) {
        self.selection.clear();
        self.selection.toggle(pickable);
    }

    pub fn hover_from_tree(&mut self, pickable: Option<Pickable>) {
        self.hovered_in_tree = pickable;
    }

    pub fn forget_document(&mut self) {
        self.selection.clear();
        self.hovered = None;
        self.hover_source = None;
        self.pending_click = None;
        self.pointer_hit = None;
        self.last_pick = None;
        self.keyboard_highlight = None;
        self.highlightable.clear();
        self.drawing = Drawing::default();
        self.annotations = Annotations::default();
        self.typed_point = TypedPoint::default();
    }

    pub fn set_navigation(&mut self, navigation: Navigation) {
        self.navigation = navigation;
        self.camera.set_projection(navigation.projection);
    }

    pub fn edit_dimension(&mut self, feature: FeatureId, constraint: ConstraintId) {
        self.annotations.request_field(feature, constraint);
    }

    pub fn rect(&self) -> Option<Rect> {
        self.rect
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
        let Some((picks, view)) = self.picks_in_flight.take() else {
            return;
        };
        if self.cursor.is_none() {
            return;
        }
        let best = picks.best_hit(result);
        self.hovered = best.map(|(pickable, _)| pickable);
        self.hover_source = Some(PickSource {
            cursor: result.cursor,
            view,
        });
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
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        if commands.available(Command::FitView) {
            self.fit_requested = true;
        }
        if self.session != model.session() {
            self.session = model.session();
            self.fit_when_computed = true;
        }
        if self.fit_when_computed
            && model.status() == RecomputeStatus::UpToDate
            && !model.bodies_pending()
        {
            self.fit_requested = true;
            self.fit_when_computed = false;
        }
        self.keyboard_commands(model, editing, commands, actions);
        let key_hints = KeyHints::new(commands);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            self.rect = Some(rect);
            self.pixels_per_point = ui.ctx().pixels_per_point();
            let response = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());

            self.track_cursor(ui, &response, rect);
            self.type_points(ui, rect, model, editing, keys_free, actions);
            self.track_sketch_cursor(model, editing);
            self.track_drawing(model, editing);
            self.navigate(ui, &response, rect);
            self.click(ui, &response, model, editing, actions);
            if keys_free {
                self.handle_keys(ui, model, editing, actions);
            }
            self.annotate(ui, rect, model, editing, actions);
            self.decorate(ui, rect, model, editing, &key_hints);
        });
    }

    pub fn build_scene(
        &mut self,
        document: &Document,
        evaluation: &Evaluation,
        meshing: &BodyMeshing,
        editing: &SketchEditing,
    ) -> BuiltScene {
        let edited = editing.feature();
        let context = editing.context();
        if edited != self.edited {
            self.edited = edited;
            self.face_edited_sketch = edited.is_some();
            self.last_pick = None;
        }
        self.bodies.update(evaluation, meshing);
        self.bodies
            .update_open(document, evaluation, meshing, context.solid);
        self.selection
            .retain_available(document, evaluation, context);
        self.hovered = self
            .hovered
            .filter(|hovered| hovered.is_available(document, evaluation, context));
        let highlighted = self.highlighted();
        let hovered: Vec<Pickable> = if self.drawing.is_active() {
            edited
                .zip(self.drawing.snap_entity())
                .map(|(feature, entity)| Pickable::SketchEntity { feature, entity })
                .into_iter()
                .collect()
        } else if let Some(annotation) = self.annotations.hovered() {
            annotation.constrained_entities(document)
        } else if let Some(row) = self.hovered_in_tree {
            match row.constrained_entities(document) {
                entities if entities.is_empty() => vec![row],
                entities => entities,
            }
        } else {
            highlighted.into_iter().collect()
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
        let mut seen = std::collections::BTreeSet::new();
        self.highlightable = built
            .picks
            .pickables()
            .filter(|pickable| seen.insert(*pickable))
            .collect();
        self.keyboard_highlight = self
            .keyboard_highlight
            .filter(|highlight| self.highlightable.contains(highlight));
        self.scene_bounds = Some(built.everything);
        let Some(view) = self.view() else {
            return built;
        };
        if self.needs_initial_fit {
            self.camera = Camera::new(view.fitted(built.fit_all()));
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
            self.picks_in_flight = Some((built.picks.clone(), view));
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
            pixels_per_point: scale,
        })
    }

    pub fn pick_was_not_issued(&mut self) {
        self.last_pick = None;
        self.picks_in_flight = None;
    }

    fn view(&self) -> Option<View> {
        let size = self.rect?.size() * self.pixels_per_point;
        let view = (size.x >= 1.0 && size.y >= 1.0)
            .then(|| self.camera.view(f64::from(size.x), f64::from(size.y)))?;
        Some(match self.scene_bounds {
            Some(bounds) => view.reaching(bounds),
            None => view,
        })
    }

    #[cfg(test)]
    pub fn viewpoint(&self) -> Viewpoint {
        self.camera.viewpoint()
    }

    #[cfg(test)]
    pub fn current_view(&self) -> Option<View> {
        self.view()
    }

    #[cfg(test)]
    pub fn keyboard_highlight(&self) -> Option<Pickable> {
        self.keyboard_highlight
    }

    #[cfg(test)]
    pub fn screen_position(&self, plane: Plane, point: Point2) -> Option<egui::Pos2> {
        let pixel = self.view()?.project(plane.to_world(point))? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn hover_through_pick(&mut self, built: &BuiltScene, pickable: Pickable) {
        let (Some(cursor), Some(id), Some(view)) =
            (self.cursor, built.picks.id_of(pickable), self.view())
        else {
            return;
        };
        self.picks_in_flight = Some((built.picks.clone(), view));
        self.apply_pick(&PickResult {
            cursor,
            hits: vec![caditor_render::PickHit {
                id,
                offset_points: 0.0,
                position: Point3::ZERO,
            }],
        });
    }

    fn to_pixels(&self, points: egui::Vec2) -> Vector2 {
        Vector2::new(f64::from(points.x), f64::from(points.y)) * f64::from(self.pixels_per_point)
    }

    fn hit_under(&self, cursor: Vector2) -> Option<Point3> {
        self.pointer_hit
            .filter(|hit| {
                hit.cursor.distance(cursor)
                    <= HIT_CURSOR_TOLERANCE_POINTS * f64::from(self.pixels_per_point)
            })
            .map(|hit| hit.position)
    }

    fn highlighted(&self) -> Option<Pickable> {
        self.keyboard_highlight.or(self.hovered)
    }

    fn track_cursor(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        let (pointer, moved) = ui.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.delta() != egui::Vec2::ZERO,
            )
        });
        let inside = (response.hovered() || response.dragged())
            .then_some(pointer)
            .flatten()
            .filter(|position| rect.contains(*position));
        if moved && inside.is_some() {
            self.keyboard_highlight = None;
        }
        self.cursor = inside.map(|position| self.to_pixels(position - rect.min));
        if self.cursor.is_none() {
            self.hovered = None;
            self.hover_source = None;
            self.pending_click = None;
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
                drag * self.navigation.orbit_speed,
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
        let direction = if self.navigation.invert_zoom {
            1.0
        } else {
            -1.0
        };
        let rate = ZOOM_PER_SCROLL_POINT * self.navigation.zoom_speed;
        let factor = (direction * f64::from(scroll) * rate).exp() / f64::from(pinch);
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
        let drawing = editing.active().is_some_and(|active| active.tool.draws());
        if self.hover_is_current()
            && let Some(click) = self.pending_click.take()
        {
            self.perform_click(click, model, editing, drawing, actions);
        }
        let click = Click {
            double: response.double_clicked(),
            primary: response.clicked_by(PointerButton::Primary),
            toggle: ui.input(|input| input.modifiers.shift || input.modifiers.command),
        };
        if !click.double && !click.primary {
            return;
        }
        if drawing || self.hover_is_current() {
            self.perform_click(click, model, editing, drawing, actions);
        } else {
            self.pending_click = Some(click);
        }
    }

    fn hover_is_current(&self) -> bool {
        let current = self.cursor.zip(self.view());
        self.hover_source
            .is_some_and(|source| current == Some((source.cursor, source.view)))
    }

    fn perform_click(
        &mut self,
        click: Click,
        model: &Model,
        editing: &SketchEditing,
        drawing: bool,
        actions: &mut Vec<Action>,
    ) {
        if click.double
            && editing.feature().is_none()
            && let Some(command) = open_command(self.hovered, model)
        {
            actions.push(Action::Editing(command));
            return;
        }
        if !click.primary {
            return;
        }
        if let Some(action) = pick_action(self.hovered, model, editing) {
            actions.extend(action);
            return;
        }
        if drawing {
            match self.drawing.click(model) {
                Ok(Some(transaction)) => actions.push(Action::Apply(transaction)),
                Ok(None) => {}
                Err(degenerate) => {
                    actions.push(Action::Inform(Notice::info(format!(
                        "{}.",
                        degenerate.reason()
                    ))));
                }
            }
            return;
        }
        self.select(click.toggle);
    }

    fn select(&mut self, toggle: bool) {
        match (self.hovered, toggle) {
            (Some(pickable), true) => self.selection.toggle(pickable),
            (Some(pickable), false) => self.selection.replace_with(pickable),
            (None, false) => self.selection.clear(),
            (None, true) => {}
        }
    }

    fn keyboard_commands(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        for view in StandardView::ALL {
            if commands.available(Command::View(view)) {
                let destination = self.camera.destination();
                if let Some(viewpoint) = Viewpoint::looking_from(
                    view.looking_from(),
                    destination.target,
                    destination.distance,
                ) {
                    self.camera.animate_to(viewpoint);
                }
            }
        }
        for step in CameraMove::ALL {
            if commands.available(Command::Camera(step)) {
                self.nudge(step);
            }
        }
        if commands.available(Command::ToggleProjection) {
            actions.push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Projection(self.navigation.projection.other()),
            )));
        }
        let highlightable = if self.highlightable.is_empty() {
            Err(NOTHING_TO_HIGHLIGHT)
        } else {
            Ok(())
        };
        let steps = [
            (Command::HighlightNext, 1),
            (Command::HighlightPrevious, -1),
        ];
        for (command, step) in steps {
            if commands.invoke(command, &highlightable) {
                self.step_highlight(step);
            }
        }
        if editing.active().is_some() {
            let reversible = self.drawing.reversible();
            if commands.invoke(Command::ReverseArc, &reversible) {
                self.drawing.reverse_arc();
            }
        }
        let activation = self
            .keyboard_highlight
            .ok_or("Highlight an item first, with Highlight the next item in the view");
        if commands.invoke(Command::ActivateHighlighted, &activation)
            && let Some(highlight) = self.keyboard_highlight
        {
            match pick_action(Some(highlight), model, editing) {
                Some(action) => actions.extend(action),
                None => self.selection.toggle(highlight),
            }
        }
    }

    fn step_highlight(&mut self, step: isize) {
        let count = self.highlightable.len();
        if count == 0 {
            return;
        }
        let current = self
            .keyboard_highlight
            .or(self.hovered)
            .and_then(|highlight| {
                self.highlightable
                    .iter()
                    .position(|item| *item == highlight)
            });
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count as isize) as usize,
            None if step < 0 => count - 1,
            None => 0,
        };
        self.keyboard_highlight = self.highlightable.get(next).copied();
    }

    fn nudge(&mut self, step: CameraMove) {
        let Some(view) = self.view() else {
            return;
        };
        let height = view.size().y;
        let target = view.viewpoint().target;
        let orbit = height * KEYBOARD_ORBIT_FRACTION;
        let pan = height * KEYBOARD_PAN_FRACTION;
        let units_per_pixel = view.units_per_pixel_at(view.viewpoint().distance);
        match step {
            CameraMove::OrbitLeft => self.camera.orbit(target, Vector2::new(-orbit, 0.0), height),
            CameraMove::OrbitRight => self.camera.orbit(target, Vector2::new(orbit, 0.0), height),
            CameraMove::OrbitUp => self.camera.orbit(target, Vector2::new(0.0, -orbit), height),
            CameraMove::OrbitDown => self.camera.orbit(target, Vector2::new(0.0, orbit), height),
            CameraMove::PanLeft => self.camera.pan(Vector2::new(-pan, 0.0), units_per_pixel),
            CameraMove::PanRight => self.camera.pan(Vector2::new(pan, 0.0), units_per_pixel),
            CameraMove::PanUp => self.camera.pan(Vector2::new(0.0, -pan), units_per_pixel),
            CameraMove::PanDown => self.camera.pan(Vector2::new(0.0, pan), units_per_pixel),
            CameraMove::ZoomIn => self.camera.zoom(target, 1.0 / KEYBOARD_ZOOM_FACTOR),
            CameraMove::ZoomOut => self.camera.zoom(target, KEYBOARD_ZOOM_FACTOR),
        }
    }

    fn type_points(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
        keys_free: bool,
        actions: &mut Vec<Action>,
    ) {
        let drawing_sketch = editing
            .active()
            .filter(|active| active.tool.draws())
            .and_then(|active| model.document().feature(active.feature))
            .and_then(|feature| scene::displayed_sketch(model.evaluation(), feature));
        let Some(sketch) = drawing_sketch else {
            self.typed_point.close();
            return;
        };
        if keys_free {
            self.typed_point.open_from_typing(ui.ctx());
        }
        let hint = format!("in {}   {TYPED_POINT_HINT}", model.length_unit().symbol());
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let Some(typed) = self.typed_point.show(ui.ctx(), anchor, &hint) else {
            return;
        };
        match typed_point::parse(model, &typed.text, self.drawing.last_placed()) {
            Ok(position) => {
                self.drawing.type_point(&sketch, position);
                match self.drawing.click(model) {
                    Ok(Some(transaction)) => actions.push(Action::Apply(transaction)),
                    Ok(None) => {}
                    Err(degenerate) => self
                        .typed_point
                        .open_with(typed.text, degenerate.reason().to_owned()),
                }
            }
            Err(error) => {
                self.typed_point.open_with(typed.text, error);
            }
        }
    }

    fn handle_keys(
        &mut self,
        ui: &egui::Ui,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let (escape, finish, back) = ui.input(|input| {
            (
                input.key_pressed(Key::Escape),
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Backspace),
            )
        });
        if escape {
            self.escape(editing, actions);
        }
        if finish {
            if let Some(transaction) = self.drawing.finish(model) {
                actions.push(Action::Apply(transaction));
            } else if editing.feature().is_none()
                && let Some(command) = open_command(self.keyboard_highlight, model)
            {
                actions.push(Action::Editing(command));
            }
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
        } else if self.keyboard_highlight.is_some() {
            self.keyboard_highlight = None;
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

    fn decorate(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
    ) {
        let document = model.document();
        let orientation = self.camera.viewpoint().orientation;
        let fit_label = if self.selection.is_empty() {
            "Fit all"
        } else {
            "Fit selection"
        };
        match view_cube::show(ui, rect, orientation, fit_label, &key_hints.fit) {
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
        let hovered = self.annotations.hovered().or(self.highlighted());
        if let Some(hovered) = hovered.filter(|_| !self.drawing.is_active()) {
            let label = canvas::label(
                painter,
                rect.left_top() + vec2(LABEL_MARGIN, LABEL_MARGIN),
                Align2::LEFT_TOP,
                hovered.describe(document, model.evaluation()),
                FontId::proportional(13.0),
                canvas::TEXT,
            );
            if self.keyboard_highlight.is_some() {
                canvas::label(
                    painter,
                    label.left_bottom() + vec2(0.0, LABEL_MARGIN / 3.0),
                    Align2::LEFT_TOP,
                    &key_hints.highlight,
                    FontId::proportional(11.0),
                    canvas::MUTED,
                );
            }
        }
        canvas::wrapped_label(
            painter,
            rect.right_bottom() - vec2(LABEL_MARGIN, LABEL_MARGIN),
            Align2::RIGHT_BOTTOM,
            &key_hints.navigation,
            FontId::proportional(11.0),
            canvas::MUTED,
            rect.width() - view_cube::TRIAD_WIDTH - LABEL_MARGIN,
        );
        let prompt = if editing.is_choosing_plane() {
            Some((CHOOSE_PLANE_PROMPT, CHOOSE_PLANE_HINT.to_owned()))
        } else if let Some(feature) = editing.solid() {
            let kind = document.feature(feature).map(|feature| &feature.kind);
            let prompt = match kind {
                Some(FeatureKind::Blend(_)) => CHOOSE_EDGES_PROMPT,
                Some(FeatureKind::Shell(_)) => CHOOSE_FACES_PROMPT,
                Some(FeatureKind::Datum(_)) => CHOOSE_REFERENCES_PROMPT,
                _ => CHOOSE_REGIONS_PROMPT,
            };
            Some((prompt, CHOOSE_REGIONS_HINT.to_owned()))
        } else {
            self.drawing
                .prompt()
                .filter(|_| !self.typed_point.is_open())
                .map(|prompt| {
                    let reverse = key_hints
                        .reverse
                        .as_ref()
                        .filter(|_| self.drawing.reversible().is_ok())
                        .map(|reverse| format!("{reverse}   "))
                        .unwrap_or_default();
                    (
                        prompt.text,
                        format!("{reverse}{}   {TYPE_POINT_HINT}", prompt.keys),
                    )
                })
        };
        if let Some((text, keys)) = prompt {
            let prompt = canvas::label(
                painter,
                rect.center_top() + vec2(0.0, PROMPT_MARGIN),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(16.0),
                canvas::PROMPT,
            );
            canvas::label(
                painter,
                prompt.center_bottom() + vec2(0.0, LABEL_MARGIN / 2.0),
                Align2::CENTER_TOP,
                keys,
                FontId::proportional(11.0),
                canvas::MUTED,
            );
        }
        let snap_label = editing
            .feature()
            .and_then(|feature| editing::edited_sketch(document, feature))
            .and_then(|sketch| self.drawing.snap_label(sketch));
        if let (Some(label), Some(cursor)) = (snap_label, self.cursor) {
            let position = rect.min
                + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
            canvas::label(
                painter,
                position + SNAP_LABEL_OFFSET,
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.0),
                canvas::SNAP,
            );
        }
        if let Some(position) = self.sketch_cursor {
            canvas::label(
                painter,
                rect.left_bottom() + vec2(LABEL_MARGIN, -LABEL_MARGIN),
                Align2::LEFT_BOTTOM,
                format!(
                    "x {}   y {}",
                    model.length_unit().readout_text(position.x),
                    model.length_unit().readout_text(position.y)
                ),
                FontId::monospace(11.0),
                canvas::TEXT,
            );
        }
    }
}

fn open_command(pickable: Option<Pickable>, model: &Model) -> Option<EditingCommand> {
    match pickable? {
        Pickable::SketchEntity { feature, .. } => Some(EditingCommand::Enter(feature)),
        Pickable::Datum(datum) => Some(EditingCommand::OpenSolid(datum)),
        Pickable::Face { body, face } => model
            .evaluation()
            .body(body)
            .and_then(|solid| bodies::face_origin(solid, face))
            .map(|origin| EditingCommand::OpenSolid(bodies::origin_feature(origin))),
        _ => None,
    }
}

fn pick_action(
    pickable: Option<Pickable>,
    model: &Model,
    editing: &SketchEditing,
) -> Option<Option<Action>> {
    match pickable {
        Some(Pickable::Region { feature, region }) => {
            return Some(solid_tools::toggle_region(model, feature, region).map(Action::Apply));
        }
        Some(Pickable::BlendEdge { feature, edge }) => {
            return Some(blend_tools::toggle_edge(model, feature, edge).map(Action::Apply));
        }
        Some(Pickable::ShellFace { feature, face }) => {
            return Some(shell_tools::toggle_face(model, feature, face).map(Action::Apply));
        }
        _ => {}
    }
    if !editing.is_choosing_plane() {
        return None;
    }
    let command = match pickable {
        Some(Pickable::Plane(plane)) => Some(EditingCommand::NewSketch(Some(plane))),
        Some(Pickable::Datum(datum)) if datum_tools::is_plane(model.document(), datum) => {
            Some(EditingCommand::NewSketchOnDatum(datum))
        }
        Some(pickable) => FaceChoice::of(pickable).map(EditingCommand::NewSketchOnFace),
        None => None,
    };
    Some(command.map(Action::Editing))
}

fn facing(view: &View, sketch: &EditedSketch) -> Viewpoint {
    let size = view.size();
    let facing = Viewpoint::facing(
        &sketch.plane,
        sketch.bounds.center(),
        view.viewpoint().distance,
    );
    View::new(facing, size.x, size.y)
        .with_projection(view.projection())
        .fitted(sketch.bounds)
}

struct KeyHints {
    navigation: String,
    highlight: String,
    fit: String,
    reverse: Option<String>,
}

impl KeyHints {
    fn new(commands: &CommandFrame<'_>) -> Self {
        let navigation = match commands.keys(Command::FitView) {
            Some(keys) => format!("{NAVIGATION_HINT}   {keys}: fit"),
            None => NAVIGATION_HINT.to_owned(),
        };
        let highlight = [
            (Command::ActivateHighlighted, "select or pick"),
            (Command::HighlightNext, "next"),
            (Command::HighlightPrevious, "previous"),
        ]
        .into_iter()
        .filter_map(|(command, what)| Some(format!("{}: {what}", commands.keys(command)?)))
        .chain(["Enter: open   Esc: stop highlighting".to_owned()])
        .collect::<Vec<_>>()
        .join("   ");
        Self {
            navigation,
            highlight,
            fit: commands.with_keys(Command::FitView, "Frame the view around it"),
            reverse: commands
                .keys(Command::ReverseArc)
                .map(|keys| format!("{keys}: the other way round")),
        }
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
        let built = state.build_scene(
            &document,
            &Evaluation::default(),
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );

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
        let built = state.build_scene(
            &document,
            &Evaluation::default(),
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );
        state.request(&built, true);

        let cursor = Vector2::new(120.0, 80.0);
        state.apply_pick(&PickResult {
            cursor,
            hits: vec![PickHit {
                id: PickId::from_index(ORIGIN_PICK_INDEX).unwrap(),
                offset_points: 2.0,
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
        state.build_scene(
            &document,
            &evaluation,
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );
        let entity = Pickable::SketchEntity {
            feature,
            entity: line,
        };
        state.selection.toggle(Pickable::Origin);
        state.selection.toggle(entity);

        let editing = SketchEditing::editing(feature);
        let built = state.build_scene(&document, &evaluation, &BodyMeshing::default(), &editing);
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

        state.build_scene(&document, &evaluation, &BodyMeshing::default(), &editing);
        assert!(!state.is_animating());
        let reference = Pickable::SketchEntity {
            feature,
            entity: caditor_sketch::EntityId::ORIGIN,
        };
        state.selection.toggle(reference);
        state.build_scene(&document, &evaluation, &BodyMeshing::default(), &editing);
        assert!(state.selection.contains(reference));
        state.build_scene(
            &document,
            &evaluation,
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );
        assert_eq!(state.selection.iter().collect::<Vec<_>>(), vec![entity]);
    }

    #[test]
    fn the_first_scene_fits_the_camera_and_later_fits_animate() {
        let document = Document::default();
        let mut state = state_with_cursor();
        let initial = state.camera.viewpoint();
        state.build_scene(
            &document,
            &Evaluation::default(),
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );
        assert_ne!(state.camera.viewpoint(), initial);
        assert!(!state.is_animating());

        state.fit_requested = true;
        state
            .selection
            .replace_with(Pickable::Axis(crate::selection::Axis::X));
        state.build_scene(
            &document,
            &Evaluation::default(),
            &BodyMeshing::default(),
            &SketchEditing::default(),
        );
        assert!(state.is_animating());
        assert!(!state.fit_requested);
    }
}
