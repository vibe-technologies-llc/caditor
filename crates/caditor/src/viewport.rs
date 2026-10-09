use std::{sync::Arc, time::Duration};

use caditor_document::{FeatureId, FeatureKind, SavedView, Transaction};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Ray, Rotation3, Vector2, Vector3};
use caditor_render::{
    Camera, PickResult, ProjectionMode, Reflection, Scene, SurfaceSize, View, Viewpoint,
    ViewportRect, grid_minor_spacing,
};
use caditor_sketch::{ConstraintId, Entity, EntityId, MAX_LENGTH, Sketch, SketchClip};
use egui::{
    Align, Align2, Id, Key, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, WidgetInfo, WidgetType, accesskit::Live, pos2, vec2,
};

use crate::{
    analysis::{Analyses, FaceAnalysis},
    annotations::{self, Annotations, LabelMoving, Surface},
    blend_tools,
    bodies::{self, BodyMeshes, OpenDraft},
    body_selection,
    box_selection::{self, Catch},
    canvas,
    comb::CombDrawing,
    commands::{CameraMove, Command, CommandFrame, StandardView},
    dimensioning,
    display::Displayed,
    display_style::DisplayStyle,
    drag_solver::DragCommand,
    drawing::{Drawing, Preview},
    editing::{self, EditingCommand, SketchEditing, Tool},
    faceting::FacetLevel,
    feature_tree,
    interference_panel::{Mark, MarkKind},
    manipulator::{Manipulating, Manipulator},
    measure::MeasuredLine,
    model::{Action, Model, Notice, RecomputeStatus},
    modifying::{Hint, Modifying, Outcome},
    move_manipulator::Handle,
    offset_face_tools,
    paint_selection::{self, Painting},
    pattern_tools,
    pick_list::{self, PickList},
    preferences::{InputMode, Navigation, PreferenceChange, PreferencesCommand},
    primitive_tools, projecting,
    reference_picking::{self, Slot},
    saved_views,
    scene::{self, BuiltScene, EditedSketch, Highlight, PickTable, SketchShapes, Sources},
    scene_cache::{Overlay, Revisions, SceneCache, SceneInputs},
    scene_description::{Item, SceneDescription},
    scene_palette::Contrast,
    selection::{Pickable, Selection, SelectionFilter},
    selection_sets,
    shape_modes::ShapeMode,
    shell_tools,
    sketch_drag::{self, BoxMode, Grab, Moving, ScreenArea, Transform, Transforming},
    sketch_placement::{self, DatumTarget, FaceChoice},
    sketch_toolbar, sketch_tools,
    snap::{Hold, Pointer, Screen},
    snapshot, solid_tools,
    trimming::{self, Trimming},
    typed_point::{self, TypedPoint},
    view_aids::ViewAids,
    view_cube::{self, CubeAction},
    visibility,
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_POINTS: f64 = 1.5;
const PROMPT_MARGIN: f32 = 16.0;
const PROMPT_MAX_WIDTH: f32 = 720.0;
const READOUT_ROOM: f32 = 200.0;
const DRAG_DRAWS_FROM_PRESS: f64 = 12.0;
const SIZE_READOUT_OFFSET: egui::Vec2 = vec2(14.0, 26.0);
pub const PLACE_AT_A_POINT: &str = "While drawing, Space places at the highlighted point or curve: \
                                    highlight one of the sketch's points or curves, or the origin.";
const VIEWPORT_NAME: &str = "3D view";
const ISOMETRIC_NOT_CHANGED: &str = "The Isometric view has not been changed";
const NOT_IN_A_SKETCH: &str = "Edit a sketch to look straight at it";
pub const DRAG_BLOCKED: &str = "The constraints do not allow it there";
pub const DRAG_CONFLICT: &str = "Nothing moves while constraints conflict";
const READOUT_GAP: f32 = 4.0;
const CLIPBOARD_MARK: &str = "caditor sketch geometry: ";
const NOTHING_TO_COPY: &str = "Select sketch geometry to copy";
const NOTHING_TO_PASTE: &str = "Copy or cut sketch geometry first";
const PASTE_SHIFT_FRACTION: f64 = 0.25;
const PASTE_SHIFT_FLOOR: f64 = 1.0;
const NAVIGATION_HINT: &str =
    "Right-drag: orbit   Middle-drag or Shift+right-drag: pan   Scroll: zoom";
pub const CHOOSE_PLANE_PROMPT: &str = "Click a plane or a flat face to sketch on";
const CHOOSE_PLANE_HINT: &str = "Esc: cancel";
const CHOOSE_REGIONS_PROMPT: &str = "Click regions of the sketch to include or leave them out";
const CHOOSE_REGIONS_HINT: &str = "Esc: done";
const CHOOSE_EDGES_PROMPT: &str = "Click edges to add them or leave them out";
const PROJECT_PROMPT: &str =
    "Click an edge, corner or face of a body, or a curve of another sketch, to project it";
const INTERSECT_PROMPT: &str = "Click a face to draw where the sketch cuts it, Shift-click for its \
     whole body, or click a datum plane";
const CHOOSE_FACES_PROMPT: &str = "Click flat faces to open them or close them again";
const CHOOSE_MOVED_FACES_PROMPT: &str = "Click faces to move them or leave them out again";
const CHOOSE_BODIES_PROMPT: &str = "Choose the operation and the two bodies in the feature's panel";
const CHOOSE_MOVE_PROMPT: &str =
    "Drag an arrow or a square, or enter the turns and distances in the feature's panel";
const CHOOSE_SCALE_PROMPT: &str = "Enter the factor and the centre in the feature's panel";
const CHOOSE_MIRROR_PROMPT: &str =
    "Choose the plane in the feature's panel, or select a plane or flat face and use it from there";
const CHOOSE_SPLIT_PROMPT: &str = "Choose what to split along in the feature's panel, or select a \
                                   plane, flat face, sketch curve or other body and use it from \
                                   there";
const CHOOSE_MATE_PROMPT: &str = "Choose the faces or axes to mate in the feature's panel, or \
                                  select one and use it from there";
const CHOOSE_HOLE_PROMPT: &str = "Choose the hole's style and sizes in the feature's panel";
const CHOOSE_PRIMITIVE_PROMPT: &str =
    "Enter the sizes and position in the feature's panel, or choose in the view where it goes";
const CHOOSE_REFERENCES_PROMPT: &str = "Select planes, faces, axes or edges for the feature's panel, or choose them in the view from it";
const SNAP_LABEL_OFFSET: egui::Vec2 = vec2(14.0, 10.0);
const KEYBOARD_ORBIT_FRACTION: f64 = 1.0 / 12.0;
const KEYBOARD_PAN_FRACTION: f64 = 0.1;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.25;
const TYPED_POINT_OFFSET: f32 = 64.0;
const FREE_PLACEMENT_HINT: &str = "Ctrl: place freely";
const HELD_SNAP_HINT: &str = "Alt: snap to the grid or nearby geometry";
const TYPE_POINT_HINT: &str = "Type x, y or length < angle for an exact point";
const TYPED_POINT_HINT: &str = "@: from the last point   A length alone goes toward the pointer   \
                                Enter: place   Esc: cancel";
const TYPED_SIDES_HINT: &str = "6 sides: set the sides";
const SCRUB_HINT: &str = "Shift: move sideways to set the sides";
const MOVE_HINT: &str = "@: by an offset   Enter: move   Esc: cancel";
const BOX_FILL_OPACITY: f32 = 0.12;
const BOX_STROKE_WIDTH: f32 = 1.0;
const BOX_DASH: f32 = 6.0;
const BOX_GAP: f32 = 4.0;
const NOTHING_TO_HIGHLIGHT: &str = "Nothing in the view can be picked";
const MEASURE_LABEL_LIFT: f32 = 6.0;
const PROBLEM_LABEL_LIFT: f32 = 9.0;
const PLACE_SHARE: f64 = 0.2;
const MIN_PLACE_REACH: f64 = 1.0;
const BACK_TO_SELECT: &str = "Esc: back to Select";
const LIST_NOT_WITH_TOOL: &str =
    "This tool picks its own targets: press Esc to go back to Select first";
const NO_TARGET_HIGHLIGHTED: &str =
    "Highlight a piece or an end first, with Highlight the next item in the view";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchScreen {
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

    fn to_sketch(&self, point: Vector2) -> Option<Point2> {
        let ray = self.view.ray_through(point * self.pixels_per_point)?;
        let distance = ray.intersect_plane(&self.plane)?;
        within_reach(self.plane.to_local(ray.at(distance)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Availability {
    selection: u64,
    revision: u64,
    evaluation: u64,
    context: editing::Context,
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
    Box {
        feature: FeatureId,
        area: ScreenArea,
    },
    ModelBox {
        area: ScreenArea,
    },
    Paint(Painting),
    Trim {
        feature: FeatureId,
        from: Point2,
    },
    Pull {
        feature: FeatureId,
    },
    Manipulate(Manipulating),
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
    pub(crate) hovered: Option<Pickable>,
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
    requested_viewpoint: Option<Viewpoint>,
    edited: Option<FeatureId>,
    face_edited_sketch: bool,
    checked_availability: Option<Availability>,
    sketch_cursor: Option<Point2>,
    drawing: Drawing,
    trimming: Trimming,
    modifying: Modifying,
    annotations: Annotations,
    bodies: BodyMeshes,
    navigation: Navigation,
    keyboard_highlight: Option<Pickable>,
    pick_list: Option<PickList>,
    list_hold: bool,
    last_cursor: Option<Vector2>,
    typed_point: TypedPoint,
    moving: Option<Moving>,
    transforming: Option<Transforming>,
    moving_label: Option<LabelMoving>,
    press: Option<Press>,
    draw_press: Option<Vector2>,
    placing_freely: bool,
    snap_held: bool,
    scrubbing: bool,
    primary: Option<PrimaryDrag>,
    hovered_in_tree: Option<Pickable>,
    chosen_rows: Vec<FeatureId>,
    home_applied: bool,
    session: u64,
    fit_when_computed: bool,
    scene_bounds: Option<Aabb>,
    measured: Option<(MeasuredLine, String)>,
    problems: Vec<Problem>,
    interference: Vec<Mark>,
    comb: Option<Arc<CombDrawing>>,
    framed_place: Option<Point3>,
    look_from: Option<Vector3>,
    scenes: SceneCache,
    description: SceneDescription,
    filter: SelectionFilter,
    filter_applies: bool,
    style: DisplayStyle,
    contrast: Contrast,
    snapping: bool,
    grid_snapping: bool,
    lasso: bool,
    paint: bool,
    select_through: bool,
    typed_dimensions: bool,
    first_dimension_scales: bool,
    glyphs_shown: bool,
    aids: ViewAids,
    analyses: Analyses,
    manipulator: Option<Manipulator>,
    manipulator_hover: Option<Handle>,
    clipboard: Option<Copied>,
    system_clipboard: Option<String>,
    dimensioning: Option<FeatureId>,
}

#[derive(Debug, Clone, PartialEq)]
struct Copied {
    feature: FeatureId,
    clip: SketchClip,
}

#[derive(Debug, Clone, PartialEq)]
struct Problem {
    place: Point3,
    label: String,
}

fn clipped(clip: &SketchClip) -> String {
    let parts: Vec<String> = [
        (clip.curve_count(), "curve", "curves"),
        (clip.lone_point_count(), "point", "points"),
        (clip.constraint_count(), "constraint", "constraints"),
    ]
    .into_iter()
    .filter(|(amount, _, _)| *amount > 0)
    .map(|(amount, singular, plural)| feature_tree::count(amount, singular, plural))
    .collect();
    match parts.as_slice() {
        [] => "nothing".to_owned(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn problems(model: &Model) -> Vec<Problem> {
    let document = model.document();
    model
        .evaluation()
        .failures()
        .filter_map(|(feature, error)| {
            let name = &document.feature(feature)?.name;
            Some(Problem {
                place: error.place?,
                label: format!("{name} failed here"),
            })
        })
        .collect()
}

fn camera_looking_from(view: SavedView, projection: ProjectionMode) -> Camera {
    let mut camera = Camera::new(saved_views::viewpoint(view));
    camera.set_projection(projection);
    camera
}

pub fn initial_viewpoint() -> Viewpoint {
    Viewpoint::looking_from(INITIAL_LOOK_FROM, Point3::ZERO, INITIAL_DISTANCE).unwrap_or(
        Viewpoint {
            target: Point3::ZERO,
            orientation: Rotation3::IDENTITY,
            distance: INITIAL_DISTANCE,
        },
    )
}

impl ViewportState {
    pub fn new() -> Self {
        Self {
            camera: Camera::new(initial_viewpoint()),
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
            requested_viewpoint: None,
            edited: None,
            face_edited_sketch: false,
            checked_availability: None,
            sketch_cursor: None,
            drawing: Drawing::default(),
            trimming: Trimming::default(),
            modifying: Modifying::default(),
            annotations: Annotations::default(),
            bodies: BodyMeshes::default(),
            navigation: Navigation::default(),
            keyboard_highlight: None,
            pick_list: None,
            list_hold: false,
            last_cursor: None,
            typed_point: TypedPoint::default(),
            moving: None,
            transforming: None,
            moving_label: None,
            press: None,
            draw_press: None,
            placing_freely: false,
            snap_held: false,
            scrubbing: false,
            primary: None,
            hovered_in_tree: None,
            chosen_rows: Vec::new(),
            home_applied: false,
            session: 0,
            fit_when_computed: false,
            scene_bounds: None,
            measured: None,
            problems: Vec::new(),
            interference: Vec::new(),
            comb: None,
            framed_place: None,
            look_from: None,
            scenes: SceneCache::default(),
            description: SceneDescription::default(),
            filter: SelectionFilter::default(),
            filter_applies: true,
            style: DisplayStyle::default(),
            contrast: Contrast::default(),
            snapping: true,
            grid_snapping: false,
            lasso: false,
            paint: false,
            select_through: false,
            typed_dimensions: true,
            first_dimension_scales: false,
            glyphs_shown: true,
            aids: ViewAids::default(),
            analyses: Analyses::default(),
            manipulator: None,
            manipulator_hover: None,
            clipboard: None,
            system_clipboard: None,
            dimensioning: None,
        }
    }

    pub fn snapping(&self) -> bool {
        self.snapping
    }

    pub fn grid_snapping(&self) -> bool {
        self.grid_snapping
    }

    pub fn lasso(&self) -> bool {
        self.lasso
    }

    pub fn paint(&self) -> bool {
        self.paint
    }

    pub fn projection(&self) -> ProjectionMode {
        self.navigation.projection
    }

    pub fn select_through(&self) -> bool {
        self.select_through
    }

    pub fn typed_dimensions(&self) -> bool {
        self.typed_dimensions
    }

    pub fn first_dimension_scales(&self) -> bool {
        self.first_dimension_scales
    }

    pub fn aids(&self) -> ViewAids {
        self.aids
    }

    #[cfg(test)]
    pub fn toggle_centres_of_mass_for_screenshots(&mut self) {
        self.aids.centres_of_mass = !self.aids.centres_of_mass;
    }

    pub fn set_analysis(&mut self, analysis: Option<FaceAnalysis>, reflection: Option<Reflection>) {
        self.aids.analysis = analysis;
        self.aids.reflection = reflection;
    }

    pub fn analyses(&self) -> &Analyses {
        &self.analyses
    }

    pub fn glyphs_shown(&self) -> bool {
        self.glyphs_shown
    }

    pub fn style(&self) -> DisplayStyle {
        self.style
    }

    pub fn set_style(&mut self, style: DisplayStyle) {
        self.style = style;
    }

    pub fn set_contrast(&mut self, contrast: Contrast) {
        self.contrast = contrast;
    }

    pub fn filter(&self) -> SelectionFilter {
        self.filter
    }

    pub fn filter_applies(&self) -> bool {
        self.filter_applies
    }

    pub fn set_filter(&mut self, filter: SelectionFilter) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.hovered = None;
        self.keyboard_highlight = None;
        self.last_pick = None;
    }

    fn active_filter(&self) -> SelectionFilter {
        if self.filter_applies {
            self.filter
        } else {
            SelectionFilter::Everything
        }
    }

    pub fn bodies(&self) -> &BodyMeshes {
        &self.bodies
    }

    pub fn set_measured(&mut self, measured: Option<(MeasuredLine, String)>) {
        self.measured = measured;
    }

    pub fn set_interference(&mut self, marks: Vec<Mark>) {
        self.interference = marks;
    }

    pub fn set_comb(&mut self, comb: Option<Arc<CombDrawing>>) {
        self.comb = comb;
    }

    pub fn destination(&self) -> Viewpoint {
        self.camera.destination()
    }

    pub fn show_saved_view(&mut self, view: SavedView) {
        self.camera.animate_to(saved_views::viewpoint(view));
    }

    fn start_from_home(&mut self, model: &Model) {
        match model.document().saved_views().home {
            Some(home) => {
                self.camera = camera_looking_from(home, self.navigation.projection);
                self.home_applied = true;
            }
            None if self.home_applied => {
                self.camera = Camera::new(initial_viewpoint());
                self.camera.set_projection(self.navigation.projection);
                self.home_applied = false;
            }
            None => {}
        }
    }

    pub fn show_place(&mut self, place: Point3) {
        self.framed_place = Some(place);
    }

    pub fn select_only(&mut self, pickable: Pickable) {
        self.selection.clear();
        self.selection.toggle(pickable);
    }

    pub fn select_exactly(&mut self, pickables: Vec<Pickable>) {
        self.selection.replace_with_all(pickables);
    }

    pub fn hover_from_tree(&mut self, pickable: Option<Pickable>) {
        self.hovered_in_tree = pickable;
    }

    pub fn show_chosen_rows(&mut self, rows: Vec<FeatureId>) {
        self.chosen_rows = rows;
    }

    fn rows_to_highlight(&self, context: editing::Context) -> Vec<FeatureId> {
        match context.sketch.is_none() && context.solid.is_none() {
            true => self.chosen_rows.clone(),
            false => Vec::new(),
        }
    }

    pub fn forget_document(&mut self) {
        self.selection.clear();
        self.chosen_rows.clear();
        self.hovered = None;
        self.hover_source = None;
        self.pending_click = None;
        self.pointer_hit = None;
        self.last_pick = None;
        self.keyboard_highlight = None;
        self.pick_list = None;
        self.list_hold = false;
        self.last_cursor = None;
        self.scenes = SceneCache::default();
        self.description.forget();
        self.drawing = Drawing::default();
        self.trimming = Trimming::default();
        self.modifying = Modifying::default();
        self.annotations = Annotations::default();
        self.typed_point = TypedPoint::default();
        self.moving = None;
        self.moving_label = None;
        self.transforming = None;
        self.clipboard = None;
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

    pub fn replace_selection(&mut self, pickables: Vec<Pickable>) {
        self.selection.replace_with_all(pickables);
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
        let best = picks.best_hit(result, self.active_filter());
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
            self.start_from_home(model);
        }
        if self.fit_when_computed
            && model.status() == RecomputeStatus::UpToDate
            && !model.bodies_pending()
        {
            self.fit_requested = true;
            self.fit_when_computed = false;
        }
        self.keyboard_commands(model, editing, commands, actions);
        if let Some(text) = self.system_clipboard.take() {
            ui.ctx().copy_text(text);
        }
        let key_hints = KeyHints::new(commands);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            self.rect = Some(rect);
            self.pixels_per_point = ui.ctx().pixels_per_point();
            let response = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, VIEWPORT_NAME));
            let described = self.description.of(model, editing.feature()).to_owned();
            let accessible = ui
                .ctx()
                .accesskit_node_builder(response.id, |node| node.set_description(described))
                .is_some();
            if accessible {
                let items = self.description.items(model, editing.feature());
                scene_items(ui, response.id, rect.min, items);
            }

            self.track_cursor(ui, &response, rect);
            self.track_manipulator(model, editing);
            self.track_sketch_cursor(model, editing);
            self.track_drawing(model, editing);
            self.type_points(ui, rect, model, editing, keys_free, actions);
            self.navigate(ui, &response, rect);
            self.drag_primary(ui, &response, model, editing, actions);
            self.click(ui, &response, model, editing, actions);
            self.hold_to_list(ui, &response, model, editing, actions);
            self.show_pick_list(ui, model, editing, actions);
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
        self.filter_applies =
            context.sketch.is_none() && context.solid.is_none() && !context.choosing_plane;
        if edited != self.edited {
            self.edited = edited;
            self.face_edited_sketch = edited.is_some();
            self.last_pick = None;
        }
        self.bodies.update(evaluation, &display.meshing);
        let drafted = model.draft_body_result();
        self.bodies.update_open(
            document,
            evaluation,
            &display.meshing,
            context.solid,
            OpenDraft {
                evaluation: model.draft_evaluation(),
                result: drafted.as_ref().map(|(body, result)| (*body, result)),
                cuts: model.draft_cuts(),
                moved: model.draft_placement(),
            },
        );
        let availability = Availability {
            selection: self.selection.generation(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            context,
        };
        if self.checked_availability != Some(availability) {
            self.selection
                .retain_available(document, evaluation, context);
            self.checked_availability = Some(Availability {
                selection: self.selection.generation(),
                ..availability
            });
        }
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
        } else if let Some(annotation) =
            self.annotations.hovered().or(highlighted
                .filter(|highlight| matches!(highlight, Pickable::SketchConstraint { .. })))
        {
            annotation.constrained_entities(document)
        } else if let Some(row) = self.hovered_in_tree {
            match row.constrained_entities(document) {
                entities if entities.is_empty() => vec![row],
                entities => entities,
            }
        } else {
            highlighted
                .into_iter()
                .flat_map(|highlighted| self.whole_body_of(model, highlighted))
                .collect()
        };
        let chosen_rows = self.rows_to_highlight(context);
        let view = self.view();
        let sources = Sources {
            document,
            evaluation,
            bodies: &self.bodies,
            sketches: &display.sketches,
            style: self.style,
            aids: self.aids,
            analyses: &self.analyses,
            contrast: self.contrast,
        };
        self.scenes.update(&SceneInputs {
            sources: &sources,
            revisions: Revisions {
                document: model.revision(),
                evaluation: model.evaluation_generation(),
                sketches: display.sketches.generation(),
                bodies: self.bodies.generation(),
                style: self.style,
                aids: self.aids,
                analysed: self.analyses.finished(),
                contrast: self.contrast,
            },
            context,
            highlight: Highlight {
                selection: &self.selection,
                hovered: &hovered,
                chosen_rows: &chosen_rows,
            },
            view: view.as_ref(),
        });
        let faceting = self.scenes.faceting();
        let plane = self.scenes.edited_plane();
        self.problems = problems(model);
        self.scenes.show(Overlay {
            contrast: self.contrast,
            plane,
            previews: vec![
                self.drawing.preview(faceting),
                self.trimming.preview(faceting),
                self.modifying.preview(faceting),
                self.grab_preview(),
            ],
            measured: self.measured.as_ref().map(|(line, _)| [line.from, line.to]),
            problems: self.problems.iter().map(|problem| problem.place).collect(),
            interference: self.interference.clone(),
            comb: self.comb.clone(),
            manipulator: self
                .manipulator
                .map(|manipulator| manipulator.drawn(self.manipulator_hover)),
        });
        if let Some(highlight) = self.keyboard_highlight
            && !self.scenes.highlightable().contains(&highlight)
            && !(matches!(highlight, Pickable::SketchConstraint { .. })
                && highlight.is_available(document, evaluation, context))
        {
            self.keyboard_highlight = None;
        }
        let built = self.scenes.built()?;
        self.scene_bounds = Some(built.everything);
        if self.needs_initial_fit
            && let Some(home) = model.document().saved_views().home
        {
            self.camera = camera_looking_from(home, self.navigation.projection);
            self.home_applied = true;
        }
        let Some(view) = self.view() else {
            return Some(built);
        };
        if self.needs_initial_fit {
            self.camera = Camera::new(view.fitted(built.fit_all()));
            self.camera.set_projection(self.navigation.projection);
            self.needs_initial_fit = false;
        } else if self.face_edited_sketch {
            if let Some(sketch) = built.edited {
                self.camera.animate_to(facing(&view, &sketch));
            }
            self.face_edited_sketch = false;
        } else if let Some(direction) = self.look_from {
            if let Some(bounds) = built.bounds_of(&sources, self.selection.iter()) {
                self.camera.animate_to(looking_at(&view, direction, bounds));
            }
        } else if let Some(place) = self.framed_place {
            let reach = (built.fit_all().bounding_radius() * PLACE_SHARE).max(MIN_PLACE_REACH);
            let around = Aabb::from_point(place).expanded(reach);
            self.camera.animate_to(view.fitted(around));
        } else if self.fit_requested {
            let everything = built.fit_all();
            let bounds = if !self.selection.is_empty() {
                built
                    .bounds_of(&sources, self.selection.iter())
                    .unwrap_or(everything)
            } else {
                built
                    .bounds_of_features(&sources, &chosen_rows)
                    .unwrap_or(everything)
            };
            self.camera.animate_to(view.fitted(bounds));
        }
        self.fit_requested = false;
        self.framed_place = None;
        self.look_from = None;
        Some(built)
    }

    pub fn scene(&self) -> Option<&Scene> {
        self.scenes.built().map(|built| &built.scene)
    }

    pub fn request(&mut self, can_pick: bool) -> Option<ViewportRequest> {
        let rect = self.rect?;
        let view = self.view()?;
        let generation = self.scenes.generation();
        let viewpoint = self.camera.viewpoint();
        let moving = self
            .requested_viewpoint
            .is_some_and(|previous| previous != viewpoint);
        self.requested_viewpoint = Some(viewpoint);
        let pick_at = self
            .cursor
            .filter(|_| can_pick && !moving)
            .and_then(|cursor| {
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
            style: self.style,
            aids: ViewAids::default(),
            analyses: &Analyses::default(),
            contrast: Contrast::Standard,
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
                chosen_rows: &[],
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

    pub fn thumbnail(&self, model: &Model, bodies: &[FeatureId], size: SurfaceSize) -> ImageView {
        let meshes = self.bodies.only(bodies);
        let snapshot = snapshot::of_bodies(model.document(), model.evaluation(), &meshes, size);
        ImageView {
            view: snapshot.view,
            scene: snapshot.scene,
            pixels_per_point: snapshot.pixels_per_point,
        }
    }

    pub fn pick_was_not_issued(&mut self) {
        self.last_pick = None;
        self.picks_in_flight = None;
    }

    pub fn pick_again(&mut self) {
        self.last_pick = None;
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
    pub fn listed(&self) -> Option<Vec<String>> {
        self.pick_list
            .as_ref()
            .map(|list| list.rows().iter().map(|row| row.words.clone()).collect())
    }

    #[cfg(test)]
    pub fn listed_highlight(&self) -> Option<Pickable> {
        self.pick_list.as_ref().and_then(PickList::highlighted)
    }

    #[cfg(test)]
    pub fn screen_position(&self, plane: Plane, point: Point2) -> Option<egui::Pos2> {
        let pixel = self.view()?.project(plane.to_world(point))? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn handle_position(&self, handle: Handle, along: f64) -> Option<egui::Pos2> {
        let world = self.manipulator?.grip(handle, along)?;
        let pixel = self.view()?.project(world)? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn manipulator_step(&self) -> Option<f64> {
        self.manipulator.map(|manipulator| manipulator.step())
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
        self.pick_list
            .as_ref()
            .and_then(PickList::highlighted)
            .or(self.keyboard_highlight)
            .or(self.hovered)
    }

    fn track_cursor(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        self.placing_freely = ui.input(|input| input.modifiers.command);
        self.snap_held = ui.input(|input| input.modifiers.alt);
        self.scrubbing = ui.input(|input| input.modifiers.shift);
        if self.scrubbing && self.drawing.is_scrubbing() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
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
        self.last_cursor = self.cursor.or(self.last_cursor);
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

        let (shift, alt, ctrl, primary_down, secondary_down) = ui.input(|input| {
            (
                input.modifiers.shift,
                input.modifiers.alt,
                input.modifiers.command,
                input.pointer.primary_down(),
                input.pointer.secondary_down(),
            )
        });
        let drag = self.to_pixels(response.drag_delta());
        let held_by_sketch = self.primary.is_some() || self.draw_press.is_some();
        let buttons = DragButtons {
            primary: response.dragged_by(PointerButton::Primary) && !held_by_sketch,
            secondary: response.dragged_by(PointerButton::Secondary),
            middle: response.dragged_by(PointerButton::Middle),
            chorded: primary_down || secondary_down,
        };
        let motion = drag_motion(self.navigation.input_mode, buttons, shift, alt, ctrl);
        if drag != Vector2::ZERO {
            match motion {
                Some(DragMotion::Orbit) => self.orbit_by(&view, drag, rect, self.drag_anchor),
                Some(DragMotion::Pan) => self.pan_by(&view, drag, self.drag_anchor),
                Some(DragMotion::Zoom) => {
                    let rate = ZOOM_PER_SCROLL_POINT * self.navigation.zoom_speed;
                    let pivot = self.drag_anchor.unwrap_or(view.viewpoint().target);
                    self.camera.zoom(pivot, (drag.y * rate).exp());
                }
                None => {}
            }
        }

        if !response.hovered() && !response.dragged() {
            return;
        }
        let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta, input.zoom_delta()));
        let mut scrolled_to_zoom = scroll.y;
        if self.navigation.input_mode == InputMode::Laptop {
            scrolled_to_zoom = 0.0;
            let sliding = self.to_pixels(scroll);
            if sliding != Vector2::ZERO {
                let anchor = self.cursor.and_then(|cursor| self.hit_under(cursor));
                if alt {
                    self.pan_by(&view, sliding, anchor);
                } else {
                    self.orbit_by(&view, sliding, rect, anchor);
                }
            }
        }
        let direction = if self.navigation.invert_zoom {
            1.0
        } else {
            -1.0
        };
        let rate = ZOOM_PER_SCROLL_POINT * self.navigation.zoom_speed;
        let factor = (direction * f64::from(scrolled_to_zoom) * rate).exp() / f64::from(pinch);
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

    fn orbit_by(&mut self, view: &View, drag: Vector2, rect: Rect, anchor: Option<Point3>) {
        let pivot = anchor.unwrap_or(view.viewpoint().target);
        self.camera.orbit(
            pivot,
            drag * self.navigation.orbit_speed,
            f64::from(rect.height() * self.pixels_per_point),
        );
    }

    fn pan_by(&mut self, view: &View, drag: Vector2, anchor: Option<Point3>) {
        let depth = anchor
            .map(|anchor| view.view_depth(anchor))
            .filter(|depth| *depth > view.near_plane())
            .unwrap_or(view.viewpoint().distance);
        self.camera.pan(drag, view.units_per_pixel_at(depth));
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
        within_reach(plane.to_local(ray.at(distance)))
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
            let navigating = ui.input(|input| {
                (self.navigation.input_mode == InputMode::Laptop && input.modifiers.alt)
                    || input.pointer.middle_down()
            });
            let press = self.press.take().filter(|_| !navigating);
            self.draw_press = press
                .filter(|_| {
                    editing.active().is_some_and(|active| active.tool.draws())
                        && !self.drawing.in_progress()
                })
                .map(|press| press.cursor);
            self.primary = press.and_then(|press| self.begin_primary(press, model, editing));
            if matches!(self.primary, Some(PrimaryDrag::Paint(_))) && !toggle {
                self.selection.clear();
            }
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
        let grab_snapping = match &self.primary {
            Some(PrimaryDrag::Grab(grab)) => self.grab_snapping(model, grab.feature()),
            _ => None,
        };
        let ray = cursor.and_then(|cursor| self.view()?.ray_through(cursor));
        let free = self.placing_freely;
        let hold = self.hold();
        match &mut self.primary {
            Some(PrimaryDrag::Manipulate(manipulating))
                if editing.solid() != Some(manipulating.feature) =>
            {
                actions.push(Action::Preview {
                    feature: manipulating.feature,
                    draft: None,
                });
                self.primary = None;
            }
            Some(PrimaryDrag::Manipulate(manipulating)) => {
                if let Some(ray) = ray
                    && manipulating.follow(ray, free)
                {
                    actions.push(Action::Preview {
                        feature: manipulating.feature,
                        draft: manipulating.transaction(model),
                    });
                }
            }
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
                let snapping = grab_snapping
                    .as_ref()
                    .map(|(sketch, screen)| (&**sketch, screen));
                if let Some(command) = sketch_cursor.and_then(|at| grab.follow(at, snapping, hold))
                {
                    actions.push(Action::Drag(command));
                }
            }
            Some(PrimaryDrag::Box { area, .. } | PrimaryDrag::ModelBox { area }) => {
                if let Some(cursor) = cursor {
                    area.reach(cursor / f64::from(self.pixels_per_point));
                }
            }
            Some(PrimaryDrag::Paint(painting)) => {
                if let Some(cursor) = cursor {
                    let samples = painting.stroke_to(cursor, f64::from(self.pixels_per_point));
                    self.paint_faces(model, editing, &samples);
                }
            }
            Some(PrimaryDrag::Trim { .. } | PrimaryDrag::Pull { .. }) | None => {}
        }
        if released {
            self.press = None;
            match self.primary.take() {
                Some(PrimaryDrag::Grab(grab)) if grab.has_moved() => {
                    actions.push(Action::Drag(grab.finish()));
                }
                Some(PrimaryDrag::Box { feature, area }) => {
                    self.select_within(model, feature, &area, toggle);
                }
                Some(PrimaryDrag::ModelBox { area }) => {
                    self.select_in_model(model, editing, &area, toggle);
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
                Some(PrimaryDrag::Manipulate(manipulating)) => {
                    match manipulating
                        .transaction(model)
                        .filter(|_| manipulating.has_moved())
                    {
                        Some(transaction) => actions.push(Action::Apply(transaction)),
                        None => actions.push(Action::Preview {
                            feature: manipulating.feature,
                            draft: None,
                        }),
                    }
                }
                Some(PrimaryDrag::Grab(_) | PrimaryDrag::Paint(_)) | None => {}
            }
        }
    }

    fn paint_faces(&mut self, model: &Model, editing: &SketchEditing, samples: &[Vector2]) {
        let (Some(built), Some(view)) = (self.scenes.built(), self.view()) else {
            return;
        };
        let document = model.document();
        let evaluation = model.evaluation();
        let context = editing.context();
        let mut painted: Vec<Pickable> = Vec::new();
        for at in samples {
            for face in paint_selection::faces_under(
                built,
                &view,
                *at,
                self.pixels_per_point,
                self.select_through,
            ) {
                if !painted.contains(&face)
                    && !self.selection.contains(face)
                    && face.is_available(document, evaluation, context)
                {
                    painted.push(face);
                }
            }
        }
        let added: Vec<Pickable> = painted
            .into_iter()
            .flat_map(|face| self.whole_body_of(model, face))
            .filter(|item| !self.selection.contains(*item))
            .collect();
        if !added.is_empty() {
            self.selection.extend(added);
        }
    }

    fn track_manipulator(&mut self, model: &Model, editing: &SketchEditing) {
        let open = editing
            .solid()
            .filter(|_| !editing.context().choosing_in_view);
        let pixels_per_point = f64::from(self.pixels_per_point);
        self.manipulator = self
            .view()
            .and_then(|view| Manipulator::of(model, open, &view, pixels_per_point));
        self.manipulator_hover = match &self.primary {
            Some(PrimaryDrag::Manipulate(manipulating)) => Some(manipulating.handle),
            Some(_) => None,
            None => self.manipulator.zip(self.view()).zip(self.cursor).and_then(
                |((manipulator, view), cursor)| manipulator.hit(&view, cursor, pixels_per_point),
            ),
        };
    }

    fn manipulate_from(&self, press: Press, model: &Model) -> Option<Manipulating> {
        let manipulator = self.manipulator?;
        let view = self.view()?;
        let handle = manipulator.hit(&view, press.cursor, f64::from(self.pixels_per_point))?;
        let ray = view.ray_through(press.cursor)?;
        Manipulating::begin(model, &manipulator, handle, ray)
    }

    fn grab_preview(&self) -> Preview {
        match &self.primary {
            Some(PrimaryDrag::Grab(grab)) => Preview {
                snap: grab.landing().map(|landing| landing.position),
                guides: grab.guides().to_vec(),
                ..Preview::default()
            },
            _ => Preview::default(),
        }
    }

    fn grab_snapping<'a>(
        &self,
        model: &'a Model,
        feature: FeatureId,
    ) -> Option<(Displayed<'a>, SketchScreen)> {
        if self.placing_freely || !(self.snapping || self.snap_held) {
            return None;
        }
        let owner = model.document().feature(feature)?;
        let sketch = model.displayed_sketch(owner)?;
        let screen = self.sketch_screen(sketch.plane())?;
        Some((sketch, screen))
    }

    fn minor_grid_spacing(&self) -> Option<f64> {
        self.scenes
            .built()
            .and_then(|built| built.scene.grid.as_ref())
            .zip(self.view())
            .map(|(grid, view)| grid_minor_spacing(grid, &view))
    }

    fn hold(&self) -> Option<Hold> {
        (self.snap_held && !self.placing_freely).then(|| Hold {
            grid: self.minor_grid_spacing(),
        })
    }

    fn begin_primary(
        &self,
        press: Press,
        model: &Model,
        editing: &SketchEditing,
    ) -> Option<PrimaryDrag> {
        if editing.feature().is_none()
            && editing.solid().is_none()
            && editing.picking().is_none()
            && !editing.is_choosing_plane()
        {
            if self.paint && paint_selection::takes_faces(self.active_filter()) {
                return Some(PrimaryDrag::Paint(Painting::new(
                    self.selection.clone(),
                    press.cursor,
                )));
            }
            let at = press.cursor / f64::from(self.pixels_per_point);
            return Some(PrimaryDrag::ModelBox {
                area: ScreenArea::starting_at(at, self.lasso),
            });
        }
        if let Some(manipulating) = self.manipulate_from(press, model) {
            return Some(PrimaryDrag::Manipulate(manipulating));
        }
        let active = editing.active().filter(|active| !active.tool.draws())?;
        let feature = active.feature;
        match active.tool {
            Tool::Trim => {
                let from = self.on_sketch(model, feature, press.cursor)?;
                return Some(PrimaryDrag::Trim { feature, from });
            }
            Tool::Offset | Tool::Fillet | Tool::Chamfer => {
                return Some(PrimaryDrag::Pull { feature });
            }
            Tool::Extend
            | Tool::Mirror
            | Tool::RectangularPattern
            | Tool::CircularPattern
            | Tool::TangentCircle
            | Tool::Project
            | Tool::Intersect
            | Tool::Dimension
            | Tool::BlendCurve => return None,
            _ => {}
        }
        let projected = |entity: EntityId| {
            edited_sketch(model, editing).is_some_and(|sketch| sketch.is_projected(entity))
        };
        let grabbed = match press.hovered {
            Some(Pickable::SketchEntity {
                feature: owner,
                entity,
            }) if owner == feature && !entity.is_reference() && !projected(entity) => Some(entity),
            _ => None,
        };
        let Some(grabbed) = grabbed else {
            let at = press.cursor / f64::from(self.pixels_per_point);
            return Some(PrimaryDrag::Box {
                feature,
                area: ScreenArea::starting_at(at, self.lasso),
            });
        };
        let owner = model.document().feature(feature)?;
        let sketch = model.displayed_sketch(owner)?;
        let from = self.on_sketch(model, feature, press.cursor)?;
        let selected = sketch_tools::selected_entities(&self.selection, feature);
        Grab::of(&sketch, feature, grabbed, &selected, from).map(PrimaryDrag::Grab)
    }

    fn select_within(
        &mut self,
        model: &Model,
        feature: FeatureId,
        area: &ScreenArea,
        toggle: bool,
    ) {
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

    fn select_in_model(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        area: &ScreenArea,
        keep: bool,
    ) {
        if !box_selection::is_a_box(area) {
            return;
        }
        let Some(view) = self.view() else {
            return;
        };
        let document = model.document();
        let evaluation = model.evaluation();
        let scale = f64::from(self.pixels_per_point);
        let mut caught: Vec<Pickable> = match Catch::of(self.active_filter()) {
            Catch::SketchGeometry => document
                .active_features()
                .filter(|feature| visibility::is_shown(document, feature.id()))
                .filter_map(|feature| Some((feature.id(), model.displayed_sketch(feature)?)))
                .flat_map(|(feature, sketch)| {
                    let screen = SketchScreen {
                        view,
                        plane: sketch.plane(),
                        pixels_per_point: scale,
                    };
                    sketch_drag::within(&sketch, &screen, area, self.scenes.faceting())
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
                .collect(),
            catch => {
                let seen = |point: Point3| {
                    let at = view.project(point)? / scale;
                    let depth = view.view_depth(point);
                    Some(box_selection::Seen {
                        at,
                        depth,
                        units_per_point: view.units_per_pixel_at(depth) * scale,
                    })
                };
                let shown: Vec<_> = self
                    .bodies
                    .iter()
                    .filter(|(body, _)| visibility::is_shown(document, *body))
                    .collect();
                let occlusion = if self.select_through {
                    box_selection::Occlusion::open()
                } else {
                    box_selection::Occlusion::of(
                        shown
                            .iter()
                            .filter_map(|(_, mesh)| mesh.source().solid()?.mesh()),
                        &seen,
                        area,
                    )
                };
                let looking = box_selection::Looking {
                    seen: &seen,
                    occlusion: &occlusion,
                };
                shown
                    .iter()
                    .flat_map(|(body, mesh)| {
                        box_selection::within_body(*body, mesh, &looking, area, catch)
                    })
                    .collect()
            }
        };
        if self.active_filter() == SelectionFilter::Bodies {
            let touched: Vec<FeatureId> = caught.iter().filter_map(|item| item.body()).collect();
            caught = body_selection::whole_bodies(model, &touched, body_selection::Kind::Faces);
        }
        let context = editing.context();
        caught.retain(|pickable| pickable.is_available(document, evaluation, context));
        if !keep {
            self.selection.clear();
        }
        for pickable in caught {
            if !self.selection.contains(pickable) {
                self.selection.toggle(pickable);
            }
        }
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
        self.drawing
            .place_freely(self.placing_freely || !(self.snapping || self.snap_held));
        self.drawing
            .snap_to_grid(self.minor_grid_spacing().filter(|_| self.grid_snapping));
        self.drawing.hold_snap(self.hold());
        let scrub_from = self
            .cursor
            .filter(|_| self.scrubbing)
            .map(|cursor| cursor.x / f64::from(self.pixels_per_point));
        self.drawing.scrub_sides(scrub_from);
        self.trimming.sync(editing.active(), displayed.as_deref());
        let dimensioning = editing
            .active()
            .filter(|active| active.tool.dimensions())
            .map(|active| active.feature);
        if dimensioning != self.dimensioning {
            if dimensioning.is_some() {
                self.selection.clear();
            }
            self.dimensioning = dimensioning;
        }
        let selected = editing
            .feature()
            .map(|feature| sketch_tools::selected_entities(&self.selection, feature))
            .unwrap_or_default();
        self.modifying.sync(
            editing.active(),
            editing.modes(),
            displayed.as_deref(),
            &selected,
        );
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
        if self.list_hold {
            if !ui.input(|input| input.pointer.primary_down()) {
                self.list_hold = false;
            }
            return;
        }
        let drawing = editing
            .active()
            .is_some_and(|active| active.tool.draws() || active.tool.modifies());
        if self.hover_is_current()
            && let Some(click) = self.pending_click.take()
        {
            self.perform_click(click, model, editing, drawing, actions);
        }
        let placed_by_dragging = editing.active().is_some_and(|active| active.tool.draws())
            && response.drag_stopped_by(PointerButton::Primary);
        if placed_by_dragging {
            self.place_press_point(model, editing, drawing, actions);
        }
        if !response.dragged() {
            self.draw_press = None;
        }
        let click = Click {
            double: response.double_clicked(),
            primary: response.clicked_by(PointerButton::Primary) || placed_by_dragging,
            toggle: ui.input(|input| input.modifiers.shift || input.modifiers.command),
        };
        if (!click.double && !click.primary) || self.manipulator_hover.is_some() {
            return;
        }
        if drawing || self.hover_is_current() {
            self.perform_click(click, model, editing, drawing, actions);
        } else {
            self.pending_click = Some(click);
        }
    }

    fn place_press_point(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        drawing: bool,
        actions: &mut Vec<Action>,
    ) {
        let (Some(from), Some(to), Some(feature)) =
            (self.draw_press.take(), self.cursor, editing.feature())
        else {
            return;
        };
        let apart = (to - from).length() / f64::from(self.pixels_per_point);
        if apart < DRAG_DRAWS_FROM_PRESS || self.drawing.in_progress() {
            return;
        }
        let (cursor, sketch_cursor) = (self.cursor, self.sketch_cursor);
        self.cursor = Some(from);
        self.sketch_cursor = self.on_sketch(model, feature, from);
        self.track_drawing(model, editing);
        if self.sketch_cursor.is_some() {
            let click = Click {
                double: false,
                primary: true,
                toggle: false,
            };
            self.perform_click(click, model, editing, drawing, actions);
        }
        self.cursor = cursor;
        self.sketch_cursor = sketch_cursor;
        self.track_drawing(model, editing);
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
        let ray = self
            .cursor
            .and_then(|cursor| self.view()?.ray_through(cursor));
        if let Some(action) = pick_action(self.hovered, ray, model, editing, click.toggle) {
            actions.extend(action);
            return;
        }
        if let Some(feature) = self.dimensioning {
            self.dimension_click(model, feature, self.hovered, actions);
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
        if click.double && self.select_chain(model, editing) {
            return;
        }
        self.select(model, click.toggle);
    }

    fn dimension_click(
        &mut self,
        model: &Model,
        feature: FeatureId,
        hovered: Option<Pickable>,
        actions: &mut Vec<Action>,
    ) {
        let mut picks = sketch_tools::selected_entities(&self.selection, feature);
        let Some(shown) = model
            .document()
            .feature(feature)
            .and_then(|owner| model.displayed_sketch(owner))
        else {
            return;
        };
        let letting_go = matches!(
            hovered,
            Some(Pickable::SketchEntity { feature: owner, entity }) if owner == feature && picks.contains(&entity)
        );
        if dimensioning::awaits_placement(&shown, &picks) && !letting_go {
            self.place_dimension(model, feature, &picks, actions);
            return;
        }
        let picked = match hovered {
            Some(Pickable::SketchEntity {
                feature: owner,
                entity,
            }) if owner == feature => entity,
            _ => {
                if !picks.is_empty() {
                    self.place_dimension(model, feature, &picks, actions);
                }
                return;
            }
        };
        let pickable = Pickable::SketchEntity {
            feature,
            entity: picked,
        };
        if picks.contains(&picked) {
            self.selection.toggle(pickable);
            return;
        }
        picks.push(picked);
        match dimensioning::fitting(&shown, &picks) {
            dimensioning::Fit::Refused(reason) => {
                actions.push(Action::Inform(Notice::info(format!("{reason}."))));
            }
            dimensioning::Fit::Ready(_)
                if picks.len() > 1 && !dimensioning::awaits_placement(&shown, &picks) =>
            {
                self.place_dimension(model, feature, &picks, actions);
            }
            dimensioning::Fit::Ready(_) | dimensioning::Fit::Waiting => {
                self.selection.toggle(pickable);
            }
        }
    }

    fn place_dimension(
        &mut self,
        model: &Model,
        feature: FeatureId,
        picks: &[EntityId],
        actions: &mut Vec<Action>,
    ) {
        match dimensioning::dimension(model, feature, picks, self.sketch_cursor) {
            Ok(added) => self.dimension_added(feature, added, actions),
            Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
        }
    }

    fn dimension_added(
        &mut self,
        feature: FeatureId,
        added: sketch_tools::Added,
        actions: &mut Vec<Action>,
    ) {
        let typed = added
            .constraints
            .first()
            .copied()
            .filter(|constraint| !added.references.contains(constraint));
        actions.push(Action::Apply(added.transaction));
        if !added.references.is_empty() {
            actions.push(Action::Inform(Notice::info(
                sketch_toolbar::REFERENCE_ADDED,
            )));
        }
        if let Some(constraint) = typed {
            self.edit_dimension(feature, constraint);
        }
        self.selection.clear();
    }

    fn picked_dimension(&self, model: &Model) -> Option<(FeatureId, Vec<EntityId>)> {
        let feature = self.dimensioning?;
        let picks = sketch_tools::selected_entities(&self.selection, feature);
        let owner = model.document().feature(feature)?;
        let shown = model.displayed_sketch(owner)?;
        matches!(
            dimensioning::fitting(&shown, &picks),
            dimensioning::Fit::Ready(_)
        )
        .then_some((feature, picks))
    }

    fn select_chain(&mut self, model: &Model, editing: &SketchEditing) -> bool {
        let Some(Pickable::SketchEntity { feature, entity }) = self.hovered else {
            return false;
        };
        let Some(sketch) = edited_sketch(model, editing) else {
            return false;
        };
        let chain = sketch.offset_chain_through(entity);
        let [first, rest @ ..] = chain.as_slice() else {
            return false;
        };
        if rest.is_empty() {
            return false;
        }
        self.selection.replace_with(Pickable::SketchEntity {
            feature,
            entity: *first,
        });
        for entity in rest {
            self.selection.toggle(Pickable::SketchEntity {
                feature,
                entity: *entity,
            });
        }
        true
    }

    fn select(&mut self, model: &Model, toggle: bool) {
        match (self.hovered, toggle) {
            (Some(pickable), true) => self.toggle_chosen(model, pickable),
            (Some(pickable), false) => {
                let chosen = self.whole_body_of(model, pickable);
                self.selection.replace_with_all(chosen);
            }
            (None, false) => self.selection.clear(),
            (None, true) => {}
        }
    }

    fn whole_body_of(&self, model: &Model, pickable: Pickable) -> Vec<Pickable> {
        match pickable {
            Pickable::Face { body, .. } if self.active_filter() == SelectionFilter::Bodies => {
                let faces =
                    body_selection::whole_bodies(model, &[body], body_selection::Kind::Faces);
                if faces.is_empty() {
                    vec![pickable]
                } else {
                    faces
                }
            }
            _ => vec![pickable],
        }
    }

    fn toggle_chosen(&mut self, model: &Model, pickable: Pickable) {
        let chosen = self.whole_body_of(model, pickable);
        if chosen.iter().all(|item| self.selection.contains(*item)) {
            for item in chosen {
                self.selection.toggle(item);
            }
        } else {
            self.selection.extend(chosen);
        }
    }

    fn keyboard_commands(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let home = model.document().saved_views().home;
        for view in StandardView::ALL {
            if commands.available(Command::View(view)) {
                if let (StandardView::Isometric, Some(home)) = (view, home) {
                    self.show_saved_view(home);
                    continue;
                }
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
        self.view_commands(model, commands, actions);
        let facing = editing.feature().ok_or(NOT_IN_A_SKETCH);
        if commands.invoke(Command::LookAtSketch, &facing) && facing.is_ok() {
            self.face_edited_sketch = true;
        }
        let looking = sketch_placement::face_to_look_at(model, &self.selection);
        if commands.invoke(Command::LookAtFace, &looking)
            && let Ok(direction) = looking
        {
            self.look_from = Some(direction);
        }
        self.selection_commands(model, editing, commands, actions);
        for step in CameraMove::ALL {
            if commands.available(Command::Camera(step)) {
                self.nudge(step);
            }
        }
        if commands.available(Command::ToggleGlyphs) {
            self.glyphs_shown = !self.glyphs_shown;
        }
        if commands.available(Command::ToggleCentresOfMass) {
            self.aids.centres_of_mass = !self.aids.centres_of_mass;
            if !self.aids.centres_of_mass {
                self.selection
                    .retain(|pickable| !matches!(pickable, Pickable::CentreOfMass(_)));
                self.hovered = self
                    .hovered
                    .filter(|hovered| !matches!(hovered, Pickable::CentreOfMass(_)));
            }
        }
        if commands.available(Command::ToggleSnapping) {
            self.snapping = !self.snapping;
        }
        if commands.available(Command::ToggleGridSnapping) {
            self.grid_snapping = !self.grid_snapping;
        }
        if commands.available(Command::ToggleLasso) {
            self.lasso = !self.lasso;
        }
        if commands.available(Command::TogglePaintSelection) {
            self.paint = !self.paint;
        }
        if commands.available(Command::ToggleSelectThrough) {
            self.select_through = !self.select_through;
        }
        if commands.available(Command::ToggleTypedDimensions) {
            self.typed_dimensions = !self.typed_dimensions;
        }
        if commands.available(Command::ToggleFirstDimensionScales) {
            self.first_dimension_scales = !self.first_dimension_scales;
        }
        for style in DisplayStyle::ALL {
            if commands.available(Command::Style(style)) {
                self.set_style(style);
            }
        }
        for filter in SelectionFilter::ALL {
            if commands.available(Command::Filter(filter)) {
                self.set_filter(filter);
            }
        }
        if commands.available(Command::CycleSelectionPriority) {
            self.set_filter(self.filter.next_priority());
        }
        if commands.available(Command::ToggleProjection) {
            actions.push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Projection(self.navigation.projection.other()),
            )));
        }
        if commands.available(Command::AutomaticProjection) {
            actions.push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::Projection(self.navigation.projection.automatic_toggled()),
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
                    None => self.step_highlight(step, model, editing),
                }
            }
        }
        let listing = self.list_availability();
        if commands.invoke(Command::ListUnderPointer, &listing) && listing.is_ok() {
            let centre = self.view().map(|view| view.size() / 2.0);
            if let Some(cursor) = self.cursor.or(self.last_cursor).or(centre) {
                self.open_pick_list(model, editing, cursor, actions);
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
            self.sketch_commands(
                model,
                active.feature,
                active.tool.draws(),
                commands,
                actions,
            );
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
        let face = self
            .keyboard_highlight
            .or(self.hovered)
            .filter(|target| matches!(target, Pickable::Face { .. }))
            .filter(|_| {
                editing
                    .active()
                    .is_some_and(|active| active.tool.intersects())
            })
            .ok_or(projecting::NO_FACE_HIGHLIGHTED);
        if commands.invoke(Command::IntersectBody, &face)
            && let Ok(face) = face
        {
            actions.extend(pick_action(Some(face), None, model, editing, true).unwrap_or_default());
            return;
        }
        let activation = self
            .keyboard_highlight
            .ok_or("Highlight an item first, with Highlight the next item in the view");
        if commands.invoke(Command::ActivateHighlighted, &activation)
            && let Some(highlight) = self.keyboard_highlight
        {
            self.choose(model, editing, highlight, true, actions);
        }
    }

    fn choose(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        pickable: Pickable,
        toggle: bool,
        actions: &mut Vec<Action>,
    ) {
        if let Some(placed) = self.place_at_highlight(model, editing, pickable) {
            actions.extend(placed);
            return;
        }
        if let Some(feature) = self.dimensioning {
            self.dimension_click(model, feature, Some(pickable), actions);
            return;
        }
        match pick_action(Some(pickable), None, model, editing, false) {
            Some(action) => actions.extend(action),
            None if toggle => self.toggle_chosen(model, pickable),
            None => {
                let chosen = self.whole_body_of(model, pickable);
                self.selection.replace_with_all(chosen);
            }
        }
    }

    fn list_availability(&self) -> Result<(), &'static str> {
        if self.trimming.is_active() || self.modifying.is_active() {
            Err(LIST_NOT_WITH_TOOL)
        } else if !self.scenes.has_pickables() {
            Err(NOTHING_TO_HIGHLIGHT)
        } else {
            Ok(())
        }
    }

    fn open_pick_list(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        cursor: Vector2,
        actions: &mut Vec<Action>,
    ) {
        let (Some(built), Some(view), Some(rect)) = (self.scenes.built(), self.view(), self.rect)
        else {
            return;
        };
        let hits = built
            .scene
            .hits_through(&view, cursor, self.pixels_per_point);
        let filter = self.active_filter();
        let projecting = editing.active().filter(|active| active.tool.projects());
        let listed: Vec<Pickable> = built
            .picks
            .listed(&hits, filter)
            .into_iter()
            .filter(|pickable| {
                projecting.is_none_or(|active| projecting::projectable(model, active, *pickable))
            })
            .collect();
        let rows = pick_list::rows(
            model.document(),
            model.evaluation(),
            listed,
            filter == SelectionFilter::Bodies,
        );
        if rows.is_empty() {
            actions.push(Action::Inform(Notice::info(format!(
                "{}.",
                pick_list::NOTHING_UNDER
            ))));
            return;
        }
        let at = cursor / f64::from(self.pixels_per_point);
        let pointer = rect.min + vec2(at.x as f32, at.y as f32);
        self.keyboard_highlight = None;
        self.pick_list = Some(PickList::new(pointer, rows, model.revision()));
    }

    fn hold_to_list(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let tool_takes_presses = editing
            .active()
            .is_some_and(|active| active.tool.draws() || active.tool.modifies());
        if self.pick_list.is_some()
            || self.primary.is_some()
            || self.manipulator_hover.is_some()
            || tool_takes_presses
            || self.list_availability().is_err()
        {
            return;
        }
        let Some(press) = self.press else {
            return;
        };
        let (down, still, held_for, alt) = ui.input(|input| {
            let pointer = &input.pointer;
            (
                pointer.primary_down(),
                pointer.could_any_button_be_click(),
                pointer.press_start_time().map(|start| input.time - start),
                input.modifiers.alt,
            )
        });
        let navigating = self.navigation.input_mode == InputMode::Laptop && alt;
        let Some(held_for) = held_for else {
            return;
        };
        if !down || !still || navigating || !response.is_pointer_button_down_on() {
            return;
        }
        let wait = pick_list::HOLD_TO_LIST.as_secs_f64() - held_for;
        if wait > 0.0 {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(wait));
            return;
        }
        self.press = None;
        self.list_hold = true;
        self.open_pick_list(model, editing, press.cursor, actions);
    }

    fn show_pick_list(
        &mut self,
        ui: &egui::Ui,
        model: &Model,
        editing: &SketchEditing,
        actions: &mut Vec<Action>,
    ) {
        let Some(list) = self.pick_list.as_mut() else {
            return;
        };
        if list.opened() != model.revision() {
            self.pick_list = None;
            return;
        }
        match list.show(ui.ctx(), model.document()) {
            pick_list::Outcome::Open => {}
            pick_list::Outcome::Closed => self.pick_list = None,
            pick_list::Outcome::Chosen { pickable, toggle } => {
                self.pick_list = None;
                self.choose(model, editing, pickable, toggle, actions);
            }
        }
    }

    fn place_at_highlight(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        highlight: Pickable,
    ) -> Option<Option<Action>> {
        let active = editing.active().filter(|active| active.tool.draws())?;
        let sketch = edited_sketch(model, editing)?;
        match highlight {
            Pickable::Origin => self.drawing.type_point(&sketch, Point2::ZERO),
            Pickable::SketchEntity { feature, entity } if feature == active.feature => {
                match sketch.entity(entity) {
                    Some(Entity::Point(position)) => self.drawing.type_point(&sketch, *position),
                    Some(_) => {
                        if let Err(reason) = self.drawing.type_on_curve(&sketch, entity) {
                            return Some(Some(Action::Inform(Notice::info(format!("{reason}.")))));
                        }
                    }
                    None => return Some(Some(Action::Inform(Notice::info(PLACE_AT_A_POINT)))),
                }
            }
            _ => return Some(Some(Action::Inform(Notice::info(PLACE_AT_A_POINT)))),
        }
        Some(match self.drawing.click(model) {
            Ok(transaction) => transaction.map(Action::Apply),
            Err(refusal) => Some(Action::Inform(Notice::info(refusal.reason()))),
        })
    }

    fn view_commands(
        &mut self,
        model: &Model,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let document = model.document();
        let current = saved_views::saved_view(self.camera.destination());
        if commands.available(Command::SaveView) {
            let name = document.saved_views().unused_name();
            match saved_views::save(document, &name, current) {
                Ok(transaction) => {
                    actions.push(Action::Apply(transaction));
                    actions.push(Action::Inform(Notice::info(format!(
                        "Saved the view as {name}. Rename it, or see them all, in Saved views."
                    ))));
                }
                Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
            }
        }
        if commands.available(Command::SavedViews) {
            actions.push(Action::Preferences(PreferencesCommand::ShowSavedViews));
        }
        if commands.available(Command::SetHomeView) {
            match saved_views::set_home(document, current) {
                Ok(transaction) => {
                    actions.push(Action::Apply(transaction));
                    actions.push(Action::Inform(Notice::info(
                        "The Isometric view now shows the model like this.",
                    )));
                }
                Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
            }
        }
        let redefined = match document.saved_views().home {
            Some(_) => Ok(()),
            None => Err(ISOMETRIC_NOT_CHANGED),
        };
        if commands.invoke(Command::ResetHomeView, &redefined) {
            match saved_views::reset_home(document) {
                Ok(transaction) => actions.push(Action::Apply(transaction)),
                Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
            }
        }
    }

    fn selection_commands(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let in_sketch = editing.feature().is_some();
        let everything = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_select_all(model, &self.selection, self.active_filter()),
        );
        if commands.invoke(Command::SelectAllShapes, &everything)
            && let Ok(kind) = everything
        {
            self.selection
                .replace_with_all(body_selection::select_all(model, kind));
        }
        let tangent = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_tangent_edges(&self.selection),
        );
        if commands.invoke(Command::SelectTangentEdges, &tangent) && tangent.is_ok() {
            let followed = body_selection::tangent_edges(model, &self.selection);
            if followed.is_empty() {
                actions.push(Action::Inform(Notice::info(
                    body_selection::NO_TANGENT_EDGES,
                )));
            }
            self.selection.extend(followed);
        }
        let tangent_faces = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_tangent_faces(&self.selection),
        );
        if commands.invoke(Command::SelectTangentFaces, &tangent_faces) && tangent_faces.is_ok() {
            let spread = body_selection::tangent_faces_of(model, &self.selection);
            if spread.is_empty() {
                actions.push(Action::Inform(Notice::info(
                    body_selection::NO_TANGENT_FACES,
                )));
            }
            self.selection.extend(spread);
        }
        let hole =
            body_selection::outside_sketch(in_sketch, body_selection::offer_hole(&self.selection));
        if commands.invoke(Command::SelectHole, &hole) && hole.is_ok() {
            let walls = body_selection::hole_of(model, &self.selection);
            if walls.is_empty() {
                actions.push(Action::Inform(Notice::info(body_selection::NO_HOLE)));
            }
            self.selection.extend(walls);
        }
        let boundary = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_face_edges(&self.selection),
        );
        if commands.invoke(Command::SelectFaceEdges, &boundary) && boundary.is_ok() {
            let around = body_selection::face_edges(model, &self.selection);
            if around.is_empty() {
                actions.push(Action::Inform(Notice::info(body_selection::NO_FACE_EDGES)));
            } else {
                self.selection.replace_with_all(around);
            }
        }
        let whole = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_whole_bodies(&self.selection),
        );
        if commands.invoke(Command::SelectBody, &whole)
            && let Ok(bodies) = whole
        {
            let kind = body_selection::Kind::chosen(self.filter, &self.selection)
                .unwrap_or(body_selection::Kind::Faces);
            self.selection
                .replace_with_all(body_selection::whole_bodies(model, &bodies, kind));
        }
        self.selection_set_commands(model, in_sketch, commands, actions);
    }

    fn selection_set_commands(
        &self,
        model: &Model,
        in_sketch: bool,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let keepable =
            body_selection::outside_sketch(
                in_sketch,
                match self.selection.iter().any(|pickable| {
                    matches!(pickable, Pickable::Face { .. } | Pickable::Edge { .. })
                }) {
                    true => Ok(()),
                    false => Err(selection_sets::NOTHING_TO_KEEP),
                },
            );
        if commands.invoke(Command::SaveSelectionSet, &keepable) && keepable.is_ok() {
            let name = model.document().selection_sets().unused_name();
            match selection_sets::save(model, &self.selection, &name) {
                Ok(saved) => {
                    actions.push(Action::Apply(saved.transaction));
                    let left_out = selection_sets::left_out_note(saved.left_out)
                        .map(|note| format!(" {note}"))
                        .unwrap_or_default();
                    actions.push(Action::Inform(Notice::info(format!(
                        "Saved the selection as {name}. Rename it, select it again or see them \
                         all in Selection sets.{left_out}"
                    ))));
                }
                Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
            }
        }
        if commands.available(Command::SelectionSets) {
            actions.push(Action::Preferences(PreferencesCommand::ShowSelectionSets));
        }
    }

    fn sketch_commands(
        &mut self,
        model: &Model,
        feature: FeatureId,
        drawing: bool,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let Some(owner) = model.document().feature(feature) else {
            return;
        };
        let Some(sketch) = model.displayed_sketch(owner) else {
            return;
        };
        let selected = sketch_tools::selected_entities(&self.selection, feature);
        let moving = Moving::offered(&sketch, feature, &selected, drawing);
        let label = annotations::label_to_move(
            &sketch,
            &selected,
            &sketch_tools::selected_constraints(&self.selection, feature),
            drawing,
        )
        .ok()
        .and_then(|constraint| {
            Some(LabelMoving {
                feature,
                constraint,
                place: self.annotations.label_place(feature, constraint)?,
            })
        });
        let offer = match (&moving, label) {
            (Err(reason), None) => Err(reason.clone()),
            _ => Ok(()),
        };
        if commands.invoke(Command::MoveGeometry, &offer) {
            match moving {
                Ok(moving) => {
                    self.moving = Some(moving);
                    self.moving_label = None;
                }
                Err(_) => self.moving_label = label,
            }
            self.transforming = None;
            self.typed_point.open();
        }
        for (command, transform) in [
            (Command::RotateGeometry, Transform::Rotate),
            (Command::ScaleGeometry, Transform::Scale),
        ] {
            let transforming =
                Transforming::offered(&sketch, feature, &selected, drawing, transform);
            if commands.invoke(command, &transforming)
                && let Ok(transforming) = transforming
            {
                self.transforming = Some(transforming);
                self.moving = None;
                self.moving_label = None;
                self.typed_point.open();
            }
        }
        let everything = sketch_drag::select_all(&sketch);
        if commands.invoke(Command::SelectAll, &everything)
            && let Ok(everything) = everything
        {
            self.add_to_selection(feature, everything, false);
        }
        let projected = owner
            .kind
            .sketch()
            .map(|definition| definition.projected().collect())
            .unwrap_or_default();
        let free = sketch_drag::select_free(&sketch, &projected, model.settled_solution(feature));
        if commands.invoke(Command::SelectFree, &free)
            && let Ok(free) = free
        {
            self.add_to_selection(feature, free, false);
        }
        self.clipboard_commands(model, feature, &sketch, &selected, commands, actions);
    }

    fn clipboard_commands(
        &mut self,
        model: &Model,
        feature: FeatureId,
        sketch: &Sketch,
        selected: &[EntityId],
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let copyable = if selected.iter().any(|entity| !entity.is_reference()) {
            Ok(())
        } else {
            Err(NOTHING_TO_COPY.to_owned())
        };
        let copying = commands.invoke(Command::CopyGeometry, &copyable);
        let cutting = commands.invoke(Command::CutGeometry, &copyable);
        if (copying || cutting) && copyable.is_ok() {
            match sketch.clip(selected) {
                Ok(clip) => {
                    let what = clipped(&clip);
                    self.clipboard = Some(Copied { feature, clip });
                    self.system_clipboard = Some(format!("{CLIPBOARD_MARK}{what}"));
                    if cutting {
                        let mut transaction = sketch_tools::settled_transaction(
                            model,
                            feature,
                            format!("Cut {what}"),
                        );
                        transaction.remove_sketch_items(
                            feature,
                            selected
                                .iter()
                                .copied()
                                .filter(|entity| !entity.is_reference()),
                            [],
                        );
                        self.selection.clear();
                        actions.push(Action::Apply(transaction.finish()));
                    } else {
                        actions.push(Action::Inform(Notice::info(format!("Copied {what}."))));
                    }
                }
                Err(error) => actions.push(Action::Inform(Notice::info(format!(
                    "Nothing was copied: {error}."
                )))),
            }
        }
        let pastable = match &self.clipboard {
            Some(_) => Ok(()),
            None => Err(NOTHING_TO_PASTE.to_owned()),
        };
        if commands.invoke(Command::PasteGeometry, &pastable)
            && let Some(copied) = self.clipboard.clone()
        {
            let offset = self.paste_offset(&copied, feature);
            let mut pasted = Vec::new();
            let label = format!("Paste {}", clipped(&copied.clip));
            let transaction = trimming::reshaped(model, feature, label, |working| {
                pasted = working
                    .paste(&copied.clip, offset)
                    .map_err(|error| format!("The geometry could not be pasted: {error}."))?;
                Ok(())
            });
            match transaction {
                Ok(transaction) => {
                    actions.push(Action::Apply(transaction));
                    self.add_to_selection(feature, pasted, false);
                }
                Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
            }
        }
    }

    fn paste_offset(&self, copied: &Copied, feature: FeatureId) -> Vector2 {
        let centre = copied.clip.centre().unwrap_or(Point2::ZERO);
        if let Some(cursor) = self.sketch_cursor {
            return cursor - centre;
        }
        if copied.feature != feature {
            return Vector2::ZERO;
        }
        Vector2::splat(copied.clip.size().max(PASTE_SHIFT_FLOOR) * PASTE_SHIFT_FRACTION)
    }

    fn step_highlight(&mut self, step: isize, model: &Model, editing: &SketchEditing) {
        let filter = self.active_filter();
        let projecting = editing.active().filter(|active| active.tool.projects());
        let constraints: Vec<Pickable> = editing
            .feature()
            .filter(|_| projecting.is_none())
            .and_then(|feature| {
                let sketch = model.document().feature(feature)?.kind.sketch()?;
                Some(
                    sketch
                        .constraints()
                        .map(|(constraint, _)| Pickable::SketchConstraint {
                            feature,
                            constraint,
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        let highlightable: Vec<Pickable> = self
            .scenes
            .highlightable()
            .iter()
            .copied()
            .filter(|pickable| filter.allows(*pickable))
            .filter(|pickable| {
                projecting.is_none_or(|active| projecting::projectable(model, active, *pickable))
            })
            .chain(constraints)
            .collect();
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
        self.moving_label = self
            .moving_label
            .take()
            .filter(|moving| editing.feature() == Some(moving.feature));
        if self.moving_label.is_some() {
            self.type_label_move(ui, rect, model, actions);
            return;
        }
        self.transforming = self
            .transforming
            .take()
            .filter(|transforming| editing.feature() == Some(transforming.feature));
        if self.transforming.is_some() {
            self.type_transform(ui, rect, model, actions);
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
        let sides = if self.drawing.can_type_sides() {
            format!("   {TYPED_SIDES_HINT}")
        } else {
            String::new()
        };
        let hint = format!(
            "Lengths in {}   {TYPED_POINT_HINT}{sides}",
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
        if let Some(count) = typed_point::sides(&typed.text) {
            if let Err(reason) = self.drawing.set_sides(count) {
                self.typed_point.open_with(typed.text, reason.to_owned());
            }
            return;
        }
        let from = typed_point::From {
            last: self.drawing.last_placed(),
            toward: self.drawing.pointer_position(),
        };
        match typed_point::parse_placed(model, &typed.text, from) {
            Ok(mut placed) => {
                if !self.typed_dimensions {
                    placed.dimensions.clear();
                }
                self.drawing.type_dimensioned(&sketch, placed);
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
        self.modifying.show_text(model, self.typed_point.text());
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
        self.modifying.show_text(model, None);
        match self.modifying.enter_text(model, &typed.text) {
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

    fn type_label_move(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        model: &Model,
        actions: &mut Vec<Action>,
    ) {
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
        let Some(moving) = self.moving_label.take() else {
            return;
        };
        let Some(typed) = typed else {
            if self.typed_point.is_open() {
                self.moving_label = Some(moving);
            }
            return;
        };
        let from = typed_point::From {
            last: Some(moving.place.at),
            toward: None,
        };
        match typed_point::parse(model, &typed.text, from) {
            Ok(target) => actions.push(Action::Apply(moving.transaction(model, target))),
            Err(error) => {
                self.typed_point.open_with(typed.text, error);
                self.moving_label = Some(moving);
            }
        }
    }

    fn type_transform(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        model: &Model,
        actions: &mut Vec<Action>,
    ) {
        let Some(transform) = self
            .transforming
            .as_ref()
            .map(|transforming| transforming.transform)
        else {
            return;
        };
        let field = typed_point::transform_field(transform);
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let typed = self.typed_point.show(
            ui.ctx(),
            top_band(rect),
            anchor,
            field.label,
            field.hint,
            field.placeholder,
        );
        let Some(transforming) = self.transforming.take() else {
            return;
        };
        let Some(typed) = typed else {
            if self.typed_point.is_open() {
                self.transforming = Some(transforming);
            }
            return;
        };
        let commands =
            typed_point::parse_transform(model, &typed.text, transform).and_then(|amount| {
                match transform {
                    Transform::Rotate => transforming.rotated(amount),
                    Transform::Scale => transforming.scaled(amount),
                }
            });
        match commands {
            Ok(commands) => actions.extend(commands.into_iter().map(Action::Drag)),
            Err(error) => {
                self.typed_point.open_with(typed.text, error);
                self.transforming = Some(transforming);
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
            } else if let Some((feature, picks)) = self.picked_dimension(model) {
                match dimensioning::dimension(model, feature, &picks, None) {
                    Ok(added) => self.dimension_added(feature, added, actions),
                    Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
                }
            } else if let Some(Pickable::SketchConstraint {
                feature,
                constraint,
            }) = self.keyboard_highlight
                && editing.feature() == Some(feature)
                && !self.drawing.is_active()
                && model
                    .document()
                    .feature(feature)
                    .and_then(|owner| owner.kind.sketch())
                    .and_then(|sketch| sketch.constraint(constraint))
                    .is_some_and(|defined| defined.dimension().is_some())
            {
                self.annotations.request_field(feature, constraint);
            } else if editing.feature().is_none()
                && let Some(command) = open_command(self.keyboard_highlight, model)
            {
                actions.push(Action::Editing(command));
            } else if editing.feature().is_none()
                && editing.picking().is_none()
                && editing.solid().is_some()
            {
                actions.push(Action::Editing(EditingCommand::CloseSolid));
            }
        }
        if back && self.drawing.in_progress() && self.drawing.remove_last(model.undo_label()) {
            actions.push(Action::Undo);
        }
    }

    fn escape(&mut self, editing: &SketchEditing, actions: &mut Vec<Action>) {
        let active = editing.active();
        if self.pick_list.take().is_some() {
            return;
        }
        if let Some(primary) = self.primary.take() {
            match primary {
                PrimaryDrag::Grab(_) => actions.push(Action::Drag(DragCommand::Cancel)),
                PrimaryDrag::Trim { .. } => self.trimming.cancel_path(),
                PrimaryDrag::Manipulate(manipulating) => actions.push(Action::Preview {
                    feature: manipulating.feature,
                    draft: None,
                }),
                PrimaryDrag::Paint(painting) => self.selection = painting.before,
                PrimaryDrag::Box { .. }
                | PrimaryDrag::ModelBox { .. }
                | PrimaryDrag::Pull { .. } => {}
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
        } else if self.dimensioning.is_some() && !self.selection.is_empty() {
            self.selection.clear();
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
                && !self.modifying.is_active()
                && self.dimensioning.is_none(),
            glyphs: self.glyphs_shown,
            highlight: self.keyboard_highlight,
            first_dimension_scales: self.first_dimension_scales,
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
        let fit_label = if self.selection.is_empty() && self.chosen_rows.is_empty() {
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
        if let Some(PrimaryDrag::Box { area, .. } | PrimaryDrag::ModelBox { area }) = &self.primary
        {
            paint_area(painter, rect, area);
        }
        if let Some((line, label)) = &self.measured
            && let Some(view) = self.view()
            && let Some(pixel) = view.project(line.from.midpoint(line.to))
        {
            let position =
                rect.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32) / self.pixels_per_point;
            if rect.contains(position) {
                let shown = canvas::label(
                    painter,
                    position + vec2(0.0, -MEASURE_LABEL_LIFT),
                    Align2::CENTER_BOTTOM,
                    label,
                    canvas::body(),
                    canvas::MEASURE,
                );
                canvas::announce(ui, shown, "measure", label, None);
            }
        }
        if let Some(view) = self.view() {
            let interference = self.interference.iter().map(|mark| {
                let color = match mark.kind {
                    MarkKind::Overlap => canvas::ERROR,
                    MarkKind::Touch => canvas::MEASURE,
                    MarkKind::Unchecked => canvas::WARNING,
                };
                (mark.place, &mark.label, color)
            });
            let failures = self
                .problems
                .iter()
                .map(|problem| (problem.place, &problem.label, canvas::ERROR));
            for (index, (place, label, color)) in failures.chain(interference).enumerate() {
                let Some(pixel) = view.project(place) else {
                    continue;
                };
                let position = rect.min
                    + egui::Vec2::new(pixel.x as f32, pixel.y as f32) / self.pixels_per_point;
                if rect.contains(position) {
                    let shown = canvas::label(
                        painter,
                        position + vec2(0.0, -PROBLEM_LABEL_LIFT),
                        Align2::CENTER_BOTTOM,
                        label,
                        canvas::small(),
                        color,
                    );
                    canvas::announce(ui, shown, &format!("problem {index}"), label, None);
                }
            }
        }
        let band = top_band(rect);
        let prompt = self.prompt(model, editing, key_hints).map(|(text, keys)| {
            let area = paint_prompt(painter, rect, band, &text, &keys);
            let spoken = format!("{text}. {keys}");
            canvas::announce(ui, area, "prompt", &spoken, Some(Live::Polite));
            area
        });
        self.paint_description(ui, model, editing, key_hints, band, prompt);
        let snap_label = editing
            .feature()
            .and_then(|feature| editing::edited_sketch(document, feature))
            .and_then(|sketch| {
                self.drawing
                    .snap_label(sketch)
                    .or_else(|| match &self.primary {
                        Some(PrimaryDrag::Grab(grab)) => {
                            grab.landing().and_then(|landing| landing.label(sketch))
                        }
                        _ => None,
                    })
            });
        if let (Some(label), Some(cursor)) = (snap_label, self.cursor) {
            let position = rect.min
                + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
            let shown = canvas::label(
                painter,
                position + SNAP_LABEL_OFFSET,
                Align2::LEFT_TOP,
                &label,
                canvas::small(),
                canvas::SNAP,
            );
            canvas::announce(ui, shown, "snap", &label, None);
        }
        let moved = match &self.primary {
            Some(PrimaryDrag::Manipulate(manipulating)) => {
                Some(manipulating.readout(model.units()))
            }
            _ => None,
        };
        if let (Some(size), Some(cursor)) =
            (self.drawing.readout(model.units()).or(moved), self.cursor)
        {
            let position = rect.min
                + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
            let shown = canvas::label(
                painter,
                position + SIZE_READOUT_OFFSET,
                Align2::LEFT_TOP,
                &size,
                canvas::small(),
                canvas::TEXT,
            );
            canvas::announce(ui, shown, "drawing size", &size, None);
        }
        if model.drag_blocked()
            && let Some(cursor) = self.cursor
        {
            let position = rect.min
                + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
            let cue = editing
                .feature()
                .and_then(|feature| model.sketch_conflict(feature))
                .map_or_else(
                    || DRAG_BLOCKED.to_owned(),
                    |conflict| format!("{DRAG_CONFLICT}: {conflict}"),
                );
            let shown = canvas::label(
                painter,
                position + SNAP_LABEL_OFFSET,
                Align2::LEFT_TOP,
                &cue,
                canvas::small(),
                canvas::WARNING,
            );
            canvas::announce(ui, shown, "drag blocked", &cue, Some(Live::Polite));
        }
        let readout_left = rect.left() + view_cube::TRIAD_WIDTH;
        let bottom_left = pos2(readout_left, rect.bottom() - canvas::MARGIN);
        let grid_text = self
            .scenes
            .built()
            .and_then(|built| built.scene.grid.as_ref())
            .zip(self.view())
            .map(|(grid, view)| {
                model
                    .length_unit()
                    .grid_text(grid_minor_spacing(grid, &view))
            });
        let grid_area = grid_text.map(|text| {
            let area = canvas::label(
                painter,
                bottom_left,
                Align2::LEFT_BOTTOM,
                &text,
                canvas::readout(),
                canvas::TEXT,
            );
            let response = ui.interact(area, ui.id().with("grid spacing"), Sense::hover());
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
            area
        });
        let readout_bottom = grid_area.map_or(bottom_left.y, |area| area.top() - READOUT_GAP);
        if let Some(position) = self.sketch_cursor {
            canvas::label(
                painter,
                pos2(readout_left, readout_bottom),
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
        let grid_right = grid_area.map_or(readout_left, |area| area.right() + canvas::MARGIN);
        let hints_left = if editing.feature().is_some() {
            grid_right.max(readout_left + READOUT_ROOM)
        } else {
            grid_right
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
                Some(FeatureKind::OffsetFace(_)) => CHOOSE_MOVED_FACES_PROMPT,
                Some(FeatureKind::Combine(_)) => CHOOSE_BODIES_PROMPT,
                Some(FeatureKind::Move(_)) => CHOOSE_MOVE_PROMPT,
                Some(FeatureKind::Scale(_)) => CHOOSE_SCALE_PROMPT,
                Some(FeatureKind::Mirror(_)) => CHOOSE_MIRROR_PROMPT,
                Some(FeatureKind::Split(_)) => CHOOSE_SPLIT_PROMPT,
                Some(FeatureKind::Mate(_)) => CHOOSE_MATE_PROMPT,
                Some(FeatureKind::Hole(_)) => CHOOSE_HOLE_PROMPT,
                Some(FeatureKind::Primitive(_)) => CHOOSE_PRIMITIVE_PROMPT,
                Some(FeatureKind::Datum(_) | FeatureKind::Pattern(_)) => CHOOSE_REFERENCES_PROMPT,
                _ => CHOOSE_REGIONS_PROMPT,
            };
            Some((prompt.to_owned(), CHOOSE_REGIONS_HINT.to_owned()))
        } else if editing
            .active()
            .is_some_and(|active| active.tool.intersects())
        {
            Some((INTERSECT_PROMPT.to_owned(), key_hints.whole_body.clone()))
        } else if editing
            .active()
            .is_some_and(|active| active.tool.projects())
        {
            Some((PROJECT_PROMPT.to_owned(), key_hints.targets.clone()))
        } else if let Some(feature) = self.dimensioning {
            let picks = sketch_tools::selected_entities(&self.selection, feature);
            let owner = document.feature(feature)?;
            let shown = model.displayed_sketch(owner)?;
            let (text, keys) = dimensioning::prompt(&shown, &picks, self.sketch_cursor);
            Some((text, keys.to_owned()))
        } else if let Some(prompt) = self.trimming.prompt() {
            Some((prompt.to_owned(), key_hints.targets.clone()))
        } else if let Some(prompt) = self.modifying.prompt() {
            (!self.typed_point.is_open()).then(|| {
                let keys = match prompt.hint {
                    Hint::Targets => key_hints.targets.clone(),
                    Hint::Keys(keys) => keys.to_owned(),
                };
                let keys = match self.modifying.mode(editing.modes()) {
                    Some(mode) => format!("{}   {keys}", key_hints.mode(mode)),
                    None => keys,
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
                    let typed_sides = if self.drawing.can_type_sides() {
                        format!("   {TYPED_SIDES_HINT}")
                    } else {
                        String::new()
                    };
                    (
                        prompt.text,
                        format!(
                            "{mode}{reverse}{sides}{}   {FREE_PLACEMENT_HINT}   {HELD_SNAP_HINT}   {TYPE_POINT_HINT}{typed_sides}",
                            prompt.keys
                        ),
                    )
                })
        }
    }

    fn dimension_hover(
        &self,
        model: &Model,
        feature: FeatureId,
        hovered: Option<Pickable>,
    ) -> Option<String> {
        let Some(Pickable::SketchEntity {
            feature: owner,
            entity,
        }) = hovered
        else {
            return None;
        };
        if owner != feature {
            return None;
        }
        let shown = model.displayed_sketch(model.document().feature(feature)?)?;
        let picks = sketch_tools::selected_entities(&self.selection, feature);
        Some(dimensioning::hover_words(&shown, &picks, entity))
    }

    fn paint_description(
        &self,
        ui: &egui::Ui,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
        band: Rect,
        prompt: Option<Rect>,
    ) {
        let hovered = self.annotations.hovered().or(self.highlighted());
        let projecting = editing.active().filter(|active| active.tool.projects());
        let description =
            if let Some(handle) = self.manipulator_hover.filter(|_| self.primary.is_none()) {
                Some(handle.words())
            } else if let Some(active) = projecting {
                hovered
                    .filter(|hovered| projecting::projectable(model, active, *hovered))
                    .and_then(|hovered| projecting::describe(model, active, hovered))
            } else if self.trimming.is_active() {
                edited_sketch(model, editing).and_then(|sketch| self.trimming.label(&sketch))
            } else if self.modifying.is_active() {
                edited_sketch(model, editing)
                    .and_then(|sketch| self.modifying.label(&sketch, model.length_unit()))
            } else if let Some(feature) = self.dimensioning {
                self.dimension_hover(model, feature, hovered)
            } else if let Some((open, copy)) =
                editing.solid().zip(hovered).and_then(|(open, hovered)| {
                    pattern_tools::clicked_copy(model, open, hovered).map(|copy| (open, copy))
                })
            {
                Some(pattern_tools::leave_out_words(
                    model.document(),
                    open,
                    &copy,
                ))
            } else {
                hovered
                    .filter(|_| !self.drawing.is_active())
                    .map(|hovered| hovered.describe(model.document(), model.evaluation()))
            };
        let Some(description) = description else {
            return;
        };
        let painter = ui.painter();
        let label = canvas::Label::new(
            painter,
            description.clone(),
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
        let live = self.keyboard_highlight.is_some().then_some(Live::Polite);
        canvas::announce(ui, shown, "description", &description, live);
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

fn paint_area(painter: &egui::Painter, rect: Rect, area: &ScreenArea) {
    let corner = |at: Vector2| rect.min + egui::Vec2::new(at.x as f32, at.y as f32);
    let area = match area {
        ScreenArea::Box(area) => *area,
        ScreenArea::Lasso(points) => {
            let outline: Vec<egui::Pos2> = points.iter().map(|point| corner(*point)).collect();
            painter.add(Shape::closed_line(
                outline,
                Stroke::new(BOX_STROKE_WIDTH, canvas::SELECTED),
            ));
            return;
        }
    };
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMotion {
    Orbit,
    Pan,
    Zoom,
}

#[derive(Debug, Clone, Copy)]
struct DragButtons {
    primary: bool,
    secondary: bool,
    middle: bool,
    chorded: bool,
}

fn drag_motion(
    mode: InputMode,
    buttons: DragButtons,
    shift: bool,
    alt: bool,
    ctrl: bool,
) -> Option<DragMotion> {
    let right = buttons.secondary && !buttons.middle;
    let alt_drag = mode == InputMode::Laptop && alt && buttons.primary;
    let middle = buttons.middle;
    let (orbit, pan, zoom) = match mode {
        InputMode::Caditor | InputMode::Laptop => (
            (right || alt_drag) && !shift,
            middle || ((right || alt_drag) && shift),
            false,
        ),
        InputMode::Fusion360 => (
            (middle && shift) || (right && !shift),
            (middle && !shift) || (right && shift),
            false,
        ),
        InputMode::FreeCad => (
            (middle && buttons.chorded) || (right && !shift),
            (middle && !buttons.chorded) || (right && shift),
            false,
        ),
        InputMode::Blender => (
            (middle && !shift && !ctrl) || (right && !shift),
            (middle && shift) || (right && shift),
            middle && ctrl && !shift,
        ),
    };
    if zoom {
        Some(DragMotion::Zoom)
    } else if orbit {
        Some(DragMotion::Orbit)
    } else if pan {
        Some(DragMotion::Pan)
    } else {
        None
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
        Pickable::Datum(datum)
        | Pickable::FrameAxis { feature: datum, .. }
        | Pickable::FramePlane { feature: datum, .. } => Some(EditingCommand::OpenSolid(datum)),
        Pickable::Face { body, face } => bodies::shown(model.evaluation(), body)
            .and_then(|shown| bodies::face_origin(shown, face))
            .map(|origin| EditingCommand::OpenSolid(bodies::origin_feature(origin))),
        _ => None,
    }
}

fn pick_action(
    pickable: Option<Pickable>,
    ray: Option<Ray>,
    model: &Model,
    editing: &SketchEditing,
    whole: bool,
) -> Option<Vec<Action>> {
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::PrimitivePlace
    {
        return Some(
            pickable
                .map(|pickable| primitive_tools::place_click(model, picking.feature, pickable, ray))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking() {
        return Some(
            pickable
                .map(|pickable| reference_picking::click(model, picking, pickable))
                .unwrap_or_default(),
        );
    }
    if let Some(active) = editing.active().filter(|active| active.tool.projects()) {
        let target = pickable.filter(|pickable| projecting::projectable(model, active, *pickable));
        return Some(match target {
            Some(target) => match projecting::project(model, active, target, whole) {
                Ok(transaction) => vec![Action::Apply(transaction)],
                Err(reason) => vec![Action::Inform(Notice::info(format!("{reason}.")))],
            },
            None => Vec::new(),
        });
    }
    let copy = editing.solid().zip(pickable).and_then(|(open, pickable)| {
        pattern_tools::clicked_copy(model, open, pickable).map(|copy| (open, copy))
    });
    if let Some((open, copy)) = copy {
        return Some(match pattern_tools::leave_out(model, open, copy) {
            Ok(transaction) => vec![Action::Apply(transaction)],
            Err(reason) => vec![Action::Inform(Notice::info(format!("{reason}.")))],
        });
    }
    let toggled = match pickable {
        Some(Pickable::Region { feature, region }) => {
            Some(solid_tools::toggle_region(model, feature, region))
        }
        Some(Pickable::BlendEdge { feature, edge }) => {
            Some(blend_tools::toggle_edge(model, feature, edge))
        }
        Some(Pickable::ShellFace { feature, face }) => {
            let offsetting = model
                .document()
                .feature(feature)
                .is_some_and(|owner| owner.kind.offset_face().is_some());
            Some(if offsetting {
                offset_face_tools::toggle_face(model, feature, face)
            } else {
                shell_tools::toggle_face(model, feature, face)
            })
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
        Some(pickable) => match DatumTarget::of(model.document(), pickable) {
            Some(datum) => Some(EditingCommand::NewSketchOnDatum(datum)),
            None => FaceChoice::of(pickable).map(EditingCommand::NewSketchOnFace),
        },
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

fn looking_at(view: &View, direction: Vector3, bounds: Aabb) -> Viewpoint {
    let size = view.size();
    let destination =
        Viewpoint::looking_from(direction, bounds.center(), view.viewpoint().distance)
            .unwrap_or(*view.viewpoint());
    View::new(destination, size.x, size.y)
        .with_projection(view.projection())
        .fitted(bounds)
}

struct KeyHints {
    navigation: String,
    highlight: String,
    fit: String,
    reverse: Option<String>,
    sides: Option<String>,
    targets: String,
    whole_body: String,
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
        let whole_body = match commands.keys(Command::IntersectBody) {
            Some(keys) => format!("{keys}: the whole body   {targets}"),
            None => targets.clone(),
        };
        Self {
            navigation,
            highlight,
            targets,
            whole_body,
            fit: commands.with_keys(Command::FitView, "Frame the view around it"),
            reverse: commands
                .keys(Command::ReverseArc)
                .map(|keys| format!("{keys}: the other way round")),
            sides: commands
                .keys(Command::MoreSides)
                .zip(commands.keys(Command::FewerSides))
                .map(|(more, fewer)| {
                    format!("{more} or {fewer}: more or fewer sides   {SCRUB_HINT}")
                }),
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

fn within_reach(point: Point2) -> Option<Point2> {
    (point.is_finite() && point.abs().max_element() <= MAX_LENGTH).then_some(point)
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

    #[test]
    fn a_clicked_point_beyond_the_reach_of_typed_ones_is_not_on_the_sketch() {
        let edge = Point2::new(MAX_LENGTH, -MAX_LENGTH);

        assert_eq!(within_reach(edge), Some(edge));
        assert_eq!(within_reach(Point2::new(MAX_LENGTH * 1.01, 0.0)), None);
        assert_eq!(within_reach(Point2::new(0.0, f64::INFINITY)), None);
        assert_eq!(within_reach(Point2::new(f64::NAN, 0.0)), None);
    }

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
                storage: StorageConfig::default(),
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
    fn moving_the_camera_keeps_the_scene_and_picks_once_it_settles() {
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
        let settled = draw(&mut state, &model, &editing);
        let still = draw(&mut state, &model, &editing);

        assert!(orbited.same_scene(&first) && !orbited.picked);
        assert!(panned.same_scene(&first) && !panned.picked);
        assert!(zoomed_out_a_little.same_scene(&first) && !zoomed_out_a_little.picked);
        assert!(settled.same_scene(&first) && settled.picked);
        assert!(!still.picked);
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

    fn annotated(
        state: &mut ViewportState,
        context: &egui::Context,
        model: &Model,
        feature: FeatureId,
    ) {
        let rect = state.rect.unwrap();
        let input = egui::RawInput {
            screen_rect: Some(rect),
            ..egui::RawInput::default()
        };
        let editing = SketchEditing::editing(feature);
        let mut actions = Vec::new();
        let mut output = context.run_ui(input, |ui| {
            state.annotate(ui, rect, model, &editing, &mut actions);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn only_dimensions_near_the_view_are_laid_out_and_a_still_frame_lays_out_nothing() {
        let mut sketch = Sketch::new(Plane::XY);
        let near = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
        let far = sketch.add_line(Point2::new(5000.0, 0.0), Point2::new(5020.0, 0.0));
        let mut dimension = |line| {
            let Some(&Entity::Line { start, end }) = sketch.entity(line) else {
                panic!("expected a line");
            };
            sketch
                .add_constraint(caditor_sketch::Constraint::Distance {
                    from: start,
                    to: end,
                    value: caditor_expression::Expression::Measure(
                        20.0,
                        caditor_expression::Unit::Millimetre,
                    ),
                })
                .unwrap()
        };
        let shown = dimension(near);
        let hidden = dimension(far);
        let mut document = Document::default();
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Sketch", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let mut model = model_of(document);
        settle(&mut model);
        let context = egui::Context::default();
        let mut state = state_with_cursor();
        state.camera = Camera::new(Viewpoint::facing(
            &Plane::XY,
            Point3::new(10.0, 0.0, 0.0),
            100.0,
        ));

        annotated(&mut state, &context, &model, feature);
        let first = state.annotations.layouts();
        annotated(&mut state, &context, &model, feature);

        assert_eq!(state.annotations.laid_out(), vec![shown]);
        assert_eq!(state.annotations.layouts(), first);

        state.camera = Camera::new(Viewpoint::facing(
            &Plane::XY,
            Point3::new(5010.0, 0.0, 0.0),
            100.0,
        ));
        annotated(&mut state, &context, &model, feature);

        assert_eq!(state.annotations.laid_out(), vec![hidden]);
        assert_eq!(state.annotations.layouts(), first + 1);
    }
}

#[cfg(test)]
pub mod timing {
    use std::time::Instant;

    use caditor_document::{
        BodyOperation, Document, Extrude, ExtrudeExtent, RegionChoice, SolidFeature,
    };
    use caditor_expression::{Expression, Unit};
    use caditor_sketch::{Constraint, Sketch};
    use egui::{RawInput, pos2};

    use super::{
        tests::{model_of, settle},
        *,
    };

    const FRAMES: u32 = 200;
    const WARM_UP: u32 = 3;

    const PARALLEL_LINES: usize = 4000;
    const DIMENSIONED_LINES: usize = 4000;
    const EQUAL_RADII: usize = 5;

    pub fn large_sketch() -> Sketch {
        let mut sketch = Sketch::new(Plane::XY);
        let mut lines = Vec::new();
        for row in 0..100 {
            for column in 0..200 {
                let at = Point2::new(f64::from(column) * 5.0, f64::from(row) * 5.0);
                lines.push(sketch.add_line(at, at + Vector2::new(3.0, 1.0)));
            }
        }
        let mut circles = Vec::new();
        for index in 0..2000 {
            let at = Point2::new(
                f64::from(index % 50) * 20.0,
                -20.0 - f64::from(index / 50) * 20.0,
            );
            let radius = 4.0 + f64::from(index % 5);
            circles.push((sketch.add_circle(at, radius), radius));
            sketch.add_arc(at, at + Vector2::new(8.0, 0.0), at + Vector2::new(0.0, 8.0));
        }
        constrain(&mut sketch, &lines, &circles);
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

    fn constrain(sketch: &mut Sketch, lines: &[EntityId], circles: &[(EntityId, f64)]) {
        let millimetres = |value| Expression::Measure(value, Unit::Millimetre);
        for [first, second] in lines.as_chunks::<2>().0.iter().take(PARALLEL_LINES / 2) {
            sketch
                .add_constraint(Constraint::Parallel(*first, *second))
                .unwrap();
        }
        for line in lines.iter().take(DIMENSIONED_LINES) {
            let Some(&Entity::Line { start, end }) = sketch.entity(*line) else {
                panic!("expected a line");
            };
            sketch
                .add_constraint(Constraint::HorizontalDistance {
                    from: start,
                    to: end,
                    value: millimetres(3.0),
                })
                .unwrap();
        }
        for group in circles.as_chunks::<{ EQUAL_RADII * 2 }>().0 {
            for (first, second) in group.iter().zip(group.iter().skip(EQUAL_RADII)) {
                sketch
                    .add_constraint(Constraint::Radius {
                        entity: first.0,
                        value: millimetres(first.1),
                    })
                    .unwrap();
                sketch
                    .add_constraint(Constraint::Equal(first.0, second.0))
                    .unwrap();
            }
        }
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

    pub fn sketch_document(sketch: Sketch) -> (Document, FeatureId) {
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
                start: None,
                other_bodies: Vec::new(),
                taper: None,
                wall: None,
            })),
        );
        document.apply(transaction.finish()).unwrap();
        document
    }

    pub fn settled(document: Document) -> Model {
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

    impl Scenario<'_> {
        fn time_marks(
            &self,
            name: &str,
            viewpoint: Viewpoint,
            mut change: impl FnMut(&mut ViewportState, u32),
        ) {
            let context = egui::Context::default();
            context.set_fonts(crate::fonts::definitions_with(&[]));
            let mut state = placed_state();
            state.camera = Camera::new(viewpoint);
            let rect = state.rect.unwrap();
            let frame = |state: &mut ViewportState| {
                let input = RawInput {
                    screen_rect: Some(rect),
                    ..RawInput::default()
                };
                let mut actions = Vec::new();
                let mut output = context.run_ui(input, |ui| {
                    state.annotate(ui, rect, self.model, self.editing, &mut actions);
                });
                output.textures_delta.clear();
            };
            for index in 0..WARM_UP {
                change(&mut state, index);
                frame(&mut state);
            }
            let started = Instant::now();
            for index in 0..FRAMES {
                change(&mut state, index);
                frame(&mut state);
            }
            eprintln!("{name}: {:?} per frame", started.elapsed() / FRAMES);
        }
    }

    fn facing_sketch(x: f64, y: f64, distance: f64) -> Viewpoint {
        Viewpoint::facing(&Plane::XY, Point3::new(x, y, 0.0), distance)
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
        let whole = facing_sketch(565.0, 190.0, 2600.0);
        let close = facing_sketch(60.0, 60.0, 150.0);
        scenario.time_marks("marks, whole sketch, idle", whole, |_, _| {});
        scenario.time_marks("marks, whole sketch, camera moving", whole, |state, _| {
            orbit(state);
        });
        scenario.time_marks("marks, zoomed in, idle", close, |_, _| {});
        scenario.time_marks("marks, zoomed in, camera moving", close, |state, _| {
            orbit(state);
        });
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

#[cfg(test)]
mod navigation_tests {
    use super::*;

    const NONE: DragButtons = DragButtons {
        primary: false,
        secondary: false,
        middle: false,
        chorded: false,
    };
    const MIDDLE: DragButtons = DragButtons {
        middle: true,
        ..NONE
    };
    const RIGHT: DragButtons = DragButtons {
        secondary: true,
        ..NONE
    };
    const MIDDLE_WITH_LEFT: DragButtons = DragButtons {
        middle: true,
        chorded: true,
        ..NONE
    };

    #[test]
    fn each_navigation_profile_maps_its_drags_like_the_program_it_follows() {
        let motion = |mode, buttons, shift, ctrl| drag_motion(mode, buttons, shift, false, ctrl);

        assert_eq!(
            motion(InputMode::Caditor, RIGHT, false, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(
            motion(InputMode::Caditor, MIDDLE, false, false),
            Some(DragMotion::Pan)
        );
        assert_eq!(
            motion(InputMode::Fusion360, MIDDLE, false, false),
            Some(DragMotion::Pan)
        );
        assert_eq!(
            motion(InputMode::Fusion360, MIDDLE, true, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(
            motion(InputMode::FreeCad, MIDDLE, false, false),
            Some(DragMotion::Pan)
        );
        assert_eq!(
            motion(InputMode::FreeCad, MIDDLE_WITH_LEFT, false, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(
            motion(InputMode::Blender, MIDDLE, false, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(
            motion(InputMode::Blender, MIDDLE, true, false),
            Some(DragMotion::Pan)
        );
        assert_eq!(
            motion(InputMode::Blender, MIDDLE, false, true),
            Some(DragMotion::Zoom)
        );
        assert_eq!(
            motion(InputMode::Blender, RIGHT, false, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(motion(InputMode::Blender, NONE, false, false), None);
    }
}

fn scene_items(ui: &mut Ui, view: Id, corner: Pos2, items: &[Item]) {
    let nowhere = Rect::from_min_size(corner, Vec2::ZERO);
    let node = |ui: &mut Ui, id: Id, name: &str| {
        let response = ui.interact(nowhere, id, Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
        response.id
    };
    let within = |parent: Id| {
        UiBuilder::new()
            .accessibility_parent(parent)
            .max_rect(nowhere)
            .sense(Sense::empty())
    };
    ui.scope_builder(within(view), |ui| {
        for (index, item) in items.iter().enumerate() {
            let parent = node(ui, view.with(("scene item", index)), &item.name);
            if item.parts.is_empty() {
                continue;
            }
            ui.scope_builder(within(parent), |ui| {
                for (part_index, part) in item.parts.iter().enumerate() {
                    node(ui, parent.with(("part", part_index)), part);
                }
            });
        }
    });
}
