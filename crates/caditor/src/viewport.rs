use std::{borrow::Cow, fmt::Display, sync::Arc, time::Duration};

use caditor_document::{
    BlendKind, CarriedParameters, Datum, Document, Evaluation, Feature, FeatureId, FeatureKind,
    PasteOrigin, PatternKind, RegionChoice, Rgb, SavedView, SolidFeature, Transaction,
};
use caditor_file::{ClipboardError, PastedGeometry};
use caditor_geometry::{Aabb, Plane, Point2, Point3, Ray, Rotation3, Vector2, Vector3};
use caditor_kernel::EdgeName;
use caditor_render::{
    Backdrop, Camera, Lighting, MAX_SECTION_PLANES, PickHit, PickResult, PlaneProjection,
    ProjectionMode, Reflection, Scene, SectionPlane, SurfaceSize, View, Viewpoint, ViewportRect,
    grid_snap_spacing, is_cut_away, section_slack,
};
use caditor_sketch::{ClipError, ConstraintId, Entity, EntityId, Faceting, MAX_LENGTH, Sketch};
use egui::{
    Align, Align2, Id, Key, Modifiers, PointerButton, Pos2, Rect, Response, Role, Sense, Shape,
    Stroke, Ui, UiBuilder, Vec2, WidgetInfo, accesskit::Live, pos2, vec2,
};

use crate::{
    analysis::{Analyses, FaceAnalysis},
    annotations::{self, Annotations, LabelMoving, Surface},
    blend_tools,
    bodies::{self, BodyMeshes, OpenDraft},
    body_picks::{self, BodyPick, Staged},
    body_selection,
    body_snap::BodySnaps,
    box_selection::{self, Catch},
    canvas::{self, HintList, KeyHint, Rank},
    canvas_guidance::{self, HoverDelay, OrbitMark, PointerLabel, RefusalNote, Waited},
    clipboard,
    comb::CombDrawing,
    combine_tools,
    commands::{CameraMove, Command, CommandFrame, Pasted, Reason, StandardView},
    delete_face_tools, dimensioning,
    display::Displayed,
    display_picker,
    display_style::DisplayStyle,
    draft_tools,
    drag_solver::DragCommand,
    drawing::{self, Drawing, Ended, Preview},
    editing::{self, EditingCommand, SketchEditing, Tool},
    emboss_tools,
    faceting::FacetLevel,
    feature_tree,
    gearing::GearSettings,
    handle_snap::{self, Guide, Snap},
    hole_placement, hole_tools,
    image_tools::{self, CalibrationGuide},
    inference::Inferences,
    interference_panel::{Mark, MarkKind},
    isocurves::IsocurveDrawing,
    look_at::{self, LookTarget},
    manipulator::{self, Manipulating, Manipulator},
    measure::MeasuredLine,
    measurement_tools,
    menu_bar::MenuEntries,
    mirror_tools,
    model::{Action, Model, Notice, NoticeKind, RecomputeStatus},
    model_dimensions::{self, ModelDimensions, Outside},
    modifying::{HighlightNeeded, Hint, Modifying, Outcome, SketchVersion, StepError},
    move_face_tools,
    move_manipulator::Handle,
    offset_face_tools,
    paint_selection::{self, Painting},
    pattern_tools,
    pick_list::{self, PickList},
    preferences::{
        InputMode, Navigation, OrbitAround, PreferenceChange, PreferencesCommand, Sketching,
        StartingToggles, Transitions, ViewDetails, ZoomToward,
    },
    primitive_tools, projecting,
    queued::{Pending, Queued, Rerun},
    reference_picking::{self, Slot},
    replace_face_tools, reversing, saved_views,
    scene::{self, BuiltScene, EditedSketch, Highlight, PickTable, Sources},
    scene_cache::{Overlay, Revisions, SceneCache, SceneInputs},
    scene_description::{Item, SceneDescription},
    scene_palette::{self, Canvas, Contrast},
    section::{self, SectionCommand},
    sectioned_screen::SectionedScreen,
    selection::{Pickable, Selection, SelectionFilter},
    selection_sets,
    shape_modes::ShapeMode,
    shell_tools,
    sketch_drag::{self, BoxMode, Grab, Moving, ScreenArea, Transform, Transforming},
    sketch_placement::{DatumTarget, FaceChoice},
    sketch_status, sketch_toolbar, sketch_tools,
    snap::{Accept, Hold, Pointer, Screen, Target},
    snapshot, solid_tools, split_face_tools,
    start_card::{self, Start, StartCard},
    symmetric_drawing::{self, Mirror, Symmetry},
    system_motion,
    texting::TextSettings,
    toggles,
    trimming::{self, Trimming},
    typed_point::{self, Typed, TypedPoint, TypedPointError, TypedValueError},
    upload_badge::UploadRequest,
    view_aids::{DefaultBody, ViewAids},
    view_cube::{self, Corner, CubeAction, CubePlacement, CubeTexts},
    view_history::{Gesture, ViewHistory},
    view_menu::{self, Held, Place, ViewMenu},
    visibility,
};

const INITIAL_LOOK_FROM: Vector3 = Vector3::new(1.0, -1.0, 1.0);
const INITIAL_DISTANCE: f64 = 200.0;
const SUBJECT_ITEMS_READ: usize = 16;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
const HIT_CURSOR_TOLERANCE_POINTS: f64 = 1.5;
const SEEN_DEPTH_SLACK_POINTS: f64 = 7.5;
const READOUT_ROOM: f32 = 200.0;
const DRAG_DRAWS_FROM_PRESS: f64 = 12.0;
pub const PLACE_AT_A_POINT: &str = "While drawing, Space places at the highlighted point or curve: \
                                    highlight one of the sketch's points or curves, or the origin.";
const VIEWPORT_NAME: &str = "3D view";
const ISOMETRIC_NOT_CHANGED: &str = "The Isometric view has not been changed";
const NOT_IN_A_SKETCH: &str = "Edit a sketch to look straight at it";
pub const DRAG_BLOCKED: &str = "The constraints do not allow it there";
pub const DRAG_CONFLICT: &str = "Nothing moves while constraints conflict";
pub const PROJECTED_STAYS: &str =
    "Projected geometry follows the model geometry it comes from; change the model instead";
pub const REFERENCE_STAYS: &str = "The origin and axes stay put";
const READOUT_GAP: f32 = 4.0;
const READOUT_LIFTS: u8 = 3;
const NOTHING_TO_COPY: &str = "Select sketch geometry to copy";
const NOTHING_TO_PASTE: &str = "Nothing was pasted: copy or cut sketch geometry first.";
pub const CHOOSE_PLANE_PROMPT: &str = "Click a plane or a flat face to sketch on";
const CHOOSE_PLANE_HINT: &str = "Esc: cancel";
const DONE_HINT: &str = "Esc: done";
pub const SELECT_PROMPT: &str = "Choose a tool to draw, or click geometry to select it";
const PROJECT_PROMPT: &str = "Click an edge, corner or face of a body, a curve or point of another \
     sketch, an axis, a datum point or the origin, to project it";
const INTERSECT_PROMPT: &str = "Click a face to draw where the sketch cuts it, Shift-click for its \
     whole body, or click a datum plane";
const KEYBOARD_ORBIT_FRACTION: f64 = 1.0 / 12.0;
const KEYBOARD_PAN_FRACTION: f64 = 0.1;
const KEYBOARD_ZOOM_FACTOR: f64 = 1.25;
const TYPED_POINT_OFFSET: f32 = 64.0;
const FREE_PLACEMENT_HINT: &str = "Ctrl: place freely";
const SNAPPING_OFF_HINT: &str = "Snapping off";
const TAKE_BACK_HINT: &str = "Backspace: take back the last point";
const HELD_SNAP_HINT: &str = "Alt: snap to the grid or nearby geometry";
const TYPE_POINT_HINT: &str = "x,y or length<angle: type an exact point";
const TYPED_POINT_HINT: &str = "@: from the last point   A length alone goes toward the pointer   \
                                < angle alone: lock the direction   length <: lock the length   \
                                Enter: place   Esc: cancel";
pub const TYPE_VALUE_UNAVAILABLE: &str = "Choose a drawing tool, or Offset, Sketch fillet, a \
                                          pattern or Tangent circle, to type an exact value";
