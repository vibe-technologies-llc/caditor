use std::{sync::Arc, time::Duration};

use caditor_document::{FeatureId, FeatureKind, Transaction};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Rotation3, Vector2, Vector3};
use caditor_render::{Camera, PickResult, Scene, SurfaceSize, View, Viewpoint, ViewportRect};
use caditor_sketch::{ConstraintId, EntityId};
use egui::{Align, Align2, Key, PointerButton, Rect, Response, Sense, Shape, Stroke, pos2, vec2};

use crate::{
    annotations::{Annotations, Surface},
    blend_tools,
    bodies::{self, BodyMeshes},
    canvas,
    commands::{CameraMove, Command, CommandFrame, StandardView},
    datum_tools,
    display::Displayed,
    drag_solver::DragCommand,
    drawing::Drawing,
    editing::{self, EditingCommand, SketchEditing, Tool},
    faceting::FacetLevel,
    measure::MeasuredLine,
    model::{Action, Model, Notice, RecomputeStatus},
    modifying::{Hint, Modifying, Outcome, Value},
    preferences::{Navigation, PreferenceChange, PreferencesCommand},
    reference_picking,
    scene::{self, BuiltScene, EditedSketch, Highlight, PickTable, SketchShapes, Sources},
    scene_cache::{Overlay, Revisions, SceneCache, SceneInputs},
    selection::{Pickable, Selection},
    shape_modes::ShapeMode,
    shell_tools,
    sketch_drag::{self, BoxMode, Grab, Moving, ScreenBox},
    sketch_placement::FaceChoice,
    sketch_tools,
    snap::{Pointer, Screen},
    solid_tools,
    trimming::Trimming,
    typed_point::{self, TypedPoint},
    view_cube::{self, CubeAction},
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_POINTS: f64 = 1.5;
const PROMPT_MARGIN: f32 = 16.0;
const PROMPT_MAX_WIDTH: f32 = 720.0;
const READOUT_ROOM: f32 = 200.0;
const NAVIGATION_HINT: &str =
    "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom";
pub const CHOOSE_PLANE_PROMPT: &str = "Click a plane or a flat face to sketch on";
const CHOOSE_PLANE_HINT: &str = "Esc: cancel";
const CHOOSE_REGIONS_PROMPT: &str = "Click regions of the sketch to include or leave them out";
const CHOOSE_REGIONS_HINT: &str = "Esc: done";
const CHOOSE_EDGES_PROMPT: &str = "Click edges to add them or leave them out";
const CHOOSE_FACES_PROMPT: &str = "Click flat faces to open them or close them again";
const CHOOSE_REFERENCES_PROMPT: &str = "Select planes, faces, axes or edges for the feature's panel, or choose them in the view from it";
const SNAP_LABEL_OFFSET: egui::Vec2 = vec2(14.0, 10.0);
const KEYBOARD_ORBIT_FRACTION: f64 = 1.0 / 12.0;
const KEYBOARD_PAN_FRACTION: f64 = 0.1;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.25;
const TYPED_POINT_OFFSET: f32 = 64.0;
const TYPE_POINT_HINT: &str = "Type x, y or length < angle for an exact point";
const TYPED_POINT_HINT: &str = "@: from the last point   A length alone goes toward the pointer   \
                                Enter: place   Esc: cancel";
const MOVE_HINT: &str = "@: by an offset   Enter: move   Esc: cancel";
const BOX_FILL_OPACITY: f32 = 0.12;
const BOX_STROKE_WIDTH: f32 = 1.0;
const BOX_DASH: f32 = 6.0;
const BOX_GAP: f32 = 4.0;
const NOTHING_TO_HIGHLIGHT: &str = "Nothing in the view can be picked";
const MEASURE_LABEL_LIFT: f32 = 6.0;
const BACK_TO_SELECT: &str = "Esc: back to Select";
const NO_TARGET_HIGHLIGHTED: &str =
    "Highlight a piece or an end first, with Highlight the next item in the view";

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

#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    cursor: Vector2,
    hovered: Option<Pickable>,
    current: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum PrimaryDrag {
    Grab(Grab),
    Box { feature: FeatureId, area: ScreenBox },
    Trim { feature: FeatureId, from: Point2 },
    Pull { feature: FeatureId },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PickKey {
    cursor: Vector2,
    view: View,
    generation: u64,
}

pub struct ViewportRequest {
    pub view: View,
    pub rect: ViewportRect,
    pub pick_at: Option<Vector2>,
    pub pixels_per_point: f32,
}

pub struct ImageView {
    pub view: View,
    pub scene: Scene,
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
    picks_in_flight: Option<(Arc<PickTable>, View)>,
    last_pick: Option<PickKey>,
    edited: Option<FeatureId>,
    face_edited_sketch: bool,
    sketch_cursor: Option<Point2>,
    drawing: Drawing,
    trimming: Trimming,
    modifying: Modifying,
    annotations: Annotations,
    bodies: BodyMeshes,
    navigation: Navigation,
    keyboard_highlight: Option<Pickable>,
    typed_point: TypedPoint,
    moving: Option<Moving>,
    press: Option<Press>,
    primary: Option<PrimaryDrag>,
    hovered_in_tree: Option<Pickable>,
    session: u64,
    fit_when_computed: bool,
    scene_bounds: Option<Aabb>,
    measured: Option<(MeasuredLine, String)>,
    scenes: SceneCache,
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
            trimming: Trimming::default(),
            modifying: Modifying::default(),
            annotations: Annotations::default(),
            bodies: BodyMeshes::default(),
            navigation: Navigation::default(),
            keyboard_highlight: None,
            typed_point: TypedPoint::default(),
            moving: None,
            press: None,
            primary: None,
            hovered_in_tree: None,
            session: 0,
            fit_when_computed: false,
            scene_bounds: None,
            measured: None,
            scenes: SceneCache::default(),
        }
    }

    pub fn bodies(&self) -> &BodyMeshes {
        &self.bodies
    }

    pub fn set_measured(&mut self, measured: Option<(MeasuredLine, String)>) {
        self.measured = measured;
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
        self.scenes = SceneCache::default();
        self.drawing = Drawing::default();
        self.trimming = Trimming::default();
        self.modifying = Modifying::default();
        self.annotations = Annotations::default();
        self.typed_point = TypedPoint::default();
        self.moving = None;
        self.press = None;
        self.primary = None;
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
            self.track_sketch_cursor(model, editing);
            self.track_drawing(model, editing);
            self.type_points(ui, rect, model, editing, keys_free, actions);
            self.navigate(ui, &response, rect);
            self.drag_primary(ui, &response, model, editing, actions);
            self.click(ui, &response, model, editing, actions);
            if keys_free {
                self.handle_keys(ui, model, editing, actions);
            }
            self.annotate(ui, rect, model, editing, actions);
            self.decorate(ui, rect, model, editing, &key_hints);
        });
    }

    pub fn build_scene(&mut self, model: &Model, editing: &SketchEditing) -> Option<&BuiltScene> {
        let document = model.document();
        let evaluation = model.evaluation();
        let display = model.display();
        let edited = editing.feature();
        let context = editing.context();
        if edited != self.edited {
            self.edited = edited;
            self.face_edited_sketch = edited.is_some();
            self.last_pick = None;
        }
        self.bodies.update(evaluation, &display.meshing);
        self.bodies
            .update_open(document, evaluation, &display.meshing, context.solid);
        self.selection
            .retain_available(document, evaluation, context);
        self.hovered = self
            .hovered
            .filter(|hovered| hovered.is_available(document, evaluation, context));
        let highlighted = self.highlighted();
        let hovered: Vec<Pickable> = if self.drawing.is_active() {
            edited
                .into_iter()
                .flat_map(|feature| {
                    self.drawing
                        .snap_entities()
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
                .collect()
        } else if self.trimming.is_active() {
            edited
                .into_iter()
                .flat_map(|feature| {
                    self.trimming
                        .highlighted_entities()
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
                .collect()
        } else if self.modifying.is_active() {
            edited
                .into_iter()
                .flat_map(|feature| {
                    self.modifying
                        .highlighted_entities()
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
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
        let view = self.view();
        let sources = Sources {
            document,
            evaluation,
            bodies: &self.bodies,
            sketches: &display.sketches,
        };
        self.scenes.update(&SceneInputs {
            sources: &sources,
            revisions: Revisions {
                document: model.revision(),
                evaluation: model.evaluation_generation(),
                sketches: display.sketches.generation(),
                bodies: self.bodies.generation(),
            },
            context,
            highlight: Highlight {
                selection: &self.selection,
                hovered: &hovered,
            },
            view: view.as_ref(),
        });
        let faceting = self.scenes.faceting();
        let plane = self.scenes.edited_plane();
        self.scenes.show(Overlay {
            plane,
            previews: vec![
                self.drawing.preview(faceting),
                self.trimming.preview(faceting),
                self.modifying.preview(faceting),
            ],
            measured: self.measured.as_ref().map(|(line, _)| [line.from, line.to]),
        });
        if let Some(highlight) = self.keyboard_highlight
            && !self.scenes.highlightable().contains(&highlight)
        {
            self.keyboard_highlight = None;
        }
        let built = self.scenes.built()?;
        self.scene_bounds = Some(built.everything);
        let Some(view) = self.view() else {
            return Some(built);
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
        Some(built)
    }

    pub fn scene(&self) -> Option<&Scene> {
        self.scenes.built().map(|built| &built.scene)
    }

    pub fn request(&mut self, can_pick: bool) -> Option<ViewportRequest> {
        let rect = self.rect?;
        let view = self.view()?;
        let generation = self.scenes.generation();
        let pick_at = self.cursor.filter(|_| can_pick).and_then(|cursor| {
            let key = PickKey {
                cursor,
                view,
                generation,
            };
            if self.last_pick == Some(key) {
                return None;
            }
            let picks = Arc::clone(&self.scenes.built()?.picks);
            self.last_pick = Some(key);
            self.picks_in_flight = Some((picks, view));
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

    pub fn view_pixels(&self) -> Option<SurfaceSize> {
        let size = self.rect?.size() * self.pixels_per_point;
        let side = |length: f32| {
            let rounded = length.round();
            (rounded >= 1.0).then_some(rounded as u32)
        };
        Some(SurfaceSize {
            width: side(size.x)?,
            height: side(size.y)?,
        })
    }

    pub fn image(&self, model: &Model, editing: &SketchEditing, size: SurfaceSize) -> ImageView {
        let sources = Sources {
            document: model.document(),
            evaluation: model.evaluation(),
            bodies: &self.bodies,
            sketches: &model.display().sketches,
        };
        let context = editing.context();
        let view = self
            .camera
            .view(f64::from(size.width), f64::from(size.height));
        let level = FacetLevel::following(self.scenes.level(), FacetLevel::wanted_chord(&view));
        let unselected = Selection::default();
        let mut built = scene::build(
            &sources,
            &Highlight {
                selection: &unselected,
                hovered: &[],
            },
            context,
            &mut SketchShapes::new(scene::drawn_faceting(&sources, context, level.faceting())),
        );
        built.scene.grid = None;
        let view = view.reaching(built.everything);
        let shown_height = self.view_pixels().map_or(size.height, |shown| shown.height);
        ImageView {
            view,
            scene: built.scene,
            pixels_per_point: self.pixels_per_point * size.height as f32 / shown_height as f32,
        }
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
    pub fn hover_through_pick(&mut self, pickable: Pickable) {
        let picks = self.scenes.built().map(|built| Arc::clone(&built.picks));
        let (Some(cursor), Some(picks), Some(view)) = (self.cursor, picks, self.view()) else {
            return;
        };
        let Some(id) = picks.id_of(pickable) else {
            return;
        };
        self.picks_in_flight = Some((picks, view));
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
            .filter(|position| position.is_finite())
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
            self.trimming.clear_highlight();
            self.modifying.clear_highlight();
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
        self.sketch_cursor = self
            .cursor
            .and_then(|cursor| self.on_sketch(model, editing.feature()?, cursor));
    }

    fn on_sketch(&self, model: &Model, feature: FeatureId, cursor: Vector2) -> Option<Point2> {
        let plane = scene::sketch_plane(model.document(), model.evaluation(), feature)?;
        let ray = self.view()?.ray_through(cursor)?;
        let distance = ray.intersect_plane(&plane)?;
        Some(plane.to_local(ray.at(distance)))
    }

    fn sketch_screen(&self, plane: Plane) -> Option<SketchScreen> {
        Some(SketchScreen {
            view: self.view()?,
            plane,
            pixels_per_point: f64::from(self.pixels_per_point),
        })
    }

    fn drag_primary(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let (pressed, released, toggle) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.modifiers.shift || input.modifiers.command,
            )
        });
        if pressed && response.hovered() {
            self.press = self.cursor.map(|cursor| Press {
                cursor,
                hovered: self.hovered,
                current: self.hover_is_current(),
            });
        } else if let Some(press) = &mut self.press
            && !press.current
            && self
                .hover_source
                .is_some_and(|source| source.cursor == press.cursor)
        {
            press.hovered = self.hovered;
            press.current = true;
        }
        if response.drag_started_by(PointerButton::Primary) {
            self.primary = self
                .press
                .take()
                .and_then(|press| self.begin_primary(press, model, editing));
            if let Some(PrimaryDrag::Trim { from, .. }) = &self.primary {
                self.trimming.begin_path(*from);
            }
            if matches!(self.primary, Some(PrimaryDrag::Pull { .. }))
                && !self.modifying.begin_pull()
            {
                self.primary = None;
            }
        }
        let edited = editing
            .active()
            .filter(|active| !active.tool.draws())
            .map(|active| active.feature);
        let cursor = self.cursor;
        let sketch_cursor = self.sketch_cursor;
        match &mut self.primary {
            Some(PrimaryDrag::Grab(grab)) if Some(grab.feature()) != edited => {
                self.primary = None;
                actions.push(Action::Drag(DragCommand::Cancel));
            }
            Some(PrimaryDrag::Box { feature, .. }) if Some(*feature) != edited => {
                self.primary = None;
            }
            Some(PrimaryDrag::Trim { feature, .. }) if Some(*feature) != edited => {
                self.primary = None;
                self.trimming.cancel_path();
            }
            Some(PrimaryDrag::Pull { feature }) if Some(*feature) != edited => {
                self.primary = None;
            }
            Some(PrimaryDrag::Grab(grab)) => {
                if let Some(command) = sketch_cursor.and_then(|at| grab.to(at)) {
                    actions.push(Action::Drag(command));
                }
            }
            Some(PrimaryDrag::Box { area, .. }) => {
                if let Some(cursor) = cursor {
                    area.to = cursor / f64::from(self.pixels_per_point);
                }
            }
            Some(PrimaryDrag::Trim { .. } | PrimaryDrag::Pull { .. }) | None => {}
        }
        if released {
            self.press = None;
            match self.primary.take() {
                Some(PrimaryDrag::Grab(grab)) if grab.has_moved() => {
                    actions.push(Action::Drag(DragCommand::Finish));
                }
                Some(PrimaryDrag::Box { feature, area }) => {
                    self.select_within(model, feature, area, toggle);
                }
                Some(PrimaryDrag::Trim { .. }) => {
                    actions.extend(outcome_action(self.trimming.finish_path(model)));
                }
                Some(PrimaryDrag::Pull { .. }) => {
                    let outcome = match edited_sketch(model, editing) {
                        Some(sketch) => self.modifying.click(model, &sketch),
                        None => Outcome::Nothing,
                    };
                    self.modify(editing, outcome, actions);
                }
                Some(PrimaryDrag::Grab(_)) | None => {}
            }
        }
    }

    fn begin_primary(
        &self,
        press: Press,
        model: &Model,
        editing: &SketchEditing,
    ) -> Option<PrimaryDrag> {
        let active = editing.active().filter(|active| !active.tool.draws())?;
        let feature = active.feature;
        match active.tool {
            Tool::Trim => {
                let from = self.on_sketch(model, feature, press.cursor)?;
                return Some(PrimaryDrag::Trim { feature, from });
            }
            Tool::Offset | Tool::Fillet => return Some(PrimaryDrag::Pull { feature }),
            Tool::Extend | Tool::Mirror => return None,
            _ => {}
        }
        let grabbed = match press.hovered {
            Some(Pickable::SketchEntity {
                feature: owner,
                entity,
            }) if owner == feature && !entity.is_reference() => Some(entity),
            _ => None,
        };
        let Some(grabbed) = grabbed else {
            let at = press.cursor / f64::from(self.pixels_per_point);
            return Some(PrimaryDrag::Box {
                feature,
                area: ScreenBox { from: at, to: at },
            });
        };
        let owner = model.document().feature(feature)?;
        let sketch = model.displayed_sketch(owner)?;
        let from = self.on_sketch(model, feature, press.cursor)?;
        let selected = sketch_tools::selected_entities(&self.selection, feature);
        Grab::of(&sketch, feature, grabbed, &selected, from).map(PrimaryDrag::Grab)
    }

    fn select_within(&mut self, model: &Model, feature: FeatureId, area: ScreenBox, toggle: bool) {
        let Some(owner) = model.document().feature(feature) else {
            return;
        };
        let Some(sketch) = model.displayed_sketch(owner) else {
            return;
        };
        let Some(screen) = self.sketch_screen(sketch.plane()) else {
            return;
        };
        let caught = sketch_drag::within(&sketch, &screen, area, self.scenes.faceting());
        self.add_to_selection(feature, caught, toggle);
    }

    fn add_to_selection(&mut self, feature: FeatureId, entities: Vec<EntityId>, keep: bool) {
        if !keep {
            self.selection.clear();
        }
        for entity in entities {
            let pickable = Pickable::SketchEntity { feature, entity };
            if !self.selection.contains(pickable) {
                self.selection.toggle(pickable);
            }
        }
    }

    fn track_drawing(&mut self, model: &Model, editing: &SketchEditing) {
        let displayed = editing
            .feature()
            .and_then(|feature| model.document().feature(feature))
            .and_then(|feature| model.displayed_sketch(feature));
        self.drawing
            .sync(editing.active(), editing.modes(), displayed.as_deref());
        self.trimming.sync(editing.active(), displayed.as_deref());
        let selected = editing
            .feature()
            .map(|feature| sketch_tools::selected_entities(&self.selection, feature))
            .unwrap_or_default();
        self.modifying
            .sync(editing.active(), displayed.as_deref(), &selected);
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
                self.trimming.hover(&sketch, &screen, pointer);
                self.modifying
                    .hover(&sketch, &screen, pointer, self.scenes.faceting());
            }
            _ => {
                self.drawing.leave();
                self.trimming.leave();
                self.modifying.leave();
            }
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
        let drawing = editing
            .active()
            .is_some_and(|active| active.tool.draws() || active.tool.modifies());
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
        if self.trimming.is_active() {
            actions.extend(outcome_action(self.trimming.click(model)));
            return;
        }
        if self.modifying.is_active() {
            let outcome = match edited_sketch(model, editing) {
                Some(sketch) => self.modifying.click(model, &sketch),
                None => Outcome::Nothing,
            };
            self.modify(editing, outcome, actions);
            return;
        }
        if drawing {
            match self.drawing.click(model) {
                Ok(Some(transaction)) => actions.push(Action::Apply(transaction)),
                Ok(None) => {}
                Err(refusal) => {
                    actions.push(Action::Inform(Notice::info(format!(
                        "{}.",
                        refusal.reason()
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
        let trims = self.trimming.is_active();
        let targets = (trims || self.modifying.steps_targets())
            .then(|| edited_sketch(model, editing))
            .flatten();
        let highlightable = match &targets {
            Some(sketch) if trims => self.trimming.steppable(sketch),
            Some(sketch) => self.modifying.steppable(sketch),
            None if !self.scenes.has_pickables() => Err(NOTHING_TO_HIGHLIGHT),
            None => Ok(()),
        };
        let steps = [
            (Command::HighlightNext, 1),
            (Command::HighlightPrevious, -1),
        ];
        for (command, step) in steps {
            if commands.invoke(command, &highlightable) {
                match &targets {
                    Some(sketch) if trims => self.trimming.step(sketch, step),
                    Some(sketch) => self.modifying.step(sketch, step),
                    None => self.step_highlight(step),
                }
            }
        }
        if let Some(active) = editing.active() {
            let reversible = self.drawing.reversible();
            if commands.invoke(Command::ReverseArc, &reversible) {
                self.drawing.reverse_arc();
            }
            let sides = [
                (Command::MoreSides, self.drawing.more_sides(), 1),
                (Command::FewerSides, self.drawing.fewer_sides(), -1),
            ];
            for (command, availability, change) in sides {
                if commands.invoke(command, &availability) {
                    self.drawing.add_sides(change);
                }
            }
            self.sketch_commands(model, active.feature, active.tool.draws(), commands);
        }
        if self.trimming.is_active() {
            let activation = if self.trimming.has_highlight() {
                Ok(())
            } else {
                Err(NO_TARGET_HIGHLIGHTED)
            };
            if commands.invoke(Command::ActivateHighlighted, &activation) {
                actions.extend(outcome_action(self.trimming.activate(model)));
            }
            return;
        }
        if self.modifying.steps_targets() {
            let activation = self.modifying.highlight_needed();
            if commands.invoke(Command::ActivateHighlighted, &activation) {
                let outcome = self.modifying.activate(model);
                self.modify(editing, outcome, actions);
            }
            return;
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

    fn sketch_commands(
        &mut self,
        model: &Model,
        feature: FeatureId,
        drawing: bool,
        commands: &mut CommandFrame<'_>,
    ) {
        let Some(owner) = model.document().feature(feature) else {
            return;
        };
        let Some(sketch) = model.displayed_sketch(owner) else {
            return;
        };
        let selected = sketch_tools::selected_entities(&self.selection, feature);
        let moving = Moving::offered(&sketch, feature, &selected, drawing);
        if commands.invoke(Command::MoveGeometry, &moving)
            && let Ok(moving) = moving
        {
            self.moving = Some(moving);
            self.typed_point.open();
        }
        let everything = sketch_drag::select_all(&sketch);
        if commands.invoke(Command::SelectAll, &everything)
            && let Ok(everything) = everything
        {
            self.add_to_selection(feature, everything, false);
        }
    }

    fn step_highlight(&mut self, step: isize) {
        let highlightable = self.scenes.highlightable();
        let count = highlightable.len();
        if count == 0 {
            return;
        }
        let current = self
            .keyboard_highlight
            .or(self.hovered)
            .and_then(|highlight| highlightable.iter().position(|item| *item == highlight));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count as isize) as usize,
            None if step < 0 => count - 1,
            None => 0,
        };
        self.keyboard_highlight = highlightable.get(next).copied();
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
        self.moving = self
            .moving
            .take()
            .filter(|moving| editing.feature() == Some(moving.feature));
        if self.moving.is_some() {
            self.type_move(ui, rect, model, actions);
            return;
        }
        if self.modifying.value_field().is_some() {
            self.type_value(ui, rect, model, editing, keys_free, actions);
            return;
        }
        let drawing_sketch = editing
            .active()
            .filter(|active| active.tool.draws())
            .and_then(|active| model.document().feature(active.feature))
            .and_then(|feature| model.displayed_sketch(feature));
        let Some(sketch) = drawing_sketch else {
            self.typed_point.close();
            return;
        };
        if keys_free {
            self.typed_point.open_from_typing(ui.ctx());
        }
        let hint = format!(
            "Lengths in {}   {TYPED_POINT_HINT}",
            model.length_unit().symbol()
        );
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let Some(typed) = self.typed_point.show(
            ui.ctx(),
            top_band(rect),
            anchor,
            typed_point::FIELD_LABEL,
            &hint,
            typed_point::POINT_PLACEHOLDER,
        ) else {
            return;
        };
        let from = typed_point::From {
            last: self.drawing.last_placed(),
            toward: self.drawing.pointer_position(),
        };
        match typed_point::parse(model, &typed.text, from) {
            Ok(position) => {
                self.drawing.type_point(&sketch, position);
                match self.drawing.click(model) {
                    Ok(Some(transaction)) => actions.push(Action::Apply(transaction)),
                    Ok(None) => {}
                    Err(refusal) => self
                        .typed_point
                        .open_with(typed.text, refusal.reason().to_owned()),
                }
            }
            Err(error) => {
                self.typed_point.open_with(typed.text, error);
            }
        }
    }

    fn type_value(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
        keys_free: bool,
        actions: &mut Vec<Action>,
    ) {
        let Some(field) = self.modifying.value_field() else {
            return;
        };
        if keys_free {
            self.typed_point.open_from_typing(ui.ctx());
        }
        let shown = self
            .typed_point
            .text()
            .and_then(|text| Value::typed(model, text).ok())
            .map(|value| value.millimetres);
        self.modifying.show_typed(shown);
        let hint = format!(
            "Lengths in {}   Enter: {}   Esc: cancel",
            model.length_unit().symbol(),
            field.action
        );
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let Some(typed) = self.typed_point.show(
            ui.ctx(),
            top_band(rect),
            anchor,
            field.label,
            &hint,
            field.placeholder,
        ) else {
            return;
        };
        self.modifying.show_typed(None);
        let entered = Value::typed(model, &typed.text)
            .and_then(|value| self.modifying.enter_value(model, value));
        match entered {
            Ok(outcome) => self.modify(editing, outcome, actions),
            Err(error) => self.typed_point.open_with(typed.text, error),
        }
    }

    fn modify(&mut self, editing: &SketchEditing, outcome: Outcome, actions: &mut Vec<Action>) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Apply(transaction) => actions.push(Action::Apply(transaction)),
            Outcome::Select(entities) => {
                if let Some(feature) = editing.feature() {
                    self.add_to_selection(feature, entities, false);
                }
            }
            Outcome::Refused(reason) => actions.push(Action::Inform(Notice::info(reason))),
        }
    }

    fn type_move(&mut self, ui: &egui::Ui, rect: Rect, model: &Model, actions: &mut Vec<Action>) {
        let hint = format!("Lengths in {}   {MOVE_HINT}", model.length_unit().symbol());
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let typed = self.typed_point.show(
            ui.ctx(),
            top_band(rect),
            anchor,
            typed_point::MOVE_LABEL,
            &hint,
            typed_point::POINT_PLACEHOLDER,
        );
        let Some(moving) = self.moving.take() else {
            return;
        };
        let Some(typed) = typed else {
            if self.typed_point.is_open() {
                self.moving = Some(moving);
            }
            return;
        };
        let from = typed_point::From {
            last: Some(moving.anchor),
            toward: None,
        };
        match typed_point::parse(model, &typed.text, from) {
            Ok(target) => actions.extend(moving.to(target).into_iter().map(Action::Drag)),
            Err(error) => {
                self.typed_point.open_with(typed.text, error);
                self.moving = Some(moving);
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
            if self.trimming.is_active() {
                actions.extend(outcome_action(self.trimming.activate(model)));
            } else if self.modifying.is_active() {
                let outcome = match edited_sketch(model, editing) {
                    Some(sketch) => self.modifying.finish(model, &sketch),
                    None => Outcome::Nothing,
                };
                self.modify(editing, outcome, actions);
            } else if let Some(transaction) = self.drawing.finish(model) {
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
        if let Some(primary) = self.primary.take() {
            match primary {
                PrimaryDrag::Grab(_) => actions.push(Action::Drag(DragCommand::Cancel)),
                PrimaryDrag::Trim { .. } => self.trimming.cancel_path(),
                PrimaryDrag::Box { .. } | PrimaryDrag::Pull { .. } => {}
            }
        } else if editing.is_choosing_plane() {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        } else if editing.picking().is_some() {
            actions.push(Action::Editing(EditingCommand::StopPicking));
        } else if self.drawing.in_progress() {
            self.drawing.cancel();
        } else if self.keyboard_highlight.is_some() {
            self.keyboard_highlight = None;
        } else if self.trimming.has_highlight() {
            self.trimming.clear_highlight();
        } else if self.modifying.can_back_out() {
            self.modifying.back_out();
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
            interactive: !self.drawing.is_active()
                && !self.trimming.is_active()
                && !self.modifying.is_active(),
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
        if let Some(PrimaryDrag::Box { area, .. }) = &self.primary {
            paint_box(painter, rect, *area);
        }
        if let Some((line, label)) = &self.measured
            && let Some(view) = self.view()
            && let Some(pixel) = view.project(line.from.midpoint(line.to))
        {
            let position =
                rect.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32) / self.pixels_per_point;
            if rect.contains(position) {
                canvas::label(
                    painter,
                    position + vec2(0.0, -MEASURE_LABEL_LIFT),
                    Align2::CENTER_BOTTOM,
                    label,
                    canvas::body(),
                    canvas::MEASURE,
                );
            }
        }
        let band = top_band(rect);
        let prompt = self
            .prompt(model, editing, key_hints)
            .map(|(text, keys)| paint_prompt(painter, rect, band, &text, &keys));
        self.paint_description(painter, model, editing, key_hints, band, prompt);
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
                canvas::small(),
                canvas::SNAP,
            );
        }
        let readout_left = rect.left() + view_cube::TRIAD_WIDTH;
        if let Some(position) = self.sketch_cursor {
            canvas::label(
                painter,
                pos2(readout_left, rect.bottom() - canvas::MARGIN),
                Align2::LEFT_BOTTOM,
                format!(
                    "x {}   y {}",
                    model.length_unit().readout_text(position.x),
                    model.length_unit().readout_text(position.y)
                ),
                canvas::readout(),
                canvas::TEXT,
            );
        }
        let hints_left = if editing.feature().is_some() {
            readout_left + READOUT_ROOM
        } else {
            readout_left
        };
        let hints = canvas::Hints::new(
            painter,
            &key_hints.navigation,
            rect.right() - canvas::MARGIN - hints_left,
            Align::Max,
        );
        let anchor = rect.right_bottom() - vec2(canvas::MARGIN, canvas::MARGIN);
        let placed = Align2::RIGHT_BOTTOM.anchor_size(anchor, hints.size());
        let clear = !placed.intersects(view_cube::area(rect))
            && prompt.is_none_or(|prompt| !placed.intersects(prompt))
            && placed.left() >= hints_left
            && placed.top() >= rect.top();
        if clear {
            hints.paint(painter, placed.min);
        }
    }

    fn prompt(
        &self,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
    ) -> Option<(String, String)> {
        let document = model.document();
        if editing.is_choosing_plane() {
            Some((CHOOSE_PLANE_PROMPT.to_owned(), CHOOSE_PLANE_HINT.to_owned()))
        } else if let Some(picking) = editing.picking() {
            Some((
                reference_picking::prompt(model, picking),
                reference_picking::STOP_HINT.to_owned(),
            ))
        } else if let Some(feature) = editing.solid() {
            let kind = document.feature(feature).map(|feature| &feature.kind);
            let prompt = match kind {
                Some(FeatureKind::Blend(_)) => CHOOSE_EDGES_PROMPT,
                Some(FeatureKind::Shell(_)) => CHOOSE_FACES_PROMPT,
                Some(FeatureKind::Datum(_) | FeatureKind::Pattern(_)) => CHOOSE_REFERENCES_PROMPT,
                _ => CHOOSE_REGIONS_PROMPT,
            };
            Some((prompt.to_owned(), CHOOSE_REGIONS_HINT.to_owned()))
        } else if let Some(prompt) = self.trimming.prompt() {
            Some((prompt.to_owned(), key_hints.targets.clone()))
        } else if let Some(prompt) = self.modifying.prompt() {
            (!self.typed_point.is_open()).then(|| {
                let keys = match prompt.hint {
                    Hint::Targets => key_hints.targets.clone(),
                    Hint::Keys(keys) => keys.to_owned(),
                };
                (prompt.text.to_owned(), keys)
            })
        } else {
            self.drawing
                .prompt()
                .filter(|_| !self.typed_point.is_open())
                .map(|prompt| {
                    let mode = self
                        .drawing
                        .mode()
                        .map(|mode| format!("{}   ", key_hints.mode(mode)))
                        .unwrap_or_default();
                    let reverse = key_hints
                        .reverse
                        .as_ref()
                        .filter(|_| self.drawing.reversible().is_ok())
                        .map(|reverse| format!("{reverse}   "))
                        .unwrap_or_default();
                    let sides = key_hints
                        .sides
                        .as_ref()
                        .filter(|_| {
                            self.drawing.more_sides().is_ok() || self.drawing.fewer_sides().is_ok()
                        })
                        .map(|sides| format!("{sides}   "))
                        .unwrap_or_default();
                    (
                        prompt.text,
                        format!("{mode}{reverse}{sides}{}   {TYPE_POINT_HINT}", prompt.keys),
                    )
                })
        }
    }

    fn paint_description(
        &self,
        painter: &egui::Painter,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
        band: Rect,
        prompt: Option<Rect>,
    ) {
        let hovered = self.annotations.hovered().or(self.highlighted());
        let description = if self.trimming.is_active() {
            edited_sketch(model, editing).and_then(|sketch| self.trimming.label(&sketch))
        } else if self.modifying.is_active() {
            edited_sketch(model, editing)
                .and_then(|sketch| self.modifying.label(&sketch, model.length_unit()))
        } else {
            hovered
                .filter(|_| !self.drawing.is_active())
                .map(|hovered| hovered.describe(model.document(), model.evaluation()))
        };
        let Some(description) = description else {
            return;
        };
        let label = canvas::Label::new(
            painter,
            description,
            canvas::body(),
            canvas::TEXT,
            band.width(),
        );
        let mut min = band.left_top();
        if let Some(prompt) = prompt
            && Rect::from_min_size(min, label.size()).intersects(prompt)
        {
            min.y = prompt.bottom() + canvas::MARGIN / 2.0;
        }
        let shown = label.paint(painter, min);
        if self.keyboard_highlight.is_some() {
            canvas::Hints::new(painter, &key_hints.highlight, band.width(), Align::Min).paint(
                painter,
                shown.left_bottom() + vec2(0.0, canvas::MARGIN / 3.0),
            );
        }
    }
}

fn top_band(rect: Rect) -> Rect {
    let right = view_cube::area(rect).left() - canvas::MARGIN;
    let left = rect.left() + canvas::MARGIN;
    Rect::from_min_max(
        pos2(left, rect.top() + canvas::MARGIN),
        pos2(right.max(left), rect.bottom()),
    )
}

fn paint_prompt(painter: &egui::Painter, rect: Rect, band: Rect, text: &str, keys: &str) -> Rect {
    let width = band.width().min(PROMPT_MAX_WIDTH);
    let title = canvas::Label::new(painter, text, canvas::title(), canvas::PROMPT, width);
    let hints = canvas::Hints::new(painter, keys, width, Align::Center);
    let half = title.size().x.max(hints.size().x) / 2.0;
    let x = rect
        .center()
        .x
        .min(band.right() - half)
        .max(band.left() + half);
    let shown = title.paint_at(
        painter,
        pos2(x, rect.top() + PROMPT_MARGIN),
        Align2::CENTER_TOP,
    );
    let hinted = hints.paint_at(
        painter,
        pos2(x, shown.bottom() + canvas::MARGIN / 2.0),
        Align2::CENTER_TOP,
    );
    shown.union(hinted)
}

fn paint_box(painter: &egui::Painter, rect: Rect, area: ScreenBox) {
    let corner = |at: Vector2| rect.min + egui::Vec2::new(at.x as f32, at.y as f32);
    let drawn = Rect::from_two_pos(corner(area.from), corner(area.to));
    match area.mode() {
        BoxMode::Window => {
            painter.rect_filled(
                drawn,
                0.0,
                canvas::SELECTED.gamma_multiply(BOX_FILL_OPACITY),
            );
            painter.rect_stroke(
                drawn,
                0.0,
                Stroke::new(BOX_STROKE_WIDTH, canvas::SELECTED),
                egui::StrokeKind::Inside,
            );
        }
        BoxMode::Crossing => {
            painter.rect_filled(drawn, 0.0, canvas::SNAP.gamma_multiply(BOX_FILL_OPACITY));
            let outline = [
                drawn.left_top(),
                drawn.right_top(),
                drawn.right_bottom(),
                drawn.left_bottom(),
                drawn.left_top(),
            ];
            painter.extend(Shape::dashed_line(
                &outline,
                Stroke::new(BOX_STROKE_WIDTH, canvas::SNAP),
                BOX_DASH,
                BOX_GAP,
            ));
        }
    }
}

fn edited_sketch<'a>(model: &'a Model, editing: &SketchEditing) -> Option<Displayed<'a>> {
    let feature = model.document().feature(editing.feature()?)?;
    model.displayed_sketch(feature)
}

fn outcome_action(outcome: Result<Option<Transaction>, String>) -> Option<Action> {
    match outcome {
        Ok(transaction) => transaction.map(Action::Apply),
        Err(reason) => Some(Action::Inform(Notice::info(reason))),
    }
}

fn open_command(pickable: Option<Pickable>, model: &Model) -> Option<EditingCommand> {
    match pickable? {
        Pickable::SketchEntity { feature, .. } => Some(EditingCommand::Enter(feature)),
        Pickable::Datum(datum) => Some(EditingCommand::OpenSolid(datum)),
        Pickable::Face { body, face } => bodies::shown(model.evaluation(), body)
            .and_then(|shown| bodies::face_origin(shown, face))
            .map(|origin| EditingCommand::OpenSolid(bodies::origin_feature(origin))),
        _ => None,
    }
}

fn pick_action(
    pickable: Option<Pickable>,
    model: &Model,
    editing: &SketchEditing,
) -> Option<Vec<Action>> {
    if let Some(picking) = editing.picking() {
        return Some(
            pickable
                .map(|pickable| reference_picking::click(model, picking, pickable))
                .unwrap_or_default(),
        );
    }
    let toggled = match pickable {
        Some(Pickable::Region { feature, region }) => {
            Some(solid_tools::toggle_region(model, feature, region))
        }
        Some(Pickable::BlendEdge { feature, edge }) => {
            Some(blend_tools::toggle_edge(model, feature, edge))
        }
        Some(Pickable::ShellFace { feature, face }) => {
            Some(shell_tools::toggle_face(model, feature, face))
        }
        _ => None,
    };
    if let Some(toggled) = toggled {
        return Some(toggled.map(Action::Apply).into_iter().collect());
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
    Some(command.map(Action::Editing).into_iter().collect())
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
    sides: Option<String>,
    targets: String,
    tools: Vec<(Tool, String)>,
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
        let targets = [
            (Command::HighlightNext, "next"),
            (Command::HighlightPrevious, "previous"),
        ]
        .into_iter()
        .filter_map(|(command, what)| Some(format!("{}: {what}", commands.keys(command)?)))
        .chain([format!(
            "Enter: act on the highlighted one   {BACK_TO_SELECT}"
        )])
        .collect::<Vec<_>>()
        .join("   ");
        Self {
            navigation,
            highlight,
            targets,
            fit: commands.with_keys(Command::FitView, "Frame the view around it"),
            reverse: commands
                .keys(Command::ReverseArc)
                .map(|keys| format!("{keys}: the other way round")),
            sides: commands
                .keys(Command::MoreSides)
                .zip(commands.keys(Command::FewerSides))
                .map(|(more, fewer)| format!("{more} or {fewer}: more or fewer sides")),
            tools: Tool::ALL
                .into_iter()
                .filter_map(|tool| Some((tool, commands.keys(Command::SketchTool(tool))?)))
                .collect(),
        }
    }

    fn mode(&self, mode: ShapeMode) -> String {
        let keys = self
            .tools
            .iter()
            .find(|(tool, _)| *tool == mode.tool())
            .map(|(_, keys)| keys.as_str());
        mode.hint(keys)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use caditor_document::{Document, Edit};
    use caditor_file::StorageConfig;
    use caditor_kernel::Accuracy;
    use caditor_render::{Batch, Line, PickHit, PickId};
    use caditor_sketch::Sketch;
    use egui::pos2;

    use super::*;
    use crate::model::Services;

    const ORIGIN_PICK_INDEX: usize = 6;
    const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(60);

    fn state_with_cursor() -> ViewportState {
        let mut state = ViewportState::new();
        state.rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)));
        state.cursor = Some(Vector2::new(120.0, 80.0));
        state
    }

    pub fn model_of(document: Document) -> Model {
        Model::new(
            document,
            Services {
                make_waker: Box::new(|| Box::new(|| {})),
                storage: StorageConfig { recovery_dir: None },
                panic_flush: Arc::default(),
            },
        )
    }

    pub fn settle(model: &mut Model) {
        let deadline = Instant::now() + RECOMPUTE_TIMEOUT;
        while matches!(model.status(), RecomputeStatus::Running { .. }) || model.bodies_pending() {
            assert!(Instant::now() < deadline, "the recompute did not finish");
            model.poll();
            std::thread::yield_now();
        }
    }

    fn sketched(plane: Plane) -> (Document, FeatureId, EntityId, EntityId) {
        let mut document = Document::default();
        let mut sketch = Sketch::new(plane);
        let line = sketch.add_line(Point2::new(10.0, 0.0), Point2::new(30.0, 20.0));
        let circle = sketch.add_circle(Point2::new(-20.0, 5.0), 8.0);
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Side", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature, line, circle)
    }

    #[test]
    fn picks_once_per_unchanged_state_and_retries_when_not_issued() {
        let model = model_of(Document::default());
        let mut state = state_with_cursor();
        state.build_scene(&model, &SketchEditing::default());

        let first = state.request(true).unwrap();
        assert_eq!(first.pick_at, Some(Vector2::new(120.0, 80.0)));
        assert_eq!(first.rect.width, 400.0);
        assert_eq!(state.request(true).unwrap().pick_at, None);

        state.pick_was_not_issued();
        assert!(state.request(false).unwrap().pick_at.is_none());
        assert!(state.request(true).unwrap().pick_at.is_some());
    }

    #[test]
    fn a_pick_result_sets_the_hovered_item_and_the_hit_under_the_cursor() {
        let model = model_of(Document::default());
        let mut state = state_with_cursor();
        state.build_scene(&model, &SketchEditing::default());
        state.request(true);

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
        let (document, feature, line, _) = sketched(Plane::XZ);
        let model = model_of(document);
        let mut state = state_with_cursor();
        state.build_scene(&model, &SketchEditing::default());
        let entity = Pickable::SketchEntity {
            feature,
            entity: line,
        };
        state.selection.toggle(Pickable::Origin);
        state.selection.toggle(entity);

        let editing = SketchEditing::editing(feature);
        let edited = state
            .build_scene(&model, &editing)
            .and_then(|built| built.edited)
            .unwrap();
        assert!(state.is_animating());
        assert_eq!(state.selection.iter().collect::<Vec<_>>(), vec![entity]);
        state.advance(Duration::from_secs(1));
        let viewpoint = state.camera.viewpoint();
        assert!(viewpoint.forward().distance(Vector3::Y) < 1e-9);
        assert!(viewpoint.up().distance(Vector3::Z) < 1e-9);
        let view = state.view().unwrap();
        for corner in edited.bounds.corners() {
            let pixel = view.project(corner).unwrap();
            assert!(pixel.x >= 0.0 && pixel.x <= view.size().x);
            assert!(pixel.y >= 0.0 && pixel.y <= view.size().y);
        }

        state.build_scene(&model, &editing);
        assert!(!state.is_animating());
        let reference = Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        };
        state.selection.toggle(reference);
        state.build_scene(&model, &editing);
        assert!(state.selection.contains(reference));
        state.build_scene(&model, &SketchEditing::default());
        assert_eq!(state.selection.iter().collect::<Vec<_>>(), vec![entity]);
    }

    #[test]
    fn the_first_scene_fits_the_camera_and_later_fits_animate() {
        let model = model_of(Document::default());
        let mut state = state_with_cursor();
        let initial = state.camera.viewpoint();
        state.build_scene(&model, &SketchEditing::default());
        assert_ne!(state.camera.viewpoint(), initial);
        assert!(!state.is_animating());

        state.fit_requested = true;
        state
            .selection
            .replace_with(Pickable::Axis(crate::selection::Axis::X));
        state.build_scene(&model, &SketchEditing::default());
        assert!(state.is_animating());
        assert!(!state.fit_requested);
    }

    #[derive(Debug)]
    struct Drawn {
        generation: u64,
        base: Arc<Batch>,
        picks: Arc<PickTable>,
        overlay: Option<Arc<Batch>>,
        picked: bool,
    }

    impl Drawn {
        fn same_scene(&self, other: &Self) -> bool {
            self.generation == other.generation && Arc::ptr_eq(&self.base, &other.base)
        }

        fn same_overlay(&self, other: &Self) -> bool {
            match (&self.overlay, &other.overlay) {
                (Some(this), Some(that)) => Arc::ptr_eq(this, that),
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
            }
        }

        fn lines_of(&self, pickable: Pickable) -> Vec<Line> {
            let pick = self.picks.id_of(pickable);
            self.base
                .lines
                .iter()
                .filter(|line| line.pick.is_some() && line.pick == pick)
                .cloned()
                .collect()
        }
    }

    fn draw(state: &mut ViewportState, model: &Model, editing: &SketchEditing) -> Drawn {
        let built = state.build_scene(model, editing).unwrap();
        let drawn = Drawn {
            generation: built.generation,
            base: Arc::clone(&built.scene.batches[0]),
            picks: Arc::clone(&built.picks),
            overlay: built.scene.batches.get(1).cloned(),
            picked: false,
        };
        let picked = state
            .request(true)
            .is_some_and(|request| request.pick_at.is_some());
        Drawn { picked, ..drawn }
    }

    #[test]
    fn an_unchanged_frame_reuses_the_scene_and_asks_for_no_pick() {
        let (document, ..) = sketched(Plane::XY);
        let mut model = model_of(document);
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        draw(&mut state, &model, &editing);

        let first = draw(&mut state, &model, &editing);
        let again = draw(&mut state, &model, &editing);
        let still = draw(&mut state, &model, &editing);

        assert!(first.picked);
        assert!(again.same_scene(&first) && still.same_scene(&first));
        assert!(!again.picked && !still.picked);
        assert!(first.overlay.is_none() && again.same_overlay(&first));
    }

    #[test]
    fn moving_the_camera_keeps_the_scene_but_picks_again() {
        let (document, ..) = sketched(Plane::XY);
        let mut model = model_of(document);
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        draw(&mut state, &model, &editing);
        let first = draw(&mut state, &model, &editing);

        state
            .camera
            .orbit(Point3::ZERO, Vector2::new(30.0, 10.0), 300.0);
        let orbited = draw(&mut state, &model, &editing);
        let target = state.camera.viewpoint().target;
        state.camera.pan(Vector2::new(40.0, 0.0), 0.1);
        let panned = draw(&mut state, &model, &editing);
        state.camera.zoom(target, 1.3);
        let zoomed_out_a_little = draw(&mut state, &model, &editing);

        assert!(orbited.same_scene(&first) && orbited.picked);
        assert!(panned.same_scene(&first) && panned.picked);
        assert!(zoomed_out_a_little.same_scene(&first) && zoomed_out_a_little.picked);
    }

    #[test]
    fn zooming_far_in_refacets_curves_finer_and_hover_draws_the_same_polyline() {
        let (document, feature, _, circle) = sketched(Plane::XY);
        let mut model = model_of(document);
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        let circle = Pickable::SketchEntity {
            feature,
            entity: circle,
        };
        draw(&mut state, &model, &editing);
        let first = draw(&mut state, &model, &editing);
        let coarse = first.lines_of(circle);

        let target = state.camera.viewpoint().target;
        state.camera.zoom(target, 1.0 / 16.0);
        let zoomed = draw(&mut state, &model, &editing);
        let fine = zoomed.lines_of(circle);
        state.hovered = Some(circle);
        let hovered = draw(&mut state, &model, &editing);
        let highlighted = hovered.lines_of(circle);

        assert!(!zoomed.same_scene(&first));
        assert!(
            fine.len() > coarse.len() * 2,
            "{} {}",
            fine.len(),
            coarse.len()
        );
        assert_eq!(
            highlighted
                .iter()
                .map(|line| (line.start, line.end))
                .collect::<Vec<_>>(),
            fine.iter()
                .map(|line| (line.start, line.end))
                .collect::<Vec<_>>()
        );
        assert!(highlighted.iter().all(|line| line.width > fine[0].width));
    }

    #[test]
    fn each_change_rebuilds_what_it_changes_in_the_frame_it_happens() {
        let (document, feature, line, _) = sketched(Plane::XY);
        let mut model = model_of(document);
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        let line = Pickable::SketchEntity {
            feature,
            entity: line,
        };
        draw(&mut state, &model, &editing);
        let idle = draw(&mut state, &model, &editing);

        state.hovered = Some(line);
        let hovered = draw(&mut state, &model, &editing);
        let hovered_color = hovered.lines_of(line)[0].color;
        state.selection.toggle(line);
        let selected = draw(&mut state, &model, &editing);
        state.set_measured(Some((
            MeasuredLine {
                from: Point3::ZERO,
                to: Point3::new(10.0, 0.0, 0.0),
                accuracy: Accuracy::Exact,
            },
            "10 mm".to_owned(),
        )));
        let measured = draw(&mut state, &model, &editing);
        let measured_again = draw(&mut state, &model, &editing);
        state.set_measured(None);
        let unmeasured = draw(&mut state, &model, &editing);

        assert!(!hovered.same_scene(&idle) && hovered.picked);
        assert_ne!(hovered_color, idle.lines_of(line)[0].color);
        assert!(!selected.same_scene(&hovered));
        assert!(measured.same_scene(&selected) && !measured.picked);
        assert_eq!(
            measured.overlay.as_ref().map(|batch| batch.lines.len()),
            Some(1)
        );
        assert!(measured_again.same_overlay(&measured));
        assert!(unmeasured.same_scene(&selected) && unmeasured.overlay.is_none());

        model.perform(Action::Apply(Transaction::single(
            "Hide the sketch",
            Edit::SetFeatureHidden {
                id: feature,
                hidden: true,
            },
        )));
        let edited = draw(&mut state, &model, &editing);
        settle(&mut model);
        let evaluated = draw(&mut state, &model, &editing);
        let entered = draw(&mut state, &model, &SketchEditing::editing(feature));

        assert!(!edited.same_scene(&unmeasured) && edited.picked);
        assert!(edited.lines_of(line).is_empty());
        assert!(!evaluated.same_scene(&edited));
        assert!(!entered.same_scene(&evaluated));
    }
}

#[cfg(test)]
mod timing {
    use std::time::Instant;

    use caditor_document::{
        BodyOperation, Document, Extrude, ExtrudeExtent, RegionChoice, SolidFeature,
    };
    use caditor_sketch::Sketch;
    use egui::pos2;

    use super::{
        tests::{model_of, settle},
        *,
    };

    const FRAMES: u32 = 200;
    const WARM_UP: u32 = 3;

    fn large_sketch() -> Sketch {
        let mut sketch = Sketch::new(Plane::XY);
        for row in 0..100 {
            for column in 0..200 {
                let at = Point2::new(f64::from(column) * 5.0, f64::from(row) * 5.0);
                sketch.add_line(at, at + Vector2::new(3.0, 1.0));
            }
        }
        for index in 0..2000 {
            let at = Point2::new(
                f64::from(index % 50) * 20.0,
                -20.0 - f64::from(index / 50) * 20.0,
            );
            sketch.add_circle(at, 4.0 + f64::from(index % 5));
            sketch.add_arc(at, at + Vector2::new(8.0, 0.0), at + Vector2::new(0.0, 8.0));
        }
        for index in 0..200 {
            let at = Point2::new(1100.0, f64::from(index) * 6.0);
            sketch.add_spline(&[
                at,
                at + Vector2::new(10.0, 4.0),
                at + Vector2::new(20.0, -4.0),
                at + Vector2::new(30.0, 0.0),
            ]);
        }
        sketch
    }

    fn holed_plate() -> Sketch {
        let mut sketch = Sketch::new(Plane::XY);
        let corners = [
            Point2::new(0.0, 0.0),
            Point2::new(420.0, 0.0),
            Point2::new(420.0, 420.0),
            Point2::new(0.0, 420.0),
        ];
        let points: Vec<EntityId> = corners
            .iter()
            .map(|corner| sketch.add_point(*corner))
            .collect();
        for index in 0..4 {
            sketch
                .insert_entity(
                    EntityId::from_raw(sketch.next_id()),
                    caditor_sketch::Entity::Line {
                        start: points[index],
                        end: points[(index + 1) % 4],
                    },
                )
                .unwrap();
        }
        for row in 0..20 {
            for column in 0..20 {
                let at = Point2::new(
                    20.0 + f64::from(column) * 20.0,
                    20.0 + f64::from(row) * 20.0,
                );
                sketch.add_circle(at, 6.0);
            }
        }
        sketch
    }

    fn sketch_document(sketch: Sketch) -> (Document, FeatureId) {
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Large sketch", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature)
    }

    fn plate_document() -> Document {
        let mut document = Document::default();
        let mut transaction = document.transaction("Add plate");
        let sketch = transaction.add_feature("Plate sketch", FeatureKind::from(holed_plate()));
        let distance = transaction.parse("5 mm").unwrap();
        transaction.add_feature(
            "Plate",
            FeatureKind::Solid(SolidFeature::Extrude(Extrude {
                sketch,
                regions: RegionChoice::All,
                extent: ExtrudeExtent::one_side(distance, false),
                operation: BodyOperation::NewBody,
            })),
        );
        document.apply(transaction.finish()).unwrap();
        document
    }

    fn settled(document: Document) -> Model {
        let started = Instant::now();
        let mut model = model_of(document);
        settle(&mut model);
        eprintln!("evaluated and meshed in {:?}", started.elapsed());
        model
    }

    fn placed_state() -> ViewportState {
        let mut state = ViewportState::new();
        state.rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0)));
        state.cursor = Some(Vector2::new(800.0, 500.0));
        state
    }

    struct Scenario<'a> {
        model: &'a Model,
        editing: &'a SketchEditing,
    }

    impl Scenario<'_> {
        fn frame(&self, state: &mut ViewportState) {
            state.build_scene(self.model, self.editing);
            state.request(true);
        }

        fn time(&self, name: &str, mut change: impl FnMut(&mut ViewportState, u32)) {
            let mut state = placed_state();
            for frame in 0..WARM_UP {
                change(&mut state, frame);
                self.frame(&mut state);
            }
            let started = Instant::now();
            for frame in 0..FRAMES {
                change(&mut state, frame);
                self.frame(&mut state);
            }
            let per_frame = started.elapsed() / FRAMES;
            let lines = state.scene().map_or(0, |scene| scene.lines().count());
            eprintln!("{name}: {per_frame:?} per frame, {lines} line segments");
        }
    }

    fn hover(state: &mut ViewportState, pickable: Pickable) {
        state.hovered = Some(pickable);
        state.hover_source = None;
    }

    fn orbit(state: &mut ViewportState) {
        state
            .camera
            .orbit(Point3::ZERO, Vector2::new(2.0, 0.0), 1000.0);
    }

    #[test]
    #[ignore = "a timing benchmark: cargo test --release -p caditor frame_costs -- --ignored --nocapture"]
    fn frame_costs_on_a_large_sketch_and_a_large_model() {
        let (document, sketch) = sketch_document(large_sketch());
        let model = settled(document);
        let editing = SketchEditing::editing(sketch);
        let scenario = Scenario {
            model: &model,
            editing: &editing,
        };
        let entities: Vec<EntityId> = model
            .document()
            .feature(sketch)
            .and_then(|feature| feature.kind.sketch())
            .map(|sketch| sketch.entities().map(|(id, _)| id).take(2).collect())
            .unwrap();
        scenario.time("sketch, idle", |_, _| {});
        scenario.time("sketch, camera moving", |state, _| orbit(state));
        scenario.time("sketch, hover changing", |state, frame| {
            let entity = entities[frame as usize % entities.len()];
            hover(
                state,
                Pickable::SketchEntity {
                    feature: sketch,
                    entity,
                },
            );
        });

        let model = settled(plate_document());
        let editing = SketchEditing::default();
        let scenario = Scenario {
            model: &model,
            editing: &editing,
        };
        let mut state = placed_state();
        let faces: Vec<Pickable> = state
            .build_scene(&model, &editing)
            .unwrap()
            .picks
            .pickables()
            .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
            .take(2)
            .collect();
        scenario.time("model, idle", |_, _| {});
        scenario.time("model, camera moving", |state, _| orbit(state));
        scenario.time("model, hover changing", |state, frame| {
            hover(state, faces[frame as usize % faces.len()]);
        });
    }
}