const TYPED_SIDES_HINT: &str = "6 sides: set the sides";
const TYPED_RHO_HINT: &str = "0.3 rho: set its shape";
const SCRUB_HINT: &str = "Shift: move sideways to set the sides";
const MOVE_HINT: &str = "@: by an offset   Enter: move   Esc: cancel";
const BOX_FILL_OPACITY: f32 = 0.12;
const BOX_STROKE_WIDTH: f32 = 1.0;
const BOX_DASH: f32 = 6.0;
const BOX_GAP: f32 = 4.0;
const THROUGH_STROKE_WIDTH: f32 = 4.0;
pub const TAKING_SEEN_PROMPT: &str = "Taking only what can be seen";
pub const TAKING_HIDDEN_PROMPT: &str = "Taking what is hidden behind too";
pub const HOLD_ALT_HINT: &str = "Alt: also take what is hidden";
pub const LET_GO_OF_ALT_HINT: &str = "Alt: let go to take only what can be seen";
const SELECT_THROUGH_ON_HINT: &str = "Select through is on: View › Selection tools turns it off";
const MEASURE_LABEL_LIFT: f32 = 6.0;
const PROBLEM_LABEL_LIFT: f32 = 9.0;
const PLACE_SHARE: f64 = 0.2;
const MIN_PLACE_REACH: f64 = 1.0;
const NO_VIEW_YET: &str = "The 3D view is not shown yet";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum PasteRefusal {
    #[error(transparent)]
    SketchGone(#[from] trimming::SketchGone),
    #[error("The geometry could not be pasted: {source}.")]
    Paste { source: ClipError },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum CopyError {
    #[error("Nothing was copied: {0}.")]
    Clip(ClipError),
    #[error("Nothing was copied: {0}.")]
    Text(ClipboardError),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum SymmetryError {
    #[error("{}", symmetric_drawing::NoLineSelected)]
    NoLineSelected,
    #[error("{refusal}")]
    ChosenLine {
        refusal: symmetric_drawing::NoLineSelected,
    },
}

impl Reason for SymmetryError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
enum StartCardRefusal {
    #[error("{}", start_card::NOT_EMPTY)]
    NotEmpty,
    #[error("{}", start_card::ALREADY_SHOWN)]
    AlreadyShown,
}

impl Reason for StartCardRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
enum ListRefusal {
    #[error("This tool picks its own targets: press Esc to go back to Select first")]
    WithTool,
    #[error("{}", StepError::NothingToPick)]
    NothingToHighlight,
}

impl Reason for ListRefusal {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
enum TypedEditError {
    #[error("{}", Pending::SketchShown)]
    SketchShown,
    #[error(transparent)]
    Moved(#[from] sketch_drag::MoveError),
    #[error(transparent)]
    Transformed(#[from] sketch_drag::TransformError),
    #[error(transparent)]
    Point(#[from] TypedPointError),
    #[error(transparent)]
    Value(#[from] TypedValueError),
}

impl TypedEditError {
    fn pending(&self) -> Option<Pending> {
        matches!(self, Self::SketchShown).then_some(Pending::SketchShown)
    }
}

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

impl SketchScreen {
    pub fn new(view: View, plane: Plane, pixels_per_point: f64) -> Self {
        Self {
            view,
            plane,
            pixels_per_point,
        }
    }

    pub fn projector(&self) -> SketchProjector {
        SketchProjector(
            self.view
                .plane_projection(&self.plane, self.pixels_per_point),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchProjector(PlaneProjection);

impl Screen for SketchProjector {
    fn to_screen(&self, point: Point2) -> Option<Vector2> {
        self.0.project(point)
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
struct PendingClick {
    at: PickSource,
    click: Click,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    cursor: Vector2,
    view: View,
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
    Unmovable {
        feature: FeatureId,
        held: Unmovable,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unmovable {
    Projected,
    Reference,
}

impl Unmovable {
    fn reason(self) -> &'static str {
        match self {
            Self::Projected => PROJECTED_STAYS,
            Self::Reference => REFERENCE_STAYS,
        }
    }
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
    pending_click: Option<PendingClick>,
    pointer_hit: Option<PointerHit>,
    cursor: Option<Vector2>,
    rect: Option<Rect>,
    pixels_per_point: f32,
    drag_anchor: Option<Point3>,
    needs_initial_fit: bool,
    fit_requested: Option<FitAsked>,
    history: ViewHistory,
    clock: f64,
    picks_in_flight: Option<(Arc<PickTable>, View)>,
    last_pick: Option<PickKey>,
    requested_viewpoint: Option<Viewpoint>,
    edited: Option<FeatureId>,
    face_edited_sketch: Option<SketchFacing>,
    checked_availability: Option<Availability>,
    sketch_cursor: Option<Point2>,
    drawing: Drawing,
    trimming: Trimming,
    modifying: Modifying,
    annotations: Annotations,
    bodies: BodyMeshes,
    navigation: Navigation,
    sketching: Sketching,
    view_cube_shown: bool,
    cube_placement: CubePlacement,
    keyboard_highlight: Option<Pickable>,
    pick_list: Option<PickList>,
    list_hold: bool,
    last_cursor: Option<Vector2>,
    context_menu: Option<ViewMenu>,
    menu_waits_for_pick: Option<PickSource>,
    menus_opened: u64,
    list_at: Option<Vector2>,
    row_to_choose: Option<FeatureId>,
    typed_point: TypedPoint,
    resubmitted: Option<String>,
    typed_owner: Option<(FeatureId, Tool)>,
    typed_handle: Option<manipulator::Typing>,
    typed_handle_preview: Option<String>,
    handle_snap: Option<(Vector2, Guide, Option<Snap>)>,
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
    previewed: Vec<Pickable>,
    chosen_rows: Vec<FeatureId>,
    lit_rows: Option<(LitRowsBasis, LitRows)>,
    hovered_row: Option<FeatureId>,
    tree_dismissed: bool,
    home_applied: bool,
    session: u64,
    fit_when_computed: bool,
    scene_bounds: Option<Aabb>,
    measured: Option<(MeasuredLine, String)>,
    calibration: Option<CalibrationGuide>,
    problems: Vec<Problem>,
    kept: Vec<measurement_tools::Shown>,
    interference: Vec<Mark>,
    uploading: bool,
    upload_stopped: bool,
    upload_request: Option<UploadRequest>,
    comb: Option<Arc<CombDrawing>>,
    isocurves: Option<Arc<IsocurveDrawing>>,
    framed_place: Option<Point3>,
    look_from: Option<LookTarget>,
    scenes: SceneCache,
    description: SceneDescription,
    filter: SelectionFilter,
    filter_applies: bool,
    style: DisplayStyle,
    style_before_see_through: DisplayStyle,
    contrast: Contrast,
    canvas: Canvas,
    grid: Option<egui::Color32>,
    snapping: bool,
    grid_snapping: bool,
    lasso: bool,
    paint: bool,
    select_through: bool,
    through_held: bool,
    typed_dimensions: bool,
    symmetry: Option<Symmetry>,
    mirror: Option<Mirror>,
    first_dimension_scales: bool,
    glyphs_shown: bool,
    dimensions_shown: bool,
    model_dimensions: ModelDimensions,
    aids: ViewAids,
    section: Vec<SectionPlane>,
    sketch_slice: bool,
    analyses: Analyses,
    manipulator: Option<Manipulator>,
    manipulator_hover: Option<Handle>,
    dimensioning: Option<FeatureId>,
    body_snaps: Arc<BodySnaps>,
    body_hovered: Option<Pickable>,
    more_keys: bool,
    chosen_commands: Vec<Command>,
    display_button: Option<Response>,
    display_area: Rect,
    start_card: StartCard,
    covered: bool,
    refusal: Option<RefusalNote>,
    hover_delay: HoverDelay,
    orbit_mark: Option<OrbitMark>,
    pointer_labels: Vec<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Described {
    Highlighted(String),
    Hovered(String),
    Tool(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FitAsked {
    ByUser,
    OnOpening,
}

#[derive(Debug, Clone, PartialEq)]
struct Problem {
    place: Point3,
    label: String,
}

fn with_their_bodies(document: &Document, rows: Vec<FeatureId>) -> Vec<FeatureId> {
    let bodies: Vec<FeatureId> = rows
        .iter()
        .filter_map(|row| row_body(document.feature(*row)?))
        .collect();
    let mut lit = rows;
    lit.extend(bodies);
    lit.sort_unstable();
    lit.dedup();
    lit
}

fn mirrored_bodies(model: &Model, context: editing::Context) -> Vec<FeatureId> {
    context
        .solid
        .and_then(|open| model.document().feature(open))
        .and_then(|feature| match &feature.kind {
            FeatureKind::Mirror(mirror) if mirror.mirrors_whole_body() => {
                Some(mirror.bodies().collect())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn chooses_bodies(editing: &SketchEditing) -> bool {
    editing.choosing_bodies()
        || editing
            .picking()
            .is_some_and(|picking| picking.slot == Slot::MirrorBodies)
}

fn whole_body_hovered(model: &Model, pickable: Pickable) -> Vec<Pickable> {
    let Some(body) = pickable.body() else {
        return vec![pickable];
    };
    match body_selection::whole_bodies(model, &[body], body_selection::Kind::Faces) {
        faces if faces.is_empty() => vec![pickable],
        faces => faces,
    }
}

fn row_body(feature: &Feature) -> Option<FeatureId> {
    match feature.makes_body() {
        true => Some(feature.id()),
        false => feature.body(),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct LitRows {
    rows: Vec<FeatureId>,
    made: Vec<Pickable>,
}

impl LitRows {
    fn of(document: &Document, evaluation: &Evaluation, rows: &[FeatureId]) -> Self {
        let mut lit = Self::default();
        for row in rows {
            lit.rows.push(*row);
            let Some(body) = document.feature(*row).and_then(row_body) else {
                continue;
            };
            let made = match body == *row {
                true => Vec::new(),
                false => bodies::shown(evaluation, body)
                    .map(|result| bodies::made_by(body, &result.solid, *row))
                    .unwrap_or_default(),
            };
            match made.is_empty() {
                true => lit.rows.push(body),
                false => lit.made.extend(made),
            }
        }
        lit.rows.sort_unstable();
        lit.rows.dedup();
        lit
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LitRowsBasis {
    rows: Vec<FeatureId>,
    revision: u64,
    evaluation: u64,
}

fn hides_what_lies_behind(pickable: Pickable) -> bool {
    matches!(pickable, Pickable::Face { .. } | Pickable::ShellFace { .. })
}

fn copied_text(
    model: &Model,
    feature: FeatureId,
    sketch: &Sketch,
    selected: &[EntityId],
) -> Result<(String, String), CopyError> {
    let clip = sketch.clip(selected).map_err(CopyError::Clip)?;
    let parameters = CarriedParameters::of(model.document(), model.parameters(), clip.parameters());
    let text =
        caditor_file::sketch_clipboard_text(&clip, &parameters, &clipboard::source(model), feature)
            .map_err(CopyError::Text)?;
    Ok((text, clipboard::described(&clip)))
}

fn paste_note(pasted: &PastedGeometry) -> Option<String> {
    let mut parts = Vec::new();
    if pasted.inlined > 0 {
        parts.push(format!(
            "{} written as {}, since this model has no parameter of {} name.",
            feature_tree::count(pasted.inlined, "value", "values"),
            if pasted.inlined == 1 {
                "a number"
            } else {
                "numbers"
            },
            if pasted.inlined == 1 { "its" } else { "their" },
        ));
    }
    parts.extend(pasted.notes.iter().cloned());
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn clear_of(label: Rect, keep_out: Rect, view: Rect) -> Rect {
    if !label.intersects(keep_out) {
        return label;
    }
    if keep_out.center().x < view.center().x {
        let shift = keep_out.right() + canvas::MARGIN - label.left();
        let moved = label.translate(vec2(shift, 0.0));
        let past_right = (moved.right() - view.right()).max(0.0);
        return moved.translate(vec2(-past_right, 0.0));
    }
    let shift = keep_out.left() - canvas::MARGIN - label.right();
    let moved = label.translate(vec2(shift, 0.0));
    let past_left = (view.left() - moved.left()).max(0.0);
    moved.translate(vec2(past_left, 0.0))
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
    pub fn gear_settings(&mut self) -> &mut GearSettings {
        self.modifying.gear_settings()
    }

    pub fn text_settings(&mut self) -> &mut TextSettings {
        self.modifying.text_settings()
    }

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
            fit_requested: None,
            history: ViewHistory::default(),
            clock: 0.0,
            picks_in_flight: None,
            last_pick: None,
            requested_viewpoint: None,
            edited: None,
            face_edited_sketch: None,
            checked_availability: None,
            sketch_cursor: None,
            drawing: Drawing::default(),
            trimming: Trimming::default(),
            modifying: Modifying::default(),
            annotations: Annotations::default(),
            bodies: BodyMeshes::default(),
            navigation: Navigation::default(),
            sketching: Sketching::default(),
            view_cube_shown: true,
            cube_placement: CubePlacement::default(),
            keyboard_highlight: None,
            pick_list: None,
            list_hold: false,
            last_cursor: None,
            context_menu: None,
            menu_waits_for_pick: None,
            menus_opened: 0,
            list_at: None,
            row_to_choose: None,
            typed_point: TypedPoint::default(),
            resubmitted: None,
            typed_owner: None,
            typed_handle: None,
            typed_handle_preview: None,
            handle_snap: None,
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
            previewed: Vec::new(),
            chosen_rows: Vec::new(),
            lit_rows: None,
            hovered_row: None,
            tree_dismissed: false,
            home_applied: false,
            session: 0,
            fit_when_computed: false,
            scene_bounds: None,
            measured: None,
            calibration: None,
            problems: Vec::new(),
            kept: Vec::new(),
            interference: Vec::new(),
            uploading: false,
            upload_stopped: false,
            upload_request: None,
            comb: None,
            isocurves: None,
            framed_place: None,
            look_from: None,
            scenes: SceneCache::default(),
            description: SceneDescription::default(),
            filter: SelectionFilter::default(),
            filter_applies: true,
            style: DisplayStyle::default(),
            style_before_see_through: DisplayStyle::default(),
            contrast: Contrast::default(),
            canvas: Canvas::default(),
            grid: None,
            snapping: true,
            grid_snapping: false,
            lasso: false,
            paint: false,
            select_through: false,
            through_held: false,
            typed_dimensions: true,
            symmetry: None,
            mirror: None,
            first_dimension_scales: false,
            glyphs_shown: true,
            dimensions_shown: false,
            model_dimensions: ModelDimensions::default(),
            aids: ViewAids::default(),
            section: Vec::new(),
            sketch_slice: false,
            analyses: Analyses::default(),
            manipulator: None,
            manipulator_hover: None,
            dimensioning: None,
            body_snaps: Arc::default(),
            body_hovered: None,
            more_keys: false,
            chosen_commands: Vec::new(),
            display_button: None,
            display_area: Rect::NOTHING,
            start_card: StartCard::default(),
            covered: false,
            refusal: None,
            hover_delay: HoverDelay::default(),
            orbit_mark: None,
            pointer_labels: Vec::new(),
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

    pub fn draws_symmetrically(&self) -> bool {
        self.symmetry.is_some()
    }

    fn symmetry_offered(
        &self,
        model: &Model,
        editing: &SketchEditing,
    ) -> Result<Symmetry, SymmetryError> {
        let feature = editing.feature().ok_or(SymmetryError::NoLineSelected)?;
        let shown = model
            .document()
            .feature(feature)
            .and_then(|owner| model.displayed_sketch(owner))
            .ok_or(SymmetryError::NoLineSelected)?;
        let selected = sketch_tools::selected_entities(&self.selection, feature);
        let line = symmetric_drawing::chosen_line(&shown, &selected)
            .map_err(|refusal| SymmetryError::ChosenLine { refusal })?;
        Ok(Symmetry { feature, line })
    }

    fn symmetry_command(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        if editing.feature().is_none() {
            return;
        }
        let offered = match self.symmetry {
            Some(symmetry) => Ok(symmetry),
            None => self.symmetry_offered(model, editing),
        };
        if !commands.invoke(Command::DrawSymmetrically, &offered) {
            return;
        }
        if self.symmetry.take().is_some() {
            if commands.state_unseen(Command::DrawSymmetrically) {
                actions.push(Action::Inform(Notice::info(symmetric_drawing::TURNED_OFF)));
            }
            return;
        }
        let Ok(symmetry) = offered else {
            return;
        };
        self.symmetry = Some(symmetry);
        let label = model
            .document()
            .feature(symmetry.feature)
            .and_then(|owner| model.displayed_sketch(owner))
            .map(|shown| shown.entity_label(symmetry.line))
            .unwrap_or_default();
        actions.push(Action::Inform(Notice::info(symmetric_drawing::turned_on(
            &label,
        ))));
    }

    pub fn first_dimension_scales(&self) -> bool {
        self.first_dimension_scales
    }

    pub fn aids(&self) -> ViewAids {
        self.aids
    }

    pub fn set_uploading(&mut self, uploading: bool) {
        self.uploading = uploading;
    }

    pub fn is_uploading(&self) -> bool {
        self.uploading
    }

    pub fn set_upload_stopped(&mut self, stopped: bool) {
        self.upload_stopped = stopped;
    }

    pub fn is_upload_stopped(&self) -> bool {
        self.upload_stopped
    }

    pub fn request_upload(&mut self, request: UploadRequest) {
        self.upload_request = Some(request);
    }

    pub fn take_upload_request(&mut self) -> Option<UploadRequest> {
        self.upload_request.take()
    }

    pub fn set_section(&mut self, section: Vec<SectionPlane>) {
        self.section = section;
    }

    pub fn set_lighting(&mut self, lighting: Lighting) {
        self.scenes.set_lighting(Some(lighting));
    }

    pub fn set_backdrop(&mut self, backdrop: Option<Backdrop>) {
        self.scenes.set_backdrop(backdrop);
    }

    pub fn model_centre(&self) -> Option<Point3> {
        Some(self.scenes.built()?.model?.center())
    }

    pub fn sketch_slice(&self) -> bool {
        self.sketch_slice
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

    #[cfg(test)]
    pub fn analyses_mut(&mut self) -> &mut Analyses {
        &mut self.analyses
    }

    pub fn glyphs_shown(&self) -> bool {
        self.glyphs_shown
    }

    pub fn dimensions_shown(&self) -> bool {
        self.dimensions_shown
    }

    #[cfg(test)]
    pub fn model_labels(&self) -> &[Pickable] {
        self.model_dimensions.highlightable()
    }

    #[cfg(test)]
    pub fn model_dimensions(&self) -> &ModelDimensions {
        &self.model_dimensions
    }

    pub fn style(&self) -> DisplayStyle {
        self.style
    }

    pub fn set_style(&mut self, style: DisplayStyle) {
        if !self.style.is_translucent() {
            self.style_before_see_through = self.style;
        }
        self.style = style;
    }

    fn choose_style(&mut self, style: DisplayStyle) {
        if style == self.style && style.is_translucent() {
            self.style = self.style_before_see_through;
        } else {
            self.set_style(style);
        }
    }

    pub fn cover(&mut self, covered: bool) {
        self.covered = covered;
    }

    pub fn set_start_card(&mut self, enabled: bool) {
        self.start_card.set_enabled(enabled);
    }

    pub fn set_contrast(&mut self, contrast: Contrast) {
        self.contrast = contrast;
    }

    pub fn set_canvas(&mut self, canvas: Canvas) {
        self.canvas = canvas;
    }

    pub fn canvas(&self) -> Canvas {
        self.canvas
    }

    pub fn set_grid(&mut self, grid: Option<egui::Color32>) {
        self.grid = grid;
    }

    pub fn set_default_body(&mut self, colour: Rgb) {
        self.aids.body = DefaultBody(colour);
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

    pub fn set_isocurves(&mut self, isocurves: Option<Arc<IsocurveDrawing>>) {
        self.isocurves = isocurves;
    }

    pub fn destination(&self) -> Viewpoint {
        self.camera.destination()
    }

    pub fn show_saved_view(&mut self, view: SavedView) {
        self.go_to(saved_views::viewpoint(view));
    }

    fn go_to(&mut self, viewpoint: Viewpoint) {
        self.history.leave(self.camera.destination());
        self.camera.animate_to(viewpoint);
    }

    fn look_from(&mut self, view: StandardView, home: Option<SavedView>) {
        if let (StandardView::Isometric, Some(home)) = (view, home) {
            self.show_saved_view(home);
            return;
        }
        let destination = self.camera.destination();
        if let Some(viewpoint) = Viewpoint::looking_from(
            view.looking_from(),
            destination.target,
            destination.distance,
        ) {
            self.go_to(viewpoint);
        }
    }

    fn start_from_home(&mut self, model: &Model) {
        match model.document().saved_views().home {
            Some(home) => {
                self.camera = self.fresh_camera(saved_views::viewpoint(home));
                self.home_applied = true;
            }
            None if self.home_applied => {
                self.camera = self.fresh_camera(initial_viewpoint());
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

    pub fn preview_entities(&mut self, previewed: Vec<Pickable>) {
        self.previewed = previewed;
    }

    pub fn show_chosen_rows(&mut self, rows: Vec<FeatureId>) {
        self.chosen_rows = rows;
    }

    pub fn hover_row(&mut self, row: Option<FeatureId>) {
        self.hovered_row = row;
    }

    pub fn take_row_to_choose(&mut self) -> Option<FeatureId> {
        self.row_to_choose.take()
    }

    pub fn take_tree_dismissal(&mut self) -> bool {
        std::mem::take(&mut self.tree_dismissed)
    }

    fn rows_to_highlight(&self, context: editing::Context) -> Vec<FeatureId> {
        if context.sketch.is_some() || context.solid.is_some() {
            return Vec::new();
        }
        let mut rows = self.chosen_rows.clone();
        if let Some(hovered) = self.hovered_row.filter(|row| !rows.contains(row)) {
            rows.push(hovered);
        }
        rows
    }

    fn lit_rows(&mut self, model: &Model, rows: Vec<FeatureId>) -> LitRows {
        let basis = LitRowsBasis {
            rows,
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
        };
        match &self.lit_rows {
            Some((kept, lit)) if *kept == basis => lit.clone(),
            Some(_) | None => {
                let lit = LitRows::of(model.document(), model.evaluation(), &basis.rows);
                self.lit_rows = Some((basis, lit.clone()));
                lit
            }
        }
    }

    fn shown_section(&self, edited: Option<Plane>) -> Vec<SectionPlane> {
        let slice = edited
            .filter(|_| self.sketch_slice)
            .map(section::sketch_slice);
        slice
            .into_iter()
            .chain(self.section.iter().copied())
            .take(MAX_SECTION_PLANES)
            .collect()
    }

    pub fn forget_document(&mut self) {
        self.selection.clear();
        self.chosen_rows.clear();
        self.lit_rows = None;
        self.hovered_row = None;
        self.previewed.clear();
        self.hovered = None;
        self.hover_source = None;
        self.pending_click = None;
        self.pointer_hit = None;
        self.last_pick = None;
        self.keyboard_highlight = None;
        self.pick_list = None;
        self.list_hold = false;
        self.last_cursor = None;
        self.context_menu = None;
        self.menu_waits_for_pick = None;
        self.list_at = None;
        self.scenes = SceneCache::default();
        self.description.forget();
        self.drawing = Drawing::default();
        self.trimming = Trimming::default();
        self.modifying = Modifying::default();
        self.annotations = Annotations::default();
        self.model_dimensions.clear();
        self.typed_point = TypedPoint::default();
        self.moving = None;
        self.moving_label = None;
        self.transforming = None;
        self.press = None;
        self.primary = None;
    }

    pub fn set_navigation(&mut self, navigation: Navigation) {
        self.navigation = navigation;
        self.camera.set_projection(navigation.projection);
        self.camera
            .set_field_of_view(navigation.field_of_view.degrees());
        if navigation.transitions == Transitions::System {
            system_motion::watch();
        }
        self.follow_system_motion();
    }

    fn follow_system_motion(&mut self) {
        self.camera
            .set_transition_scale(self.navigation.transitions.scale(system_motion::reduces()));
    }

    #[cfg(test)]
    pub fn look_toward(&mut self, direction: Vector3) {
        let destination = self.camera.destination();
        if let Some(viewpoint) =
            Viewpoint::looking_from(direction, destination.target, destination.distance)
        {
            self.camera = self.fresh_camera(viewpoint);
        }
    }

    fn fresh_camera(&self, viewpoint: Viewpoint) -> Camera {
        let mut camera = Camera::new(viewpoint);
        camera.set_projection(self.navigation.projection);
        camera.set_field_of_view(self.navigation.field_of_view.degrees());
        camera.set_transition_scale(self.navigation.transitions.scale(system_motion::reduces()));
        camera
    }

    pub fn set_view_details(&mut self, details: ViewDetails) {
        self.aids.edges = details.edges;
        self.view_cube_shown = details.view_cube;
        self.cube_placement = details.cube;
    }

    pub fn set_sketching(&mut self, sketching: Sketching) {
        self.sketching = sketching;
    }

    pub fn start_with(&mut self, toggles: StartingToggles) {
        self.glyphs_shown = toggles.glyphs;
        self.snapping = toggles.snapping;
        self.grid_snapping = toggles.grid_snapping;
        self.typed_dimensions = toggles.typed_dimensions;
        if self.dimensions_shown && !toggles.model_dimensions {
            self.model_dimensions.clear();
        }
        self.dimensions_shown = toggles.model_dimensions;
    }

    fn cube_area(&self, rect: Rect) -> Rect {
        if self.view_cube_shown {
            self.cube_placement.area(rect)
        } else {
            Rect::from_min_size(self.cube_placement.corner.point(rect), Vec2::ZERO)
        }
    }

    fn bottom_strip_left(&self, rect: Rect) -> f32 {
        if self.view_cube_shown && self.cube_placement.corner == Corner::BottomLeft {
            self.cube_area(rect).right()
        } else {
            rect.left()
        }
    }

    fn bottom_strip_right(&self, rect: Rect) -> f32 {
        if self.view_cube_shown && self.cube_placement.corner == Corner::BottomRight {
            self.cube_area(rect).left()
        } else {
            rect.right()
        }
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

    pub fn body_snaps(&self) -> &BodySnaps {
        &self.body_snaps
    }

    pub fn replace_selection(&mut self, pickables: Vec<Pickable>) {
        self.selection.replace_with_all(pickables);
    }

    #[cfg(test)]
    pub fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    #[cfg(test)]
    pub fn show_more_keys(&mut self, shown: bool) {
        self.more_keys = shown;
    }

    #[cfg(test)]
    pub fn orbit_mark(&self) -> Option<OrbitMark> {
        self.orbit_mark
    }

    pub fn is_drawing(&self) -> bool {
        self.drawing.in_progress()
    }

    pub fn refuses_mode(&self, mode: ShapeMode) -> Option<String> {
        self.drawing.refuses_mode(mode)
    }

    pub fn arc_on_tangent_arc_key(&self, editing: &SketchEditing, commands: &mut CommandFrame) {
        let drawing_lines = editing
            .active()
            .is_some_and(|active| active.tool == Tool::Line);
        if drawing_lines
            && self.drawing.in_progress()
            && commands.take(Command::SketchTool(Tool::TangentArc))
        {
            commands.trigger(Command::ChainArc);
        }
    }

    pub fn is_animating(&self) -> bool {
        self.camera.is_animating()
    }

    pub fn advance(&mut self, elapsed: Duration) {
        if self.navigation.transitions == Transitions::System {
            self.follow_system_motion();
        }
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
        self.clock = ui.input(|input| input.time);
        if commands.available(Command::FitView) {
            self.fit_requested = Some(FitAsked::ByUser);
        }
        if self.session != model.session() {
            self.session = model.session();
            self.fit_when_computed = true;
            self.history.clear();
            self.start_from_home(model);
        }
        if self.fit_when_computed
            && model.status() == RecomputeStatus::UpToDate
            && !model.bodies_pending()
        {
            self.fit_requested = Some(FitAsked::OnOpening);
            self.fit_when_computed = false;
        }
        let bring_back = self.start_card_return(model, editing);
        if commands.invoke(Command::ShowStartCard, &bring_back) {
            self.start_card.bring_back(model.session());
        }
        self.keyboard_commands(model, editing, commands, actions);
        let key_hints = KeyHints::new(commands, self.navigation.input_mode);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            self.rect = Some(rect);
            self.pixels_per_point = ui.ctx().pixels_per_point();
            let response = ui.interact(
                rect,
                ui.scope_id().with("viewport"),
                Sense::click_and_drag(),
            );
            response.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, VIEWPORT_NAME));
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
            manipulator::publish_dragged(ui.ctx(), self.dragged_feature());
            self.track_sketch_cursor(model, editing);
            self.track_drawing(model, editing);
            self.type_points(ui, rect, model, editing, keys_free, actions);
            self.navigate(ui, &response, rect);
            self.drag_primary(ui, &response, model, editing, actions);
            self.click(ui, &response, model, editing, actions);
            self.context_click(&response, model, editing);
            self.hold_to_list(ui, &response, model, editing, actions);
            self.show_pick_list(ui, model, editing, actions);
            if self.context_menu.is_some()
                && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.context_menu = None;
            }
            if keys_free {
                self.handle_keys(ui, model, editing, actions);
            }
            self.annotate(ui, rect, model, editing, actions);
            self.decorate(ui, rect, model, editing, &key_hints);
        });
        if self.start_card.take_stop_asked() {
            actions.push(Action::Preferences(PreferencesCommand::Change(
                PreferenceChange::ShowStartCard(false),
            )));
        }
    }

    fn start_card_return(
        &self,
        model: &Model,
        editing: &SketchEditing,
    ) -> Result<(), StartCardRefusal> {
        if !is_blank(model, editing) {
            Err(StartCardRefusal::NotEmpty)
        } else if self.start_card.shown_in(model.session()) {
            Err(StartCardRefusal::AlreadyShown)
        } else {
            Ok(())
        }
    }

    pub fn build_scene(&mut self, model: &Model, editing: &SketchEditing) -> Option<&BuiltScene> {
        let document = model.document();
        let evaluation = model.evaluation();
        let display = model.display();
        let edited = editing.feature();
        let context = editing.context().with_principal(model.principal_view());
        self.filter_applies =
            context.sketch.is_none() && context.solid.is_none() && !context.choosing_plane;
        if edited != self.edited {
            self.edited = edited;
            self.face_edited_sketch = edited
                .filter(|_| self.sketching.look_at_on_entering)
                .map(|_| SketchFacing::Entering);
            self.last_pick = None;
        }
        self.bodies.update_when_changed(
            (evaluation, model.evaluation_generation()),
            &display.meshing,
        );
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
        let grabbed = match &self.primary {
            Some(PrimaryDrag::Grab(grab)) => Some(grab),
            _ => None,
        };
        let mut hovered: Vec<Pickable> = if let Some(grab) = grabbed {
            edited
                .zip(edited_sketch(model, editing))
                .into_iter()
                .flat_map(|(feature, sketch)| {
                    grab.snap_entities(&sketch)
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
                .collect()
        } else if self.drawing.is_active() {
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
        } else if self.modifying.is_active() && !self.modifying.gathers() {
            edited
                .into_iter()
                .flat_map(|feature| {
                    self.modifying
                        .highlighted_entities()
                        .into_iter()
                        .map(move |entity| Pickable::SketchEntity { feature, entity })
                })
                .collect()
        } else if let Some(annotation) = self
            .annotations
            .hovered()
            .or(self.model_dimensions.hovered())
            .or(highlighted)
            .filter(|highlight| matches!(highlight, Pickable::SketchConstraint { .. }))
        {
            annotation.constrained_entities(document)
        } else if let Some(row) = self.hovered_in_tree {
            match row.constrained_entities(document) {
                entities if entities.is_empty() => vec![row],
                entities => entities,
            }
        } else if highlighted.is_none() && !self.previewed.is_empty() {
            self.previewed.clone()
        } else {
            highlighted
                .into_iter()
                .flat_map(|highlighted| match chooses_bodies(editing) {
                    true => whole_body_hovered(model, highlighted),
                    false => self.whole_body_of(model, highlighted),
                })
                .collect()
        };
        let mut rows = self.rows_to_highlight(context);
        rows.extend(mirrored_bodies(model, context));
        let lit = self.lit_rows(model, rows.clone());
        hovered.extend(lit.made);
        hovered.sort_unstable();
        hovered.dedup();
        let framed_rows = with_their_bodies(model.document(), rows);
        let view = self.view();
        let sources = scene_sources(
            model,
            SourceParts {
                bodies: &self.bodies,
                style: self.style,
                aids: self.aids,
                analyses: &self.analyses,
                contrast: self.contrast,
                canvas: self.canvas,
            },
            context.solid,
        );
        self.scenes.update(&SceneInputs {
            sources: &sources,
            revisions: Revisions {
                document: model.revision(),
                evaluation: model.evaluation_generation(),
                sketches: display.sketches.generation(),
                bodies: self.bodies.generation(),
                masses: if self.aids.centres_of_mass {
                    display.meshing.masses_measured()
                } else {
                    0
                },
                style: self.style,
                aids: self.aids,
                analysed: self.analyses.finished(),
                contrast: self.contrast,
                canvas: self.canvas,
                draft: model.draft_generation(),
                pictures: model.pictures().generation(),
            },
            context,
            highlight: Highlight {
                selection: &self.selection,
                hovered: &hovered,
                chosen_rows: &lit.rows,
            },
            view: view.as_ref(),
        });
        let faceting = self.scenes.faceting();
        let plane = self.scenes.edited_plane();
        self.scenes.set_section(self.shown_section(plane));
        self.scenes
            .set_grid(self.grid.map(scene_palette::grid_colour));
        self.problems = problems(model);
        self.kept = measurement_tools::shown(model);
        let pointer_ray = self
            .cursor
            .zip(view.as_ref())
            .and_then(|(cursor, view)| view.ray_through(cursor));
        self.calibration = image_tools::calibration_guide(model, editing, pointer_ray);
        self.scenes.show(Overlay {
            contrast: self.contrast,
            canvas: self.canvas,
            plane,
            previews: vec![
                self.drawn_preview(faceting),
                self.trimming.preview(faceting),
                self.modifying.preview(faceting),
                self.grab_preview(),
                self.body_preview(editing),
            ],
            measured: self.measured.as_ref().map(|(line, _)| [line.from, line.to]),
            calibration: self.calibration.clone(),
            problems: self.problems.iter().map(|problem| problem.place).collect(),
            kept: self.kept.iter().filter_map(|kept| kept.line).collect(),
            interference: self.interference.clone(),
            comb: self.comb.clone(),
            isocurves: self.isocurves.clone(),
            manipulator: self
                .manipulator
                .as_ref()
                .map(|manipulator| manipulator.drawn(self.manipulator_hover)),
        });
        if let Some(highlight) = self.keyboard_highlight
            && !self.scenes.can_highlight(highlight)
            && !(matches!(highlight, Pickable::SketchConstraint { .. })
                && highlight.is_available(document, evaluation, context))
            && !self.model_dimensions.holds(highlight)
            && !(self.steps_body_items(editing) && self.has_body_item(highlight, editing))
        {
            self.keyboard_highlight = None;
        }
        let built = self.scenes.built()?;
        self.scene_bounds = Some(built.everything);
        if self.needs_initial_fit
            && let Some(home) = model.document().saved_views().home
        {
            self.camera = self.fresh_camera(saved_views::viewpoint(home));
            self.home_applied = true;
        }
        let Some(view) = self.view() else {
            return Some(built);
        };
        let heading = self.destination_view().unwrap_or(view);
        let mut going = None;
        if self.needs_initial_fit {
            self.camera = self.fresh_camera(view.fitted(built.fit_all()));
            self.needs_initial_fit = false;
        } else if let Some(asked) = self.face_edited_sketch.take() {
            going = built.edited.map(|sketch| {
                let facing = match asked {
                    SketchFacing::Entering => entering(&self.camera, &heading, &sketch),
                    SketchFacing::Again => facing(&self.camera, &heading, &sketch),
                };
                (facing, FitAsked::ByUser)
            });
        } else if let Some(target) = self.look_from {
            going = built
                .bounds_of(&sources, self.selection.iter())
                .map(|bounds| {
                    let looking = looking_at(&self.camera, &heading, target, bounds);
                    (looking, FitAsked::ByUser)
                });
        } else if let Some(place) = self.framed_place {
            let reach = (built.fit_all().bounding_radius() * PLACE_SHARE).max(MIN_PLACE_REACH);
            let around = Aabb::from_point(place).expanded(reach);
            going = Some((heading.fitted(around), FitAsked::ByUser));
        } else if let Some(asked) = self.fit_requested {
            let everything = built.fit_all();
            let bounds = if !self.selection.is_empty() {
                built
                    .bounds_of(&sources, self.selection.iter())
                    .unwrap_or(everything)
            } else {
                built
                    .bounds_of_features(&sources, &framed_rows)
                    .unwrap_or(everything)
            };
            going = Some((heading.fitted(bounds), asked));
        }
        if let Some((viewpoint, asked)) = going {
            if asked == FitAsked::ByUser {
                self.history.leave(self.camera.destination());
            }
            self.camera.animate_to(viewpoint);
        }
        self.fit_requested = None;
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

    pub fn image(
        &self,
        model: &Model,
        editing: &SketchEditing,
        size: SurfaceSize,
        left_out: &[FeatureId],
    ) -> ImageView {
        let kept = (!left_out.is_empty()).then(|| self.bodies.without(left_out));
        let sources = Sources {
            document: model.document(),
            evaluation: model.evaluation(),
            bodies: kept.as_ref().unwrap_or(&self.bodies),
            sketches: &model.display().sketches,
            style: self.style,
            aids: ViewAids::default(),
            analyses: &Analyses::default(),
            contrast: Contrast::Standard,
            canvas: self.canvas,
            draft: None,
            pictures: Some(model.pictures()),
        };
        let context = editing.context().with_principal(model.principal_view());
        let view = self
            .camera
            .view(f64::from(size.width), f64::from(size.height));
        let level = FacetLevel::following(self.scenes.level(), FacetLevel::wanted_chord(&view));
        let unselected = Selection::default();
        let mut built = scene::build_keeping(
            &sources,
            &Highlight {
                selection: &unselected,
                hovered: &[],
                chosen_rows: &[],
            },
            context,
            (
                &mut self
                    .scenes
                    .sketch_shapes()
                    .faceted_at(scene::drawn_faceting(&sources, context, level.faceting())),
                &mut self.scenes.parts().clone(),
            ),
        );
        built.scene.grid = None;
        built.scene.section = self.scenes.section().to_vec();
        built.scene.lighting = self.scenes.lighting().filter(|_| built.rendered);
        built.scene.backdrop = self.scenes.backdrop().filter(|_| built.rendered);
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
        let snapshot = snapshot::of_bodies(
            model.document(),
            model.evaluation(),
            &meshes,
            size,
            Canvas::default(),
            Some(model.pictures()),
        );
        ImageView {
            view: snapshot.view,
            scene: snapshot.scene,
            pixels_per_point: snapshot.pixels_per_point,
        }
    }

    pub fn preview(&self, model: &Model, size: SurfaceSize) -> ImageView {
        let shown: Vec<FeatureId> = self.bodies.iter().map(|(id, _)| id).collect();
        let snapshot = snapshot::of_bodies_alone(
            model.document(),
            model.evaluation(),
            &self.bodies.only(&shown),
            size,
            self.canvas,
            Some(model.pictures()),
            model.principal_view().kept,
        );
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
        let size = self.view_size()?;
        let view = self.camera.view(size.x, size.y);
        Some(match self.scene_bounds {
            Some(bounds) => view.reaching(bounds),
            None => view,
        })
    }

    fn destination_view(&self) -> Option<View> {
        let size = self.view_size()?;
        Some(self.camera.destination_view(size.x, size.y))
    }

    fn view_size(&self) -> Option<Vector2> {
        let size = self.rect?.size() * self.pixels_per_point;
        (size.x >= 1.0 && size.y >= 1.0).then(|| Vector2::new(f64::from(size.x), f64::from(size.y)))
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
    pub fn snap_highlight(&self) -> Vec<EntityId> {
        self.drawing.snap_entities()
    }

    #[cfg(test)]
    pub fn grab_highlight(&self, sketch: &Sketch) -> Vec<EntityId> {
        match &self.primary {
            Some(PrimaryDrag::Grab(grab)) => grab.snap_entities(sketch),
            _ => Vec::new(),
        }
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
    pub fn calibration(&self) -> Option<&CalibrationGuide> {
        self.calibration.as_ref()
    }

    #[cfg(test)]
    pub fn handle_position(&self, handle: Handle, along: f64) -> Option<egui::Pos2> {
        let world = self.manipulator.as_ref()?.grip(handle, along)?;
        let pixel = self.view()?.project(world)? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn handle_grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        self.manipulator.as_ref()?.grip(handle, along)
    }

    #[cfg(test)]
    pub fn screen_of(&self, point: Point3) -> Option<egui::Pos2> {
        let pixel = self.view()?.project(point)? / f64::from(self.pixels_per_point);
        Some(self.rect?.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32))
    }

    #[cfg(test)]
    pub fn handle_foot(&self, handle: Handle) -> Option<Point3> {
        self.manipulator.as_ref()?.foot(handle)
    }

    #[cfg(test)]
    pub fn manipulator_step(&self) -> Option<f64> {
        self.manipulator.as_ref().map(Manipulator::step)
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
            hits: vec![PickHit {
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
            .or(self.pointed())
    }

    fn pointed(&self) -> Option<Pickable> {
        self.body_hovered.or(self.hovered)
    }

    fn steps_body_items(&self, editing: &SketchEditing) -> bool {
        editing
            .active()
            .is_some_and(|active| matches!(active.tool, Tool::Select | Tool::Dimension))
    }

    fn has_body_item(&self, pickable: Pickable, editing: &SketchEditing) -> bool {
        editing
            .feature()
            .and_then(|feature| BodyPick::of(pickable, feature))
            .and_then(|pick| self.body_snaps.find(pick.body, pick.item))
            .is_some()
    }

    fn steppable_body_items(&self, editing: &SketchEditing) -> Vec<Pickable> {
        match editing.feature() {
            Some(feature) if self.steps_body_items(editing) => self
                .body_snaps
                .steppable()
                .into_iter()
                .map(|(body, item)| BodyPick { body, item }.pickable(feature))
                .collect(),
            Some(_) | None => Vec::new(),
        }
    }

    fn body_hover(
        &self,
        editing: &SketchEditing,
        sketch: &Sketch,
        screen: &SketchScreen,
        pointer: Option<Pointer>,
    ) -> Option<Pickable> {
        let active = editing
            .active()
            .filter(|active| matches!(active.tool, Tool::Select | Tool::Dimension))?;
        let pointer = pointer.filter(|_| self.primary.is_none())?;
        let on_sketch = match self.hovered {
            Some(Pickable::SketchEntity { feature, entity }) if feature == active.feature => {
                Some(entity)
            }
            Some(Pickable::SketchEntity { .. }) | None => None,
            Some(_) => return None,
        };
        let point_like = |entity: EntityId| {
            entity == EntityId::ORIGIN || matches!(sketch.entity(entity), Some(Entity::Point(_)))
        };
        if on_sketch.is_some_and(point_like) {
            return None;
        }
        let snapped = self
            .body_snaps
            .point(screen, pointer, Accept::Points, Inferences::default())
            .or_else(|| {
                on_sketch
                    .is_none()
                    .then(|| {
                        self.body_snaps.edge(
                            screen,
                            pointer,
                            Accept::Anything,
                            Inferences::default(),
                        )
                    })
                    .flatten()
            })?;
        let Target::Body(target) = snapped.target else {
            return None;
        };
        let (body, item) = self.body_snaps.item(target)?;
        Some(BodyPick { body, item }.pickable(active.feature))
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
            self.pointer_hit = None;
            self.last_pick = None;
        }
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &Response, rect: Rect) {
        let Some(view) = self.view() else {
            return;
        };
        if !ui.input(|input| input.pointer.any_down()) {
            self.history.stop_dragging();
            if let Some(mark) = &mut self.orbit_mark {
                mark.held = false;
            }
        }
        if response.drag_started() {
            self.drag_anchor = self.cursor.and_then(|cursor| self.hit_under(cursor));
        }
        if response.double_clicked_by(PointerButton::Middle) {
            self.fit_requested = Some(FitAsked::ByUser);
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
        if drag != Vector2::ZERO && motion.is_some() {
            self.history
                .moving(Gesture::Drag, self.camera.destination(), self.clock);
            match motion {
                Some(DragMotion::Orbit) => self.orbit_by(&view, drag, rect, self.drag_anchor),
                Some(DragMotion::Pan) => self.pan_by(&view, drag, self.drag_anchor),
                Some(DragMotion::Zoom) => {
                    let pivot = self
                        .drag_anchor
                        .filter(|_| self.navigation.zoom_toward == ZoomToward::Pointer)
                        .unwrap_or(view.viewpoint().target);
                    self.camera
                        .zoom(pivot, zoom_factor(&self.navigation, -drag.y));
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
                self.history
                    .moving(Gesture::Wheel, self.camera.destination(), self.clock);
                let anchor = self.cursor.and_then(|cursor| self.hit_under(cursor));
                if alt {
                    self.pan_by(&view, sliding, anchor);
                } else {
                    self.orbit_by(&view, sliding, rect, anchor);
                }
            }
        }
        let factor = zoom_factor(&self.navigation, f64::from(scrolled_to_zoom))
            * pinch_factor(&self.navigation, f64::from(pinch));
        if (factor - 1.0).abs() < 1e-9 {
            return;
        }
        let Some(cursor) = self.cursor else {
            return;
        };
        let anchor = match self.navigation.zoom_toward {
            ZoomToward::Pointer => self
                .hit_under(cursor)
                .or_else(|| view.focal_point_under(cursor))
                .unwrap_or(view.viewpoint().target),
            ZoomToward::Centre => view.viewpoint().target,
        };
        self.history
            .moving(Gesture::Wheel, self.camera.destination(), self.clock);
        self.camera.zoom(anchor, factor);
    }

    fn orbit_with_cube(&mut self, rect: Rect, drag: egui::Vec2) {
        let Some(view) = self.view() else {
            return;
        };
        self.history
            .moving(Gesture::Drag, self.camera.destination(), self.clock);
        self.orbit_by(&view, self.to_pixels(drag), rect, None);
    }

    fn orbit_by(&mut self, view: &View, drag: Vector2, rect: Rect, anchor: Option<Point3>) {
        let pivot = anchor
            .filter(|_| self.navigation.orbit_around == OrbitAround::Pointer)
            .unwrap_or(view.viewpoint().target);
        let turned = |inverted: bool| if inverted { -1.0 } else { 1.0 };
        let drag = Vector2::new(
            drag.x * turned(self.navigation.invert_orbit_x),
            drag.y * turned(self.navigation.invert_orbit_y),
        );
        self.orbit_mark = Some(OrbitMark {
            pivot,
            at: self.clock,
            held: true,
        });
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
        let (pressed, released, toggle, alt) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.modifiers.shift || input.modifiers.command,
                input.modifiers.alt,
            )
        });
        if pressed && response.hovered() {
            self.press = self.cursor.zip(self.view()).map(|(cursor, view)| Press {
                cursor,
                view,
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
            let press = self
                .press
                .take()
                .filter(|_| !navigating)
                .map(|press| self.caught_up(press));
            let arcing = press.is_some_and(|press| self.arc_from_press(model, editing, press));
            self.draw_press = press
                .filter(|_| {
                    editing.active().is_some_and(|active| active.tool.draws())
                        && (arcing || !self.drawing.in_progress())
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
        self.through_held = alt
            && matches!(
                self.primary,
                Some(PrimaryDrag::ModelBox { .. } | PrimaryDrag::Paint(_))
            );
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
        let snap = match &self.primary {
            Some(PrimaryDrag::Manipulate(manipulating)) if !free => {
                self.snap_handle(model, manipulating.feature, manipulating.guide())
            }
            _ => None,
        };
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
                    && manipulating.follow(ray, free, snap)
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
            Some(PrimaryDrag::Pull { feature } | PrimaryDrag::Unmovable { feature, .. })
                if Some(*feature) != edited =>
            {
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
            Some(
                PrimaryDrag::Trim { .. } | PrimaryDrag::Pull { .. } | PrimaryDrag::Unmovable { .. },
            )
            | None => {}
        }
        if released {
            self.press = None;
            match self.primary.take() {
                Some(PrimaryDrag::Grab(grab)) if grab.has_moved() => {
                    actions.push(Action::Drag(grab.finish()));
                }
                Some(PrimaryDrag::Box { feature, area }) => {
                    let keep = toggle || self.modifying.gathers();
                    self.select_within(model, feature, &area, keep);
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
                        Some(transaction) => {
                            let announced = reversing::announcement(model.document(), &transaction);
                            actions.push(Action::Apply(transaction));
                            actions.extend(announced.map(Action::Inform));
                        }
                        None => actions.push(Action::Preview {
                            feature: manipulating.feature,
                            draft: None,
                        }),
                    }
                }
                Some(
                    PrimaryDrag::Grab(_) | PrimaryDrag::Paint(_) | PrimaryDrag::Unmovable { .. },
                )
                | None => {}
            }
        }
    }

    fn paint_faces(&mut self, model: &Model, editing: &SketchEditing, samples: &[Vector2]) {
        let (Some(built), Some(view)) = (self.scenes.built(), self.view()) else {
            return;
        };
        let document = model.document();
        let evaluation = model.evaluation();
        let context = editing.context().with_principal(model.principal_view());
        let mut painted: Vec<Pickable> = Vec::new();
        for at in samples {
            for face in paint_selection::faces_under(
                built,
                &view,
                *at,
                self.pixels_per_point,
                self.reaches_through(),
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
            None => self
                .manipulator
                .as_ref()
                .zip(self.view())
                .zip(self.cursor)
                .and_then(|((manipulator, view), cursor)| {
                    manipulator.hit(&view, cursor, pixels_per_point)
                }),
        };
    }

    fn snap_handle(&mut self, model: &Model, feature: FeatureId, guide: Guide) -> Option<Snap> {
        let cursor = self.cursor?;
        if let Some((at, guided, snap)) = self.handle_snap
            && at == cursor
            && guided == guide
        {
            return snap;
        }
        let snap = self
            .scenes
            .built()
            .zip(self.view())
            .and_then(|(built, view)| {
                handle_snap::Probe {
                    model,
                    feature,
                    built,
                    view: &view,
                    cursor,
                    pixels_per_point: self.pixels_per_point,
                    guide,
                }
                .snap()
            });
        self.handle_snap = Some((cursor, guide, snap));
        snap
    }

    fn manipulate_from(&self, press: Press, model: &Model) -> Option<Manipulating> {
        let manipulator = self.manipulator.as_ref()?;
        let view = self.view()?;
        let handle = manipulator.hit(&view, press.cursor, f64::from(self.pixels_per_point))?;
        let ray = view.ray_through(press.cursor)?;
        Manipulating::begin(model, manipulator, handle, ray)
    }

    fn grab_preview(&self) -> Preview {
        match &self.primary {
            Some(PrimaryDrag::Grab(grab)) => {
                let landing = grab.landing();
                let body_edge = landing
                    .and_then(|landing| match landing.target {
                        Some(Target::Body(target)) => self.body_snaps.path(target),
                        _ => None,
                    })
                    .unwrap_or_default();
                let edge_guides = body_edge.windows(2).filter_map(|pair| match pair {
                    [start, end] => Some([*start, *end]),
                    _ => None,
                });
                Preview {
                    snap: landing.map(|landing| landing.position),
                    guides: grab.guides().iter().copied().chain(edge_guides).collect(),
                    ..Preview::default()
                }
            }
            _ => Preview::default(),
        }
    }

    fn body_preview(&self, editing: &SketchEditing) -> Preview {
        let Some(feature) = editing.feature() else {
            return Preview::default();
        };
        let mut preview = Preview::default();
        let hovered = self
            .highlighted()
            .and_then(|hovered| BodyPick::of(hovered, feature))
            .into_iter()
            .map(|pick| (pick, true));
        let picked = body_picks::selected(&self.selection, feature)
            .into_iter()
            .map(|pick| (pick, false));
        for (pick, hovered) in hovered.chain(picked) {
            let Some(target) = self.body_snaps.find(pick.body, pick.item) else {
                continue;
            };
            match (self.body_snaps.position(target), hovered) {
                (Some(position), true) => preview.snap = Some(position),
                (Some(position), false) => preview.points.push(position),
                (None, true) => preview.guides.extend(
                    self.body_snaps
                        .path(target)
                        .unwrap_or_default()
                        .windows(2)
                        .filter_map(|pair| match pair {
                            [start, end] => Some([*start, *end]),
                            _ => None,
                        }),
                ),
                (None, false) => preview.curves.extend(self.body_snaps.path(target)),
            }
        }
        preview
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

    fn snap_grid_spacing(&self) -> Option<f64> {
        self.scenes
            .built()
            .and_then(|built| built.scene.grid.as_ref())
            .zip(self.view())
            .map(|(grid, view)| grid_snap_spacing(grid, &view, f64::from(self.pixels_per_point)))
    }

    fn hold(&self) -> Option<Hold> {
        (self.snap_held && !self.placing_freely).then(|| Hold {
            grid: self.snap_grid_spacing(),
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
            && !editing.choosing_bodies()
        {
            if self.paint
                && self.style.picks_faces()
                && paint_selection::takes_faces(self.active_filter())
            {
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
            Tool::Mirror | Tool::RectangularPattern | Tool::CircularPattern
                if self.modifying.gathers() =>
            {
                let at = press.cursor / f64::from(self.pixels_per_point);
                return Some(PrimaryDrag::Box {
                    feature,
                    area: ScreenArea::starting_at(at, self.lasso),
                });
            }
            Tool::Extend
            | Tool::Mirror
            | Tool::RectangularPattern
            | Tool::CircularPattern
            | Tool::TangentCircle
            | Tool::Gear
            | Tool::Text
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
            }) if owner == feature => Some(entity),
            _ => None,
        };
        let held = grabbed.and_then(|entity| {
            if entity.is_reference() {
                Some(Unmovable::Reference)
            } else if projected(entity) {
                Some(Unmovable::Projected)
            } else {
                None
            }
        });
        if let Some(held) = held {
            return Some(PrimaryDrag::Unmovable { feature, held });
        }
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
        let grabbed = sketch
            .text(grabbed)
            .map(|text| text.anchor)
            .or_else(|| sketch.gear(grabbed).map(|gear| gear.centre))
            .unwrap_or(grabbed);
        Grab::of(&sketch, feature, grabbed, &selected, from)
            .map(|grab| PrimaryDrag::Grab(grab.snapping_to_bodies(&self.body_snaps)))
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
        let faces_pick = self.style.picks_faces();
        let mut caught: Vec<Pickable> = match Catch::of(self.active_filter(), faces_pick) {
            Catch::SketchGeometry => {
                let section = self.scenes.section();
                let slack = section_slack(view.viewpoint().distance);
                document
                    .active_features()
                    .filter(|feature| visibility::is_shown(document, feature.id()))
                    .filter_map(|feature| Some((feature.id(), model.displayed_sketch(feature)?)))
                    .flat_map(|(feature, sketch)| {
                        let screen = SectionedScreen {
                            screen: SketchScreen {
                                view,
                                plane: sketch.plane(),
                                pixels_per_point: scale,
                            },
                            plane: sketch.plane(),
                            section,
                            slack,
                        };
                        sketch_drag::within(&sketch, &screen, area, self.scenes.faceting())
                            .into_iter()
                            .map(move |entity| Pickable::SketchEntity { feature, entity })
                    })
                    .collect()
            }
            catch => {
                let section = self.scenes.section();
                let slack = section_slack(view.viewpoint().distance);
                let seen = |point: Point3| {
                    if is_cut_away(section, point, slack) {
                        return None;
                    }
                    let at = view.project(point)? / scale;
                    let depth = view.view_depth(point);
                    Some(box_selection::Seen {
                        at,
                        depth,
                        units_per_point: view.units_per_pixel_at(depth) * scale,
                    })
                };
                let screen = |point: Point3| Some(view.project(point)? / scale);
                let shown: Vec<_> = self
                    .bodies
                    .iter()
                    .filter(|(body, _)| visibility::is_shown(document, *body))
                    .filter(|(_, mesh)| box_selection::may_reach(mesh.bounds(), &screen, area))
                    .collect();
                let occlusion = if self.reaches_through() || !faces_pick {
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
        let context = editing.context().with_principal(model.principal_view());
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
        self.mirror = self
            .symmetry
            .filter(|symmetry| editing.feature() == Some(symmetry.feature))
            .zip(displayed.as_deref())
            .and_then(|(symmetry, sketch)| Mirror::of(sketch, symmetry.line));
        if self.mirror.is_none() {
            self.symmetry = None;
        }
        self.drawing
            .draw_about(self.mirror.as_ref().map(|mirror| mirror.line));
        if let Some(feature) = editing.feature()
            && let Some(refreshed) = BodySnaps::refreshed(&self.body_snaps, model, feature)
        {
            self.body_snaps = refreshed;
        }
        self.drawing.use_bodies(&self.body_snaps);
        self.drawing
            .place_freely(self.placing_freely || !(self.snapping || self.snap_held));
        self.drawing
            .snap_to_grid(self.snap_grid_spacing().filter(|_| self.grid_snapping));
        self.drawing.hold_snap(self.hold());
        self.drawing.infer(self.sketching.inferred);
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
            self.selection.clear();
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
                self.body_hovered = self.body_hover(editing, &sketch, &screen, pointer);
                self.trimming.hover(&sketch, &screen, pointer);
                let version = editing
                    .feature()
                    .map(|feature| SketchVersion::of(model, feature));
                self.modifying
                    .hover(&sketch, &screen, pointer, (self.scenes.faceting(), version));
            }
            _ => {
                self.drawing.leave();
                self.body_hovered = None;
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
        let before = actions.len();
        self.act_on_click(ui, response, model, editing, actions);
        let refused = actions
            .get(before..)
            .unwrap_or_default()
            .iter()
            .rev()
            .find_map(|action| match action {
                Action::Inform(notice)
                    if matches!(notice.kind, NoticeKind::Warning | NoticeKind::Error) =>
                {
                    Some(notice.text.clone())
                }
                _ => None,
            });
        if let Some(text) = refused {
            self.refusal = Some(RefusalNote::new(text, self.clock));
        }
    }

    fn act_on_click(
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
        if let Some(pending) = self.pending_click
            && let Some(hovered) = self.hover_at(pending.at)
        {
            self.pending_click = None;
            self.perform_click_at(pending, hovered, model, editing, drawing, actions);
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
        if (!click.double && !click.primary)
            || self.manipulator_hover.is_some()
            || self.context_menu.is_some()
        {
            return;
        }
        if drawing || self.hover_is_current() {
            self.perform_click(click, model, editing, drawing, actions);
        } else {
            self.pending_click = self.pick_source().map(|at| PendingClick { at, click });
        }
    }

    fn perform_click_at(
        &mut self,
        pending: PendingClick,
        hovered: Option<Pickable>,
        model: &Model,
        editing: &SketchEditing,
        drawing: bool,
        actions: &mut Vec<Action>,
    ) {
        if self.hover_is_current() && self.hover_source == Some(pending.at) {
            self.perform_click(pending.click, model, editing, drawing, actions);
            return;
        }
        let kept = (self.cursor, self.sketch_cursor, self.hovered);
        let cursor = pending.at.cursor;
        self.cursor = Some(cursor);
        self.sketch_cursor = editing
            .feature()
            .and_then(|feature| self.on_sketch(model, feature, cursor));
        self.hovered = hovered;
        self.track_drawing(model, editing);

        self.perform_click(pending.click, model, editing, drawing, actions);

        (self.cursor, self.sketch_cursor, self.hovered) = kept;
        self.track_drawing(model, editing);
    }

    fn pick_source(&self) -> Option<PickSource> {
        Some(PickSource {
            cursor: self.cursor?,
            view: self.view()?,
        })
    }

    fn hover_at(&self, at: PickSource) -> Option<Option<Pickable>> {
        if self.hover_source == Some(at) {
            return Some(self.hovered);
        }
        let replaced = self.cursor.is_none() || self.hover_is_current();
        replaced.then(|| self.seen_through(at))
    }

    fn caught_up(&self, press: Press) -> Press {
        if press.current {
            return press;
        }
        let at = PickSource {
            cursor: press.cursor,
            view: press.view,
        };
        Press {
            hovered: self.seen_through(at),
            current: true,
            ..press
        }
    }

    fn seen_through(&self, at: PickSource) -> Option<Pickable> {
        let built = self.scenes.built()?;
        let hits = built
            .scene
            .hits_through(&at.view, at.cursor, self.pixels_per_point);
        let reached = built.picks.within_reach(&hits);
        let depth = |hit: &PickHit| at.view.view_depth(hit.position);
        let front = reached
            .iter()
            .filter(|(pickable, _)| hides_what_lies_behind(*pickable))
            .map(|(_, hit)| depth(hit))
            .fold(f64::INFINITY, f64::min);
        let deepest_seen = if front.is_finite() {
            front
                + at.view.units_per_pixel_at(front)
                    * f64::from(self.pixels_per_point)
                    * SEEN_DEPTH_SLACK_POINTS
        } else {
            f64::INFINITY
        };
        let seen = reached
            .into_iter()
            .map(|(_, hit)| hit)
            .filter(|hit| depth(hit) <= deepest_seen)
            .collect();
        built
            .picks
            .best_hit(
                &PickResult {
                    cursor: at.cursor,
                    hits: seen,
                },
                self.active_filter(),
            )
            .map(|(pickable, _)| pickable)
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

    fn arc_from_press(&mut self, model: &Model, editing: &SketchEditing, press: Press) -> bool {
        let drawing_lines = editing
            .active()
            .is_some_and(|active| active.tool == Tool::Line);
        let Some(feature) = editing.feature().filter(|_| drawing_lines) else {
            return false;
        };
        if !self.drawing.in_progress() {
            return false;
        }
        let (cursor, sketch_cursor) = (self.cursor, self.sketch_cursor);
        self.cursor = Some(press.cursor);
        self.sketch_cursor = self.on_sketch(model, feature, press.cursor);
        self.track_drawing(model, editing);
        let sketch = edited_sketch(model, editing);
        let arcing = self.drawing.arc_from_press(sketch.as_deref());
        self.cursor = cursor;
        self.sketch_cursor = sketch_cursor;
        self.track_drawing(model, editing);
        arcing
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
        if self.leaves_primitive_place(editing, self.hovered) {
            actions.push(Action::Editing(EditingCommand::StopPicking));
            self.select(model, click.toggle);
            return;
        }
        let ray = self
            .cursor
            .and_then(|cursor| self.view()?.ray_through(cursor));
        if let Some(action) = pick_action(
            (self.hovered, ray),
            (model, &self.selection),
            editing,
            click.toggle,
        ) {
            actions.extend(action);
            return;
        }
        if let Some(feature) = self.dimensioning {
            self.dimension_click(model, feature, self.pointed(), actions);
            return;
        }
        if self.trimming.is_active() {
            actions.extend(outcome_action(self.trimming.click(model)));
            return;
        }
        if self.modifying.gathers() {
            self.gather_click(editing, self.hovered);
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
                    actions.push(Action::Inform(Notice::warning(format!(
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

    fn leaves_primitive_place(&self, editing: &SketchEditing, pickable: Option<Pickable>) -> bool {
        editing
            .picking()
            .filter(|picking| picking.slot == Slot::PrimitivePlace)
            .zip(pickable)
            .is_some_and(|(picking, pickable)| {
                primitive_tools::is_part_of(picking.feature, pickable)
            })
    }

    fn gather_click(&mut self, editing: &SketchEditing, hovered: Option<Pickable>) {
        if let Some(
            pickable @ Pickable::SketchEntity {
                feature: owner,
                entity,
            },
        ) = hovered
            && editing.feature() == Some(owner)
            && !entity.is_reference()
        {
            self.selection.toggle(pickable);
        }
    }

    fn dimension_click(
        &mut self,
        model: &Model,
        feature: FeatureId,
        hovered: Option<Pickable>,
        actions: &mut Vec<Action>,
    ) {
        let Ok(staged) = self.staged_picks(model, feature, None) else {
            return;
        };
        let pickable = hovered.filter(|hovered| match hovered {
            Pickable::SketchEntity { feature: owner, .. } => *owner == feature,
            Pickable::BodyItem { sketch, .. } => *sketch == feature,
            _ => false,
        });
        let letting_go = pickable.is_some_and(|pickable| self.selection.contains(pickable));
        if dimensioning::awaits_placement(&staged.shown, &staged.picks) && !letting_go {
            self.place_dimension(model, feature, &staged, actions);
            return;
        }
        let Some(pickable) = pickable else {
            if !staged.picks.is_empty() {
                self.place_dimension(model, feature, &staged, actions);
            }
            return;
        };
        if letting_go {
            self.selection.toggle(pickable);
            return;
        }
        let staged = match self.staged_picks(model, feature, Some(pickable)) {
            Ok(staged) => staged,
            Err(unstaged) => {
                actions.push(Action::Inform(Notice::warning(format!("{unstaged}."))));
                return;
            }
        };
        match dimensioning::fitting(&staged.shown, &staged.picks) {
            dimensioning::Fit::Refused(reason) => {
                actions.push(Action::Inform(Notice::warning(format!("{reason}."))));
            }
            dimensioning::Fit::Ready(_)
                if staged.picks.len() > 1
                    && !dimensioning::awaits_placement(&staged.shown, &staged.picks) =>
            {
                self.place_dimension(model, feature, &staged, actions);
            }
            dimensioning::Fit::Ready(_) | dimensioning::Fit::Waiting => {
                self.selection.toggle(pickable);
            }
        }
    }

    fn staged_picks(
        &self,
        model: &Model,
        feature: FeatureId,
        adding: Option<Pickable>,
    ) -> Result<Staged, body_picks::Unstaged> {
        let mut entities = sketch_tools::selected_entities(&self.selection, feature);
        let mut bodies = body_picks::selected(&self.selection, feature);
        match adding {
            Some(Pickable::SketchEntity { entity, .. }) if !entities.contains(&entity) => {
                entities.push(entity);
            }
            Some(pickable) => {
                bodies.extend(BodyPick::of(pickable, feature).filter(|pick| !bodies.contains(pick)))
            }
            None => {}
        }
        Staged::new(model, feature, &self.body_snaps, &entities, &bodies)
    }

    fn place_dimension(
        &mut self,
        model: &Model,
        feature: FeatureId,
        staged: &Staged,
        actions: &mut Vec<Action>,
    ) {
        match dimensioning::dimension(model, feature, staged, self.sketch_cursor) {
            Ok(added) => self.dimension_added(feature, added, actions),
            Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
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

    fn picked_dimension(&self, model: &Model) -> Option<(FeatureId, Staged)> {
        let feature = self.dimensioning?;
        let staged = self.staged_picks(model, feature, None).ok()?;
        matches!(
            dimensioning::fitting(&staged.shown, &staged.picks),
            dimensioning::Fit::Ready(_)
        )
        .then_some((feature, staged))
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
        match (self.pointed(), toggle) {
            (Some(pickable), true) => self.toggle_chosen(model, pickable),
            (Some(pickable), false) => {
                let chosen = self.whole_body_of(model, pickable);
                self.selection.replace_with_all(chosen);
            }
            (None, false) if self.selection.is_empty() => self.tree_dismissed = true,
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

    fn drawn_preview(&self, faceting: Faceting) -> Preview {
        let mut preview = self.drawing.preview(faceting);
        if let Some(mirror) = &self.mirror {
            mirror.add_image(&mut preview);
        }
        preview
    }

    fn keyboard_commands(
        &mut self,
        model: &Model,
        editing: &SketchEditing,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        self.symmetry_command(model, editing, commands, actions);
        let home = model.document().saved_views().home;
        for view in StandardView::ALL {
            if commands.available(Command::View(view)) {
                self.look_from(view, home);
            }
        }
        let back = self.history.availability(self.camera.destination());
        if commands.invoke(Command::PreviousView, &back)
            && let Some(viewpoint) = self.history.back(self.camera.destination())
        {
            self.camera.animate_to(viewpoint);
        }
        self.view_commands(model, commands, actions);
        let facing = editing.feature().ok_or(NOT_IN_A_SKETCH);
        if commands.invoke(Command::LookAtSketch, &facing) && facing.is_ok() {
            self.face_edited_sketch = Some(SketchFacing::Again);
        }
        let looking = look_at::look_target(model, &self.selection);
        if commands.invoke(Command::LookAtFace, &looking)
            && let Ok(target) = looking
        {
            self.look_from = Some(target);
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
        if commands.available(Command::ToggleDimensions) {
            self.dimensions_shown = !self.dimensions_shown;
            if !self.dimensions_shown {
                self.model_dimensions.clear();
                self.keyboard_highlight = self.keyboard_highlight.filter(|highlight| {
                    !matches!(
                        highlight,
                        Pickable::FeatureValue { .. } | Pickable::SketchConstraint { .. }
                    ) || editing.feature().is_some()
                });
            }
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
        if commands.available(Command::ToggleControlPolygons) {
            self.aids.control_polygons_hidden = !self.aids.control_polygons_hidden;
        }
        for (command, setting) in [
            (Command::ToggleSnapping, &mut self.snapping),
            (Command::ToggleGridSnapping, &mut self.grid_snapping),
            (Command::ToggleLasso, &mut self.lasso),
            (Command::TogglePaintSelection, &mut self.paint),
            (Command::ToggleSelectThrough, &mut self.select_through),
            (Command::ToggleTypedDimensions, &mut self.typed_dimensions),
        ] {
            if !commands.available(command) {
                continue;
            }
            *setting = !*setting;
            if commands.state_unseen(command)
                && let Some(text) = toggles::quiet_toggle_notice(command, *setting)
            {
                actions.push(Action::Inform(Notice::info(text)));
            }
        }
        if commands.available(Command::Section(SectionCommand::SliceSketch)) {
            self.sketch_slice = !self.sketch_slice;
        }
        if commands.available(Command::ToggleFirstDimensionScales) {
            self.first_dimension_scales = !self.first_dimension_scales;
        }
        for style in DisplayStyle::ALL {
            if commands.available(Command::Style(style)) {
                self.choose_style(style);
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
            None if !self.scenes.has_pickables() => Err(StepError::NothingToPick),
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
        let list_at = self.list_at.take();
        if commands.invoke(Command::ListUnderPointer, &listing) && listing.is_ok() {
            let centre = self.view().map(|view| view.size() / 2.0);
            if let Some(cursor) = list_at.or(self.cursor).or(self.last_cursor).or(centre) {
                self.open_pick_list(model, editing, cursor, actions);
            }
        }
        let menu = self.view().map(|_| ()).ok_or(NO_VIEW_YET);
        if commands.invoke(Command::ContextMenu, &menu) && menu.is_ok() {
            self.open_menu_from_keys(model, editing);
        }
        if let Some(active) = editing.active() {
            let typable = active.tool.draws()
                || self.modifying.value_field().is_some()
                || self.moving.is_some()
                || self.moving_label.is_some()
                || self.transforming.is_some();
            let typing = match typable {
                true => Ok(()),
                false => Err(TYPE_VALUE_UNAVAILABLE),
            };
            if commands.invoke(Command::TypeValue, &typing) && typable {
                self.typed_point.open_or_resume();
            }
            let reversible = self.drawing.reversible();
            if commands.invoke(Command::ReverseArc, &reversible) {
                self.drawing.reverse_arc();
            }
            let chain_arc = self.drawing.chain_arc();
            if commands.invoke(Command::ChainArc, &chain_arc) && chain_arc.is_ok() {
                let sketch = edited_sketch(model, editing);
                if let Err(reason) = self.drawing.toggle_chain_arc(sketch.as_deref()) {
                    actions.push(Action::Inform(Notice::warning(format!(
                        "{}: {reason}",
                        Command::ChainArc.title()
                    ))));
                }
            }
            self.shape_commands(model, commands, actions);
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
                Err(HighlightNeeded::Target)
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
            actions.extend(
                pick_action((Some(face), None), (model, &self.selection), editing, true)
                    .unwrap_or_default(),
            );
            return;
        }
        let sketch = editing
            .active()
            .ok_or(projecting::NoSketchToProject)
            .and_then(|active| {
                projecting::sketch_to_project(
                    model,
                    active,
                    self.keyboard_highlight.or(self.hovered),
                    &self.chosen_rows,
                )
                .map(|source| (active, source))
            });
        if commands.invoke(Command::ProjectSketch, &sketch)
            && let Ok((active, source)) = sketch
        {
            actions.extend(projected(projecting::project_sketch(
                model,
                active.feature,
                source,
            )));
            return;
        }
        let activation = self
            .keyboard_highlight
            .ok_or("Highlight an item first, with Highlight the next item in the view");
        if commands.invoke(Command::ActivateHighlighted, &activation)
            && let Some(highlight) = self.keyboard_highlight
            && (editing.feature().is_some() || !self.model_dimensions.open(highlight))
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
        let leaves_place = self.leaves_primitive_place(editing, Some(pickable));
        if leaves_place {
            actions.push(Action::Editing(EditingCommand::StopPicking));
        }
        let action = pick_action(
            (Some(pickable), None),
            (model, &self.selection),
            editing,
            false,
        )
        .filter(|_| !leaves_place);
        match action {
            Some(action) => actions.extend(action),
            None if toggle => self.toggle_chosen(model, pickable),
            None => {
                let chosen = self.whole_body_of(model, pickable);
                self.selection.replace_with_all(chosen);
            }
        }
    }

    fn list_availability(&self) -> Result<(), ListRefusal> {
        if self.trimming.is_active() || self.modifying.is_active() {
            Err(ListRefusal::WithTool)
        } else if !self.scenes.has_pickables() {
            Err(ListRefusal::NothingToHighlight)
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
            Pickable::SketchEntity { feature, entity } if feature == active.feature => match sketch
                .entity(entity)
            {
                Some(Entity::Point(position)) => self.drawing.type_point(&sketch, *position),
                Some(_) => {
                    if let Err(reason) = self.drawing.type_on_curve(&sketch, entity) {
                        return Some(Some(Action::Inform(Notice::warning(format!("{reason}.")))));
                    }
                }
                None => return Some(Some(Action::Inform(Notice::warning(PLACE_AT_A_POINT)))),
            },
            _ => return Some(Some(Action::Inform(Notice::warning(PLACE_AT_A_POINT)))),
        }
        Some(match self.drawing.click(model) {
            Ok(transaction) => transaction.map(Action::Apply),
            Err(refusal) => Some(Action::Inform(Notice::warning(refusal.reason()))),
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
                    actions.push(Action::Inform(Notice::success(format!(
                        "Saved the view as {name}. Rename it, or see them all, in Saved views."
                    ))));
                }
                Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
            }
        }
        if commands.available(Command::SavedViews) {
            actions.push(Action::Preferences(PreferencesCommand::ShowSavedViews));
        }
        if commands.available(Command::SetHomeView) {
            match saved_views::set_home(document, current) {
                Ok(transaction) => {
                    actions.push(Action::Apply(transaction));
                    actions.push(Action::Inform(Notice::success(
                        "The Isometric view now shows the model like this.",
                    )));
                }
                Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
            }
        }
        let redefined = match document.saved_views().home {
            Some(_) => Ok(()),
            None => Err(ISOMETRIC_NOT_CHANGED),
        };
        if commands.invoke(Command::ResetHomeView, &redefined) {
            match saved_views::reset_home(document) {
                Ok(transaction) => actions.push(Action::Apply(transaction)),
                Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
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
        let inverse = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_select_all(model, &self.selection, self.active_filter()),
        );
        if commands.invoke(Command::SelectInverse, &inverse)
            && let Ok(kind) = inverse
        {
            let flipped =
                body_selection::invert(model, &self.selection, kind, self.active_filter());
            if flipped.is_empty() {
                actions.push(Action::Inform(Notice::info(
                    body_selection::NOTHING_TO_INVERT,
                )));
            } else {
                self.selection.replace_with_all(flipped);
            }
        }
        let made_together = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_feature_faces(&self.selection),
        );
        if commands.invoke(Command::SelectFeatureFaces, &made_together) && made_together.is_ok() {
            let faces = body_selection::feature_faces(model, &self.selection);
            if faces.is_empty() {
                actions.push(Action::Inform(Notice::info(
                    body_selection::NO_FEATURE_FACES,
                )));
            }
            self.selection.extend(faces);
        }
        let ring =
            body_selection::outside_sketch(in_sketch, body_selection::offer_loop(&self.selection));
        if commands.invoke(Command::SelectLoop, &ring) && ring.is_ok() {
            let edges = body_selection::loops_of(model, &self.selection);
            if edges.is_empty() {
                actions.push(Action::Inform(Notice::info(body_selection::NO_LOOP)));
            } else {
                self.selection.replace_with_all(edges);
            }
        }
        let alike = body_selection::outside_sketch(
            in_sketch,
            body_selection::offer_similar(&self.selection),
        );
        if commands.invoke(Command::SelectSimilar, &alike) && alike.is_ok() {
            let similar = body_selection::similar_to(model, &self.selection);
            match similar.words {
                Some(words) if !similar.found.is_empty() => {
                    actions.push(Action::Inform(Notice::info(format!("Selected {words}"))));
                }
                _ => actions.push(Action::Inform(Notice::info(body_selection::NO_SIMILAR))),
            }
            self.selection.extend(similar.found);
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
                    false => Err(body_selection::SelectionRefusal::NothingToKeep),
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
                    actions.push(Action::Inform(Notice::success(format!(
                        "Saved the selection as {name}. Rename it, select it again or see them \
                         all in Selection sets.{left_out}"
                    ))));
                }
                Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
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
            (Err(reason), None) => Err(*reason),
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
        let everything = sketch_drag::can_select_all(&sketch);
        if commands.invoke(Command::SelectAll, &everything)
            && let Ok(everything) = sketch_drag::select_all(&sketch)
        {
            self.add_to_selection(feature, everything, false);
        }
        let projected = owner
            .kind
            .sketch()
            .map(|definition| definition.projected().collect())
            .unwrap_or_default();
        let solution = model.settled_solution(feature);
        let free = sketch_drag::can_select_free(&sketch, &projected, solution);
        if commands.invoke(Command::SelectFree, &free)
            && let Ok(free) = sketch_drag::select_free(&sketch, &projected, solution)
        {
            self.add_to_selection(feature, free, false);
        }
        let open = sketch_status::open_ends(model.evaluation(), feature);
        if commands.invoke(Command::SelectOpenEnds, &open)
            && let Ok(open) = open
        {
            self.add_to_selection(feature, open, false);
            self.fit_requested = Some(FitAsked::ByUser);
        }
        let redundant = sketch_status::redundant_constraints(model.evaluation(), feature);
        if commands.invoke(Command::SelectRedundant, &redundant)
            && let Ok(redundant) = redundant
        {
            self.selection
                .replace_with_all(redundant.into_iter().map(|constraint| {
                    Pickable::SketchConstraint {
                        feature,
                        constraint,
                    }
                }));
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
            Err(NOTHING_TO_COPY)
        };
        let copying = commands.invoke(Command::CopyGeometry, &copyable);
        let cutting = commands.invoke(Command::CutGeometry, &copyable);
        if (copying || cutting) && copyable.is_ok() {
            match copied_text(model, feature, sketch, selected) {
                Ok((text, what)) => {
                    commands.copy(text);
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
                        actions.push(Action::Inform(Notice::success(format!("Copied {what}."))));
                    }
                }
                Err(error) => actions.push(Action::Inform(Notice::warning(error.to_string()))),
            }
        }
        if commands.invoke(Command::PasteGeometry, &Ok::<(), &'static str>(())) {
            match commands.pasted() {
                Pasted::Unread => commands.ask_for_paste(Command::PasteGeometry),
                Pasted::Nothing => actions.push(Action::Inform(Notice::warning(NOTHING_TO_PASTE))),
                Pasted::Text(text) => self.paste_text(model, feature, text, actions),
            }
        }
    }

    fn paste_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
        actions: &mut Vec<Action>,
    ) {
        let pasted = match caditor_file::read_sketch_clipboard(
            text,
            model.document(),
            &clipboard::source(model),
        ) {
            Ok(pasted) => pasted,
            Err(error) => {
                actions.push(Action::Inform(Notice::warning(format!(
                    "Nothing was pasted: {error}."
                ))));
                return;
            }
        };
        let offset = self.paste_offset(&pasted, feature);
        let what = clipboard::described(&pasted.clip);
        let mut placed = Vec::new();
        let transaction =
            trimming::reshape::<PasteRefusal>(model, feature, format!("Paste {what}"), |working| {
                placed = working
                    .paste(&pasted.clip, offset)
                    .map_err(|source| PasteRefusal::Paste { source })?;
                Ok(())
            });
        match transaction {
            Ok(transaction) => {
                actions.push(Action::Apply(transaction));
                self.add_to_selection(feature, placed, false);
                if let Some(note) = paste_note(&pasted) {
                    actions.push(Action::Inform(Notice::warning(note)));
                }
            }
            Err(reason) => actions.push(Action::Inform(Notice::warning(reason.to_string()))),
        }
    }

    fn paste_offset(&self, pasted: &PastedGeometry, feature: FeatureId) -> Vector2 {
        let centre = pasted.clip.centre().unwrap_or(Point2::ZERO);
        if let Some(cursor) = self.sketch_cursor {
            return cursor - centre;
        }
        let same_sketch =
            pasted.origin == PasteOrigin::ThisDocument && pasted.sketch == Some(feature);
        if !same_sketch {
            return Vector2::ZERO;
        }
        clipboard::same_sketch_shift(&pasted.clip)
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
        let body_items = self.steppable_body_items(editing);
        let highlightable: Vec<Pickable> = self
            .scenes
            .highlightable()
            .iter()
            .copied()
            .filter(|pickable| filter.allows(*pickable))
            .filter(|pickable| {
                projecting.is_none_or(|active| projecting::projectable(model, active, *pickable))
            })
            .chain(body_items)
            .chain(constraints)
            .chain(
                self.model_dimensions
                    .highlightable()
                    .iter()
                    .copied()
                    .filter(|_| editing.feature().is_none() && self.dimensions_shown),
            )
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
        let Some(view) = self.destination_view() else {
            return;
        };
        let from = *view.viewpoint();
        let height = view.size().y;
        let target = from.target;
        let orbit = height * KEYBOARD_ORBIT_FRACTION * self.navigation.orbit_speed;
        let pan = height * KEYBOARD_PAN_FRACTION;
        let units_per_pixel = view.units_per_pixel_at(from.distance);
        let moved = match step {
            CameraMove::OrbitLeft => from.orbited(target, Vector2::new(-orbit, 0.0), height),
            CameraMove::OrbitRight => from.orbited(target, Vector2::new(orbit, 0.0), height),
            CameraMove::OrbitUp => from.orbited(target, Vector2::new(0.0, -orbit), height),
            CameraMove::OrbitDown => from.orbited(target, Vector2::new(0.0, orbit), height),
            CameraMove::PanLeft => from.panned(Vector2::new(-pan, 0.0), units_per_pixel),
            CameraMove::PanRight => from.panned(Vector2::new(pan, 0.0), units_per_pixel),
            CameraMove::PanUp => from.panned(Vector2::new(0.0, -pan), units_per_pixel),
            CameraMove::PanDown => from.panned(Vector2::new(0.0, pan), units_per_pixel),
            CameraMove::ZoomIn => from.zoomed(target, 1.0 / KEYBOARD_ZOOM_FACTOR),
            CameraMove::ZoomOut => from.zoomed(target, KEYBOARD_ZOOM_FACTOR),
        };
        if let Some(moved) = moved {
            self.history.moving(Gesture::Keys, from, self.clock);
            self.camera.glide_to(moved);
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
        let owner = editing.active().map(|active| (active.feature, active.tool));
        if owner != self.typed_owner {
            self.typed_owner = owner;
            self.typed_point.close();
        }
        let typed_waits = matches!(model.queued().map(Queued::rerun), Some(Rerun::Typed(_)));
        if typed_waits && !self.typed_point.is_open() && self.resubmitted.is_none() {
            actions.push(Action::DropQueued);
        }
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
        if editing.feature().is_none() {
            self.type_handle(ui, rect, model, editing, keys_free, actions);
            return;
        }
        self.typed_handle = None;
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
        } else if self.drawing.can_type_rho() {
            format!("   {TYPED_RHO_HINT}")
        } else {
            String::new()
        };
        let hint = format!(
            "Lengths in {}   {TYPED_POINT_HINT}{sides}",
            model.length_unit().symbol()
        );
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let from = typed_point::From {
            last: self.drawing.last_placed(),
            toward: self.drawing.pointer_position(),
        };
        let Some(typed) = self.typed_point.show(
            ui.ctx(),
            top_band(rect, self.cube_area(rect), self.display_area),
            anchor,
            typed_point::FIELD_LABEL,
            &hint,
            typed_point::POINT_PLACEHOLDER,
        ) else {
            self.preview_typed_point(model, &sketch, from);
            return;
        };
        if let Some(count) = typed_point::sides(&typed.text) {
            if let Err(reason) = self.drawing.set_sides(count) {
                self.typed_point
                    .open_with(typed.entered, reason.to_string());
            }
            return;
        }
        if let Some(rho) = typed_point::rho(model, &typed.text) {
            let set = rho
                .map_err(|error| error.to_string())
                .and_then(|(value, typed)| {
                    self.drawing
                        .set_rho(value, typed)
                        .map_err(|error| error.to_string())
                });
            if let Err(reason) = set {
                self.typed_point.open_with(typed.entered, reason);
            }
            return;
        }
        if let Some(heading) = typed_point::heading(model, &typed.text, from) {
            match heading {
                Ok(heading) => {
                    let dimensions = heading
                        .dimension
                        .filter(|_| self.typed_dimensions)
                        .into_iter()
                        .collect();
                    self.drawing.lock_heading(heading.degrees, dimensions);
                }
                Err(error) => self.typed_point.open_with(typed.entered, error),
            }
            return;
        }
        if let Some(lock) = typed_point::length_lock(model, &typed.text, from) {
            let locked = lock.map_err(|error| error.to_string()).and_then(|lock| {
                let dimensions = lock
                    .dimension
                    .filter(|_| self.typed_dimensions)
                    .into_iter()
                    .collect();
                self.drawing
                    .lock_length(lock.length, dimensions)
                    .map_err(|error| error.to_string())
            });
            if let Err(error) = locked {
                self.typed_point.open_with(typed.entered, error);
            }
            return;
        }
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
                        .open_with(typed.entered, refusal.reason().to_owned()),
                }
            }
            Err(error) => {
                self.typed_point.open_with(typed.entered, error);
            }
        }
    }

    fn preview_typed_point(&mut self, model: &Model, sketch: &Sketch, from: typed_point::From) {
        let Some(text) = self.typed_point.typing_text() else {
            return;
        };
        if let Some(heading) = typed_point::heading(model, text, from) {
            if let Ok(heading) = heading {
                self.drawing.preview_heading(sketch, heading.degrees);
            }
            return;
        }
        if let Some(lock) = typed_point::length_lock(model, text, from) {
            if let Ok(lock) = lock {
                self.drawing.preview_length(sketch, lock.length);
            }
            return;
        }
        if let Ok(placed) = typed_point::parse_placed(model, text, from) {
            self.drawing.preview_typed(sketch, placed.position);
        }
    }

    fn type_handle(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
        keys_free: bool,
        actions: &mut Vec<Action>,
    ) {
        let open = editing
            .solid()
            .filter(|_| !editing.context().choosing_in_view);
        if self
            .typed_handle
            .as_ref()
            .is_some_and(|typing| Some(typing.feature) != open)
        {
            self.typed_handle = None;
            self.typed_point.close();
        }
        if self.typed_handle.is_none() {
            let held = match &self.primary {
                Some(PrimaryDrag::Manipulate(manipulating)) => Some(manipulating.handle),
                Some(_) => None,
                None => self.manipulator_hover,
            };
            let typing = held
                .zip(self.manipulator.as_ref())
                .and_then(|(handle, manipulator)| manipulator.typing(model, handle));
            let Some(typing) = typing else {
                self.typed_point.close();
                return;
            };
            if keys_free {
                self.typed_point.open_from_typing(ui.ctx());
            }
            if !self.typed_point.is_open() {
                return;
            }
            if let Some(PrimaryDrag::Manipulate(manipulating)) = self.primary.take() {
                actions.push(Action::Preview {
                    feature: manipulating.feature,
                    draft: None,
                });
            }
            self.typed_handle = Some(typing);
            self.typed_handle_preview = None;
        }
        let Some(typing) = self.typed_handle.clone() else {
            return;
        };
        let text = self.typed_point.typing_text().map(str::to_owned);
        if text != self.typed_handle_preview {
            let draft = text.as_deref().and_then(|text| {
                let values = typing.parse(model, text).ok()?;
                typing.transaction(model, values).ok()
            });
            actions.push(Action::Preview {
                feature: typing.feature,
                draft,
            });
            self.typed_handle_preview = text;
        }
        let hint = format!(
            "Lengths in {}   Enter: set {}   Esc: cancel",
            model.length_unit().symbol(),
            typing.label
        );
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let typed = self.typed_point.show(
            ui.ctx(),
            top_band(rect, self.cube_area(rect), self.display_area),
            anchor,
            &typing.label,
            &hint,
            typing.placeholder(),
        );
        let Some(typed) = typed else {
            if !self.typed_point.is_open() {
                self.typed_handle = None;
                self.typed_handle_preview = None;
                actions.push(Action::Preview {
                    feature: typing.feature,
                    draft: None,
                });
            }
            return;
        };
        match typing
            .parse(model, &typed.text)
            .and_then(|values| typing.transaction(model, values))
        {
            Ok(transaction) => {
                self.typed_handle = None;
                self.typed_handle_preview = None;
                let announced = reversing::announcement(model.document(), &transaction);
                actions.push(Action::Apply(transaction));
                actions.extend(announced.map(Action::Inform));
            }
            Err(error) => {
                self.typed_handle_preview = None;
                self.typed_point.open_with(typed.entered, error);
            }
        }
    }

    pub fn dragged_feature(&self) -> Option<FeatureId> {
        match &self.primary {
            Some(PrimaryDrag::Manipulate(manipulating)) => Some(manipulating.feature),
            _ => self.typed_handle.as_ref().map(|typing| typing.feature),
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
        self.modifying
            .show_text(model, self.typed_point.typing_text());
        let hint = format!(
            "Lengths in {}   Enter: {}   Esc: cancel",
            model.length_unit().symbol(),
            field.action
        );
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let placeholder = self
            .modifying
            .last_value()
            .unwrap_or(field.placeholder)
            .to_owned();
        let Some(typed) = self.typed_point.show(
            ui.ctx(),
            top_band(rect, self.cube_area(rect), self.display_area),
            anchor,
            field.label,
            &hint,
            &placeholder,
        ) else {
            return;
        };
        self.modifying.show_text(model, None);
        match self.modifying.enter_text(model, &typed.text) {
            Ok(outcome) => self.modify(editing, outcome, actions),
            Err(error) => self.typed_point.open_with(typed.entered, error),
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
            Outcome::Refused(reason) => {
                actions.push(Action::Inform(Notice::warning(reason.to_string())))
            }
        }
    }

    fn type_move(&mut self, ui: &egui::Ui, rect: Rect, model: &Model, actions: &mut Vec<Action>) {
        let hint = format!("Lengths in {}   {MOVE_HINT}", model.length_unit().symbol());
        let anchor = rect.center_top() + vec2(0.0, TYPED_POINT_OFFSET);
        let typed = self.typed_point.show(
            ui.ctx(),
            top_band(rect, self.cube_area(rect), self.display_area),
            anchor,
            typed_point::MOVE_LABEL,
            &hint,
            typed_point::POINT_PLACEHOLDER,
        );
        let Some(moving) = self.moving.take() else {
            return;
        };
        let Some(typed) = typed.or_else(|| self.take_resubmitted()) else {
            if self.typed_point.is_open() {
                self.moving = Some(moving);
            }
            return;
        };
        let fresh = fresh_from_sketch(model, moving.feature, |sketch| moving.refreshed(sketch));
        let target = fresh.and_then(|fresh| {
            let from = typed_point::From {
                last: Some(fresh.anchor),
                toward: None,
            };
            let target = typed_point::parse(model, &typed.text, from)?;
            Ok(fresh.to(target))
        });
        match target {
            Ok(commands) => actions.extend(commands.into_iter().map(Action::Drag)),
            Err(error) => {
                queue_typed(&typed.entered, typed_point::MOVE_LABEL, &error, actions);
                self.typed_point.open_with(typed.entered, error.to_string());
                self.moving = Some(moving);
            }
        }
    }

    pub fn resubmit_typed(&mut self, entered: String) {
        self.resubmitted = Some(entered);
    }

    fn take_resubmitted(&mut self) -> Option<Typed> {
        self.resubmitted.take().map(|entered| Typed::of(&entered))
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
            top_band(rect, self.cube_area(rect), self.display_area),
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
                self.typed_point.open_with(typed.entered, error);
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
            top_band(rect, self.cube_area(rect), self.display_area),
            anchor,
            field.label,
            field.hint,
            field.placeholder,
        );
        let Some(transforming) = self.transforming.take() else {
            return;
        };
        let Some(typed) = typed.or_else(|| self.take_resubmitted()) else {
            if self.typed_point.is_open() {
                self.transforming = Some(transforming);
            }
            return;
        };
        let commands = fresh_from_sketch(model, transforming.feature, |sketch| {
            transforming.refreshed(sketch)
        })
        .and_then(|fresh| {
            let amount = typed_point::parse_transform(model, &typed.text, transform)?;
            match transform {
                Transform::Rotate => fresh.rotated(amount),
                Transform::Scale => fresh.scaled(amount),
            }
            .map_err(TypedEditError::from)
        });
        match commands {
            Ok(commands) => actions.extend(commands.into_iter().map(Action::Drag)),
            Err(error) => {
                queue_typed(&typed.entered, field.label, &error, actions);
                self.typed_point.open_with(typed.entered, error.to_string());
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
                pressed_without_repeat(input, Key::Escape),
                input.key_pressed(Key::Enter),
                input.key_pressed(Key::Backspace) || input.key_pressed(Key::Delete),
            )
        });
        if escape {
            self.escape(model.queued().is_some(), editing, actions);
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
            } else if let Some(ended) = self.drawing.finish(model) {
                actions.extend(ended_action(ended));
            } else if let Some((feature, staged)) = self.picked_dimension(model) {
                match dimensioning::dimension(model, feature, &staged, None) {
                    Ok(added) => self.dimension_added(feature, added, actions),
                    Err(reason) => {
                        actions.push(Action::Inform(Notice::warning(reason.to_string())))
                    }
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
                && let Some(highlight) = self.keyboard_highlight
                && self.model_dimensions.open(highlight)
            {
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
        if back && self.drawing.has_heading() {
            self.drawing.release_heading();
        } else if back && self.drawing.in_progress() {
            self.take_back_point(model, actions);
        }
    }

    fn take_back_point(&mut self, model: &Model, actions: &mut Vec<Action>) {
        if self.drawing.remove_last(model.undo_label()) {
            actions.push(Action::Undo);
        }
    }

    fn shape_commands(
        &mut self,
        model: &Model,
        commands: &mut CommandFrame<'_>,
        actions: &mut Vec<Action>,
    ) {
        let started = self.drawing.started();
        if commands.invoke(Command::TakeBackPoint, &started) {
            self.take_back_point(model, actions);
        }
        if commands.invoke(Command::FinishShape, &self.drawing.finishable())
            && let Some(ended) = self.drawing.finish(model)
        {
            actions.extend(ended_action(ended));
        }
        if commands.invoke(Command::CancelShape, &started) {
            self.drawing.cancel();
        }
    }

    fn context_click(&mut self, response: &Response, model: &Model, editing: &SketchEditing) {
        if let Some(at) = self.menu_waits_for_pick
            && let Some(hovered) = self.hover_at(at)
        {
            self.menu_waits_for_pick = None;
            self.open_menu(at.cursor, hovered, hovered.is_some(), model, editing);
        }
        if !response.secondary_clicked() {
            return;
        }
        let Some(cursor) = self.cursor else {
            return;
        };
        if self.menu_selects(editing) && !self.hover_is_current() {
            self.menu_waits_for_pick = self.pick_source();
            self.context_menu = None;
        } else {
            self.open_menu(cursor, self.hovered, self.hovered.is_some(), model, editing);
        }
    }

    fn menu_selects(&self, editing: &SketchEditing) -> bool {
        !editing.is_choosing_plane()
            && !editing.choosing_bodies()
            && editing.picking().is_none()
            && editing
                .active()
                .is_none_or(|active| active.tool == Tool::Select)
            && self.dimensioning.is_none()
    }

    fn menu_place(&self, editing: &SketchEditing, on_item: bool) -> Place {
        if editing.active().is_none() {
            Place::Model {
                on_item,
                feature_open: editing.solid().is_some(),
                selected: !self.selection.is_empty(),
            }
        } else if self.drawing.in_progress() {
            Place::Shape
        } else {
            Place::Sketch {
                on_item: on_item || !self.selection.is_empty(),
            }
        }
    }

    fn open_menu(
        &mut self,
        cursor: Vector2,
        target: Option<Pickable>,
        on_item: bool,
        model: &Model,
        editing: &SketchEditing,
    ) {
        let Some(rect) = self.rect else {
            return;
        };
        if self.menu_selects(editing)
            && let Some(pickable) = target
            && !self.selection.contains(pickable)
        {
            let chosen = self.whole_body_of(model, pickable);
            self.selection.replace_with_all(chosen);
        }
        let at = cursor / f64::from(self.pixels_per_point);
        let anchor = rect.min + vec2(at.x as f32, at.y as f32);
        let place = self.menu_place(editing, on_item);
        let held = match place {
            Place::Model { on_item: true, .. } => Held::of(model, &self.selection),
            Place::Model { .. } | Place::Sketch { .. } | Place::Shape => Held::default(),
        };
        self.pick_list = None;
        self.menus_opened += 1;
        self.context_menu = Some(ViewMenu::new(
            anchor,
            cursor,
            (place, held),
            model.revision(),
            self.menus_opened,
        ));
    }

    fn open_menu_from_keys(&mut self, model: &Model, editing: &SketchEditing) {
        let target = self.keyboard_highlight;
        let centre = self.view().map(|view| view.size() / 2.0);
        let selected: Vec<Pickable> = self.selection.iter().collect();
        let cursor = target
            .and_then(|highlight| {
                self.body_item_on_screen(model, editing, highlight)
                    .or_else(|| self.screen_centre_of(model, editing, vec![highlight]))
            })
            .or_else(|| self.screen_centre_of(model, editing, selected))
            .or(centre);
        let on_item = target.is_some() || !self.selection.is_empty();
        if let Some(cursor) = cursor {
            self.open_menu(cursor, target, on_item, model, editing);
        }
    }

    fn body_item_on_screen(
        &self,
        model: &Model,
        editing: &SketchEditing,
        pickable: Pickable,
    ) -> Option<Vector2> {
        let feature = editing.feature()?;
        let pick = BodyPick::of(pickable, feature)?;
        let target = self.body_snaps.find(pick.body, pick.item)?;
        let on_sketch = self.body_snaps.position(target).or_else(|| {
            let path = self.body_snaps.path(target)?;
            path.get(path.len() / 2).copied()
        })?;
        let plane = scene::sketch_plane(model.document(), model.evaluation(), feature)?;
        let view = self.view()?;
        let size = view.size();
        view.project(plane.to_world(on_sketch))
            .filter(|pixel| (0.0..=size.x).contains(&pixel.x) && (0.0..=size.y).contains(&pixel.y))
    }

    fn screen_centre_of(
        &self,
        model: &Model,
        editing: &SketchEditing,
        pickables: Vec<Pickable>,
    ) -> Option<Vector2> {
        let built = self.scenes.built()?;
        let view = self.view()?;
        let sources = scene_sources(
            model,
            SourceParts {
                bodies: &self.bodies,
                style: self.style,
                aids: self.aids,
                analyses: &self.analyses,
                contrast: self.contrast,
                canvas: self.canvas,
            },
            editing.context().solid,
        );
        let centre = built.bounds_of(&sources, pickables)?.center();
        let size = view.size();
        view.project(centre)
            .filter(|pixel| (0.0..=size.x).contains(&pixel.x) && (0.0..=size.y).contains(&pixel.y))
    }

    pub fn show_menu(
        &mut self,
        ctx: &egui::Context,
        model: &Model,
        editing: &SketchEditing,
        entries: MenuEntries<'_>,
        blocked: bool,
    ) -> Vec<Command> {
        let mut chosen = std::mem::take(&mut self.chosen_commands);
        if blocked {
            chosen.clear();
        }
        if let Some(button) = self.display_button.take().filter(|_| !blocked) {
            chosen.extend(display_picker::menu(&button, entries.fresh()));
        }
        chosen.extend(self.show_context_menu(ctx, model, editing, entries, blocked));
        chosen
    }

    fn show_context_menu(
        &mut self,
        ctx: &egui::Context,
        model: &Model,
        editing: &SketchEditing,
        entries: MenuEntries<'_>,
        blocked: bool,
    ) -> Vec<Command> {
        let Some(menu) = self.context_menu.as_ref() else {
            return Vec::new();
        };
        let still = menu.opened() == model.revision()
            && menu.place().is_like(self.menu_place(editing, false));
        if blocked || !still {
            self.context_menu = None;
            return Vec::new();
        }
        let Some(menu) = self.context_menu.as_mut() else {
            return Vec::new();
        };
        match menu.show(ctx, entries) {
            view_menu::Outcome::Open => Vec::new(),
            view_menu::Outcome::Closed => {
                self.context_menu = None;
                Vec::new()
            }
            view_menu::Outcome::Chosen(chosen) => {
                if chosen.contains(&Command::ListUnderPointer) {
                    self.list_at = Some(menu.cursor());
                }
                if chosen
                    .iter()
                    .any(|command| view_menu::FEATURE_COMMANDS.contains(command))
                {
                    self.row_to_choose = menu.held().feature();
                }
                self.context_menu = None;
                chosen
            }
        }
    }

    #[cfg(test)]
    pub fn context_menu(&self) -> Option<Place> {
        self.context_menu.as_ref().map(ViewMenu::place)
    }

    #[cfg(test)]
    pub fn context_menu_anchor(&self) -> Option<Pos2> {
        self.context_menu.as_ref().map(ViewMenu::anchor)
    }

    fn escape(&mut self, queued: bool, editing: &SketchEditing, actions: &mut Vec<Action>) {
        let active = editing.active();
        if self.pick_list.take().is_some() {
            return;
        }
        if queued {
            actions.push(Action::DropQueued);
            return;
        }
        if self.typed_point.is_waiting() {
            self.typed_point.close();
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
                | PrimaryDrag::Pull { .. }
                | PrimaryDrag::Unmovable { .. } => {}
            }
        } else if editing.is_choosing_plane() {
            actions.push(Action::Editing(EditingCommand::CancelNewSketch));
        } else if editing.combining().is_some() {
            actions.push(Action::Editing(EditingCommand::CancelCombine));
        } else if editing.is_choosing_mirrored() {
            actions.push(Action::Editing(EditingCommand::CancelMirrored));
        } else if editing.picking().is_some() {
            actions.push(Action::Editing(EditingCommand::StopPicking));
        } else if self.drawing.has_heading() {
            self.drawing.release_heading();
        } else if self.drawing.in_progress() {
            self.drawing.cancel();
        } else if self.keyboard_highlight.is_some() {
            self.keyboard_highlight = None;
        } else if self.trimming.has_highlight() {
            self.trimming.clear_highlight();
        } else if self.modifying.can_back_out() {
            self.modifying.back_out();
        } else if (self.dimensioning.is_some() || self.modifying.lets_go_of_selection())
            && !self.selection.is_empty()
        {
            self.selection.clear();
        } else if let Some(active) = active
            && active.tool != Tool::Select
        {
            actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
        } else if !self.selection.is_empty() {
            self.selection.clear();
        } else if active.is_some() {
            if self.sketching.escape_finishes {
                actions.push(Action::Editing(EditingCommand::Finish));
            }
        } else if editing.solid().is_some() {
            actions.push(Action::Editing(EditingCommand::CloseSolid));
        } else {
            self.tree_dismissed = true;
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
        if editing.feature().is_some() || !self.dimensions_shown {
            self.model_dimensions.clear();
        } else if let Some(view) = self.view() {
            let subjects = self.dimension_subjects(model, editing);
            let outside = Outside {
                rect,
                view: &view,
                pixels_per_point: f64::from(self.pixels_per_point),
                subjects: &subjects,
                highlight: self.keyboard_highlight,
                interactive: true,
            };
            self.model_dimensions
                .show(ui, model, &outside, &mut self.selection, actions);
        }
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
            outside: false,
            taken: &[],
        };
        self.annotations
            .show(ui, model, &surface, &mut self.selection, actions);
    }

    fn dimension_subjects(&self, model: &Model, editing: &SketchEditing) -> Vec<FeatureId> {
        let subject = |pickable: Pickable| match pickable {
            Pickable::FeatureValue { feature, .. } | Pickable::SketchConstraint { feature, .. } => {
                Some(feature)
            }
            _ => feature_of(pickable, model),
        };
        let mut subjects: Vec<FeatureId> = Vec::new();
        let candidates = self
            .highlighted()
            .and_then(subject)
            .into_iter()
            .chain(self.rows_to_highlight(editing.context()))
            .chain(
                self.selection
                    .iter()
                    .take(SUBJECT_ITEMS_READ)
                    .filter_map(subject),
            );
        for candidate in candidates {
            if Some(candidate) != editing.solid() && !subjects.contains(&candidate) {
                subjects.push(candidate);
            }
            if subjects.len() >= model_dimensions::MAX_SUBJECTS {
                break;
            }
        }
        subjects
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
        let home = document.saved_views().home;
        let view_keys: Vec<(Vector3, String)> = key_hints
            .views
            .iter()
            .filter(|(view, _)| *view != StandardView::Isometric || home.is_none())
            .map(|(view, keys)| (view.looking_from(), keys.clone()))
            .collect();
        let texts = CubeTexts {
            fit_label,
            fit_hover: &key_hints.fit,
            home_hover: &key_hints.home,
            view_keys: &view_keys,
        };
        let acted = if self.view_cube_shown {
            view_cube::show(ui, (rect, self.cube_placement), orientation, &texts)
        } else {
            None
        };
        let cube_corner = self.view_cube_shown.then_some(self.cube_placement.corner);
        self.display_area = display_picker::area(ui, rect, cube_corner, self.style);
        self.display_button = Some(display_picker::show(ui, self.display_area, self.style));
        match acted {
            Some(CubeAction::LookFrom(direction)) => {
                let destination = self.camera.destination();
                if let Some(viewpoint) =
                    Viewpoint::looking_from(direction, destination.target, destination.distance)
                {
                    self.go_to(viewpoint);
                }
            }
            Some(CubeAction::Orbit(drag)) => self.orbit_with_cube(rect, drag),
            Some(CubeAction::Fit) => self.fit_requested = Some(FitAsked::ByUser),
            Some(CubeAction::Home) => self.look_from(StandardView::Isometric, home),
            None => {}
        }
        view_cube::show_axis_triad(
            ui,
            pos2(self.bottom_strip_left(rect), rect.bottom()),
            orientation,
            self.contrast.palette(self.canvas),
        );

        let painter = ui.painter();
        match &self.primary {
            Some(PrimaryDrag::Box { area, .. }) => paint_area(painter, rect, area, false),
            Some(PrimaryDrag::ModelBox { area }) => {
                let through = self.reach_matters() && self.reaches_through();
                paint_area(painter, rect, area, through);
            }
            _ => {}
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
                    canvas::chrome(painter.ctx()).measure,
                );
                canvas::announce(ui, shown, "measure", label, None);
            }
        }
        if let Some(guide) = &self.calibration
            && let Some(to) = guide.to
            && let Some(label) = &guide.label
            && let Some(view) = self.view()
            && let Some(pixel) = view.project(guide.from.midpoint(to))
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
                    canvas::chrome(painter.ctx()).measure,
                );
                canvas::announce(ui, shown, "calibration", label, None);
            }
        }
        if let Some(view) = self.view() {
            let interference = self.interference.iter().map(|mark| {
                let color = match mark.kind {
                    MarkKind::Overlap => canvas::chrome(painter.ctx()).error,
                    MarkKind::Touch => canvas::chrome(painter.ctx()).measure,
                    MarkKind::Unchecked => canvas::chrome(painter.ctx()).warning,
                };
                (mark.place, &mark.label, color)
            });
            let failures = self.problems.iter().map(|problem| {
                (
                    problem.place,
                    &problem.label,
                    canvas::chrome(painter.ctx()).error,
                )
            });
            let kept = self.kept.iter().map(|kept| {
                (
                    kept.anchor,
                    &kept.label,
                    canvas::chrome(painter.ctx()).measure,
                )
            });
            for (index, (place, label, color)) in
                failures.chain(interference).chain(kept).enumerate()
            {
                let Some(pixel) = view.project(place) else {
                    continue;
                };
                let position = rect.min
                    + egui::Vec2::new(pixel.x as f32, pixel.y as f32) / self.pixels_per_point;
                if rect.contains(position) {
                    let chip =
                        canvas::Label::new(painter, label, canvas::small(), color, f32::INFINITY);
                    let wanted = Align2::CENTER_BOTTOM
                        .anchor_size(position + vec2(0.0, -PROBLEM_LABEL_LIFT), chip.size());
                    let placed = clear_of(wanted, self.cube_area(rect), rect);
                    let shown = chip.paint(painter, placed.min);
                    canvas::announce(ui, shown, &format!("problem {index}"), label, None);
                }
            }
        }
        let cube = self.cube_area(rect);
        let band = top_band(rect, cube, self.display_area);
        let bottom_reserved = if cube.center().y > rect.center().y {
            view_cube::TRIAD_WIDTH.max(rect.bottom() - cube.top())
        } else {
            view_cube::TRIAD_WIDTH
        };
        let mut taken: Vec<Rect> = vec![cube, self.display_area];
        let prompt = match self.prompt(model, editing, key_hints) {
            Some((text, keys)) => Some(canvas_guidance::show_prompt(
                ui,
                rect,
                band,
                bottom_reserved,
                &canvas_guidance::Prompt {
                    text: &text,
                    keys: &keys,
                },
                &mut self.more_keys,
            )),
            None => self.show_start_card(ui, rect, cube, model, editing, key_hints),
        };
        taken.extend(prompt);
        self.paint_orbit_pivot(ui, rect);
        let keyboard_description = self.paint_pointer_labels(ui, rect, model, editing);
        taken.extend(self.pointer_labels.iter().copied());
        if let Some(described) = keyboard_description {
            taken.push(self.paint_keyboard_description(ui, key_hints, band, prompt, &described));
        }
        let painter = ui.painter();
        let readout_left = self.bottom_strip_left(rect) + view_cube::TRIAD_WIDTH;
        let bottom_left = pos2(readout_left, rect.bottom() - canvas::MARGIN);
        let grid_text = self
            .scenes
            .built()
            .and_then(|built| built.scene.grid.as_ref())
            .zip(self.view())
            .map(|(grid, view)| {
                model.length_unit().grid_text(grid_snap_spacing(
                    grid,
                    &view,
                    f64::from(self.pixels_per_point),
                ))
            });
        let labels: Vec<Rect> = self.annotations.label_rects().collect();
        let grid_area = grid_text.and_then(|text| {
            let label = canvas::Label::new(
                painter,
                &text,
                canvas::readout(),
                canvas::chrome(painter.ctx()).text,
                f32::INFINITY,
            );
            let wanted = Align2::LEFT_BOTTOM.anchor_size(bottom_left, label.size());
            canvas_guidance::first_clear(&[wanted], &taken, rect).map(|wanted| {
                let area = label.paint(painter, wanted.min);
                let response =
                    ui.interact(area, ui.scope_id().with("grid spacing"), Sense::hover());
                response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
                area
            })
        });
        taken.extend(grid_area);
        let readout_bottom = grid_area.map_or(bottom_left.y, |area| area.top() - READOUT_GAP);
        let readout_area = self.sketch_cursor.and_then(|position| {
            let text = format!(
                "x {}   y {}",
                model.units().readout_text(position.x),
                model.units().readout_text(position.y)
            );
            let label = canvas::Label::new(
                painter,
                text,
                canvas::readout(),
                canvas::chrome(painter.ctx()).text,
                f32::INFINITY,
            );
            let size = label.size();
            let above = Align2::LEFT_BOTTOM.anchor_size(pos2(readout_left, readout_bottom), size);
            let beside = grid_area.map(|grid| {
                Align2::LEFT_BOTTOM
                    .anchor_size(pos2(grid.right() + canvas::MARGIN, grid.bottom()), size)
            });
            let candidates: Vec<Rect> =
                [above]
                    .into_iter()
                    .chain(beside)
                    .chain((1..=READOUT_LIFTS).map(|lift| {
                        above.translate(vec2(0.0, -(size.y + READOUT_GAP) * lift as f32))
                    }))
                    .collect();
            let clear_of_labels: Vec<Rect> = taken.iter().chain(&labels).copied().collect();
            canvas_guidance::first_clear(&candidates, &clear_of_labels, rect)
                .map(|placed| label.paint(painter, placed.min))
        });
        taken.extend(readout_area);
        let grid_right = grid_area.map_or(readout_left, |area| area.right() + canvas::MARGIN);
        let hints_left = if editing.feature().is_some() {
            grid_right.max(readout_left + READOUT_ROOM)
        } else {
            grid_right
        };
        let hints_right = self.bottom_strip_right(rect) - canvas::MARGIN;
        let hints = canvas::Hints::new(
            painter,
            &key_hints.navigation,
            hints_right - hints_left,
            Align::Max,
        );
        let anchor = pos2(hints_right, rect.bottom() - canvas::MARGIN);
        let placed = Align2::RIGHT_BOTTOM.anchor_size(anchor, hints.size());
        if placed.left() >= hints_left
            && let Some(placed) = canvas_guidance::first_clear(&[placed], &taken, rect)
        {
            hints.paint(painter, placed.min);
        }
    }

    fn show_start_card(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        cube: Rect,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
    ) -> Option<Rect> {
        let shown = is_blank(model, editing)
            && !self.covered
            && self.primary.is_none()
            && self.start_card.shown_in(model.session());
        if !shown {
            return None;
        }
        let card = self
            .start_card
            .show(ui, rect, cube, model.session(), &key_hints.starts);
        self.chosen_commands.extend(card.chosen);
        Some(card.area)
    }

    fn paint_orbit_pivot(&mut self, ui: &Ui, rect: Rect) {
        let Some(mark) = self.orbit_mark else {
            return;
        };
        if !mark.shown(self.clock) {
            self.orbit_mark = None;
            return;
        }
        if !mark.held {
            let remaining = canvas_guidance::ORBIT_MARK_LINGER - (self.clock - mark.at);
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(remaining.max(0.0)));
        }
        let Some(pixel) = self.view().and_then(|view| view.project(mark.pivot)) else {
            return;
        };
        let at = rect.min + egui::Vec2::new(pixel.x as f32, pixel.y as f32) / self.pixels_per_point;
        if rect.contains(at) {
            canvas_guidance::paint_pivot(ui, at, self.contrast.palette(self.canvas));
        }
    }

    fn paint_pointer_labels(
        &mut self,
        ui: &Ui,
        rect: Rect,
        model: &Model,
        editing: &SketchEditing,
    ) -> Option<String> {
        self.pointer_labels.clear();
        let chrome = canvas::chrome(ui.ctx());
        let mut labels = Vec::new();
        if let Some(note) = &self.refusal {
            match note.opacity(self.clock) {
                Some(opacity) => {
                    let wait = note.repaint_after(self.clock);
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(wait));
                    labels.push(
                        PointerLabel::new("refused", note.text(), chrome.warning).faded(opacity),
                    );
                }
                None => self.refusal = None,
            }
        }
        let document = model.document();
        let snap_label = editing
            .feature()
            .and_then(|feature| editing::edited_sketch(document, feature))
            .and_then(|sketch| {
                self.drawing
                    .snap_label(sketch)
                    .or_else(|| match &self.primary {
                        Some(PrimaryDrag::Grab(grab)) => grab.landing_label(sketch),
                        _ => None,
                    })
            });
        labels.extend(snap_label.map(|label| PointerLabel::new("snap", label, chrome.snap)));
        let moved = match &self.primary {
            Some(PrimaryDrag::Manipulate(manipulating)) => {
                Some(manipulating.readout(model.units()))
            }
            _ => None,
        };
        labels.extend(
            self.drawing
                .readout(model.units())
                .or(moved)
                .map(|size| PointerLabel::new("drawing size", size, chrome.text)),
        );
        let unmovable = match &self.primary {
            Some(PrimaryDrag::Unmovable { held, .. }) => Some(held.reason().to_owned()),
            _ => None,
        };
        let blocked = model.drag_blocked().then(|| {
            editing
                .feature()
                .and_then(|feature| model.sketch_conflict(feature))
                .map_or_else(
                    || DRAG_BLOCKED.to_owned(),
                    |conflict| format!("{DRAG_CONFLICT}: {conflict}"),
                )
        });
        labels.extend(
            unmovable.or(blocked).map(|cue| {
                PointerLabel::new("drag blocked", cue, chrome.warning).live(Live::Polite)
            }),
        );
        let described = self.description_words(model, editing);
        let mut keyboard_description = None;
        match described {
            Some(Described::Highlighted(text)) => keyboard_description = Some(text),
            Some(Described::Tool(text)) => {
                self.hover_delay.waited(None, self.clock);
                labels.push(PointerLabel::new("description", text, chrome.text).description());
            }
            Some(Described::Hovered(text)) => {
                match self.hover_delay.waited(Some(&text), self.clock) {
                    Waited::Shown => labels
                        .push(PointerLabel::new("description", text, chrome.text).description()),
                    Waited::Waiting(remaining) => ui
                        .ctx()
                        .request_repaint_after(Duration::from_secs_f64(remaining)),
                    Waited::Nothing => {}
                }
            }
            None => {
                self.hover_delay.waited(None, self.clock);
            }
        }
        let Some(cursor) = self.cursor else {
            return keyboard_description.or_else(|| {
                labels
                    .into_iter()
                    .find(|label| label.name == "description")
                    .map(|label| label.text)
            });
        };
        let pointer =
            rect.min + egui::Vec2::new(cursor.x as f32, cursor.y as f32) / self.pixels_per_point;
        self.pointer_labels = canvas_guidance::paint_beside_pointer(ui, pointer, rect, labels);
        keyboard_description
    }

    fn description_words(&self, model: &Model, editing: &SketchEditing) -> Option<Described> {
        let hovered = self.annotations.hovered().or(self.highlighted());
        let projecting = editing.active().filter(|active| active.tool.projects());
        if let Some(handle) = self.manipulator_hover.filter(|_| self.primary.is_none()) {
            return Some(Described::Tool(
                self.manipulator
                    .as_ref()
                    .map_or_else(|| handle.words(), |shown| shown.words(model, handle)),
            ));
        }
        let tool = if let Some(active) = projecting {
            hovered
                .filter(|hovered| projecting::projectable(model, active, *hovered))
                .and_then(|hovered| projecting::describe(model, active, hovered))
        } else if self.trimming.is_active() {
            edited_sketch(model, editing).and_then(|sketch| self.trimming.label(&sketch))
        } else if self.modifying.is_active() && !self.modifying.gathers() {
            edited_sketch(model, editing)
                .and_then(|sketch| self.modifying.label(&sketch, model.units()))
        } else if let Some(label) = self
            .modifying
            .gathers()
            .then(|| edited_sketch(model, editing))
            .flatten()
            .and_then(|sketch| self.modifying.label(&sketch, model.units()))
        {
            Some(label)
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
            let words = hovered
                .filter(|_| !self.drawing.is_active())
                .map(|hovered| hovered.describe(model.document(), model.evaluation()))?;
            return Some(if self.described_in_band() {
                Described::Highlighted(words)
            } else {
                Described::Hovered(words)
            });
        };
        let words = tool?;
        Some(if self.described_in_band() {
            Described::Highlighted(words)
        } else {
            Described::Tool(words)
        })
    }

    fn described_in_band(&self) -> bool {
        self.keyboard_highlight.is_some()
            || self
                .pick_list
                .as_ref()
                .and_then(PickList::highlighted)
                .is_some()
    }

    fn paint_keyboard_description(
        &self,
        ui: &Ui,
        key_hints: &KeyHints,
        band: Rect,
        prompt: Option<Rect>,
        description: &str,
    ) -> Rect {
        let painter = ui.painter();
        let label = canvas::Label::new(
            painter,
            description,
            canvas::body(),
            canvas::chrome(painter.ctx()).text,
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
        canvas::announce(ui, shown, "description", description, live);
        if self.keyboard_highlight.is_none() {
            return shown;
        }
        let hinted = canvas::Hints::new(painter, &key_hints.highlight, band.width(), Align::Min)
            .paint(
                painter,
                shown.left_bottom() + vec2(0.0, canvas::MARGIN / 3.0),
            );
        shown.union(hinted)
    }

    fn prompt(
        &self,
        model: &Model,
        editing: &SketchEditing,
        key_hints: &KeyHints,
    ) -> Option<(String, HintList)> {
        let document = model.document();
        if let Some(reach) = self.reach_prompt(key_hints) {
            Some(reach)
        } else if editing.is_choosing_plane() {
            Some((
                CHOOSE_PLANE_PROMPT.to_owned(),
                HintList::parsed(CHOOSE_PLANE_HINT),
            ))
        } else if let Some(combining) = editing.combining() {
            let prompt = match combining.target {
                Some(target) => combine_tools::tool_prompt(model, target),
                None => combine_tools::CHOOSE_TARGET_PROMPT.to_owned(),
            };
            Some((prompt, HintList::parsed(CHOOSE_PLANE_HINT)))
        } else if editing.is_choosing_mirrored() {
            Some((
                mirror_tools::CHOOSE_BODIES_PROMPT.to_owned(),
                HintList::parsed(CHOOSE_PLANE_HINT),
            ))
        } else if let Some(picking) = editing.picking() {
            Some((
                reference_picking::prompt(model, picking),
                HintList::parsed(reference_picking::STOP_HINT),
            ))
        } else if let Some(feature) = editing.solid() {
            let kind = document.feature(feature).map(|feature| &feature.kind);
            let (text, clicks) = feature_prompt(kind);
            let keys = match clicks {
                Clicks::InTheView => HintList::parsed(DONE_HINT),
                Clicks::InThePanel => HintList::default(),
            };
            Some((text.to_owned(), keys))
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
            let staged = self.staged_picks(model, feature, None).ok()?;
            let (text, keys) = dimensioning::prompt(
                &staged.shown,
                staged.names(),
                &staged.picks,
                self.sketch_cursor,
            );
            Some((text, HintList::parsed(keys)))
        } else if let Some(prompt) = self.trimming.prompt() {
            Some((prompt.to_owned(), key_hints.targets.clone()))
        } else if let Some(prompt) = self.modifying.prompt() {
            (!self.typed_point.is_open()).then(|| {
                let keys = match prompt.hint {
                    Hint::Targets => key_hints.targets.clone(),
                    Hint::Keys(keys) => HintList::parsed(keys),
                    Hint::Text(keys) => HintList::parsed(&keys),
                };
                let keys = match self.modifying.mode(editing.modes()) {
                    Some(mode) => key_hints.mode(mode).and(keys),
                    None => keys,
                };
                (prompt.text.to_owned(), keys)
            })
        } else if let Some(prompt) = self.drawing.prompt() {
            (!self.typed_point.is_open()).then(|| {
                let text = match &self.mirror {
                    Some(mirror) => symmetric_drawing::prompt(&prompt.text, &mirror.label),
                    None => prompt.text.clone(),
                };
                (text, self.drawing_hints(&prompt, key_hints))
            })
        } else {
            self.select_prompt(editing, key_hints)
        }
    }

    fn drawing_hints(&self, prompt: &drawing::Prompt, key_hints: &KeyHints) -> HintList {
        let mut keys = self
            .drawing
            .mode()
            .map(|mode| key_hints.mode(mode))
            .unwrap_or_default();
        if let Some(chain_arc) = key_hints
            .chain_arc
            .as_ref()
            .filter(|_| self.drawing.chain_arc().is_ok())
        {
            let next = if self.drawing.arcs_next() {
                "a line instead"
            } else {
                "a tangent arc next"
            };
            keys.push(KeyHint::new(chain_arc, next).ranked(Rank::Useful));
        }
        if let Some(reverse) = key_hints
            .reverse
            .as_ref()
            .filter(|_| self.drawing.reversible().is_ok())
        {
            keys.push(reverse.clone());
        }
        if let Some(sides) = key_hints
            .sides
            .as_ref()
            .filter(|_| self.drawing.more_sides().is_ok() || self.drawing.fewer_sides().is_ok())
        {
            keys.push(sides.clone());
            keys = keys.and(HintList::parsed_as(SCRUB_HINT, Rank::Extra));
        }
        keys = keys.and(prompt.hints());
        if self.drawing.in_progress() {
            keys = keys.and(HintList::parsed(TAKE_BACK_HINT));
        }
        if self.snapping {
            keys = keys.and(HintList::parsed_as(FREE_PLACEMENT_HINT, Rank::Extra));
        } else {
            keys = keys.and(HintList::parsed(SNAPPING_OFF_HINT));
        }
        keys = keys.and(HintList::parsed_as(HELD_SNAP_HINT, Rank::Extra));
        keys = keys.and(HintList::parsed_as(TYPE_POINT_HINT, Rank::Useful));
        if self.drawing.can_type_sides() {
            keys = keys.and(HintList::parsed_as(TYPED_SIDES_HINT, Rank::Extra));
        } else if self.drawing.can_type_rho() {
            keys = keys.and(HintList::parsed_as(TYPED_RHO_HINT, Rank::Extra));
        }
        keys
    }

    fn select_prompt(
        &self,
        editing: &SketchEditing,
        key_hints: &KeyHints,
    ) -> Option<(String, HintList)> {
        let active = editing.active()?;
        if active.tool != Tool::Select || self.typed_point.is_open() || self.primary.is_some() {
            return None;
        }
        let mut keys = HintList::default()
            .with(KeyHint::new("Drag", "move geometry").ranked(Rank::Useful))
            .with(KeyHint::new("Shift+click", "add to the selection").ranked(Rank::Extra));
        if self.selection.is_empty() {
            let finish = match (&key_hints.finish, self.sketching.escape_finishes) {
                (Some(finish), true) => Some(format!("Esc or {finish}")),
                (None, true) => Some("Esc".to_owned()),
                (Some(finish), false) => Some(finish.clone()),
                (None, false) => None,
            };
            if let Some(finish) = finish {
                keys.push(KeyHint::new(&finish, "finish the sketch"));
            }
        } else {
            keys.push(KeyHint::new("Esc", "clear the selection"));
        }
        Some((SELECT_PROMPT.to_owned(), keys))
    }
    fn reaches_through(&self) -> bool {
        self.select_through || self.through_held
    }

    fn reach_matters(&self) -> bool {
        match &self.primary {
            Some(PrimaryDrag::ModelBox { .. }) => {
                self.style.picks_faces()
                    && Catch::of(self.active_filter(), true) != Catch::SketchGeometry
            }
            Some(PrimaryDrag::Paint(_)) => true,
            _ => false,
        }
    }

    fn reach_prompt(&self, key_hints: &KeyHints) -> Option<(String, HintList)> {
        if !self.reach_matters() {
            return None;
        }
        let (text, keys) = if self.select_through {
            (TAKING_HIDDEN_PROMPT, key_hints.select_through_on.clone())
        } else if self.through_held {
            (TAKING_HIDDEN_PROMPT, HintList::parsed(LET_GO_OF_ALT_HINT))
        } else {
            (TAKING_SEEN_PROMPT, HintList::parsed(HOLD_ALT_HINT))
        };
        Some((text.to_owned(), keys))
    }

    fn dimension_hover(
        &self,
        model: &Model,
        feature: FeatureId,
        hovered: Option<Pickable>,
    ) -> Option<String> {
        let hovered = hovered?;
        if hovered.owner() != Some(feature) {
            return None;
        }
        let picked = self.selection.contains(hovered);
        let staged = match self.staged_picks(model, feature, Some(hovered)) {
            Ok(staged) => staged,
            Err(unstaged) => return Some(unstaged.to_string()),
        };
        let hovered_id = match hovered {
            Pickable::SketchEntity { entity, .. } => entity,
            Pickable::BodyItem { body, item, .. } => staged.body_entity(BodyPick { body, item })?,
            _ => return None,
        };
        let picks: Vec<EntityId> = staged
            .picks
            .iter()
            .copied()
            .filter(|pick| picked || *pick != hovered_id)
            .collect();
        Some(dimensioning::hover_words(
            &staged.shown,
            staged.names(),
            &picks,
            hovered_id,
        ))
    }
}

fn pressed_without_repeat(input: &egui::InputState, pressed: Key) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                ..
            } if *key == pressed
        )
    })
}

fn is_blank(model: &Model, editing: &SketchEditing) -> bool {
    model.document().features().next().is_none()
        && editing.feature().is_none()
        && editing.solid().is_none()
        && !editing.is_choosing_plane()
}

fn top_band(rect: Rect, cube: Rect, display: Rect) -> Rect {
    let mut left = rect.left() + canvas::MARGIN;
    let mut right = rect.right() - canvas::MARGIN;
    for keep_out in [cube, display] {
        if keep_out.is_positive() && keep_out.center().y <= rect.center().y {
            if keep_out.center().x < rect.center().x {
                left = left.max(keep_out.right() + canvas::MARGIN);
            } else {
                right = right.min(keep_out.left() - canvas::MARGIN);
            }
        }
    }
    Rect::from_min_max(
        pos2(left, rect.top() + canvas::MARGIN),
        pos2(right.max(left), rect.bottom()),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clicks {
    InTheView,
    InThePanel,
}

fn feature_prompt(kind: Option<&FeatureKind>) -> (&'static str, Clicks) {
    let in_view = |text| (text, Clicks::InTheView);
    let in_panel = |text| (text, Clicks::InThePanel);
    match kind {
        Some(FeatureKind::Blend(blend)) => match blend.kind {
            BlendKind::Fillet => in_view("Click edges to fillet, and again to leave one out"),
            BlendKind::Chamfer => in_view("Click edges to chamfer, and again to leave one out"),
        },
        Some(FeatureKind::Shell(_)) => {
            in_view("Click a face to shell the body open there, and again to close it")
        }
        Some(FeatureKind::OffsetFace(_)) => {
            in_view("Click faces to move them, and again to leave one out")
        }
        Some(FeatureKind::MoveFace(_)) => {
            in_view("Click faces to move them, and again to leave one out")
        }
        Some(FeatureKind::Draft(_)) => {
            in_view("Click faces to draft them, and again to leave one out")
        }
        Some(FeatureKind::ReplaceFace(_)) => {
            in_view("Click faces to replace them, and again to leave one out")
        }
        Some(FeatureKind::DeleteFace(_)) => {
            in_view("Click faces to delete them, and again to keep one")
        }
        Some(FeatureKind::Emboss(emboss)) => match emboss.regions {
            RegionChoice::All => in_view("Click faces to emboss, and again to leave one out"),
            RegionChoice::Chosen(_) => in_view(
                "Click faces and regions of the sketch to emboss, and again to leave one out",
            ),
        },
        Some(FeatureKind::SplitFace(_)) => {
            in_view("Click faces to split, and again to leave one out")
        }
        Some(FeatureKind::Solid(SolidFeature::Extrude(_))) => {
            in_view("Click regions of the sketch to extrude, and again to leave one out")
        }
        Some(FeatureKind::Solid(SolidFeature::Revolve(_))) => {
            in_view("Click regions of the sketch to revolve, and again to leave one out")
        }
        Some(FeatureKind::Solid(SolidFeature::Sweep(_))) => {
            in_view("Click regions of the sketch to sweep, and again to leave one out")
        }
        Some(FeatureKind::Pattern(pattern)) => in_view(match pattern.kind {
            PatternKind::Linear { .. } => {
                "Click a copy to leave it out; set the direction, count and spacing in the panel"
            }
            PatternKind::Circular(_) => {
                "Click a copy to leave it out; set the axis, count and angle in the panel"
            }
            PatternKind::Curve(_) => {
                "Click a copy to leave it out; set the curve to follow and the count in the panel"
            }
            PatternKind::Points(_) => {
                "Click a copy to leave it out; set the sketch whose points the copies go to in \
                 the panel"
            }
        }),
        Some(FeatureKind::Datum(datum)) => in_panel(match datum {
            Datum::Plane(_) => "Choose in the panel the plane or flat face the plane starts from",
            Datum::PlaneThrough(_) => {
                "Choose in the panel the points, planes or edges the plane passes through"
            }
            Datum::Axis(_) => {
                "Choose in the panel the edge, axis, planes or points the axis runs along"
            }
            Datum::Point(_) | Datum::PointBy(_) => {
                "Choose in the panel the corner, edge or point where the point goes"
            }
            Datum::Frame(_) => "Choose in the panel the frame's origin, X axis and XY plane",
        }),
        Some(FeatureKind::Combine(_)) => {
            in_panel("Choose the operation and the two bodies in the panel")
        }
        Some(FeatureKind::Move(_)) => {
            in_panel("Drag an arrow or a square, or type the turns and distances in the panel")
        }
        Some(FeatureKind::Scale(_)) => {
            in_panel("Type the factor and choose the centre in the panel")
        }
        Some(FeatureKind::Mirror(mirror)) if mirror.mirrors_whole_body() => {
            in_view("Click a plane to mirror across, or choose it and the bodies in the panel")
        }
        Some(FeatureKind::Mirror(_)) => in_panel("Choose the plane to mirror across in the panel"),
        Some(FeatureKind::Split(_)) => in_panel("Choose what to split along in the panel"),
        Some(FeatureKind::Mate(_)) => in_panel("Choose the faces or axes to mate in the panel"),
        Some(FeatureKind::Hole(_)) => {
            in_panel("Choose the hole's style and sizes, and where it goes, in the panel")
        }
        Some(FeatureKind::Primitive(_)) => {
            in_panel("Type the sizes and choose where the shape goes in the panel")
        }
        Some(FeatureKind::Pipe(_)) => {
            in_panel("Choose the path, the section and its sizes in the panel")
        }
        Some(FeatureKind::Loft(_)) => in_panel(
            "Choose the profiles in order, where it ends and how its sides run in the panel",
        ),
        Some(FeatureKind::Coil(_)) => {
            in_panel("Choose the axis, the turns and the section in the panel")
        }
        Some(FeatureKind::Helix(_)) => {
            in_panel("Choose the axis, the diameter and the turns in the panel")
        }
        Some(FeatureKind::Rib(_)) => in_panel(
            "Choose the sketch, the body, which way it runs and its thickness in the panel",
        ),
        Some(FeatureKind::Thread(_)) => {
            in_panel("Choose the face to thread, its size and its length in the panel")
        }
        Some(FeatureKind::ReferenceImage(_)) => {
            in_panel("Choose where the picture lies, its size and how it shows in the panel")
        }
        Some(
            FeatureKind::Sketch(_)
            | FeatureKind::Import(_)
            | FeatureKind::Remove(_)
            | FeatureKind::Measurement(_),
        )
        | None => in_panel("Change the feature in its panel"),
    }
}

fn paint_area(painter: &egui::Painter, rect: Rect, area: &ScreenArea, through: bool) {
    let chrome = canvas::chrome(painter.ctx());
    let corner = |at: Vector2| rect.min + egui::Vec2::new(at.x as f32, at.y as f32);
    let (outline, colour, dashed) = match area {
        ScreenArea::Lasso(points) => (
            points.iter().map(|point| corner(*point)).collect(),
            chrome.selected,
            false,
        ),
        ScreenArea::Box(area) => {
            let drawn = Rect::from_two_pos(corner(area.from), corner(area.to));
            let (colour, dashed) = match area.mode() {
                BoxMode::Window => (chrome.selected, false),
                BoxMode::Crossing => (chrome.snap, true),
            };
            painter.rect_filled(drawn, 0.0, colour.gamma_multiply(BOX_FILL_OPACITY));
            let outline: Vec<egui::Pos2> = vec![
                drawn.left_top(),
                drawn.right_top(),
                drawn.right_bottom(),
                drawn.left_bottom(),
            ];
            (outline, colour, dashed)
        }
    };
    let Some(first) = outline.first().copied() else {
        return;
    };
    let closed: Vec<egui::Pos2> = outline.into_iter().chain([first]).collect();
    let strokes = if through {
        vec![
            Stroke::new(THROUGH_STROKE_WIDTH, colour),
            Stroke::new(BOX_STROKE_WIDTH, chrome.panel),
        ]
    } else {
        vec![Stroke::new(BOX_STROKE_WIDTH, colour)]
    };
    for stroke in strokes {
        if dashed {
            painter.extend(Shape::dashed_line(&closed, stroke, BOX_DASH, BOX_GAP));
        } else {
            painter.add(Shape::line(closed.clone(), stroke));
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
        InputMode::SolidWorks => (
            (middle && !shift && !ctrl) || (right && !shift),
            (middle && ctrl && !shift) || (right && shift),
            middle && shift && !ctrl,
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

fn zoom_factor(navigation: &Navigation, zooming_in: f64) -> f64 {
    let direction = if navigation.invert_zoom { 1.0 } else { -1.0 };
    (direction * zooming_in * ZOOM_PER_SCROLL_POINT * navigation.zoom_speed).exp()
}

fn pinch_factor(navigation: &Navigation, spread: f64) -> f64 {
    if navigation.invert_zoom {
        spread
    } else {
        spread.recip()
    }
}

fn fresh_from_sketch<T, E: Into<TypedEditError>>(
    model: &Model,
    feature: FeatureId,
    refresh: impl FnOnce(&Sketch) -> Result<T, E>,
) -> Result<T, TypedEditError> {
    let shown = model
        .document()
        .feature(feature)
        .and_then(|owner| model.displayed_sketch(owner))
        .ok_or(TypedEditError::SketchShown)?;
    refresh(&shown).map_err(Into::into)
}

fn queue_typed(entered: &str, title: &str, error: &TypedEditError, actions: &mut Vec<Action>) {
    if let Some(pending) = error.pending() {
        let rerun = Rerun::Typed(entered.to_owned());
        actions.push(Action::Queue(Box::new(Queued::new(rerun, title, pending))));
    }
}

fn edited_sketch<'a>(model: &'a Model, editing: &SketchEditing) -> Option<Displayed<'a>> {
    let feature = model.document().feature(editing.feature()?)?;
    model.displayed_sketch(feature)
}

struct SourceParts<'a> {
    bodies: &'a BodyMeshes,
    style: DisplayStyle,
    aids: ViewAids,
    analyses: &'a Analyses,
    contrast: Contrast,
    canvas: Canvas,
}

fn scene_sources<'a>(
    model: &'a Model,
    parts: SourceParts<'a>,
    solid: Option<FeatureId>,
) -> Sources<'a> {
    Sources {
        document: model.document(),
        evaluation: model.evaluation(),
        bodies: parts.bodies,
        sketches: &model.display().sketches,
        style: parts.style,
        aids: parts.aids,
        analyses: parts.analyses,
        contrast: parts.contrast,
        canvas: parts.canvas,
        draft: solid.and_then(|feature| model.draft_evaluation_of(feature)),
        pictures: Some(model.pictures()),
    }
}

fn ended_action(ended: Ended) -> Option<Action> {
    match ended {
        Ended::Drawn(transaction) => Some(Action::Apply(transaction)),
        Ended::Stopped => None,
        Ended::Refused(refusal) => Some(Action::Inform(Notice::warning(refusal.reason()))),
    }
}

fn outcome_action<E: Display>(outcome: Result<Option<Transaction>, E>) -> Option<Action> {
    match outcome {
        Ok(transaction) => transaction.map(Action::Apply),
        Err(reason) => Some(Action::Inform(Notice::warning(reason.to_string()))),
    }
}

fn open_command(pickable: Option<Pickable>, model: &Model) -> Option<EditingCommand> {
    let pickable = pickable?;
    match pickable {
        Pickable::SketchEntity { .. } => feature_of(pickable, model).map(EditingCommand::Enter),
        Pickable::Datum(_)
        | Pickable::FrameAxis { .. }
        | Pickable::FramePlane { .. }
        | Pickable::Face { .. } => feature_of(pickable, model).map(EditingCommand::OpenSolid),
        _ => None,
    }
}

pub fn feature_of(pickable: Pickable, model: &Model) -> Option<FeatureId> {
    match pickable {
        Pickable::SketchEntity { feature, .. }
        | Pickable::SketchRegion { feature, .. }
        | Pickable::Datum(feature)
        | Pickable::FrameAxis { feature, .. }
        | Pickable::FramePlane { feature, .. } => Some(feature),
        Pickable::Face { body, face } => bodies::shown(model.evaluation(), body)
            .and_then(|shown| bodies::face_origin(shown, face))
            .map(bodies::origin_feature),
        Pickable::Edge { body, edge } => edge_feature(model, body, edge),
        _ => None,
    }
}

pub fn tree_row_of(pickable: Pickable, model: &Model) -> Option<FeatureId> {
    match pickable {
        Pickable::Face { .. } | Pickable::Edge { .. } => feature_of(pickable, model)
            .filter(|feature| model.document().feature(*feature).is_some())
            .or_else(|| pickable.owner()),
        _ => pickable.owner(),
    }
}

fn edge_feature(model: &Model, body: FeatureId, edge: EdgeName) -> Option<FeatureId> {
    let shown = bodies::shown(model.evaluation(), body)?;
    let found = bodies::find_edge(shown, edge)?;
    let document = model.document();
    caditor_document::edge_faces(&shown.solid, found)
        .into_iter()
        .filter_map(|face| shown.solid.face(face)?.origin())
        .map(bodies::origin_feature)
        .max_by_key(|feature| document.feature_index(*feature))
}

fn projected(
    projection: Result<projecting::Projection, projecting::ProjectionError>,
) -> Vec<Action> {
    match projection {
        Ok(projection) => std::iter::once(Action::Apply(projection.transaction))
            .chain(
                projection
                    .left_out
                    .map(|words| Action::Inform(Notice::info(format!("{words}.")))),
            )
            .collect(),
        Err(reason) => vec![Action::Inform(Notice::warning(format!("{reason}.")))],
    }
}

fn pick_action(
    (pickable, ray): (Option<Pickable>, Option<Ray>),
    (model, selection): (&Model, &Selection),
    editing: &SketchEditing,
    whole: bool,
) -> Option<Vec<Action>> {
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::ImageCalibration
    {
        return Some(image_tools::calibration_click(
            model,
            editing,
            picking.feature,
            ray,
        ));
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::ImagePlace
    {
        return Some(
            pickable
                .map(|pickable| image_tools::place_click(model, picking.feature, pickable, ray))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::PrimitivePlace
    {
        return Some(
            pickable
                .map(|pickable| primitive_tools::place_click(model, picking.feature, pickable, ray))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::HolePlace
    {
        return Some(
            pickable
                .map(|pickable| hole_tools::place_click(model, picking.feature, pickable, ray))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::ShellOwnThickness
    {
        return Some(
            pickable
                .map(|pickable| shell_tools::own_thickness_click(model, picking.feature, pickable))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::DraftOwnAngle
    {
        return Some(
            pickable
                .map(|pickable| draft_tools::own_angle_click(model, picking.feature, pickable))
                .unwrap_or_default(),
        );
    }
    if let Some(picking) = editing.picking()
        && picking.slot == Slot::HoleAdd
    {
        return Some(
            pickable
                .map(|pickable| hole_placement::add_click(model, picking.feature, pickable, ray))
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
            Some(target) => projected(projecting::project(model, active, target, whole)),
            None => Vec::new(),
        });
    }
    let copy = editing.solid().zip(pickable).and_then(|(open, pickable)| {
        pattern_tools::clicked_copy(model, open, pickable).map(|copy| (open, copy))
    });
    if let Some((open, copy)) = copy {
        return Some(match pattern_tools::leave_out(model, open, copy) {
            Ok(transaction) => vec![Action::Apply(transaction)],
            Err(reason) => vec![Action::Inform(Notice::warning(format!("{reason}.")))],
        });
    }
    let plane = editing
        .solid()
        .zip(pickable)
        .and_then(|(open, pickable)| mirror_tools::plane_click(model, open, pickable));
    if plane.is_some() {
        return plane;
    }
    let toggled = match pickable {
        Some(Pickable::Region { feature, region }) => {
            let emboss = model
                .document()
                .feature(feature)
                .is_some_and(|owner| owner.kind.emboss().is_some());
            Some(if emboss {
                emboss_tools::toggle_region(model, feature, region)
            } else {
                solid_tools::toggle_region(model, feature, region)
            })
        }
        Some(Pickable::BlendEdge { feature, edge }) => {
            Some(blend_tools::toggle_edge(model, feature, edge))
        }
        Some(Pickable::ShellFace { feature, face }) => {
            let kind = model.document().feature(feature).map(|owner| &owner.kind);
            Some(match kind {
                Some(FeatureKind::OffsetFace(_)) => {
                    offset_face_tools::toggle_face(model, feature, face)
                }
                Some(FeatureKind::DeleteFace(_)) => {
                    delete_face_tools::toggle_face(model, feature, face)
                }
                Some(FeatureKind::MoveFace(_)) => {
                    move_face_tools::toggle_face(model, feature, face)
                }
                Some(FeatureKind::Draft(_)) => draft_tools::toggle_face(model, feature, face),
                Some(FeatureKind::ReplaceFace(_)) => {
                    replace_face_tools::toggle_face(model, feature, face)
                }
                Some(FeatureKind::Emboss(_)) => emboss_tools::toggle_face(model, feature, face),
                Some(FeatureKind::SplitFace(_)) => {
                    split_face_tools::toggle_face(model, feature, face)
                }
                _ => shell_tools::toggle_face(model, feature, face),
            })
        }
        _ => None,
    };
    if let Some(toggled) = toggled {
        return Some(toggled.map(Action::Apply).into_iter().collect());
    }
    if editing.combining().is_some() {
        return Some(
            pickable
                .and_then(Pickable::body)
                .map(|body| Action::Editing(EditingCommand::CombineWith(body)))
                .into_iter()
                .collect(),
        );
    }
    if editing.is_choosing_mirrored() {
        if pickable.is_some_and(|pickable| mirror_tools::names_a_plane(model, pickable)) {
            return None;
        }
        return Some(
            pickable
                .and_then(Pickable::body)
                .map(|body| mirror_tools::chosen_body_actions(model, selection, body))
                .unwrap_or_default(),
        );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SketchFacing {
    Entering,
    Again,
}

fn entering(camera: &Camera, heading: &View, sketch: &EditedSketch) -> Viewpoint {
    let size = heading.size();
    let (centre, distance) = (sketch.bounds.center(), heading.viewpoint().distance);
    let seen_from_behind = sketch.plane.signed_distance(heading.viewpoint().eye()) < 0.0;
    let facing = if seen_from_behind {
        Viewpoint::facing_from_behind(&sketch.plane, centre, distance)
    } else {
        Viewpoint::facing(&sketch.plane, centre, distance)
    };
    camera
        .view_from(facing, size.x, size.y)
        .fitted(sketch.bounds)
}

fn facing(camera: &Camera, heading: &View, sketch: &EditedSketch) -> Viewpoint {
    looking_at(
        camera,
        heading,
        LookTarget::Plane(sketch.plane),
        sketch.bounds,
    )
}

fn looking_at(camera: &Camera, heading: &View, target: LookTarget, bounds: Aabb) -> Viewpoint {
    let size = heading.size();
    let destination = target.viewpoint(heading.viewpoint(), bounds.center());
    camera.view_from(destination, size.x, size.y).fitted(bounds)
}

struct KeyHints {
    navigation: HintList,
    highlight: HintList,
    fit: String,
    home: String,
    views: Vec<(StandardView, String)>,
    reverse: Option<KeyHint>,
    chain_arc: Option<String>,
    sides: Option<KeyHint>,
    targets: HintList,
    whole_body: HintList,
    select_through_on: HintList,
    finish: Option<String>,
    tools: Vec<(Tool, String)>,
    starts: Vec<Start>,
}

impl KeyHints {
    fn new(commands: &CommandFrame<'_>, input_mode: InputMode) -> Self {
        let mut navigation = HintList::parsed(input_mode.navigation_hint());
        if let Some(keys) = commands.keys(Command::FitView) {
            navigation.push(KeyHint::new(&keys, "fit"));
        }
        let highlight = [
            (Command::ActivateHighlighted, "select or pick"),
            (Command::HighlightNext, "next"),
            (Command::HighlightPrevious, "previous"),
        ]
        .into_iter()
        .filter_map(|(command, what)| Some(KeyHint::new(&commands.keys(command)?, what)))
        .chain([
            KeyHint::new("Enter", "open"),
            KeyHint::new("Esc", "stop highlighting"),
        ])
        .collect();
        let targets: HintList = [
            (Command::HighlightNext, "next"),
            (Command::HighlightPrevious, "previous"),
        ]
        .into_iter()
        .filter_map(|(command, what)| {
            Some(KeyHint::new(&commands.keys(command)?, what).ranked(Rank::Useful))
        })
        .chain([
            KeyHint::new("Enter", "act on the highlighted one").ranked(Rank::Useful),
            KeyHint::new("Esc", "back to Select"),
        ])
        .collect();
        let whole_body = match commands.keys(Command::IntersectBody) {
            Some(keys) => HintList::default()
                .with(KeyHint::new(&keys, "the whole body"))
                .and(targets.clone()),
            None => targets.clone(),
        };
        let select_through_on = match commands.keys(Command::ToggleSelectThrough) {
            Some(keys) => HintList::default().with(KeyHint::new(&keys, "turn Select through off")),
            None => HintList::parsed(SELECT_THROUGH_ON_HINT),
        };
        Self {
            navigation,
            highlight,
            targets,
            whole_body,
            select_through_on,
            fit: commands.with_keys(Command::FitView, "Frame the view around it"),
            home: commands.with_keys(
                Command::View(StandardView::Isometric),
                "Go to the Isometric view",
            ),
            views: StandardView::ALL
                .into_iter()
                .filter_map(|view| Some((view, commands.keys(Command::View(view))?)))
                .collect(),
            reverse: commands
                .keys(Command::ReverseArc)
                .map(|keys| KeyHint::new(&keys, "the other way round")),
            chain_arc: commands
                .keys(Command::ChainArc)
                .or_else(|| commands.keys(Command::SketchTool(Tool::TangentArc))),
            sides: commands
                .keys(Command::MoreSides)
                .zip(commands.keys(Command::FewerSides))
                .map(|(more, fewer)| {
                    KeyHint::new(&format!("{more} or {fewer}"), "more or fewer sides")
                }),
            finish: commands.keys(Command::FinishSketch),
            tools: Tool::ALL
                .into_iter()
                .filter_map(|tool| Some((tool, commands.keys(Command::SketchTool(tool))?)))
                .collect(),
            starts: [
                (
                    Command::NewSketch,
                    start_card::START_WITH_SKETCH,
                    "Sketch on the plane or flat face you click next, then extrude it",
                ),
                (
                    Command::NewBox,
                    start_card::START_WITH_BOX,
                    "Add a box and set its sizes in its panel",
                ),
                (
                    Command::Import,
                    start_card::START_WITH_IMPORT,
                    "Bring in a STEP, mesh, DXF or SVG file",
                ),
            ]
            .into_iter()
            .map(|(command, label, hover)| Start {
                command,
                label,
                hover: commands.with_keys(command, hover),
            })
            .collect(),
        }
    }

    fn mode(&self, mode: ShapeMode) -> HintList {
        let keys = self
            .tools
            .iter()
            .find(|(tool, _)| *tool == mode.tool())
            .map(|(_, keys)| keys.as_str());
        HintList::parsed_as(&mode.hint(keys), Rank::Useful)
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
    fn a_problem_label_moves_left_of_the_view_cube_and_stays_in_the_view() {
        let view = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        let cube = Rect::from_min_max(pos2(640.0, 0.0), pos2(800.0, 200.0));
        let clear = Rect::from_min_max(pos2(100.0, 300.0), pos2(300.0, 320.0));
        let over = Rect::from_min_max(pos2(560.0, 150.0), pos2(700.0, 170.0));
        let wide = Rect::from_min_max(pos2(0.0, 150.0), pos2(700.0, 170.0));

        let moved = clear_of(over, cube, view);
        let kept_in = clear_of(wide, cube, view);

        assert_eq!(clear_of(clear, cube, view), clear);
        assert!(!moved.intersects(cube));
        assert_eq!(moved.size(), over.size());
        assert_eq!(moved.top(), over.top());
        assert_eq!(kept_in.left(), view.left());
    }

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
    fn entering_a_sketch_faces_it_from_the_side_the_camera_is_on() {
        for (from_below, forward) in [(false, -Vector3::Z), (true, Vector3::Z)] {
            let (document, feature, _, _) = sketched(Plane::XY);
            let model = model_of(document);
            let mut state = state_with_cursor();
            state.build_scene(&model, &SketchEditing::default());
            state.camera = Camera::new(if from_below {
                Viewpoint::facing_from_behind(&Plane::XY, Point3::ZERO, 100.0)
            } else {
                Viewpoint::facing(&Plane::XY, Point3::ZERO, 100.0)
            });

            state.build_scene(&model, &SketchEditing::editing(feature));
            state.advance(Duration::from_secs(1));

            let viewpoint = state.camera.viewpoint();
            assert!(viewpoint.forward().distance(forward) < 1e-9);
            assert!(viewpoint.up().distance(Vector3::Y) < 1e-9);
        }
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

        state.fit_requested = Some(FitAsked::ByUser);
        state
            .selection
            .replace_with(Pickable::Axis(crate::selection::Axis::X));
        state.build_scene(&model, &SketchEditing::default());
        assert!(state.is_animating());
        assert!(state.fit_requested.is_none());
    }

    #[derive(Debug)]
    struct Drawn {
        generation: u64,
        base: Vec<Arc<Batch>>,
        picks: Arc<PickTable>,
        overlay: Option<Arc<Batch>>,
        picked: bool,
    }

    impl Drawn {
        fn same_scene(&self, other: &Self) -> bool {
            self.generation == other.generation
                && self.base.len() == other.base.len()
                && self
                    .base
                    .iter()
                    .zip(&other.base)
                    .all(|(this, that)| Arc::ptr_eq(this, that))
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
                .iter()
                .flat_map(|batch| batch.all_lines())
                .filter(|line| line.pick.is_some() && line.pick == pick)
                .collect()
        }
    }

    fn draw(state: &mut ViewportState, model: &Model, editing: &SketchEditing) -> Drawn {
        let built = state.build_scene(model, editing).unwrap().clone();
        let overlay = state.scenes.overlay_batch().cloned();
        let drawn = Drawn {
            generation: built.generation,
            base: built
                .scene
                .batches
                .iter()
                .filter(|batch| {
                    overlay
                        .as_ref()
                        .is_none_or(|overlay| !Arc::ptr_eq(overlay, batch))
                })
                .cloned()
                .collect(),
            picks: Arc::clone(&built.picks),
            overlay,
            picked: false,
        };
        let picked = state
            .request(true)
            .is_some_and(|request| request.pick_at.is_some());
        Drawn { picked, ..drawn }
    }

    fn pick_ids(built: &BuiltScene) -> Vec<(Pickable, Option<PickId>)> {
        built
            .picks
            .pickables()
            .map(|pickable| (pickable, built.picks.id_of(pickable)))
            .collect()
    }

    fn kept_batches(before: &BuiltScene, after: &BuiltScene) -> usize {
        after
            .scene
            .batches
            .iter()
            .filter(|batch| {
                before
                    .scene
                    .batches
                    .iter()
                    .any(|earlier| Arc::ptr_eq(earlier, batch))
            })
            .count()
    }

    #[test]
    fn each_body_draws_its_own_batch_and_a_hover_or_a_hidden_body_moves_no_other_pick_id() {
        let mut model = model_of(super::timing::copies_document(3));
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        let first = state.build_scene(&model, &editing).unwrap().clone();
        let edge_of = |built: &BuiltScene, wanted: FeatureId| {
            built
                .picks
                .pickables()
                .find(|pickable| matches!(pickable, Pickable::Edge { body, .. } if *body == wanted))
                .unwrap()
        };
        let bodies: Vec<FeatureId> = model
            .document()
            .features()
            .map(|feature| feature.id())
            .collect();
        let face = first
            .picks
            .pickables()
            .find(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == bodies[0]))
            .unwrap();

        state.hovered = Some(face);
        let face_hovered = state.build_scene(&model, &editing).unwrap().clone();
        state.hovered = Some(edge_of(&first, bodies[0]));
        let edge_hovered = state.build_scene(&model, &editing).unwrap().clone();
        state.hovered = None;
        let unhovered = state.build_scene(&model, &editing).unwrap().clone();
        let hidden = visibility::toggle(model.document().feature(bodies[1]).unwrap()).unwrap();
        model.perform(Action::Apply(hidden));
        settle(&mut model);
        let one_hidden = state.build_scene(&model, &editing).unwrap().clone();
        let without_the_hidden = |built: &BuiltScene| {
            pick_ids(built)
                .into_iter()
                .filter(|(pickable, _)| visibility::owner(*pickable) != Some(bodies[1]))
                .collect::<Vec<_>>()
        };

        assert_eq!(first.scene.batches.len(), 4);
        assert_eq!(pick_ids(&face_hovered), pick_ids(&first));
        assert_eq!(pick_ids(&edge_hovered), pick_ids(&first));
        assert_eq!(kept_batches(&first, &face_hovered), 3);
        assert_eq!(kept_batches(&first, &edge_hovered), 2);
        assert_eq!(one_hidden.scene.batches.len(), 3);
        assert_eq!(without_the_hidden(&one_hidden), without_the_hidden(&first));
        assert_eq!(kept_batches(&unhovered, &one_hidden), 2);
    }

    fn two_sketches() -> (Document, [(FeatureId, EntityId); 2]) {
        let (mut document, first, first_line, _) = sketched(Plane::XY);
        let mut sketch = Sketch::new(Plane::XY);
        let second_line = sketch.add_line(Point2::new(60.0, 0.0), Point2::new(80.0, 10.0));
        sketch.add_circle(Point2::new(70.0, 30.0), 5.0);
        let mut transaction = document.transaction("Add sketch");
        let second = transaction.add_feature("Beside", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, [(first, first_line), (second, second_line)])
    }

    fn batch_drawing(built: &BuiltScene, pickable: Pickable) -> Arc<Batch> {
        let pick = built.picks.id_of(pickable);
        built
            .scene
            .batches
            .iter()
            .find(|batch| {
                batch
                    .all_lines()
                    .any(|line| line.pick.is_some() && line.pick == pick)
            })
            .cloned()
            .unwrap()
    }

    fn sketch_pick_ids(built: &BuiltScene, sketch: FeatureId) -> Vec<(Pickable, Option<PickId>)> {
        pick_ids(built)
            .into_iter()
            .filter(|(pickable, _)| {
                matches!(pickable, Pickable::SketchEntity { feature, .. } if *feature == sketch)
            })
            .collect()
    }

    #[test]
    fn each_sketch_draws_its_own_batch_and_a_change_to_one_refacets_and_uploads_only_it() {
        let (document, [(first, first_line), (second, second_line)]) = two_sketches();
        let mut model = model_of(document);
        settle(&mut model);
        let editing = SketchEditing::default();
        let mut state = state_with_cursor();
        let [first_line, second_line] = [(first, first_line), (second, second_line)]
            .map(|(feature, entity)| Pickable::SketchEntity { feature, entity });
        let facetings = |state: &ViewportState| state.scenes.sketch_shapes().facetings();

        let shown = state.build_scene(&model, &editing).unwrap().clone();
        let faceted_when_shown = facetings(&state);
        state.hovered = Some(first_line);
        let hovered = state.build_scene(&model, &editing).unwrap().clone();
        let faceted_when_hovered = facetings(&state);
        state.hovered = None;
        state.selection.replace_with(second_line);
        let selected = state.build_scene(&model, &editing).unwrap().clone();
        let next_id = model
            .document()
            .feature(first)
            .and_then(|feature| feature.kind.sketch())
            .map(Sketch::next_id)
            .unwrap();
        model.perform(Action::Apply(Transaction::single(
            "Add a point",
            Edit::AddSketchEntity {
                feature: first,
                id: EntityId::from_raw(next_id),
                entity: caditor_sketch::Entity::Point(Point2::new(5.0, 5.0)),
                construction: false,
                split_from: None,
            },
        )));
        settle(&mut model);
        let changed = state.build_scene(&model, &editing).unwrap().clone();
        let faceted_when_changed = facetings(&state);
        let editing_first = SketchEditing::editing(first);
        state.selection.clear();
        let entered = state.build_scene(&model, &editing_first).unwrap().clone();
        let faceted_when_entered = facetings(&state);
        state.hovered = Some(first_line);
        let hovered_while_editing = state.build_scene(&model, &editing_first).unwrap().clone();

        assert_eq!(shown.scene.batches.len(), 1 + 2 + 2);
        assert_eq!(faceted_when_shown, 2);
        assert_eq!(faceted_when_hovered, faceted_when_shown);
        assert_eq!(pick_ids(&hovered), pick_ids(&shown));
        assert!(!Arc::ptr_eq(
            &batch_drawing(&hovered, first_line),
            &batch_drawing(&shown, first_line)
        ));
        assert!(Arc::ptr_eq(
            &batch_drawing(&hovered, second_line),
            &batch_drawing(&shown, second_line)
        ));
        assert_eq!(kept_batches(&shown, &hovered), 3);
        assert_eq!(kept_batches(&hovered, &selected), 2);
        assert_eq!(faceted_when_changed, faceted_when_shown + 1);
        assert_eq!(
            sketch_pick_ids(&changed, second),
            sketch_pick_ids(&selected, second)
        );
        assert!(Arc::ptr_eq(
            &batch_drawing(&changed, second_line),
            &batch_drawing(&selected, second_line)
        ));
        assert_eq!(faceted_when_entered, faceted_when_changed);
        assert_eq!(entered.scene.batches.len(), 1 + 2 + 1);
        assert_eq!(kept_batches(&entered, &hovered_while_editing), 2);
        assert!(hovered_while_editing.scene.batches.iter().any(|batch| {
            entered.scene.batches[1..]
                .iter()
                .any(|earlier| Arc::ptr_eq(earlier, batch))
                && batch.line_count() > 0
                && batch.all_lines().all(|line| line.pick.is_none())
        }));
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
        annotated_with(state, context, model, feature, Vec::new());
    }

    fn annotated_with(
        state: &mut ViewportState,
        context: &egui::Context,
        model: &Model,
        feature: FeatureId,
        events: Vec<egui::Event>,
    ) {
        let rect = state.rect.unwrap();
        let input = egui::RawInput {
            screen_rect: Some(rect),
            events,
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

    #[test]
    fn crowded_marks_thin_out_but_selected_and_highlighted_ones_stay() {
        let mut sketch = Sketch::new(Plane::XY);
        let mut lines = Vec::new();
        for index in 0..400 {
            let at = Point2::new(f64::from(index % 20), f64::from(index / 20));
            lines.push(sketch.add_line(at, at + Vector2::new(4.0, 0.2)));
        }
        for [first, second] in lines.as_chunks::<2>().0 {
            sketch
                .add_constraint(caditor_sketch::Constraint::Parallel(*first, *second))
                .unwrap();
        }
        let dimensions: Vec<_> = lines
            .iter()
            .take(100)
            .map(|line| {
                let Some(&Entity::Line { start, end }) = sketch.entity(*line) else {
                    panic!("expected a line");
                };
                sketch
                    .add_constraint(caditor_sketch::Constraint::HorizontalDistance {
                        from: start,
                        to: end,
                        value: caditor_expression::Expression::Measure(
                            4.0,
                            caditor_expression::Unit::Millimetre,
                        ),
                    })
                    .unwrap()
            })
            .collect();
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
            Point3::new(10.0, 10.0, 0.0),
            400.0,
        ));

        annotated(&mut state, &context, &model, feature);
        let glyphs = state.annotations.glyph_centres();
        let labels = state.annotations.laid_out();
        let left_out = *lines
            .iter()
            .find(|line| glyphs.iter().all(|(anchor, _)| anchor != *line))
            .unwrap();
        let unlabelled = *dimensions
            .iter()
            .find(|dimension| !labels.contains(dimension))
            .unwrap();
        let crowding = glyphs
            .iter()
            .enumerate()
            .flat_map(|(index, (_, first))| {
                glyphs
                    .iter()
                    .skip(index + 1)
                    .map(move |(_, second)| (*first - *second).abs())
            })
            .filter(|apart| apart.x < 9.0 && apart.y < 9.0)
            .count();

        assert!(
            !glyphs.is_empty() && glyphs.len() < lines.len() / 4,
            "{}",
            glyphs.len()
        );
        assert!(
            !labels.is_empty() && labels.len() < dimensions.len() / 2,
            "{}",
            labels.len()
        );
        assert_eq!(crowding, 0);

        state.selection.replace_with(Pickable::SketchEntity {
            feature,
            entity: left_out,
        });
        state.keyboard_highlight = Some(Pickable::SketchConstraint {
            feature,
            constraint: unlabelled,
        });
        annotated(&mut state, &context, &model, feature);

        assert!(
            state
                .annotations
                .glyph_centres()
                .iter()
                .any(|(anchor, _)| *anchor == left_out)
        );
        assert!(state.annotations.laid_out().contains(&unlabelled));
    }

    #[test]
    fn dense_open_ends_merge_into_rings_counting_them_that_hover_without_taking_the_pick() {
        let mut sketch = Sketch::new(Plane::XY);
        for index in 0..100 {
            let at = Point2::new(f64::from(index % 10) * 2.0, f64::from(index / 10) * 2.0);
            sketch.add_line(at, at + Vector2::new(1.0, 0.0));
        }
        sketch.add_line(Point2::new(200.0, 0.0), Point2::new(210.0, 0.0));
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
            Point3::new(100.0, 0.0, 0.0),
            600.0,
        ));

        annotated(&mut state, &context, &model, feature);
        let (rings, clusters) = state.annotations.open_end_rings();
        let clustered: usize = clusters.iter().map(|(_, ends)| ends).sum();

        assert!(!clusters.is_empty());
        assert!(clustered >= 150, "{clustered}");
        assert_eq!(rings.iter().filter(|ring| ring.x > 250.0).count(), 2);

        let (centre, _) = clusters[0];
        let marker = state.rect.unwrap().min + vec2(centre.x as f32, centre.y as f32);
        for _ in 0..3 {
            annotated_with(
                &mut state,
                &context,
                &model,
                feature,
                vec![egui::Event::PointerMoved(marker)],
            );
        }

        assert_eq!(state.annotations.hovered(), None);

        state.camera = Camera::new(Viewpoint::facing(
            &Plane::XY,
            Point3::new(10.0, 10.0, 0.0),
            40.0,
        ));
        annotated(&mut state, &context, &model, feature);
        let (rings, clusters) = state.annotations.open_end_rings();

        assert!(clusters.is_empty());
        assert!(rings.len() > 20, "{}", rings.len());
    }

    #[test]
    fn dimensions_a_few_points_across_collapse_to_findable_markers_unless_zero_length_or_chosen() {
        let mut sketch = Sketch::new(Plane::XY);
        let mut distance = |from: Point2, to: Point2| {
            let length = from.distance(to);
            let from = sketch.add_point(from);
            let to = sketch.add_point(to);
            sketch
                .add_constraint(caditor_sketch::Constraint::Distance {
                    from,
                    to,
                    value: caditor_expression::Expression::Measure(
                        length,
                        caditor_expression::Unit::Millimetre,
                    ),
                })
                .unwrap()
        };
        let tiny = distance(Point2::ZERO, Point2::new(0.1, 0.0));
        let long = distance(Point2::new(0.0, 10.0), Point2::new(20.0, 10.0));
        let zero = distance(Point2::new(10.0, -10.0), Point2::new(10.0, -10.0));
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
        let tiny_pickable = Pickable::SketchConstraint {
            feature,
            constraint: tiny,
        };

        annotated(&mut state, &context, &model, feature);
        let collapsed = state.annotations.collapsed();
        let laid_out = state.annotations.laid_out();

        assert_eq!(
            collapsed.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![tiny]
        );
        assert!(laid_out.contains(&long) && laid_out.contains(&zero));
        assert!(!laid_out.contains(&tiny));

        let (_, centre) = collapsed[0];
        let marker = state.rect.unwrap().min + vec2(centre.x as f32, centre.y as f32);
        for _ in 0..3 {
            annotated_with(
                &mut state,
                &context,
                &model,
                feature,
                vec![egui::Event::PointerMoved(marker)],
            );
        }

        assert_eq!(state.annotations.hovered(), Some(tiny_pickable));

        state.selection.replace_with(tiny_pickable);
        annotated(&mut state, &context, &model, feature);

        assert!(state.annotations.laid_out().contains(&tiny));
        assert!(state.annotations.collapsed().is_empty());

        state.selection.replace_with(Pickable::SketchConstraint {
            feature,
            constraint: long,
        });
        state.keyboard_highlight = Some(tiny_pickable);
        annotated(&mut state, &context, &model, feature);

        assert!(state.annotations.laid_out().contains(&tiny));
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

    fn beside_a_small_sketch(mut document: Document) -> (Document, FeatureId) {
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::new(-60.0, 0.0), Point2::new(-30.0, 10.0));
        sketch.add_circle(Point2::new(-50.0, 30.0), 6.0);
        let mut transaction = document.transaction("Add a small sketch");
        let feature = transaction.add_feature("Small sketch", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        (document, feature)
    }

    pub fn plate_document() -> Document {
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
                direction: None,
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

        let (document, small) = beside_a_small_sketch(model.document().clone());
        let model = settled(document);
        let small_entities: Vec<Pickable> = model
            .document()
            .feature(small)
            .and_then(|feature| feature.kind.sketch())
            .map(|sketch| {
                sketch
                    .entities()
                    .map(|(entity, _)| Pickable::SketchEntity {
                        feature: small,
                        entity,
                    })
                    .take(2)
                    .collect()
            })
            .unwrap();
        let hovering_the_small_sketch = |state: &mut ViewportState, frame: u32| {
            hover(state, small_entities[frame as usize % small_entities.len()]);
        };
        let editing = SketchEditing::default();
        let scenario = Scenario {
            model: &model,
            editing: &editing,
        };
        scenario.time(
            "two sketches, the small one's hover changing",
            hovering_the_small_sketch,
        );
        let editing = SketchEditing::editing(small);
        let scenario = Scenario {
            model: &model,
            editing: &editing,
        };
        scenario.time(
            "large sketch behind the edited small one, hover changing",
            hovering_the_small_sketch,
        );

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
        scenario.time("model, whole body hovered, idle", |state, _| {
            state.set_filter(SelectionFilter::Bodies);
            hover(state, faces[0]);
        });
        scenario.time("model, whole body hover toggling", |state, frame| {
            state.set_filter(SelectionFilter::Bodies);
            if frame % 2 == 0 {
                hover(state, faces[0]);
            } else {
                state.hovered = None;
            }
        });
    }

    const ASSEMBLY_FILE: &str = "CADITOR_IMPORT_FILE";
    const COPIES_ACROSS: usize = 40;
    const COPY_SPACING: f64 = 160.0;
    const ASSEMBLY_TIMEOUT: Duration = Duration::from_secs(3600);
    const GPU_FRAMES: u32 = 30;
    const FULL_HD: (u32, u32) = (1920, 1080);
    const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn assembly_document() -> Document {
        let cancel = caditor_document::CancelToken::never();
        let mut document = Document::default();
        let bodies = match std::env::var_os(ASSEMBLY_FILE) {
            Some(path) => {
                let started = Instant::now();
                let imported =
                    caditor_file::read_step_file(std::path::Path::new(&path), &cancel).unwrap();
                eprintln!(
                    "read {} bodies in {:?}",
                    imported.bodies.len(),
                    started.elapsed()
                );
                imported.bodies
            }
            None => copies_of_the_samples(COPIES_ACROSS * COPIES_ACROSS),
        };
        let transaction = caditor_file::bodies_transaction(&document, &bodies, "Import");
        document.apply(transaction).unwrap();
        document
    }

    pub fn copies_document(copies: usize) -> Document {
        let mut document = Document::default();
        let transaction =
            caditor_file::bodies_transaction(&document, &copies_of_the_samples(copies), "Import");
        document.apply(transaction).unwrap();
        document
    }

    fn copies_of_the_samples(copies: usize) -> Vec<caditor_file::ImportedBody> {
        let cancel = caditor_document::CancelToken::never();
        let solids: Vec<caditor_kernel::Solid> = crate::samples::Sample::ALL
            .into_iter()
            .map(|sample| {
                let document = sample.document().unwrap();
                let evaluation = caditor_document::Recompute::default().run_without_display(
                    &document,
                    &caditor_document::ModelEvaluator,
                    &cancel,
                );
                let (body, _) = evaluation.bodies().next().unwrap();
                evaluation
                    .body_result(body)
                    .unwrap()
                    .solid()
                    .unwrap()
                    .solid
                    .clone()
            })
            .collect();
        (0..copies)
            .map(|index| {
                let solid = &solids[index % solids.len()];
                let at = Vector3::new(
                    (index % COPIES_ACROSS) as f64 * COPY_SPACING,
                    (index / COPIES_ACROSS) as f64 * COPY_SPACING,
                    0.0,
                );
                let placed = solid
                    .transformed(&caditor_geometry::RigidTransform::translation(at).unwrap())
                    .unwrap();
                caditor_file::ImportedBody {
                    name: format!("Copy {index}"),
                    import: caditor_document::Import::new("copy", placed, ""),
                    colour: None,
                    opacity: None,
                    finish: None,
                    group: None,
                    faces: Vec::new(),
                }
            })
            .collect()
    }

    fn settled_assembly() -> Model {
        let started = Instant::now();
        let deadline = started + ASSEMBLY_TIMEOUT;
        let mut model = model_of(assembly_document());
        while matches!(model.status(), RecomputeStatus::Running { .. }) || model.bodies_pending() {
            assert!(Instant::now() < deadline, "the recompute did not finish");
            model.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        eprintln!("evaluated and meshed in {:?}", started.elapsed());
        model
    }

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        target: wgpu::TextureView,
    }

    impl Gpu {
        fn open() -> Option<Self> {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..wgpu::RequestAdapterOptions::default()
                }))
                .ok()?;
            eprintln!("drawing on {}", adapter.get_info().name);
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    required_limits: adapter.limits(),
                    ..wgpu::DeviceDescriptor::default()
                }))
                .ok()?;
            let target = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("timing target"),
                    size: wgpu::Extent3d {
                        width: FULL_HD.0,
                        height: FULL_HD.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TARGET_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default());
            Some(Self {
                device,
                queue,
                target,
            })
        }

        fn renderer(&self) -> caditor_render::ViewportRenderer {
            caditor_render::ViewportRenderer::new(&self.device, TARGET_FORMAT, 4)
        }

        fn draw(
            &self,
            renderer: &mut caditor_render::ViewportRenderer,
            request: &ViewportRequest,
            scene: &Scene,
        ) -> (Duration, Duration) {
            let started = Instant::now();
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            renderer.draw(
                &self.device,
                &self.queue,
                &mut encoder,
                &caditor_render::SurfaceTarget {
                    view: &self.target,
                    width: FULL_HD.0,
                    height: FULL_HD.1,
                },
                Some(&caditor_render::ViewportFrame {
                    rect: request.rect,
                    view: &request.view,
                    scene,
                    pick_at: request.pick_at,
                    pixels_per_point: request.pixels_per_point,
                }),
            );
            let submission = self.queue.submit([encoder.finish()]);
            let wake: caditor_render::Wake = Arc::new(|| {});
            renderer.after_submit(&self.device, &wake, submission);
            let submitted = Instant::now();
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let waited = submitted.elapsed();
            if request.pick_at.is_some() {
                while renderer.picking().poll(&self.device) == caditor_render::PickPoll::Pending {
                    std::thread::yield_now();
                }
            }
            (submitted - started, waited)
        }
    }

    fn full_hd_state() -> ViewportState {
        let mut state = ViewportState::new();
        state.rect = Some(Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(FULL_HD.0 as f32, FULL_HD.1 as f32),
        ));
        state.cursor = Some(Vector2::new(960.0, 540.0));
        state
    }

    struct Timed {
        built: Duration,
        drawn: Duration,
        waited: Duration,
    }

    impl Timed {
        fn report(name: &str, frames: &[Self]) {
            let count = frames.len().max(1) as u32;
            let sum = |part: fn(&Self) -> Duration| frames.iter().map(part).sum::<Duration>();
            let worst = frames
                .iter()
                .map(|frame| frame.built + frame.drawn + frame.waited)
                .max()
                .unwrap_or_default();
            eprintln!(
                "{name:<40} scene {:>10.3?}, drawing on the UI thread {:>10.3?}, GPU {:>10.3?}, worst frame {worst:.3?}",
                sum(|frame| frame.built) / count,
                sum(|frame| frame.drawn) / count,
                sum(|frame| frame.waited) / count,
            );
        }
    }

    struct Drawing<'a> {
        gpu: &'a Gpu,
        model: &'a Model,
    }

    impl Drawing<'_> {
        fn frames(
            &self,
            name: &str,
            scene: Option<fn(&Scene) -> Scene>,
            mut change: impl FnMut(&mut ViewportState, u32),
        ) {
            if !is_benched(name) {
                return;
            }
            let editing = SketchEditing::default();
            let mut state = full_hd_state();
            let mut renderer = self.gpu.renderer();
            let mut timed = Vec::new();
            let mut warming = 0;
            let mut frame = 0;
            while timed.len() < GPU_FRAMES as usize {
                change(&mut state, frame);
                frame += 1;
                let started = Instant::now();
                state.build_scene(self.model, &editing);
                let request = state.request(true).unwrap();
                let built_scene = state.scene().unwrap();
                let left_out = scene.map(|leave_out| leave_out(built_scene));
                let shown = left_out.as_ref().unwrap_or(built_scene);
                let built = started.elapsed();
                let (drawn, waited) = self.gpu.draw(&mut renderer, &request, shown);
                if renderer.is_uploading() || warming < WARM_UP {
                    warming += u32::from(!renderer.is_uploading());
                    continue;
                }
                timed.push(Timed {
                    built,
                    drawn,
                    waited,
                });
            }
            Timed::report(name, &timed);
        }
    }

    fn segments_by_size_on_screen(view: &View, scene: &Scene) {
        let sizes = [8.0, 32.0, 128.0, 512.0, f64::INFINITY];
        let mut segments = [0usize; 5];
        let mut batches = [0usize; 5];
        for batch in &scene.batches {
            let shown: Vec<caditor_render::Line> = batch
                .all_lines()
                .filter(|line| line.color.alpha > 0.0)
                .collect();
            let Some(bounds) =
                Aabb::from_points(shown.iter().flat_map(|line| [line.start, line.end]))
            else {
                continue;
            };
            let depth = view.view_depth(bounds.center()).max(view.near_plane());
            let pixels = bounds.max().distance(bounds.min()) / view.units_per_pixel_at(depth);
            let bucket = sizes.iter().position(|size| pixels < *size).unwrap_or(4);
            segments[bucket] += shown.len();
            batches[bucket] += 1;
        }
        eprintln!(
            "shown edge segments of batches under 8, 32, 128, 512 px and larger: {segments:?} in {batches:?} batches"
        );
    }

    fn is_benched(name: &str) -> bool {
        std::env::var("CADITOR_BENCH_CASE")
            .ok()
            .is_none_or(|only| name.contains(&only))
    }

    fn turned_a_little(state: &mut ViewportState, frame: u32) {
        let step = if frame.is_multiple_of(2) { 2.0 } else { -2.0 };
        let target = state.camera.viewpoint().target;
        state.camera.orbit(target, Vector2::new(step, 0.0), 1000.0);
    }

    #[test]
    #[ignore = "a timing benchmark: CADITOR_IMPORT_FILE=model.step cargo test --release -p caditor frame_costs_on_a_large_assembly -- --ignored --nocapture"]
    fn frame_costs_on_a_large_assembly() {
        let model = settled_assembly();
        let editing = SketchEditing::default();
        let mut state = full_hd_state();
        let started = Instant::now();
        let built = state.build_scene(&model, &editing).unwrap().clone();
        let scene = &built.scene;
        eprintln!(
            "first scene in {:?}: {} meshes of {} faces and {} triangles, {} silhouettes, {} batches, {} line segments, {} markers",
            started.elapsed(),
            scene.meshes.len(),
            scene
                .meshes
                .iter()
                .map(|instance| instance.faces.len())
                .sum::<usize>(),
            scene
                .meshes
                .iter()
                .map(|instance| instance.mesh.triangle_count())
                .sum::<usize>(),
            scene.silhouettes.len(),
            scene.batches.len(),
            scene.lines().count(),
            scene.markers().count(),
        );
        if let Some(view) = state.view() {
            segments_by_size_on_screen(&view, &built.scene);
        }
        let faces: Vec<Pickable> = {
            let mut seen = std::collections::BTreeSet::new();
            built
                .picks
                .pickables()
                .filter(|pickable| match pickable {
                    Pickable::Face { body, .. } => seen.insert(*body),
                    _ => false,
                })
                .step_by(97)
                .take(16)
                .collect()
        };
        let hovering = |state: &mut ViewportState, frame: u32| {
            hover(state, faces[frame as usize % faces.len()]);
        };

        let scenario = Scenario {
            model: &model,
            editing: &editing,
        };
        if is_benched("assembly, scene") {
            scenario.time("assembly, scene idle", |_, _| {});
            scenario.time("assembly, scene with the hover changing", hovering);
        }

        let Some(gpu) = Gpu::open() else {
            eprintln!("no graphics adapter, so no frame is drawn");
            return;
        };
        let drawing = Drawing {
            gpu: &gpu,
            model: &model,
        };
        drawing.frames("still, turning a little", None, turned_a_little);
        drawing.frames(
            "still, turning, no edges or vertices",
            Some(|scene| Scene {
                batches: Vec::new(),
                ..scene.clone()
            }),
            turned_a_little,
        );
        drawing.frames(
            "still, turning, no silhouettes",
            Some(|scene| Scene {
                silhouettes: Vec::new(),
                ..scene.clone()
            }),
            turned_a_little,
        );
        drawing.frames(
            "still, turning, faces only",
            Some(|scene| Scene {
                silhouettes: Vec::new(),
                batches: Vec::new(),
                ..scene.clone()
            }),
            turned_a_little,
        );
        drawing.frames("hover changing, with a pick", None, hovering);
        drawing.frames(
            "whole body hover changing, with a pick",
            None,
            |state, frame| {
                state.set_filter(SelectionFilter::Bodies);
                hovering(state, frame);
            },
        );
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
        assert_eq!(
            motion(InputMode::SolidWorks, MIDDLE, false, false),
            Some(DragMotion::Orbit)
        );
        assert_eq!(
            motion(InputMode::SolidWorks, MIDDLE, false, true),
            Some(DragMotion::Pan)
        );
        assert_eq!(
            motion(InputMode::SolidWorks, MIDDLE, true, false),
            Some(DragMotion::Zoom)
        );
        assert_eq!(
            motion(InputMode::SolidWorks, RIGHT, true, false),
            Some(DragMotion::Pan)
        );
    }

    #[test]
    fn inverting_the_zoom_turns_the_wheel_and_a_zoom_drag_round_alike() {
        let plain = Navigation::default();
        let inverted = Navigation {
            invert_zoom: true,
            ..plain
        };
        let wheel_up = 40.0;
        let drag_up = 40.0;

        assert!(zoom_factor(&plain, wheel_up) < 1.0);
        assert!(zoom_factor(&plain, drag_up) < 1.0);
        assert!(zoom_factor(&inverted, wheel_up) > 1.0);
        assert!(zoom_factor(&inverted, drag_up) > 1.0);
        assert!(
            (zoom_factor(&plain, wheel_up) * zoom_factor(&inverted, wheel_up) - 1.0).abs() < 1e-12
        );
    }

    #[test]
    fn inverting_the_zoom_turns_a_pinch_and_a_control_wheel_round_too() {
        let plain = Navigation::default();
        let inverted = Navigation {
            invert_zoom: true,
            ..plain
        };
        let spreading = 1.1;

        assert!(pinch_factor(&plain, spreading) < 1.0);
        assert!(pinch_factor(&inverted, spreading) > 1.0);
        assert!(
            (pinch_factor(&plain, spreading) * pinch_factor(&inverted, spreading) - 1.0).abs()
                < 1e-12
        );
        assert_eq!(pinch_factor(&plain, 1.0), 1.0);
        assert_eq!(pinch_factor(&inverted, 1.0), 1.0);
    }
}

fn scene_items(ui: &mut Ui, view: Id, corner: Pos2, items: &[Item]) {
    let nowhere = Rect::from_min_size(corner, Vec2::ZERO);
    let node = |ui: &mut Ui, id: Id, name: &str| {
        let response = ui.interact(nowhere, id, Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Label, true, name));
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
