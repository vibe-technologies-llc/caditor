use std::{
    f64::consts::{FRAC_PI_2, PI},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{
    BodyOperation, Document, Edit, Editor, ExtrudeExtent, Feature, FeatureId, FeatureKind,
    RegionChoice, RollbackBar, SolidFeature, SolidResult, Transaction,
};
use caditor_expression::{Expression, ParameterId, Unit};
use caditor_file::{DrawingUnit, ExportFormat, JournalEntry, Start, Storage, StorageConfig};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::MeshQuality;
use caditor_render::{Background, GraphicsInfo, Image, ImageError, Msaa, Shading};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{
    Color32, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape,
    ViewportCommand, ViewportId, ViewportIdMap, ViewportInfo,
    accesskit::{Node, NodeId, Role},
    epaint::ClippedShape,
    viewport::ResizeDirection,
};
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::{
    about, annotations,
    app::{self, Workspace},
    appearance, canvas,
    commands::{Command, Offer, RecentSlot},
    display_style::DisplayStyle,
    drawing::Refusal,
    editing::{EditingCommand, Tool},
    export::ExportCommand,
    files::{Dialogs, FileCommand, Files, FilesConfig, Respond},
    filleting,
    graphics::{CurveQuality, FrameLimit, Graphics, Hardware},
    history::HistoryCommand,
    icons,
    image_export::{ImageCommand, ReadPixels},
    import::{self, Placement},
    import_options::{Arrangement, ImportOptionsCommand, PlaneChoice},
    logo, menu_bar, mirror_panel, mirroring,
    model::{Action, Model, Notice, RecomputeStatus, Services, WakerFactory},
    offsetting,
    onboarding::Hint,
    palette::{Choice, State},
    panels::{Focus, REVEAL_FRAMES},
    preferences::{
        InputMode, PreferenceChange, Preferences, PreferencesCommand, PreferencesTab, TitleBar,
    },
    scene,
    selection::{Axis, Pickable, PrincipalPlane, SelectionFilter},
    shape_modes::{CircleMode, RectangleMode, ShapeMode},
    sketch_toolbar,
    sketch_tools::{self, ConstraintTool},
    solid_panel, split_panel, status_bar, toolbar, trimming, typed_point,
    units::LengthUnit,
    view_cube, widgets, window_frame,
};

mod feature_panels;
mod screenshots;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1400.0, 1000.0));
const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_SECONDS: f64 = 0.05;
const ANIMATION_FRAMES: usize = 5;
const STILL_FRAMES: usize = 60;
const WINDOW_SETTLE_FRAMES: usize = 5;
const FILE_TIMEOUT: Duration = Duration::from_secs(10);
const TOOLTIP_FRAMES: usize = 20;
const CAMERA_SETTLE: Duration = Duration::from_secs(5);
const DRAWN: f64 = 1e-3;

fn canonical(dir: &TempDir) -> PathBuf {
    dunce::canonicalize(dir.path()).unwrap()
}

#[derive(Clone, Default)]
struct ScriptedDialogs {
    answer: Arc<Mutex<Option<PathBuf>>>,
    #[cfg(unix)]
    no_portal: Arc<Mutex<bool>>,
    held: Arc<Mutex<Option<Vec<Respond>>>>,
}

impl ScriptedDialogs {
    fn reply(&self) -> Result<Option<PathBuf>, crate::portal::DialogError> {
        #[cfg(unix)]
        if *self.no_portal.lock() {
            return Err(crate::portal::DialogError::NoSessionBus(
                zbus::Error::Unsupported,
            ));
        }
        Ok(self.answer.lock().clone())
    }
}

impl Dialogs for ScriptedDialogs {
    fn pick_model(&self, _directory: Option<PathBuf>, respond: Respond) {
        respond(self.reply());
    }

    fn pick_save_path(&self, _directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        respond(self.reply());
    }

    fn pick_export_path(
        &self,
        _directory: Option<PathBuf>,
        _file_name: String,
        _format: ExportFormat,
        respond: Respond,
    ) {
        respond(self.reply());
    }

    fn pick_import(&self, _directory: Option<PathBuf>, respond: Respond) {
        if let Some(held) = self.held.lock().as_mut() {
            held.push(respond);
            return;
        }
        respond(self.reply());
    }

    fn pick_drawing_path(
        &self,
        _title: &str,
        _directory: Option<PathBuf>,
        _file_name: String,
        respond: Respond,
    ) {
        respond(self.reply());
    }

    fn pick_image_path(&self, _directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        respond(self.reply());
    }
}

fn no_wake() -> WakerFactory {
    Box::new(|| Box::new(|| {}))
}

struct Harness {
    context: egui::Context,
    model: Model,
    files: Files,
    dialogs: ScriptedDialogs,
    workspace: Workspace,
    events: Vec<Event>,
    texts: Vec<(String, Rect)>,
    text_clips: Vec<Rect>,
    text_colors: Vec<(String, Color32)>,
    time: f64,
    forced_hover: Option<(Pos2, Pickable)>,
    held: Modifiers,
    picks_held: bool,
    accessible: Vec<(NodeId, Node)>,
    window: ViewportInfo,
    window_commands: Vec<ViewportCommand>,
    image_failure: Option<ImageError>,
    textures: crate::overlay::TextureMirror,
    painted: Option<Painted>,
    hovered_files: Vec<egui::HoveredFile>,
}

struct Painted {
    shapes: Vec<ClippedShape>,
    pixels_per_point: f32,
}

impl Harness {
    fn new() -> Self {
        Self::with_directories(None)
    }

    fn with_directories(dir: Option<&Path>) -> Self {
        Self::starting(dir, sample_document().unwrap(), Workspace::new())
    }

    fn first_run(dir: &Path) -> Self {
        let preferences = Preferences::from_settings(caditor_file::Settings::default());
        Self::starting(
            Some(dir),
            Document::default(),
            Workspace::with_preferences(preferences),
        )
    }

    fn starting(dir: Option<&Path>, document: Document, workspace: Workspace) -> Self {
        let recovery_dir = dir.map(|dir| dir.join("recovery"));
        let dialogs = ScriptedDialogs::default();
        let services = Services {
            make_waker: no_wake(),
            storage: StorageConfig {
                recovery_dir: recovery_dir.clone(),
                ..StorageConfig::default()
            },
            panic_flush: Arc::default(),
        };
        let config = FilesConfig {
            state_dir: dir.map(|dir| dir.join("state")),
            recovery_dir,
            config_dir: dir.map(|dir| dir.join("config")),
        };
        let mut model = Model::new(document, services);
        let mut files = Files::new(config, Box::new(dialogs.clone()), no_wake());
        files.start(None, &mut model);
        let mut harness = Self {
            context: egui::Context::default(),
            model,
            files,
            dialogs,
            workspace,
            events: Vec::new(),
            texts: Vec::new(),
            text_clips: Vec::new(),
            text_colors: Vec::new(),
            time: 0.0,
            forced_hover: None,
            held: Modifiers::NONE,
            picks_held: false,
            accessible: Vec::new(),
            window: ViewportInfo::default(),
            window_commands: Vec::new(),
            image_failure: None,
            textures: crate::overlay::TextureMirror::default(),
            painted: None,
            hovered_files: Vec::new(),
        };
        harness.settle();
        harness
    }

    fn document(&self) -> &Document {
        self.model.document()
    }

    fn parameter(&self, name: &str) -> ParameterId {
        self.document().parameter_named(name).unwrap().id()
    }

    fn expression_text(&self, name: &str) -> String {
        let document = self.document();
        document.expression_text(&document.parameter_named(name).unwrap().expression)
    }

    fn frame(&mut self) {
        self.time += FRAME_SECONDS;
        if let Some((forced, _)) = self.forced_hover
            && self
                .events
                .iter()
                .any(|event| matches!(event, Event::PointerMoved(position) if *position != forced))
        {
            self.forced_hover = None;
        }
        let mut viewports = ViewportIdMap::default();
        viewports.insert(ViewportId::ROOT, self.window.clone());
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                SCREEN.size() / self.context.zoom_factor(),
            )),
            time: Some(self.time),
            events: std::mem::take(&mut self.events),
            hovered_files: self.hovered_files.clone(),
            viewports,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        self.model.poll();
        self.files
            .poll(&mut self.model, &mut self.workspace.editing);
        self.workspace.editing.sync(&self.model);
        let Self {
            context,
            model,
            files,
            workspace,
            ..
        } = self;
        let mut output = context.run_ui(input, |ui| {
            app::show(ui, model, files, workspace, &mut actions);
        });
        for (id, deltas) in std::mem::take(&mut output.textures_delta.set) {
            for delta in deltas {
                self.textures.apply(id, &delta);
            }
        }
        for id in std::mem::take(&mut output.textures_delta.free) {
            self.textures.free(id);
        }
        if self.painted.is_some() {
            self.painted = Some(Painted {
                shapes: output.shapes.clone(),
                pixels_per_point: output.pixels_per_point,
            });
        }
        if let Some(root) = output.viewport_output.get_mut(&ViewportId::ROOT) {
            self.window_commands.append(&mut root.commands);
        }
        if let Some(update) = output.platform_output.accesskit_update.take() {
            self.accessible = update.nodes;
        }
        app::perform(
            actions,
            &mut self.model,
            &mut self.files,
            &mut self.workspace,
        );
        self.render_image();
        self.model
            .mesh_before(self.workspace.editing.context().solid);
        self.workspace
            .viewport
            .build_scene(&self.model, &self.workspace.editing);
        self.answer_pick();
        self.texts.clear();
        self.text_clips.clear();
        self.text_colors.clear();
        for clipped in output.shapes {
            let ClippedShape { shape, clip_rect } = clipped;
            collect_texts(shape, &mut self.texts, &mut self.text_colors);
            self.text_clips.resize(self.texts.len(), clip_rect);
        }
    }

    fn render_image(&mut self) {
        let Some(job) = self.files.image_job() else {
            return;
        };
        let pixels: Result<ReadPixels, ImageError> = match self.image_failure.take() {
            Some(error) => Err(error),
            None => {
                let texel = match job.background {
                    Background::Viewport => [27, 28, 31, 255],
                    Background::Transparent => [0; 4],
                };
                let (width, height) = (job.size.width, job.size.height);
                let pixels = texel.repeat(width as usize * height as usize);
                Ok(Box::new(move || {
                    Ok(Image {
                        width,
                        height,
                        pixels,
                    })
                }))
            }
        };
        self.files.image_rendered(pixels, &mut self.model);
    }

    fn answer_pick(&mut self) {
        if self.picks_held {
            return;
        }
        let Some(cursor) = self
            .workspace
            .viewport
            .request(true)
            .and_then(|request| request.pick_at)
        else {
            return;
        };
        let picks = self.built().picks;
        let hits = self
            .forced_hover
            .and_then(|(_, pickable)| picks.id_of(pickable))
            .map(|id| caditor_render::PickHit {
                id,
                offset_points: 0.0,
                position: caditor_geometry::Point3::ZERO,
            })
            .into_iter()
            .collect();
        self.workspace
            .viewport
            .apply_pick(&caditor_render::PickResult { cursor, hits });
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + RECOMPUTE_TIMEOUT;
        while matches!(self.model.status(), RecomputeStatus::Running { .. })
            || self.model.bodies_pending()
        {
            assert!(Instant::now() < deadline, "the recompute did not finish");
            self.model.poll();
            std::thread::yield_now();
        }
        self.frame();
        self.frame();
    }

    fn wait_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + FILE_TIMEOUT;
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(2));
            self.frame();
        }
        self.frame();
    }

    fn wait_for_import_options(&mut self, file: &str) {
        let title = format!("Import “{file}”");
        self.wait_until("the import options are shown", |harness| {
            harness.shows(&title)
        });
    }

    fn confirm_import(&mut self, file: &str) {
        self.wait_for_import_options(file);
        self.click("Import");
    }

    fn command(&mut self, command: FileCommand) {
        self.perform(Action::File(command));
    }

    fn perform(&mut self, action: Action) {
        app::perform(
            vec![action],
            &mut self.model,
            &mut self.files,
            &mut self.workspace,
        );
        self.show_new_windows();
    }

    fn hold(&mut self, modifiers: Modifiers) {
        self.held = modifiers;
        self.events.push(Event::ModifiersChanged(modifiers));
        self.frame();
    }

    fn show_new_windows(&mut self) {
        for _ in 0..WINDOW_SETTLE_FRAMES {
            self.frame();
        }
    }

    fn answer_dialog(&self, path: Option<PathBuf>) {
        *self.dialogs.answer.lock() = path;
    }

    fn edit_width(&mut self, text: &str) {
        let width = self.parameter("width");
        self.type_into(Focus::ParameterValue(width), text);
        assert_eq!(self.expression_text("width"), text);
    }

    fn select(&mut self, items: impl IntoIterator<Item = Pickable>) {
        let selection = self.workspace.viewport.selection_mut();
        selection.clear();
        for item in items {
            selection.toggle(item);
        }
        self.frame();
    }

    fn editing(&self) -> Option<FeatureId> {
        self.workspace.editing.feature()
    }

    fn sketch(&self, feature: FeatureId) -> &Sketch {
        self.document()
            .feature(feature)
            .and_then(|feature| feature.kind.sketch())
            .unwrap()
    }

    fn shown(&self, feature: FeatureId) -> Sketch {
        let feature = self.document().feature(feature).unwrap();
        Sketch::clone(&self.model.displayed_sketch(feature).unwrap())
    }

    fn add_sketch(&mut self, sketch: Sketch) -> FeatureId {
        let taken = self
            .document()
            .features()
            .any(|feature| feature.name == "Plate");
        let name = if taken {
            crate::editing::next_feature_name(self.document(), "Plate")
        } else {
            "Plate".to_owned()
        };
        let mut transaction = self.document().transaction("Add sketch");
        let feature = transaction.add_feature(name, FeatureKind::from(sketch));
        self.perform(Action::Apply(transaction.finish()));
        self.settle();
        feature
    }

    fn edit(&mut self, feature: FeatureId) {
        self.perform(Action::Editing(EditingCommand::Enter(feature)));
        self.settle();
        assert_eq!(self.editing(), Some(feature));
    }

    fn hover(&mut self, label: &str) {
        let (_, rect) = self
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("'{label}' is not on screen"))
            .clone();
        self.events.push(Event::PointerMoved(rect.center()));
        for _ in 0..TOOLTIP_FRAMES {
            self.frame();
        }
    }

    fn button_rect(&mut self, name: &str) -> Rect {
        if self.accessible.is_empty() {
            self.context.enable_accesskit();
            self.frame();
            self.frame();
        }
        let bounds = self
            .accessible
            .iter()
            .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
            .find_map(|(_, node)| node.bounds())
            .unwrap_or_else(|| panic!("no button named '{name}'"));
        Rect::from_min_max(
            Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
            Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
        )
    }

    fn click_button(&mut self, name: &str) {
        let position = self.button_rect(name).center();
        self.click_screen(position);
        self.show_new_windows();
    }

    fn hover_button(&mut self, name: &str) {
        let position = self.button_rect(name).center();
        self.events.push(Event::PointerMoved(position));
        for _ in 0..TOOLTIP_FRAMES {
            self.frame();
        }
    }

    fn shows(&self, text: &str) -> bool {
        self.texts.iter().any(|(shown, _)| shown == text)
    }

    fn shows_hint(&self, hint: &str) -> bool {
        canvas::hint_texts(hint)
            .into_iter()
            .all(|text| self.shows(text))
    }

    fn shows_containing(&self, text: &str) -> bool {
        self.texts.iter().any(|(shown, _)| shown.contains(text))
    }

    fn add_stored_constraint(&mut self, feature: FeatureId, constraint: Constraint) {
        let mut transaction = self.document().transaction("Add constraint");
        transaction.add_sketch_constraint(feature, constraint);
        self.perform(Action::Apply(transaction.finish()));
        self.settle();
    }

    fn count_shown(&self, text: &str) -> usize {
        self.texts.iter().filter(|(shown, _)| shown == text).count()
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
    }

    fn type_text(&mut self, text: &str) {
        self.events.push(Event::Text(text.to_owned()));
        self.frame();
        self.frame();
    }

    fn replace_text(&mut self, text: &str) {
        self.key(Key::A, Modifiers::COMMAND);
        self.type_text(text);
    }

    fn focus(&mut self, focus: Focus) {
        self.workspace.panels.request_focus(focus);
        for _ in 0..10 {
            self.frame();
            if self.focused() == Some(focus.field_id()) {
                self.let_animations_finish();
                return;
            }
        }
        panic!("{focus:?} never received focus");
    }

    fn hold_still(&mut self) {
        for _ in 0..STILL_FRAMES {
            let before = self.texts.clone();
            self.frame();
            if self.texts == before {
                return;
            }
        }
        panic!("the interface kept moving");
    }

    fn let_animations_finish(&mut self) {
        for _ in 0..ANIMATION_FRAMES {
            self.frame();
        }
    }

    fn focused(&self) -> Option<Id> {
        self.context.memory(|memory| memory.focused())
    }

    fn type_into(&mut self, focus: Focus, text: &str) {
        self.focus(focus);
        self.key(Key::A, Modifiers::COMMAND);
        self.events.push(Event::Text(text.to_owned()));
        self.frame();
        self.key(Key::Enter, Modifiers::NONE);
        self.frame();
    }

    fn draw_on_new_sketch(&mut self) -> FeatureId {
        self.perform(Action::Editing(EditingCommand::NewSketch(Some(
            PrincipalPlane::Xy,
        ))));
        self.settle();
        self.workspace.viewport.advance(CAMERA_SETTLE);
        self.frame();
        self.editing().expect("the new sketch is being edited")
    }

    fn tool(&self) -> Option<Tool> {
        self.workspace.editing.active().map(|active| active.tool)
    }

    fn use_tool(&mut self, key: Key) {
        self.use_tool_with(key, Modifiers::NONE);
    }

    fn use_tool_with(&mut self, key: Key, modifiers: Modifiers) {
        self.key(key, modifiers);
        self.frame();
        self.frame();
    }

    fn on_screen(&self, point: Point2) -> Pos2 {
        let feature = self.editing().expect("a sketch is being edited");
        let plane = scene::sketch_plane(self.document(), self.model.evaluation(), feature)
            .expect("the sketch has a plane");
        self.workspace
            .viewport
            .screen_position(plane, point)
            .expect("the point is in view")
    }

    fn point_at(&mut self, point: Point2) {
        let position = self.on_screen(point);
        self.events.push(Event::PointerMoved(position));
        self.frame();
    }

    fn click_at(&mut self, point: Point2) {
        self.point_at(point);
        let position = self.on_screen(point);
        for pressed in [true, false] {
            self.events.push(Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.frame();
        }
        self.frame();
    }

    fn click(&mut self, label: &str) {
        let position = self.position_of(label);
        self.click_screen(position);
        self.show_new_windows();
    }

    fn click_leftmost(&mut self, label: &str) {
        let position = self
            .texts
            .iter()
            .filter(|(shown, _)| shown == label)
            .map(|(_, rect)| rect.center())
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap_or_else(|| panic!("'{label}' is not on screen"));
        self.click_screen(position);
        self.show_new_windows();
    }

    fn click_beside(&mut self, glyph: &str, label: &str) {
        let row = self
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("'{label}' is not on screen"))
            .1;
        let position = self
            .texts
            .iter()
            .filter(|(shown, _)| shown == glyph)
            .map(|(_, rect)| rect.center())
            .filter(|center| center.y >= row.min.y)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or_else(|| panic!("no {glyph:?} beside '{label}'"));
        self.click_screen(position);
        self.show_new_windows();
    }

    fn click_lowest(&mut self, label: &str) {
        let position = self
            .texts
            .iter()
            .filter(|(shown, _)| shown == label)
            .map(|(_, rect)| rect.center())
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or_else(|| panic!("'{label}' is not on screen"));
        self.click_screen(position);
        self.show_new_windows();
    }

    fn position_of(&self, label: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("'{label}' is not on screen"))
            .1
            .center()
    }

    fn click_screen(&mut self, position: Pos2) {
        self.events.push(Event::PointerMoved(position));
        self.frame();
        self.press(position);
    }

    fn press(&mut self, position: Pos2) {
        for pressed in [true, false] {
            self.events.push(Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.frame();
        }
    }

    fn double_click(&mut self, label: &str) {
        let position = self.position_of(label);
        self.click_screen(position);
        self.press(position);
        self.frame();
    }

    fn color_of(&self, label: &str) -> Color32 {
        self.text_colors
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("'{label}' is not on screen"))
            .1
    }

    fn built_with_meshes(&mut self, count: usize) -> scene::BuiltScene {
        let deadline = Instant::now() + FILE_TIMEOUT;
        loop {
            let built = self.built();
            if built.scene.meshes.len() + built.scene.translucent_meshes.len() == count {
                return built;
            }
            assert!(
                Instant::now() < deadline,
                "the scene never held {count} meshes"
            );
            std::thread::sleep(Duration::from_millis(2));
            self.frame();
        }
    }

    fn built_with_overlays(&mut self, count: usize) -> scene::BuiltScene {
        let deadline = Instant::now() + FILE_TIMEOUT;
        loop {
            let built = self.built();
            if built.scene.overlay_meshes.len() == count {
                return built;
            }
            assert!(
                Instant::now() < deadline,
                "the scene never held {count} overlays"
            );
            std::thread::sleep(Duration::from_millis(2));
            self.frame();
        }
    }

    fn built(&mut self) -> scene::BuiltScene {
        self.workspace
            .viewport
            .build_scene(&self.model, &self.workspace.editing)
            .cloned()
            .expect("the viewport has a scene")
    }

    fn type_into_field(&mut self, id: Id, text: &str) {
        self.context.memory_mut(|memory| memory.request_focus(id));
        self.frame();
        self.key(Key::A, Modifiers::COMMAND);
        self.events.push(Event::Text(text.to_owned()));
        self.frame();
        self.key(Key::Enter, Modifiers::NONE);
        self.frame();
    }

    fn click_pickable(&mut self, plane: Plane, point: Point2, pickable: Pickable) {
        let position = self.hover_pickable(plane, point, pickable);
        self.press(position);
        self.frame();
    }

    fn hover_pickable(&mut self, plane: Plane, point: Point2, pickable: Pickable) -> Pos2 {
        let position = self
            .workspace
            .viewport
            .screen_position(plane, point)
            .expect("the point is in view");
        self.events.push(Event::PointerMoved(position));
        self.frame();
        self.built();
        self.forced_hover = Some((position, pickable));
        self.workspace.viewport.hover_through_pick(pickable);
        position
    }

    fn solid(&self, feature: FeatureId) -> &SolidFeature {
        self.document()
            .feature(feature)
            .and_then(|feature| feature.kind.solid())
            .unwrap()
    }

    fn body_volume(&self, body: FeatureId) -> f64 {
        self.model
            .evaluation()
            .body_result(body)
            .and_then(|result| result.solid())
            .and_then(SolidResult::mesh)
            .expect("the body is meshed")
            .mass_properties()
            .volume
    }

    fn error_color(&self) -> Color32 {
        self.context.global_style().visuals.error_fg_color
    }

    fn accessible_named(&self, role: Role, name: &str) -> bool {
        self.accessible
            .iter()
            .any(|(_, node)| node.role() == role && node.label() == Some(name))
    }

    fn view_description(&self) -> Option<String> {
        self.accessible
            .iter()
            .find(|(_, node)| node.label() == Some("3D view"))
            .and_then(|(_, node)| node.description().map(str::to_owned))
    }

    fn describes(&self, text: &str) -> bool {
        self.accessible
            .iter()
            .any(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
    }

    fn unreadable_nodes(&self) -> Vec<String> {
        let private_use = |text: &str| {
            text.chars()
                .any(|character| ('\u{E000}'..='\u{F8FF}').contains(&character))
        };
        let mut hidden: Vec<NodeId> = self
            .accessible
            .iter()
            .filter(|(_, node)| node.is_hidden())
            .map(|(id, _)| *id)
            .collect();
        let mut index = 0;
        while let Some(parent) = hidden.get(index).copied() {
            if let Some((_, node)) = self.accessible.iter().find(|(id, _)| *id == parent) {
                hidden.extend_from_slice(node.children());
            }
            index += 1;
        }
        self.accessible
            .iter()
            .filter(|(id, _)| !hidden.contains(id))
            .filter(|(_, node)| {
                let unnamed_button = node.role() == Role::Button
                    && node.label().is_none_or(|label| label.trim().is_empty());
                let glyph = [node.label(), node.value()]
                    .into_iter()
                    .flatten()
                    .any(private_use);
                unnamed_button || glyph
            })
            .map(|(_, node)| format!("{node:?}"))
            .collect()
    }

    fn captioned(&self, role: Role, caption: &str) -> bool {
        let captions: Vec<NodeId> = self
            .accessible
            .iter()
            .filter(|(_, node)| node.role() == Role::Label && node.value() == Some(caption))
            .map(|(id, _)| *id)
            .collect();
        self.accessible.iter().any(|(_, node)| {
            node.role() == role
                && node
                    .labelled_by()
                    .iter()
                    .any(|label| captions.contains(label))
        })
    }
}

fn assert_failure_shown_once(harness: &Harness, name: &str) {
    assert_ne!(harness.color_of(name), harness.error_color());
    assert!(harness.shows(icons::FAILED));
}

fn collect_texts(
    shape: Shape,
    texts: &mut Vec<(String, Rect)>,
    colors: &mut Vec<(String, Color32)>,
) {
    match shape {
        Shape::Text(text) => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            let shown = text.galley.text().to_owned();
            let color = text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.color)
                .filter(|color| *color != Color32::PLACEHOLDER)
                .unwrap_or(text.fallback_color);
            colors.push((shown.clone(), color));
            texts.push((shown, rect));
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_texts(shape, texts, colors);
            }
        }
        _ => {}
    }
}

#[test]
fn editing_parameters_breaking_a_feature_and_undoing_it_works_through_the_panels() {
    let mut harness = Harness::new();
    assert!(harness.shows("Up to date"));
    let width = harness.parameter("width");
    let height = harness.parameter("height");

    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    assert_eq!(
        harness.expression_text("height"),
        "400 mm * 1 mm / (width - 30 mm)"
    );
    harness.settle();
    assert!(harness.shows("40 mm"));

    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();
    assert!(harness.shows("1 feature failed"));
    assert_failure_shown_once(&harness, "Side sketch");
    assert!(harness.shows(
        "Distance between Point 0 and Point 1 cannot be evaluated: it uses height, which has an \
         error."
    ));

    harness.click("Go to height");
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(height).field_id())
    );

    harness.type_into(Focus::ParameterValue(height), "wdth");
    assert!(harness.shows("There is no parameter named 'wdth'"));
    assert_eq!(
        harness.expression_text("height"),
        "400 mm * 1 mm / (width - 30 mm)"
    );

    harness.focus(Focus::ParameterValue(height));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.shows("There is no parameter named 'wdth'"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
    harness.settle();
    assert!(harness.shows("Up to date"));
    assert_eq!(harness.model.undo_label(), Some("Edit height"));
}

#[test]
fn a_rejected_draft_gives_way_to_a_value_changed_by_undo() {
    let mut harness = Harness::new();
    let height = harness.parameter("height");
    let original = harness.expression_text("height");

    harness.type_into(Focus::ParameterValue(height), "30 mm");
    harness.settle();
    harness.type_into(Focus::ParameterValue(height), "wdth");
    let rejected = harness.shows("There is no parameter named 'wdth'");
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert!(rejected);
    assert_eq!(harness.expression_text("height"), original);
    assert!(!harness.shows("There is no parameter named 'wdth'"));
    assert!(!harness.shows("wdth"));
}

#[test]
fn a_dimension_edited_in_the_tree_is_undoable_and_rejects_the_wrong_kind() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let sketch = harness
        .document()
        .feature(base)
        .unwrap()
        .kind
        .sketch()
        .unwrap();
    let (constraint, _) = sketch
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .unwrap();
    let focus = Focus::Dimension {
        feature: base,
        constraint,
    };

    harness.type_into(focus, "width * width");
    assert!(harness.shows("It gives an area, but a length is needed"));

    harness.type_into(focus, "width / 4");
    harness.settle();
    assert!(harness.shows("= 10 mm"));
    assert_eq!(
        harness.model.undo_label(),
        Some("Edit dimension in Base sketch")
    );
}

#[test]
fn preferences_change_units_and_navigation_and_are_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Preferences"));
    harness.click("Centimetres");
    assert_eq!(harness.model.length_unit(), LengthUnit::Centimetre);
    harness.click("Navigation");
    harness.click("Scroll up to zoom out");
    assert!(harness.workspace.preferences.navigation.invert_zoom);
    harness.click("Close");
    assert!(!harness.workspace.preferences_open);

    harness.wait_until("the preferences are saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).text("units.length") == Some("cm")
    });
    let stored =
        Preferences::from_settings(caditor_file::Settings::load(&dir.path().join("config")));
    assert_eq!(stored.unit, LengthUnit::Centimetre);
    assert!(stored.navigation.invert_zoom);

    harness.settle();
    assert!(harness.shows("2 cm"));
    let base = harness.document().features().next().unwrap().id();
    let (constraint, _) = harness
        .sketch(base)
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    harness.type_into(
        Focus::Dimension {
            feature: base,
            constraint,
        },
        "3",
    );
    let stored = harness
        .sketch(base)
        .constraint(constraint)
        .and_then(Constraint::dimension)
        .map(|value| harness.document().expression_text(value))
        .unwrap();
    assert_eq!(stored, "3 cm");
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterValue(width), "4");
    assert_eq!(harness.expression_text("width"), "4 cm");

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Defaults(PreferencesTab::General),
    )));
    assert_eq!(harness.model.length_unit(), LengthUnit::Millimetre);
    assert!(harness.shows("20 mm"));
}

#[test]
fn a_preference_that_could_not_be_saved_is_shown_as_a_notice() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("config"), b"not a folder").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    assert!(harness.model.notice().is_none());

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.5),
    )));
    harness.wait_until("the failure is shown", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Could not save your preferences: "))
    });

    let notice = harness.model.notice().unwrap();
    assert!(notice.outlasts_edits);
    assert!(notice.text.ends_with("change one again to retry saving."));
}

#[test]
fn the_side_panel_opens_as_it_was_left_and_follows_changes_to_it() {
    let left = crate::layout::PanelLayout {
        side_width: 420.0,
        features_open: true,
        parameters_open: false,
    };
    let mut settings = caditor_file::Settings::default();
    left.write(&mut settings);
    let mut preferences = Preferences::from_settings(settings);
    preferences.onboarding = crate::onboarding::Onboarding::finished();
    let mut harness = Harness::starting(
        None,
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    );

    assert_eq!(harness.workspace.panels.layout(), left);
    assert_eq!(harness.workspace.preferences.panels, left);
    assert!(!harness.shows("width"));

    harness.click(crate::panels::PARAMETERS_TITLE);
    harness.settle();

    assert!(harness.shows("width"));
    assert!(harness.workspace.preferences.panels.parameters_open);
    assert_eq!(harness.workspace.preferences.panels.side_width, 420.0);
}

#[test]
fn closing_with_unsaved_changes_asks_first_and_the_title_marks_them() {
    let mut harness = Harness::new();
    assert_eq!(app::window_title(&harness.model), "Untitled — caditor");
    harness.command(FileCommand::Quit);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());

    let mut harness = Harness::new();
    harness.edit_width("45 mm");
    assert_eq!(app::window_title(&harness.model), "*Untitled — caditor");
    harness.key(Key::Q, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Save changes to “Untitled”?"));
    harness.click("Cancel");
    assert!(!harness.shows("Save changes to “Untitled”?"));
    assert!(!harness.files.should_quit());

    harness.command(FileCommand::Quit);
    harness.click("Close without saving");
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn save_as_adds_the_model_extension_and_asks_before_replacing_what_the_dialog_did_not_name() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("Bracket v1.2.caditor");
    caditor_file::save(&Document::default(), &existing, false).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(dir.path().join("Bracket v1.2")));

    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “Bracket v1.2.caditor”?")
    });
    harness.click("Cancel");
    assert_eq!(harness.model.path(), None);
    assert_eq!(
        caditor_file::load(&existing).unwrap().document,
        Document::default()
    );

    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “Bracket v1.2.caditor”?")
    });
    harness.click("Replace");
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert_eq!(
        harness.model.path(),
        Some(dunce::canonicalize(&existing).unwrap().as_path())
    );
    assert_eq!(
        caditor_file::load(&existing).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn saving_over_a_file_another_program_changed_asks_before_replacing_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plate.caditor");
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    let mut outside = harness.model.document().clone();
    let mut transaction = outside.transaction("Elsewhere");
    transaction.add_parameter("elsewhere", transaction.parse("3 mm").unwrap());
    outside.apply(transaction.finish()).unwrap();
    caditor_file::save(&outside, &path, false).unwrap();
    harness.edit_width("41 mm");

    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the outside change is shown", |harness| {
        harness.shows("“plate.caditor” was changed by another program")
    });
    harness.click("Cancel");

    assert!(harness.model.is_dirty());
    assert_eq!(caditor_file::load(&path).unwrap().document, outside);

    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the outside change is shown", |harness| {
        harness.shows("“plate.caditor” was changed by another program")
    });
    harness.click("Replace");
    harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    let listed = caditor_file::history(&path).unwrap();
    let kept = caditor_file::load_version(&path, listed.versions[0].index).unwrap();

    assert_eq!(
        caditor_file::load(&path).unwrap().document,
        *harness.model.document()
    );
    assert_eq!(kept.document, outside);

    harness.edit_width("42 mm");
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the next change is saved", |harness| {
        !harness.model.is_dirty()
    });
    assert!(!harness.shows("“plate.caditor” was changed by another program"));
}

#[test]
fn save_as_refuses_a_model_open_in_another_window() {
    let dir = TempDir::new().unwrap();
    let other_path = dir.path().join("other.caditor");
    caditor_file::save(&Document::default(), &other_path, false).unwrap();
    let other = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.path().join("recovery")),
            ..StorageConfig::default()
        },
        Start {
            file: Some(other_path.clone()),
            on_disk: None,
            loaded_with_problems: false,
            base: Document::default(),
            folded: 0,
            entries: Vec::new(),
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    assert!(other.flusher().flush(FILE_TIMEOUT));

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.answer_dialog(Some(other_path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the save is refused", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("open in another caditor window"))
    });
    assert_eq!(harness.model.path(), None);
    assert_eq!(
        caditor_file::load(&other_path).unwrap().document,
        Document::default()
    );
    assert!(other.close(true).wait(FILE_TIMEOUT));
}

#[cfg(unix)]
#[test]
fn an_unwritable_recovery_folder_shows_that_changes_are_not_protected() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir(&recovery).unwrap();
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o500)).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("the problem is shown", |harness| {
        harness.model.unprotected().is_some()
    });
    assert!(harness.shows("Not protected"));
    harness.edit_width("45 mm");
    assert!(harness.shows("Not protected"));
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn saving_from_the_close_prompt_writes_the_file_then_quits() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.edit_width("45 mm");
    let untitled_journals = std::fs::read_dir(root.as_path().join("recovery"))
        .unwrap()
        .count();
    assert_eq!(untitled_journals, 1);

    harness.answer_dialog(Some(root.as_path().join("bracket")));
    harness.command(FileCommand::Quit);
    harness.click("Save as…");
    harness.wait_until("the model is saved", |harness| harness.files.should_quit());

    let path = root.as_path().join("bracket.caditor");
    let saved = caditor_file::load(&path).unwrap();
    assert!(saved.issues.is_empty());
    assert_eq!(saved.document, *harness.model.document());
    assert_eq!(
        std::fs::read_dir(root.as_path().join("recovery"))
            .unwrap()
            .count(),
        0
    );
    assert!(!root.as_path().join(".bracket.caditor.journal").exists());
    let recent = caditor_file::RecentFiles::load(&root.as_path().join("state"));
    assert_eq!(recent.paths(), [path]);
}

#[test]
fn save_as_names_the_document_and_a_new_edit_marks_it_unsaved_again() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert_eq!(app::window_title(&harness.model), "plate.caditor — caditor");

    harness.edit_width("41 mm");
    assert!(harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(!harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.key(Key::S, Modifiers::COMMAND);
    harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    let saved = caditor_file::load(&path).unwrap().document;
    assert_eq!(saved, *harness.model.document());
}

#[test]
fn exporting_writes_the_chosen_bodies_in_the_chosen_format_beside_the_model() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::Export(ExportCommand::Show));
    assert!(
        harness
            .shows("There are no bodies to export yet. Extrude or revolve a sketch to make one.")
    );
    assert!(!harness.shows("Close"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(!harness.files.is_blocking());

    let (extrude, _) = extruded_plate(&mut harness);
    harness.key(Key::E, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Export"));
    harness.click("3MF");
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the 3MF is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.3mf”"))
    });
    let package = std::fs::read(dir.path().join("plate.3mf")).unwrap();
    assert!(package.starts_with(b"PK\x03\x04"));

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: false,
    }));
    assert!(harness.shows("Choose at least one body to export."));
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: extrude,
        included: true,
    }));
    harness.answer_dialog(Some(dir.path().join("plate.caditor")));
    harness.click("Export…");
    harness.wait_until("the STL is written", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Exported 1 body to “plate.caditor.stl”")
        })
    });
    let stl = std::fs::read(dir.path().join("plate.caditor.stl")).unwrap();
    let triangles = u32::from_le_bytes(stl[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl.len(), 84 + 50 * triangles);
    assert!(!dir.path().join("plate.caditor").exists());

    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    assert!(!harness.shows("Resolution"));
    harness.answer_dialog(Some(dir.path().join("plate.stp")));
    harness.click("Export…");
    harness.wait_until("the STEP file is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text == "Exported 1 body to “plate.stp”.")
    });
    let step = std::fs::read_to_string(dir.path().join("plate.stp")).unwrap();
    assert!(step.starts_with("ISO-10303-21;"));
    assert!(step.contains("MANIFOLD_SOLID_BREP("));
}

#[test]
fn cancelling_the_export_file_picker_keeps_the_dialog_and_its_settings() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    harness.answer_dialog(None);

    harness.click("Export…");
    harness.settle();

    assert!(harness.shows("Format"));
    assert!(!harness.shows("Resolution"));
    assert!(harness.files.is_blocking());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        name != "plate.step"
    }));
}

#[test]
fn exporting_asks_before_replacing_a_file_the_picker_did_not_name() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.stl");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(dir.path().join("plate")));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    assert!(harness.shows(&format!(
        "A file named “plate.stl” already exists in “{}”. Replacing it overwrites what it holds.",
        dir.path().display()
    )));
    harness.click("Cancel");
    assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Replace “plate.stl”?"));
    assert!(harness.shows("Resolution"));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.stl”?")
    });
    harness.click("Replace");
    harness.wait_until("the STL is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.stl”"))
    });
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Resolution"));
}

#[test]
fn exporting_replaces_a_file_the_picker_named_itself_without_asking_again() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.stl");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STL");
    harness.answer_dialog(Some(existing.clone()));

    harness.click("Export…");
    harness.wait_until("the STL is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body to “plate.stl”"))
    });
    assert!(!harness.shows("Replace “plate.stl”?"));
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
}

#[test]
fn exporting_an_image_keeps_its_dialog_when_the_picker_is_cancelled_and_asks_before_replacing() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("plate.png");
    std::fs::write(&existing, b"precious").unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    harness.click("Transparent");
    harness.answer_dialog(None);

    harness.click("Export…");
    harness.settle();
    assert!(harness.shows("Export image"));
    assert!(!harness.files.is_exporting_image());

    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.png”?")
    });
    harness.click("Cancel");
    assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(harness.shows("Export image"));

    harness.click("Export…");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “plate.png”?")
    });
    harness.click("Replace");
    harness.wait_until("the image is written", |harness| {
        !harness.files.is_exporting_image() && !harness.shows("Replace “plate.png”?")
    });
    harness.settle();
    assert_ne!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!harness.shows("Export image"));
}

#[test]
fn a_chosen_sketch_exports_to_a_dxf_of_its_curves_and_replacing_asks_first() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));
    let id = harness.add_sketch(sketch);
    harness.settle();
    let name = harness.document().feature(id).unwrap().name.clone();
    let availability = |harness: &Harness| {
        harness
            .workspace
            .last_offers
            .iter()
            .find(|offer| offer.command == Command::ExportSketch)
            .map(|offer| offer.availability.clone())
    };
    harness.workspace.panels.selected = None;
    harness.frame();
    assert_eq!(
        availability(&harness),
        Some(Err(
            "Choose a sketch in the feature tree, or edit one, to export it".to_owned()
        ))
    );

    harness.workspace.panels.choose_only(id);
    harness.frame();
    assert_eq!(availability(&harness), Some(Ok(())));
    harness.answer_dialog(Some(dir.path().join("outline")));
    run_from_palette(&mut harness, "export sketch");
    let written = dir.path().join("outline.dxf");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows(&format!("Exported 4 objects of “{name}” to “outline.dxf”."))
    });

    let drawing = caditor_file::read_dxf(&written).unwrap();
    assert_eq!(drawing.curve_count(), 4);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);

    let cut = dir.path().join("cut.svg");
    harness.answer_dialog(Some(cut.clone()));
    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "export sketch");
    harness.wait_until("the SVG is written", |_| cut.exists());
    let svg = std::fs::read_to_string(&cut).unwrap();
    assert!(svg.starts_with("<svg "));
    assert_eq!(svg.matches("<line ").count(), 4);

    harness.answer_dialog(Some(dir.path().join("outline")));
    std::fs::write(&written, b"precious").unwrap();
    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "export sketch");
    harness.wait_until("the replacement is confirmed", |harness| {
        harness.shows("Replace “outline.dxf”?")
    });
    harness.click("Cancel");
    harness.settle();
    assert_eq!(std::fs::read(&written).unwrap(), b"precious");
}

#[test]
fn a_selected_flat_face_exports_to_a_dxf_of_its_outline() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    let availability = |harness: &Harness| {
        harness
            .workspace
            .last_offers
            .iter()
            .find(|offer| offer.command == Command::ExportFace)
            .map(|offer| offer.availability.clone())
    };

    harness.select([]);
    harness.frame();

    assert_eq!(
        availability(&harness),
        Some(Err(
            "Select one flat face of a body to export its outline".to_owned()
        ))
    );

    harness.select([top]);
    harness.frame();

    assert_eq!(availability(&harness), Some(Ok(())));

    harness.answer_dialog(Some(dir.path().join("plate")));
    run_from_palette(&mut harness, "export face");
    let written = dir.path().join("plate.dxf");
    harness.wait_until("the drawing is written", |_| written.exists());
    harness.wait_until("the export is announced", |harness| {
        harness.shows("Exported 4 curves of “Extrude 1 end face” to “plate.dxf”, in 1 loop.")
    });

    let drawing = caditor_file::read_dxf(&written).unwrap();

    assert_eq!(drawing.curve_count(), 4);
    assert_eq!(drawing.layers, vec!["Outline".to_owned()]);
}

fn png_size(path: &Path) -> (u32, u32, u8) {
    const RGBA: u8 = 6;
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&bytes[12..16], b"IHDR");
    let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(bytes[25], RGBA);
    (side(16), side(20), bytes[24])
}

#[test]
fn exporting_an_image_writes_a_png_of_the_view_without_highlights_at_the_chosen_size() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    let view = harness.workspace.viewport.view_pixels().unwrap();
    let image = harness
        .workspace
        .viewport
        .image(&harness.model, &harness.workspace.editing, view);
    harness.select([]);
    let mut plain = harness.built().scene;
    plain.grid = None;

    assert_eq!(image.scene, plain);
    assert_eq!(image.pixels_per_point, 1.0);
    assert_eq!(
        image.view.size(),
        (harness.workspace.viewport.current_view().unwrap().size()).round()
    );

    harness.key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.show_new_windows();
    assert!(harness.shows("Export image"));
    assert!(harness.shows(&format!("View size ({} × {})", view.width, view.height)));
    harness.click("2×");
    assert!(harness.shows(&format!(
        "The image will be {} × {} pixels.",
        view.width * 2,
        view.height * 2
    )));
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click("Export…");
    harness.wait_until("the image is written", |harness| {
        !harness.files.is_exporting_image()
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!(
            "Exported a {} × {} image to “plate.png”.",
            view.width * 2,
            view.height * 2
        )
    );
    assert_eq!(
        png_size(&dir.path().join("plate.png")),
        (view.width * 2, view.height * 2, 8)
    );

    harness.command(FileCommand::ExportImage(ImageCommand::Show));
    assert!(harness.shows(&format!(
        "The image will be {} × {} pixels.",
        view.width * 2,
        view.height * 2
    )));
    harness.click("Custom");
    harness.type_into_field(Id::new(("image-side", "Width")), "5000");
    harness.type_into_field(Id::new(("image-side", "Height")), "none");
    assert!(harness.shows("Enter a whole number of pixels from 1 to 8192"));
    harness.type_into_field(Id::new(("image-side", "Height")), "900");
    harness.click("4×");
    assert!(harness.shows(
        "At 4× the image would be 20000 × 3600 pixels, more than the 8192 pixels a side caditor \
         draws. Choose a smaller size or scale."
    ));
    harness.click("1×");
    harness.click("Transparent");
    assert!(harness.shows("The image will be 5000 × 900 pixels."));
    harness.answer_dialog(Some(dir.path().join("wide.PNG")));
    harness.click("Export…");
    harness.wait_until("the wide image is written", |harness| {
        !harness.files.is_exporting_image()
    });
    assert_eq!(png_size(&dir.path().join("wide.PNG")), (5000, 900, 8));

    harness.image_failure = Some(ImageError::OutOfMemory);
    harness.key(Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.show_new_windows();
    harness.answer_dialog(Some(dir.path().join("huge.png")));
    harness.click("Export…");
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Could not export “huge.png”: the graphics card does not have enough memory for an image \
         this large. Choose a smaller size or scale, or lower the anti-aliasing in Preferences › \
         Graphics."
    );
    assert!(!dir.path().join("huge.png").exists());
    assert!(!harness.files.is_exporting_image());
    assert!(!harness.files.is_blocking());
}

fn write_drawing(path: &Path, units: Option<i64>, entities: &str) {
    let header = units.map_or_else(String::new, |units| {
        format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n")
    });
    let text = format!("{header}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n");
    std::fs::write(path, text).unwrap();
}

#[test]
fn importing_a_drawing_fills_a_new_sketch_or_the_one_being_edited() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();
    harness.answer_dialog(Some(square));
    harness.key(Key::I, Modifiers::COMMAND);
    harness.frame();
    harness.confirm_import("bracket.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.document().feature(sketch).unwrap().name, "bracket");
    assert_eq!(harness.editing(), Some(sketch));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 4 curves from “bracket.dxf” into bracket."
    );
    assert!(!harness.files.is_blocking());
    harness.settle();
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let before = harness.sketch(sketch).entities().len();

    let hole = dir.path().join("hole.dxf");
    write_drawing(
        &hole,
        None,
        "0\nCIRCLE\n8\n0\n10\n15\n20\n15\n40\n5\n0\nTEXT\n8\n0\n1\nNote\n",
    );
    harness.answer_dialog(Some(hole));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.confirm_import("hole.dxf");
    harness.wait_until("the report is shown", |harness| {
        harness.shows("Imported “hole.dxf” into bracket")
    });
    assert_eq!(harness.sketch(sketch).entities().len(), before + 2);
    assert_eq!(harness.document().features().len(), features + 1);
    assert!(harness.shows(
        "The drawing does not say which unit it uses, so its numbers were read as millimetres."
    ));
    harness.click("Close");
    assert!(!harness.files.is_blocking());
    harness.perform(Action::Undo);
    assert_eq!(harness.sketch(sketch).entities().len(), before);

    let label = dir.path().join("label.dxf");
    write_drawing(&label, Some(4), "0\nTEXT\n8\n0\n1\nNote\n");
    harness.answer_dialog(Some(label));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.wait_until("the empty import is reported", |harness| {
        harness.shows("Nothing was imported from “label.dxf”")
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Nothing in “label.dxf” could be imported: the drawing has no lines, arcs, circles or \
         splines to import."
    );
    assert_eq!(harness.sketch(sketch).entities().len(), before);
    harness.click("Close");

    let picture = dir.path().join("photo.dxf");
    std::fs::write(&picture, b"\x89PNG\r\n\x1a\n").unwrap();
    harness.answer_dialog(Some(picture));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the failure is reported", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice.text == "Could not import “photo.dxf”: it is not a DXF drawing."
        })
    });
    assert_eq!(harness.document().features().len(), features + 1);

    harness.edit_width("41 mm");
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Could not import “photo.dxf”: it is not a DXF drawing.")
    );
}

fn square_drawing(path: &Path) {
    write_drawing(
        path,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
}

fn sketch_positions(sketch: &Sketch) -> Vec<Point2> {
    sketch
        .entities()
        .filter_map(|(_, entity)| match entity {
            Entity::Point(position) => Some(*position),
            _ => None,
        })
        .collect()
}

#[test]
fn the_import_options_scale_centre_and_place_a_drawing_before_it_is_added() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    square_drawing(&square);
    let features = harness.document().features().len();

    harness.answer_dialog(Some(square));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the options are shown", |harness| {
        harness.shows("Import “bracket.dxf”")
    });
    assert!(harness.shows("4 curves drawn, 30.000 mm wide and 30.000 mm high."));
    assert!(harness.shows("Sketch plane"));
    assert_eq!(harness.document().features().len(), features);

    for command in [
        ImportOptionsCommand::Unit(DrawingUnit::Centimetres),
        ImportOptionsCommand::Scale(2.0),
        ImportOptionsCommand::Recentre(true),
        ImportOptionsCommand::Plane(PlaneChoice::Xz),
    ] {
        harness.command(FileCommand::ImportOptions(command));
    }
    harness.frame();
    assert!(harness.shows("4 curves drawn, 600.000 mm wide and 600.000 mm high."));
    harness.click("Import");

    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    let imported = harness.sketch(sketch);
    assert_eq!(imported.plane(), Plane::XZ);
    let positions = sketch_positions(imported);
    assert_eq!(positions.len(), 8);
    for position in positions {
        assert!((position.x.abs() - 300.0).abs() < 1e-9, "{position}");
        assert!((position.y.abs() - 300.0).abs() < 1e-9, "{position}");
    }
    assert!(harness.shows("Imported “bracket.dxf” into bracket"));
    assert!(harness.shows_containing("You chose to read the drawing's numbers as centimetres"));
    assert!(harness.shows("You scaled the drawing by 2."));
    harness.click("Close");
    assert!(!harness.files.is_blocking());
}

#[test]
fn cancelling_the_import_options_adds_nothing_and_a_sketch_being_edited_has_no_plane_choice() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("bracket.dxf");
    square_drawing(&square);
    let features = harness.document().features().len();

    harness.answer_dialog(Some(square.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("bracket.dxf");
    assert!(harness.files.is_blocking());
    harness.click("Cancel");
    assert!(!harness.files.is_blocking());
    assert!(!harness.files.is_importing());
    assert_eq!(harness.document().features().len(), features);

    harness.answer_dialog(Some(square.clone()));
    harness.command(FileCommand::Import { into: None });
    harness.confirm_import("bracket.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.editing(), Some(sketch));

    harness.answer_dialog(Some(square));
    harness.command(FileCommand::Import {
        into: harness.editing(),
    });
    harness.wait_for_import_options("bracket.dxf");
    assert!(!harness.shows("Sketch plane"));
}

#[test]
fn the_import_options_warn_when_the_chosen_layers_hold_more_than_a_sketch() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let plan = dir.path().join("dense.dxf");
    let mut entities: String = (0..=caditor_file::MAX_DRAWING_CURVES)
        .map(|index| format!("0\nLINE\n8\nHatching\n10\n{index}\n20\n0\n11\n{index}\n21\n1\n"))
        .collect();
    entities.push_str("0\nCIRCLE\n8\nOutline\n10\n20\n20\n20\n40\n3\n");
    write_drawing(&plan, Some(4), &entities);
    let warning = crate::import_options::too_many_curves(caditor_file::MAX_DRAWING_CURVES + 2);

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("dense.dxf");

    assert!(harness.shows(&warning));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 0,
        included: false,
    }));
    harness.frame();

    assert!(!harness.shows(&warning));
    assert!(harness.shows("1 curve drawn, 6.000 mm wide and 6.000 mm high."));
}

#[test]
fn the_import_options_list_the_layers_and_leave_out_the_ones_unticked() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let plan = dir.path().join("plan.dxf");
    write_drawing(
        &plan,
        Some(4),
        "0\nLINE\n8\nWalls\n10\n0\n20\n0\n11\n40\n21\n0\n\
         0\nLINE\n8\nWalls\n10\n0\n20\n5\n11\n40\n21\n5\n\
         0\nCIRCLE\n8\nNotes\n10\n20\n20\n20\n40\n3\n",
    );
    let features = harness.document().features().len();

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("plan.dxf");
    assert!(harness.shows("Layers"));
    assert!(harness.shows("Walls (2 curves)"));
    assert!(harness.shows("Notes (1 curve)"));
    assert!(harness.shows("3 curves drawn, 40.000 mm wide and 23.000 mm high."));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 1,
        included: false,
    }));
    harness.frame();
    assert!(harness.shows("2 curves drawn, 40.000 mm wide and 5.000 mm high."));

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::AllLayers(
        false,
    )));
    harness.frame();
    assert!(harness.shows("The drawing is empty."));
    assert!(harness.files.is_blocking());

    harness.command(FileCommand::ImportOptions(ImportOptionsCommand::Layer {
        layer: 0,
        included: true,
    }));
    harness.frame();
    harness.click("Import");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    let sketch = harness.document().features().last().unwrap().id();
    let circles = harness
        .sketch(sketch)
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Circle { .. }))
        .count();
    let lines = harness
        .sketch(sketch)
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .count();
    assert_eq!((lines, circles), (2, 0));
    assert!(harness.shows("1 curve on a layer you left out was not imported."));
}

#[test]
fn a_drawing_read_while_the_model_changes_is_placed_on_the_changed_model() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();
    let width = harness.parameter("width");
    let drawing = caditor_file::read_dxf(&square).unwrap();
    let plan = import::plan_drawing(
        harness.model.base(),
        &square,
        None,
        drawing,
        Arrangement::default(),
    );

    harness.model.perform(Action::Apply(Transaction::single(
        "Edit width",
        Edit::SetParameterExpression {
            id: width,
            expression: Expression::parse_stored("42 mm").unwrap(),
        },
    )));
    let placement = import::place_drawing(
        &mut harness.model,
        &mut harness.workspace.editing,
        &square,
        Ok(plan),
    );
    let Placement::Stale(drawing, _) = placement else {
        panic!("a drawing planned before the edit was placed on the changed model");
    };
    assert_eq!(harness.document().features().len(), features);

    let plan = import::plan_drawing(
        harness.model.base(),
        &square,
        None,
        drawing,
        Arrangement::default(),
    );
    let placement = import::place_drawing(
        &mut harness.model,
        &mut harness.workspace.editing,
        &square,
        Ok(plan),
    );
    assert!(matches!(placement, Placement::Done(None)));
    harness.frame();

    let sketch = harness.document().features().last().unwrap().id();
    assert_eq!(harness.document().feature(sketch).unwrap().name, "square");
    assert_eq!(harness.sketch(sketch).entities().len(), 12);
    assert_eq!(harness.expression_text("width"), "42 mm");
    assert_eq!(harness.model.undo_label(), Some("Import square.dxf"));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 4 curves from “square.dxf” into square."
    );

    harness.perform(Action::Undo);
    assert_eq!(harness.document().features().len(), features);
    assert_eq!(harness.expression_text("width"), "42 mm");
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
}

#[test]
fn a_drawing_named_on_the_command_line_is_imported_instead_of_opened() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let features = harness.document().features().len();

    harness.files.start(Some(square), &mut harness.model);

    harness.confirm_import("square.dxf");
    harness.wait_until("the drawing is imported", |harness| {
        harness.document().features().len() == features + 1
    });
    assert_eq!(harness.model.path(), None);
}

fn hovering(paths: &[&Path]) -> Vec<egui::HoveredFile> {
    paths
        .iter()
        .map(|path| egui::HoveredFile {
            path: Some(path.to_path_buf()),
            ..egui::HoveredFile::default()
        })
        .collect()
}

#[test]
fn files_dragged_over_the_window_say_what_dropping_them_does() {
    let mut harness = Harness::new();

    harness.frame();
    assert!(!harness.shows_containing("Drop to"));

    harness.hovered_files = hovering(&[Path::new("/tmp/plate.CADITOR")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to open plate.CADITOR"));

    harness.hovered_files = hovering(&[Path::new("/tmp/outline.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to import outline.dxf"));
    assert!(harness.shows("It becomes a new sketch."));

    harness.hovered_files = hovering(&[Path::new("/tmp/a.step"), Path::new("/tmp/b.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to import 2 files"));

    harness.hovered_files = hovering(&[Path::new("/tmp/plate.caditor"), Path::new("/tmp/b.dxf")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Not one model"));

    harness.hovered_files = hovering(&[Path::new("/tmp/notes.txt")]);
    harness.frame();
    harness.frame();
    assert!(harness.shows("notes.txt may not be a drawing or a STEP file"));

    harness.hovered_files = vec![egui::HoveredFile::default()];
    harness.frame();
    harness.frame();
    assert!(harness.shows("Drop to open or import"));

    harness.hovered_files = Vec::new();
    harness.frame();
    assert!(!harness.shows_containing("Drop to"));
}

#[test]
fn dropped_drawings_are_imported_one_after_another_and_a_dropped_model_opens() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let mut harness = Harness::with_directories(Some(root.as_path()));
    let square = root.as_path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let hole = root.as_path().join("hole.dxf");
    write_drawing(&hole, Some(4), "0\nCIRCLE\n8\n0\n10\n15\n20\n15\n40\n5\n");
    let features = harness.document().features().len();

    for paths in [vec![square.clone(), hole], vec![square.clone()]] {
        app::perform(
            vec![Action::File(FileCommand::Drop { paths, into: None })],
            &mut harness.model,
            &mut harness.files,
            &mut harness.workspace,
        );
    }
    assert_eq!(
        harness.model.notice().unwrap().text,
        "An import is already running. Drop the files again once it has finished."
    );
    harness.confirm_import("square.dxf");
    harness.confirm_import("hole.dxf");
    harness.wait_until("both drawings are imported", |harness| {
        harness.document().features().len() == features + 2
    });
    let names: Vec<&str> = harness
        .document()
        .features()
        .skip(features)
        .map(|feature| feature.name.as_str())
        .collect();
    assert_eq!(names, ["square", "hole"]);

    let path = root.as_path().join("plate.CADITOR");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    harness.command(FileCommand::Drop {
        paths: vec![path.clone(), square],
        into: None,
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Drop a single model to open it, or drawings and STEP files to import them."
    );
    assert_eq!(harness.model.path(), None);

    harness.command(FileCommand::Drop {
        paths: vec![path.clone()],
        into: None,
    });
    harness.frame();
    assert!(harness.shows("If you continue without saving, your changes will be lost."));
    harness.click("Continue without saving");
    harness.wait_until("the model is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });
}

#[test]
fn a_step_export_imports_back_as_a_body_that_later_features_can_use() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    let exported = dir.path().join("plate.step");
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.click("STEP");
    harness.answer_dialog(Some(exported.clone()));
    harness.click("Export…");
    harness.wait_until("the STEP file is written", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Exported 1 body"))
    });
    harness.command(FileCommand::New);
    harness.frame();
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.settle();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(exported));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the body is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Imported 1 body from “plate.step”."
    );
    let body = harness.document().features().last().unwrap().id();
    assert!(
        harness
            .document()
            .feature(body)
            .unwrap()
            .kind
            .import()
            .is_some()
    );
    harness.settle();
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let solid = harness.model.evaluation().body(body).unwrap();
    assert_eq!(solid.faces().count(), 6);
    harness.perform(Action::Undo);
    assert_eq!(harness.document().features().len(), before);
}

fn cube_stl(side: f64) -> String {
    let corners = [
        [0.0, 0.0, 0.0],
        [side, 0.0, 0.0],
        [side, side, 0.0],
        [0.0, side, 0.0],
        [0.0, 0.0, side],
        [side, 0.0, side],
        [side, side, side],
        [0.0, side, side],
    ];
    let triangles: [[usize; 3]; 12] = [
        [0, 3, 2],
        [0, 2, 1],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let mut text = "solid cube\n".to_owned();
    for triangle in triangles {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for corner in triangle {
            let [x, y, z] = corners[corner];
            text.push_str(&format!("vertex {x} {y} {z}\n"));
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid cube\n");
    text
}

#[test]
fn an_imported_body_is_replaced_from_a_file_in_place_and_undone_as_one_step() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let small = dir.path().join("cube.stl");
    let large = dir.path().join("cube-v2.stl");
    std::fs::write(&small, cube_stl(10.0)).unwrap();
    std::fs::write(&large, cube_stl(20.0)).unwrap();
    let before = harness.document().features().len();
    harness.answer_dialog(Some(small));
    harness.command(FileCommand::Import { into: None });
    harness.wait_until("the cube is imported", |harness| {
        harness.document().features().len() == before + 1
    });
    harness.settle();
    let body = harness.document().features().last().unwrap().id();
    let name = harness.document().feature(body).unwrap().name.clone();

    assert!(volume_about(&harness, body, 1000.0));

    harness.answer_dialog(Some(large));
    harness.command(FileCommand::ReplaceImport(body));
    harness.wait_until("the cube is replaced", |harness| {
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Replace"))
    });
    harness.settle();

    assert_eq!(harness.document().features().len(), before + 1);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Replace {name} from cube-v2.stl").as_str())
    );
    assert!(volume_about(&harness, body, 8000.0));
    assert_eq!(
        harness
            .document()
            .feature(body)
            .unwrap()
            .kind
            .import()
            .unwrap()
            .source,
        "cube-v2.stl"
    );

    harness.perform(Action::Undo);
    harness.settle();

    assert!(volume_about(&harness, body, 1000.0));

    let drawing = dir.path().join("plan.dxf");
    write_drawing(
        &drawing,
        Some(4),
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n40\n21\n0\n",
    );
    harness.answer_dialog(Some(drawing));
    harness.command(FileCommand::ReplaceImport(body));
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.model.notice().unwrap().text,
        "“plan.dxf” is not a STEP, STL, OBJ or 3MF file, so it cannot replace an imported body."
    );
}

fn damage_chunk(bytes: &[u8], chunk: usize) -> Vec<u8> {
    let starts: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, window)| *window == b"CDCK")
        .map(|(start, _)| start)
        .collect();
    let end = starts.get(chunk + 1).copied().unwrap_or(bytes.len());
    let mut damaged = bytes.to_vec();
    damaged[end - 1] ^= 0x55;
    damaged
}

#[test]
fn saving_while_typing_in_a_field_commits_the_field_first() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });

    let width = harness.parameter("width");
    harness.focus(Focus::ParameterValue(width));
    harness.replace_text("52 mm");
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(width).field_id())
    );
    harness.key(Key::S, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    harness.wait_until("the typed width is saved", |harness| {
        !harness.model.is_dirty() && !harness.model.is_saving()
    });

    assert_eq!(harness.expression_text("width"), "52 mm");
    assert_eq!(harness.focused(), None);
    let reopened = caditor_file::load(&path).unwrap();
    let document = &reopened.document;
    assert_eq!(
        document.expression_text(&document.parameter_named("width").unwrap().expression),
        "52 mm"
    );
}

#[test]
fn every_save_keeps_a_version_that_can_be_restored_and_undone() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::History(HistoryCommand::Show));
    assert!(!harness.files.is_blocking());

    let path = dir.path().join("plate.caditor");
    harness.answer_dialog(Some(path.clone()));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for width in ["45 mm", "50 mm"] {
        harness.edit_width(width);
        harness.command(FileCommand::Save);
        harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    }
    let history = caditor_file::history(&path).unwrap();
    assert_eq!(history.versions.len(), 2);
    assert!(history.versions.iter().all(|version| version.available));

    harness.command(FileCommand::History(HistoryCommand::Show));
    harness.wait_until("the versions are listed", |harness| {
        harness.shows("Restore")
    });
    assert!(harness.shows("Version history"));
    harness.click("Restore");
    harness.wait_until("the version is restored", |harness| {
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("Restored the version saved"))
    });
    assert_eq!(harness.expression_text("width"), "45 mm");
    assert!(harness.model.is_dirty());
    assert!(!harness.files.is_blocking());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "50 mm");
    assert!(!harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.command(FileCommand::Save);
    harness.wait_until("the restored model is saved", |harness| {
        !harness.model.is_dirty()
    });
    let reopened = caditor_file::load(&path).unwrap();
    assert_eq!(
        reopened.document.expression_text(
            &reopened
                .document
                .parameter_named("width")
                .unwrap()
                .expression
        ),
        "45 mm"
    );
    assert_eq!(caditor_file::history(&path).unwrap().versions.len(), 3);
}

#[test]
fn opening_a_damaged_file_reports_what_was_lost_and_keeps_the_original_on_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("damaged.caditor");
    let bytes = caditor_file::encode(&sample_document().unwrap()).unwrap();
    let side_sketch_record = 4;
    let damaged = damage_chunk(&bytes, side_sketch_record);
    std::fs::write(&path, &damaged).unwrap();

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Parts of “damaged.caditor” could not be read"));
    assert!(
        harness.shows("A damaged part of the file was skipped; anything it held was left out.")
    );
    assert_eq!(harness.model.document().features().len(), 1);
    harness.click("Close");
    assert!(!harness.shows("Parts of “damaged.caditor” could not be read"));

    harness.command(FileCommand::Save);
    harness.wait_until("the model is saved", |harness| !harness.model.is_saving());
    let backup = dir.path().join("damaged.damaged.caditor");
    assert_eq!(std::fs::read(backup).unwrap(), damaged);
    assert!(harness.shows("Saved. The damaged original was kept as “damaged.damaged.caditor”."));
    assert!(caditor_file::load(&path).unwrap().issues.is_empty());
}

#[test]
fn an_unreadable_journal_is_kept_aside_and_reported_when_its_model_opens() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("plate.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let journal = root.as_path().join(".plate.caditor.journal");
    std::fs::write(&journal, b"\x89CJL\r\n\x1a\n\xff\xff\xff\xff").unwrap();

    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Unsaved changes to “plate.caditor” could not be recovered"));
    let kept: Vec<String> = std::fs::read_dir(root.as_path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".unreadable"))
        .collect();
    assert_eq!(kept.len(), 1);
    let note = caditor_file::describe_set_aside(&root.as_path().join(&kept[0]));
    assert!(harness.shows(&note), "{note}");
    harness.click("Close");
    assert!(!harness.model.is_dirty());
}

fn crashed_with_a_width_change(dir: &Path) -> Editor {
    let base = sample_document().unwrap();
    let mut editor = Editor::new(base.clone());
    let width = base.parameter_named("width").unwrap().id();
    let change = caditor_document::Transaction::single(
        "Edit width",
        caditor_document::Edit::SetParameterExpression {
            id: width,
            expression: base.parse("55 mm").unwrap(),
        },
    );
    editor.apply(change.clone()).unwrap();
    let crashed = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.join("recovery")),
            ..StorageConfig::default()
        },
        Start {
            file: None,
            on_disk: None,
            loaded_with_problems: false,
            base,
            folded: 0,
            entries: vec![JournalEntry::Apply(change)],
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    assert!(crashed.flusher().flush(FILE_TIMEOUT));
    assert!(crashed.close(false).wait(FILE_TIMEOUT));
    editor
}

#[test]
fn unsaved_work_from_a_crash_is_offered_and_restored_with_its_history() {
    let dir = TempDir::new().unwrap();
    let editor = crashed_with_a_width_change(dir.path());

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });
    assert!(harness.shows("Untitled model"));
    assert!(harness.shows("1 unsaved change, last one just now"));
    harness.click("Restore");

    assert!(!harness.shows("Recover unsaved work"));
    assert_eq!(harness.model.document(), editor.document());
    assert!(harness.model.is_dirty());
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
    assert_eq!(harness.expression_text("width"), "55 mm");
    let recovery = dir.path().join("recovery");
    harness.wait_until("the restored journal replaces the recovered one", |_| {
        std::fs::read_dir(&recovery).unwrap().count() == 1
    });
}

#[test]
fn unsaved_work_can_be_restored_with_every_feature_suppressed_and_brought_back_by_undo() {
    let dir = TempDir::new().unwrap();
    let editor = crashed_with_a_width_change(dir.path());
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.wait_until("recovery is offered", |harness| {
        harness.shows("Recover unsaved work")
    });

    harness.click("Restore suppressed");

    assert!(
        harness
            .model
            .document()
            .features()
            .all(|feature| feature.suppressed)
    );
    assert!(
        harness
            .model
            .notice()
            .unwrap()
            .text
            .starts_with("Restored with every feature suppressed.")
    );
    assert_eq!(harness.model.undo_label(), Some("Suppress every feature"));
    harness.perform(Action::Undo);
    assert_eq!(harness.model.document(), editor.document());
    assert_eq!(harness.model.undo_label(), Some("Edit width"));
}

fn fail_a_frame(harness: &mut Harness) -> app::AfterFailedFrame {
    app::after_failed_frame(
        &mut harness.workspace,
        &mut harness.model,
        &mut harness.files,
    )
}

#[test]
fn a_failed_frame_resets_the_interface_and_a_second_in_a_row_suppresses_every_feature() {
    let mut harness = Harness::new();
    let before = harness.model.document().clone();
    harness.workspace.preferences_open = true;
    harness.workspace.about_open = true;
    harness.command(FileCommand::Export(ExportCommand::Show));
    assert!(harness.files.is_blocking());

    let first = fail_a_frame(&mut harness);

    assert_eq!(first, app::AfterFailedFrame::ResetInterface);
    assert!(!harness.workspace.preferences_open);
    assert!(!harness.workspace.about_open);
    assert!(!harness.files.is_blocking());
    assert_eq!(harness.model.document(), &before);
    assert!(
        harness
            .model
            .notice()
            .unwrap()
            .text
            .starts_with("Something went wrong while drawing the window")
    );

    let second = fail_a_frame(&mut harness);

    assert_eq!(second, app::AfterFailedFrame::SuppressFeatures);
    assert!(
        harness
            .model
            .document()
            .features()
            .all(|feature| feature.suppressed)
    );
    harness.perform(Action::Undo);
    assert_eq!(harness.model.document(), &before);
    harness.frame();
    harness.workspace.frame_failures.drawn();
    assert_eq!(
        fail_a_frame(&mut harness),
        app::AfterFailedFrame::ResetInterface
    );
    for _ in 0..3 {
        fail_a_frame(&mut harness);
    }
    assert_eq!(fail_a_frame(&mut harness), app::AfterFailedFrame::GiveUp);
}

#[cfg(unix)]
#[test]
fn a_file_dialog_that_cannot_open_says_what_to_install_rather_than_doing_nothing() {
    let mut harness = Harness::new();
    *harness.dialogs.no_portal.lock() = true;

    harness.command(FileCommand::Open);
    harness.wait_until("the failure is reported", |harness| {
        harness.model.notice().is_some()
    });

    let notice = harness.model.notice().unwrap().text.clone();
    assert!(
        notice.starts_with("The file dialog could not be shown"),
        "{notice}"
    );
    assert!(notice.contains("xdg-desktop-portal"));
    *harness.dialogs.no_portal.lock() = false;
    harness.answer_dialog(None);
    harness.command(FileCommand::Open);
    harness.frame();
    assert!(!harness.files.is_blocking());
}

#[test]
fn a_missing_file_is_reported_and_dropped_from_recent_files_and_reopening_is_harmless() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert_eq!(harness.files.recent(), std::slice::from_ref(&path));

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is checked", |harness| {
        harness.shows("“kept.caditor” is already open.")
    });

    std::fs::rename(&path, root.as_path().join("moved.caditor")).unwrap();
    harness.command(FileCommand::New);
    harness.command(FileCommand::OpenPath(path));
    harness.wait_until("the failure is reported", |harness| {
        harness.shows("Could not open “kept.caditor”: it no longer exists.")
    });
    assert!(harness.files.recent().is_empty());
    assert_eq!(harness.model.path(), None);
}

#[test]
fn a_newly_opened_model_starts_with_nothing_selected_or_left_out_of_export() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let chosen = Pickable::SketchEntity {
        feature: base,
        entity: line,
    };
    let session = harness.model.session();

    harness.select([chosen]);
    harness.workspace.panels.selected = Some(base);
    harness.command(FileCommand::Export(ExportCommand::Include {
        body: base,
        included: false,
    }));
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Plate));
    if harness.shows("Continue without saving") {
        harness.click("Continue without saving");
    }
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();

    assert!(harness.document().feature(base).is_some());
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.workspace.panels.selected, None);
    assert!(harness.files.exporter_includes(base));
}

#[test]
fn clicking_a_constraint_in_the_tree_edits_its_sketch_and_selects_it() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let (constraint, _) = harness
        .sketch(base)
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .unwrap();
    let description = harness.sketch(base).describe_constraint(constraint);
    harness.edit(base);
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    let finished = harness.editing();
    let name = harness.document().feature(base).unwrap().name.clone();
    harness.click_button(&format!("Show details of {name}"));
    harness.settle();

    harness.click_leftmost(&description);
    harness.settle();

    assert_eq!(finished, None);
    assert_eq!(harness.editing(), Some(base));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchConstraint {
                feature: base,
                constraint,
            })
    );
}

#[test]
fn every_conflicting_part_of_a_sketch_is_named_in_the_error() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().clone();
    let mut sketch = base.kind.sketch().unwrap().clone();
    let line = sketch
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    sketch.add_constraint(Constraint::Vertical(line)).unwrap();
    let other = sketch.add_line(Point2::new(0.0, 90.0), Point2::new(5.0, 90.0));
    sketch
        .add_constraint(Constraint::Horizontal(other))
        .unwrap();
    sketch.add_constraint(Constraint::Vertical(other)).unwrap();
    let replacement = Feature::new(base.id(), base.name.clone(), FeatureKind::from(sketch));
    harness.model.perform(Action::Apply(Transaction::new(
        "Add conflicts",
        vec![
            Edit::RemoveFeature { id: base.id() },
            Edit::InsertFeature {
                index: 0,
                feature: Arc::new(replacement),
            },
        ],
    )));
    harness.settle();

    assert_failure_shown_once(&harness, "Base sketch");
    assert!(harness.shows_containing("The sketch has 2 separate problems."));
    assert!(harness.shows_containing("Vertical Line 2 conflicts with Horizontal Line 2."));
}

#[test]
fn a_constraint_conflict_is_named_and_leads_to_the_newest_constraint() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().clone();
    let mut sketch = base.kind.sketch().unwrap().clone();
    let line = sketch
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let vertical = sketch.add_constraint(Constraint::Vertical(line)).unwrap();
    let replacement = Feature::new(base.id(), base.name.clone(), FeatureKind::from(sketch));
    harness.model.perform(Action::Apply(Transaction::new(
        "Add vertical",
        vec![
            Edit::RemoveFeature { id: base.id() },
            Edit::InsertFeature {
                index: 0,
                feature: Arc::new(replacement),
            },
        ],
    )));
    harness.settle();

    assert_failure_shown_once(&harness, "Base sketch");
    assert!(harness.shows("Vertical Line 2 conflicts with Horizontal Line 2."));
    harness.click("Go to Vertical Line 2");
    harness.let_animations_finish();
    assert!(harness.shows("Vertical Line 2"));
    assert!(!harness.workspace.panels.wants_focus(Focus::Constraint {
        feature: base.id(),
        constraint: vertical,
    }));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(harness.shows("Up to date"));
}

fn line_ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

#[test]
fn a_new_sketch_on_the_selected_plane_is_edited_until_finished() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click("New sketch");

    let feature = harness.editing().expect("the new sketch is being edited");
    let created = harness.document().feature(feature).unwrap();
    assert_eq!(created.name, "Sketch 1");
    assert_eq!(harness.sketch(feature).plane(), Plane::XZ);
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
    assert!(harness.workspace.viewport.selection().is_empty());
    harness.settle();
    assert!(harness.shows("Editing Sketch 1"));
    assert!(harness.shows("Fully constrained"));

    harness.click("Finish sketch");
    assert_eq!(harness.editing(), None);
    assert!(!harness.shows("Editing Sketch 1"));
    assert_eq!(harness.document().features().len(), 3);

    harness.click("New sketch");
    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows("Click a plane or a flat face to sketch on"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows("Click a plane or a flat face to sketch on"));

    harness.perform(Action::Editing(EditingCommand::NewSketch(Some(
        PrincipalPlane::Yz,
    ))));
    let second = harness.editing().unwrap();
    assert_eq!(harness.document().feature(second).unwrap().name, "Sketch 2");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.editing(), None);
}

#[test]
fn undoing_the_creation_of_the_edited_sketch_ends_editing() {
    let mut harness = Harness::new();
    harness.select([Pickable::Plane(PrincipalPlane::Xy)]);
    harness.click("New sketch");
    assert!(harness.editing().is_some());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.document().features().len(), 2);
    assert!(!harness.shows("Finish sketch"));
}

#[test]
fn horizontal_from_the_selection_levels_a_line_and_updates_the_freedom() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    assert!(harness.shows("4 degrees of freedom left"));

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.click_button("Horizontal");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Add Horizontal"));
    assert!(harness.shows("3 degrees of freedom left"));
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.y - end.y).abs() < 1e-9, "{start} {end}");

    harness.key(Key::H, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert_eq!(harness.sketch(feature).constraints().len(), 1);

    harness.add_stored_constraint(feature, Constraint::Horizontal(line));
    assert!(harness.shows("1 redundant constraint"));
    assert!(harness.shows("Redundant: Horizontal Line 2 already does this. Delete one of them."));
}

fn only_constraint(sketch: &Sketch) -> (caditor_sketch::ConstraintId, Constraint) {
    let mut constraints = sketch.constraints();
    let (id, constraint) = constraints.next().unwrap();
    assert!(constraints.next().is_none());
    (id, constraint.clone())
}

fn add_distance_to_a_line(harness: &mut Harness) -> (FeatureId, EntityId, Id) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 40.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);

    harness.key(Key::D, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    let (constraint, _) = only_constraint(harness.sketch(feature));
    harness.frame();
    (feature, line, annotations::field_id(feature, constraint))
}

fn dimension_text(harness: &Harness, feature: FeatureId) -> String {
    let (_, constraint) = only_constraint(harness.sketch(feature));
    harness
        .document()
        .expression_text(constraint.dimension().unwrap())
}

#[test]
fn a_new_distance_opens_its_field_on_the_canvas_with_the_measured_value_selected() {
    let mut harness = Harness::new();
    let (feature, line, field) = add_distance_to_a_line(&mut harness);
    let (start, end) = line_ends(harness.sketch(feature), line);
    assert_eq!(
        only_constraint(harness.sketch(feature)).1,
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(50.0, Unit::Millimetre),
        }
    );
    assert_eq!(harness.focused(), Some(field));

    harness.events.push(Event::Text("width / 2".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert_eq!(dimension_text(&harness, feature), "width / 2");
    assert_eq!(harness.model.undo_label(), Some("Edit dimension in Plate"));
    harness.settle();
    assert!(harness.shows("width / 2 = 20 mm"));
    assert_ne!(harness.focused(), Some(field));
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.distance(end) - 20.0).abs() < 1e-6);
    assert_eq!(harness.editing(), Some(feature));
}

#[test]
fn an_invalid_dimension_keeps_its_text_and_escape_reverts_it() {
    let mut harness = Harness::new();
    let (feature, line, field) = add_distance_to_a_line(&mut harness);

    harness.events.push(Event::Text("5 deg".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows("It gives an angle, but a length is needed"));
    assert!(harness.shows("5 deg"));
    assert_eq!(harness.focused(), Some(field));
    assert_eq!(dimension_text(&harness, feature), "50 mm");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.shows("It gives an angle, but a length is needed"));
    assert!(!harness.shows("5 deg"));
    assert!(harness.shows("50 mm"));
    assert_eq!(dimension_text(&harness, feature), "50 mm");
    assert_eq!(harness.editing(), Some(feature));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchEntity {
                feature,
                entity: line
            })
    );
}

fn edit_base_sketch(harness: &mut Harness) -> FeatureId {
    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    base
}

fn constraint_of_kind(sketch: &Sketch, kind: &str) -> caditor_sketch::ConstraintId {
    sketch
        .constraints()
        .find(|(_, constraint)| constraint.kind_name() == kind)
        .map(|(id, _)| id)
        .unwrap()
}

#[test]
fn double_clicking_a_dimension_label_selects_it_and_opens_its_field() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let distance = constraint_of_kind(harness.sketch(base), "Distance");
    assert!(harness.shows("width = 40 mm"));

    harness.double_click("width = 40 mm");
    let pickable = Pickable::SketchConstraint {
        feature: base,
        constraint: distance,
    };
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![pickable]
    );
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(annotations::field_id(base, distance))
    );

    harness.events.push(Event::Text("width / 4".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert!(harness.shows("width / 4 = 10 mm"));
}

#[test]
fn clicking_a_glyph_selects_its_constraint_and_delete_removes_it_in_one_step() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let before = harness.document().clone();
    let horizontal = constraint_of_kind(harness.sketch(base), "Horizontal");
    let pickable = Pickable::SketchConstraint {
        feature: base,
        constraint: horizontal,
    };

    harness.click("H");
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![pickable]
    );
    assert!(harness.shows("Base sketch › Horizontal Line 2"));
    harness.events.push(Event::PointerGone);
    harness.frame();
    harness.frame();
    assert_eq!(harness.color_of("H"), canvas::SELECTED);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    let sketch = harness.sketch(base);
    assert!(sketch.constraint(horizontal).is_none());
    assert_eq!(sketch.constraints().len(), 1);
    assert_eq!(sketch.entities().len(), 3);
    assert_eq!(harness.model.undo_label(), Some("Delete Horizontal Line 2"));
    harness.frame();
    assert!(!harness.shows("H"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(*harness.document(), before);
}

#[test]
fn a_conflict_colours_the_dimensions_and_glyphs_involved() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    assert_ne!(harness.color_of("H"), canvas::ERROR);
    assert_ne!(harness.color_of("width = 40 mm"), canvas::ERROR);
    let line = entities_of_kind(harness.sketch(base), "Line")[0];
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);

    harness.key(Key::V, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    harness.add_stored_constraint(base, Constraint::Vertical(line));
    assert!(harness.shows("Conflicting constraints"));
    for mark in ["H", "V"] {
        assert_eq!(harness.color_of(mark), canvas::ERROR, "{mark}");
    }
    assert_ne!(harness.color_of("width = 40 mm"), canvas::ERROR);
}

#[test]
fn with_the_line_tool_active_clicking_on_a_glyph_draws_instead_of_selecting() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    harness.use_tool(Key::L);
    let glyph = harness.position_of("H");

    harness.click_screen(glyph);
    harness.frame();
    harness.click_at(Point2::new(20.0, 10.0));
    let lines = entities_of_kind(harness.sketch(base), "Line");
    assert_eq!(lines.len(), 2);
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
}

#[test]
fn a_constraint_button_explains_what_to_select_when_it_cannot_apply() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let (start, _) = line_ends(harness.sketch(feature), line);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: start,
    }]);

    harness.click_button("Parallel");
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
    harness.hover_button("Parallel");
    assert!(harness.shows("Make lines parallel. Select two or more lines (Shift+P)"));

    harness.key(Key::P, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Parallel: Select two or more lines"));
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
}

fn entity_pickables(feature: FeatureId, entities: &[EntityId]) -> Vec<Pickable> {
    entities
        .iter()
        .map(|entity| Pickable::SketchEntity {
            feature,
            entity: *entity,
        })
        .collect()
}

#[test]
fn a_horizontal_distance_between_two_points_is_a_dimension_edited_on_the_canvas() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_point(Point2::new(0.0, 0.0));
    let right = sketch.add_point(Point2::new(30.0, 12.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select(entity_pickables(feature, &[left, right]));

    harness.key(Key::X, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    let (constraint, added) = only_constraint(harness.sketch(feature));
    assert_eq!(
        added,
        Constraint::HorizontalDistance {
            from: left,
            to: right,
            value: Expression::Measure(30.0, Unit::Millimetre),
        }
    );
    assert_eq!(harness.model.undo_label(), Some("Add Horizontal distance"));
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(annotations::field_id(feature, constraint))
    );

    harness.events.push(Event::Text("width / 2".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert!(harness.shows("width / 2 = 20 mm"));
    let shown = harness.shown(feature);
    let (from, to) = (shown.point(left).unwrap(), shown.point(right).unwrap());
    assert!(((to.x - from.x) - 20.0).abs() < 1e-6, "{from} {to}");
    assert!(((to.y - from.y) - 12.0).abs() < 1e-6, "{from} {to}");
    assert!(harness.shows("Horizontal distance between Point 0 and Point 1"));
}

#[test]
fn a_diameter_is_labelled_with_its_sign_and_sets_the_circle() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select(entity_pickables(feature, &[circle]));

    harness.click_button("Diameter");
    harness.frame();
    let (constraint, _) = only_constraint(harness.sketch(feature));
    harness.frame();
    harness.type_into_field(annotations::field_id(feature, constraint), "14");
    harness.settle();

    assert!(harness.shows("Ø 14"));
    assert!(harness.shows("Diameter of Circle 1"));
    let (_, radius) = harness.shown(feature).circle(circle).unwrap();
    assert!((radius - 7.0).abs() < 1e-9, "{radius}");
}

#[test]
fn fixing_a_line_locks_both_ends_where_they_are_shown() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(2.0, 3.0), Point2::new(25.0, 9.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    assert!(harness.shows("4 degrees of freedom left"));
    harness.select(entity_pickables(feature, &[line]));

    harness.key(Key::F, Modifiers::SHIFT);
    harness.frame();
    harness.settle();

    let (start, end) = line_ends(harness.sketch(feature), line);
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Fix"),
        vec![
            Constraint::Fix {
                point: start,
                at: Point2::new(2.0, 3.0),
            },
            Constraint::Fix {
                point: end,
                at: Point2::new(25.0, 9.0),
            },
        ]
    );
    assert_eq!(harness.model.undo_label(), Some("Add Fix"));
    assert!(harness.shows("Fully constrained"));
    assert!(harness.shows("Fix Point 0"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(harness.sketch(feature).constraints().next().is_none());
    assert!(harness.shows("4 degrees of freedom left"));
}

#[test]
fn equal_and_parallel_take_several_lines_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let lines = [
        sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0)),
        sketch.add_line(Point2::new(0.0, 10.0), Point2::new(15.0, 14.0)),
        sketch.add_line(Point2::new(0.0, 20.0), Point2::new(30.0, 26.0)),
    ];
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.select(entity_pickables(feature, &lines));

    harness.click_button("Parallel");
    harness.click_button("Equal");
    harness.settle();

    let [first, second, third] = lines;
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Parallel"),
        vec![
            Constraint::Parallel(first, second),
            Constraint::Parallel(first, third)
        ]
    );
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Equal"),
        vec![
            Constraint::Equal(first, second),
            Constraint::Equal(first, third)
        ]
    );
    let shown = harness.shown(feature);
    let direction = |line| shown.line_direction(line).unwrap();
    for line in [second, third] {
        assert!(direction(first).perp_dot(direction(line)).abs() < 1e-6);
        assert!((direction(first).length() - direction(line).length()).abs() < 1e-6);
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(constraints_of_kind(harness.sketch(feature), "Equal").is_empty());
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Parallel").len(),
        2
    );
}

#[test]
fn a_point_goes_onto_a_spline_and_a_line_touches_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let point = sketch.add_point(Point2::new(4.0, 9.0));
    let line = sketch.add_line(Point2::new(2.0, 7.0), Point2::new(18.0, 6.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.select(entity_pickables(feature, &[spline, point]));

    harness.click_button("Coincident");
    harness.settle();
    harness.select(entity_pickables(feature, &[spline, line]));
    harness.click_button("Tangent");
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Coincident"),
        vec![Constraint::Coincident(spline, point)]
    );
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Tangent"),
        vec![Constraint::Tangent(spline, line)]
    );
    let shown = harness.shown(feature);
    let curve = shown.spline(spline).unwrap();
    let on_curve = shown.point(point).unwrap();
    let nearest = (0..=2000)
        .map(|step| curve.point_at(f64::from(step) / 2000.0).distance(on_curve))
        .fold(f64::INFINITY, f64::min);
    assert!(nearest < 1e-2, "{on_curve} is {nearest} from the spline");
    let (start, end) = shown.line_endpoints(line).unwrap();
    let highest = (0..=2000)
        .map(|step| {
            let at = curve.point_at(f64::from(step) / 2000.0);
            (end - start).normalize().perp_dot(at - start)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(highest.abs() < 1e-3, "the line is {highest} from touching");
    assert!(harness.shows("Coincident Spline 3 and Point 4"));
    assert!(harness.shows("Tangent Spline 3 and Line 7"));
    assert!(!harness.shows("Conflicting constraints"));
}

#[test]
fn symmetric_mirrors_two_points_about_the_selected_line_from_the_palette() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(-8.0, 5.0));
    let second = sketch.add_point(Point2::new(9.0, 6.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let mut chosen = entity_pickables(feature, &[first, second]);
    chosen.push(Pickable::SketchEntity {
        feature,
        entity: EntityId::VERTICAL_AXIS,
    });
    harness.select(chosen);

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.type_text("Symmetric");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    assert_eq!(
        only_constraint(harness.sketch(feature)).1,
        Constraint::Symmetric {
            first,
            second,
            about: EntityId::VERTICAL_AXIS,
        }
    );
    let shown = harness.shown(feature);
    let (a, b) = (shown.point(first).unwrap(), shown.point(second).unwrap());
    assert!(
        (a.x + b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6,
        "{a} {b}"
    );
    assert!(harness.shows("Symmetric Point 0 and Point 1 about Vertical axis"));
}

#[test]
fn a_conflicting_constraint_is_reported_and_undo_clears_it() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    assert!(harness.shows("2 degrees of freedom left"));
    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);

    harness.key(Key::V, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    harness.add_stored_constraint(base, Constraint::Vertical(line));
    assert!(harness.shows("Conflicting constraints"));
    assert!(harness.shows("Vertical Line 2 conflicts with Horizontal Line 2."));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.editing(), Some(base));
}

#[test]
fn deleting_a_point_removes_its_line_and_constraints_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let lone = sketch.add_point(Point2::new(5.0, 5.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let before = harness.document().clone();
    let (start, end) = line_ends(harness.sketch(feature), line);
    harness.select([
        Pickable::SketchEntity {
            feature,
            entity: start,
        },
        Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        },
    ]);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert!(sketch.entity(start).is_none());
    assert!(sketch.entity(line).is_none());
    assert!(sketch.entity(end).is_some());
    assert!(sketch.entity(lone).is_some());
    assert_eq!(sketch.constraints().len(), 0);
    assert_eq!(harness.model.undo_label(), Some("Delete Point 0"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(*harness.document(), before);
}

fn entities_of_kind(sketch: &Sketch, kind: &str) -> Vec<EntityId> {
    sketch
        .entities()
        .filter(|(_, entity)| entity.kind_name() == kind)
        .map(|(id, _)| id)
        .collect()
}

fn constraints_of_kind(sketch: &Sketch, kind: &str) -> Vec<Constraint> {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == kind)
        .map(|(_, constraint)| constraint.clone())
        .collect()
}

fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) < DRAWN
}

#[test]
fn a_press_dragged_or_held_with_a_drawing_tool_still_places_the_point_where_it_is_released() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.click_at(Point2::new(10.0, 10.0));
    let from = harness.on_screen(Point2::new(20.0, 20.0));
    drag_in_sketch(&mut harness, from, Point2::new(40.0, 25.0));
    harness.frame();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 1);
    let (start, end) = sketch.line_endpoints(lines[0]).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
}

fn aligned_point_pairs(sketch: &Sketch) -> Vec<Constraint> {
    sketch
        .constraints()
        .map(|(_, constraint)| constraint.clone())
        .filter(|constraint| {
            matches!(
                constraint,
                Constraint::HorizontalPoints(..) | Constraint::VerticalPoints(..)
            )
        })
        .collect()
}

#[test]
fn a_point_hovered_while_drawing_guides_later_points_into_line_with_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);
    harness.click_at(Point2::new(40.0, 30.0));
    let [guide] = entities_of_kind(harness.sketch(feature), "Point")[..] else {
        panic!("one point should be drawn");
    };
    harness.use_tool(Key::R);

    harness.point_at(Point2::new(40.0, 30.0));
    harness.click_at(Point2::new(10.0, 5.0));
    harness.point_at(Point2::new(25.0, 30.02));
    assert!(harness.shows(&format!("Horizontal from Point {guide}")));
    harness.click_at(Point2::new(25.0, 30.02));

    let sketch = harness.sketch(feature);
    let guided = sketch.point(guide).unwrap();
    let [Constraint::HorizontalPoints(corner, reference)] = aligned_point_pairs(sketch)[..] else {
        panic!("the far corner should line up beside the hovered point");
    };
    assert_eq!(reference, guide);
    assert_eq!(sketch.point(corner).map(|at| at.y), Some(guided.y));

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(70.0, 10.0));
    harness.point_at(Point2::new(40.0, 30.0));
    harness.point_at(Point2::new(40.02, 10.02));
    assert!(harness.shows(&format!("Horizontal, vertical from Point {guide}")));
    harness.click_at(Point2::new(40.02, 10.02));
    harness.key(Key::Escape, Modifiers::NONE);

    let sketch = harness.sketch(feature);
    let pairs = aligned_point_pairs(sketch);
    let [_, Constraint::VerticalPoints(end, reference)] = pairs[..] else {
        panic!("the line's end should line up under the hovered point: {pairs:?}");
    };
    let [.., line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("a line should be drawn");
    };
    let (start, _) = line_ends(sketch, line);
    assert_eq!(reference, guide);
    assert_eq!(
        sketch.point(end),
        Some(Point2::new(guided.x, sketch.point(start).unwrap().y))
    );
}

#[test]
fn a_line_drawn_to_the_side_of_one_circle_and_touching_another_keeps_both() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.click_at(Point2::new(30.0, 20.0));
    let sketch = harness.sketch(feature);
    let [circle] = entities_of_kind(sketch, "Circle")[..] else {
        panic!("one circle should be drawn");
    };
    let (centre, radius) = sketch.circle(circle).unwrap();
    let top = centre + Vector2::new(0.0, radius);
    let from = Point2::new(70.0, top.y);
    let away = from - centre;
    let touch = centre + away * (radius * radius / away.length_squared())
        - away.perp()
            * (radius * (away.length_squared() - radius * radius).sqrt() / away.length_squared());
    harness.use_tool(Key::L);

    harness.point_at(top + Vector2::new(0.02, 0.02));
    assert!(harness.shows(&format!("Top of Circle {circle}")));
    harness.click_at(from);
    harness.point_at(touch + Vector2::new(0.02, 0.02));
    assert!(harness.shows(&format!("Tangent to Circle {circle}")));
    harness.click_at(touch + Vector2::new(0.02, 0.02));
    harness.key(Key::Escape, Modifiers::NONE);

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should be drawn");
    };
    let (start, end) = line_ends(sketch, line);
    let (start_at, end_at) = sketch.line_endpoints(line).unwrap();
    assert!(constraints_of_kind(sketch, "Tangent").contains(&Constraint::Tangent(line, circle)));
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, circle))
    );
    assert!(((end_at - centre).length() - radius).abs() < 1e-9);
    assert!((end_at - centre).dot(end_at - start_at).abs() < 1e-6);
    assert_ne!(start, end);
}

#[test]
fn an_arc_starting_level_with_its_centre_and_a_spline_point_above_the_last_stay_so() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(30.0, 20.02));
    assert!(harness.shows("Horizontal"));
    harness.click_at(Point2::new(30.0, 20.02));
    harness.point_at(Point2::new(20.0, 30.0));
    harness.click_at(Point2::new(20.0, 30.0));

    let sketch = harness.sketch(feature);
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("one arc should be drawn");
    };
    let Some(Entity::Arc { center, start, .. }) = sketch.entity(arc).cloned() else {
        panic!("the arc is an arc");
    };
    let pairs = aligned_point_pairs(sketch);
    assert!(
        pairs.contains(&Constraint::HorizontalPoints(center, start)),
        "{pairs:?}"
    );

    harness.use_tool(Key::S);
    harness.click_at(Point2::new(50.0, 5.0));
    harness.click_at(Point2::new(50.02, 25.0));
    harness.click_at(Point2::new(70.0, 30.0));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let [spline] = entities_of_kind(sketch, "Spline")[..] else {
        panic!("one spline should be drawn");
    };
    let Some(Entity::Spline { control_points }) = sketch.entity(spline).cloned() else {
        panic!("the spline is a spline");
    };
    assert!(
        aligned_point_pairs(sketch).contains(&Constraint::VerticalPoints(
            control_points[0],
            control_points[1]
        ))
    );
}

#[test]
fn one_press_drag_release_draws_a_whole_line_rectangle_or_circle() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    let from = harness.on_screen(Point2::new(10.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(40.0, 25.0));
    harness.frame();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one drag should draw one line");
    };
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
    assert!(harness.shows("Click to end the line, Escape to stop"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);

    harness.use_tool(Key::R);
    let from = harness.on_screen(Point2::new(60.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(90.0, 30.0));
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 5);
}

#[test]
fn a_circle_is_drawn_by_dragging_from_its_centre_to_its_rim() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    let from = harness.on_screen(Point2::new(20.0, 35.0));
    drag_in_sketch(&mut harness, from, Point2::new(35.0, 35.0));
    harness.frame();
    let sketch = harness.sketch(feature);
    let [circle] = entities_of_kind(sketch, "Circle")[..] else {
        panic!("one drag should draw one circle");
    };
    let (centre, radius) = sketch.circle(circle).unwrap();
    assert!(near(centre, Point2::new(20.0, 35.0)), "{centre}");
    assert!((radius - 15.0).abs() < DRAWN, "{radius}");
}

#[test]
fn the_size_of_what_is_being_drawn_shows_beside_the_pointer() {
    let cases = [
        (
            Key::L,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 10.0),
            "30.00 mm   0.0°",
        ),
        (
            Key::R,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 30.0),
            "30.00 mm × 20.00 mm",
        ),
        (
            Key::C,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm",
        ),
        (
            Key::G,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm   6 sides",
        ),
        (
            Key::A,
            Point2::new(20.0, 20.0),
            Point2::new(35.0, 20.0),
            "R 15.00 mm",
        ),
        (
            Key::U,
            Point2::new(10.0, 10.0),
            Point2::new(40.0, 10.0),
            "30.00 mm",
        ),
    ];
    for (tool, start, to, expected) in cases {
        let mut harness = Harness::new();
        harness.draw_on_new_sketch();
        harness.use_tool(tool);
        harness.point_at(to);
        assert!(
            !harness.shows(expected),
            "{expected} before anything is placed"
        );

        harness.click_at(start);
        harness.point_at(to);

        assert!(harness.shows(expected), "{expected}");
    }
}

#[test]
fn arcs_and_slots_show_their_radius_sweep_and_width_while_the_second_point_is_chosen() {
    let cases = [
        (
            Key::A,
            Modifiers::ALT,
            [Point2::new(10.0, 20.0), Point2::new(20.0, 30.0)],
            Point2::new(30.0, 20.0),
            "R 10.00 mm",
        ),
        (
            Key::A,
            Modifiers::NONE,
            [Point2::new(20.0, 20.0), Point2::new(30.0, 20.0)],
            Point2::new(20.0, 30.0),
            "R 10.00 mm   90.0°",
        ),
        (
            Key::U,
            Modifiers::NONE,
            [Point2::new(10.0, 10.0), Point2::new(40.0, 10.0)],
            Point2::new(25.0, 14.0),
            "30.00 mm × Ø 8.00 mm",
        ),
    ];
    for (tool, modifiers, placed, to, expected) in cases {
        let mut harness = Harness::new();
        harness.draw_on_new_sketch();
        harness.use_tool_with(tool, modifiers);

        for point in placed {
            harness.click_at(point);
        }
        harness.point_at(to);
        harness.point_at(to);

        assert!(harness.shows(expected), "{expected}");
    }
}

#[test]
fn holding_ctrl_places_a_point_where_the_pointer_is_instead_of_snapping_to_a_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let target = sketch.add_point(Point2::new(40.0, 20.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::L);

    harness.click_at(Point2::new(10.0, 10.0));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::COMMAND));
    harness.click_at(Point2::new(40.3, 20.2));
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    harness.frame();

    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should have been drawn");
    };
    let (_, end) = line_ends(sketch, line);
    assert_ne!(end, target);
    let at = sketch.point(end).unwrap();
    assert!(at.distance(Point2::new(40.3, 20.2)) < 0.1, "{at}");
    assert!(constraints_of_kind(sketch, "Coincident").is_empty());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.click_at(Point2::new(10.0, 40.0));
    harness.click_at(Point2::new(40.3, 20.2));
    let snapped = harness.sketch(feature);
    assert_eq!(constraints_of_kind(snapped, "Coincident").len(), 1);
}

#[test]
fn a_press_that_slips_a_few_pixels_is_still_one_click_where_it_is_released() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    let from = harness.on_screen(Point2::new(10.0, 10.0));
    drag_in_sketch(&mut harness, from, Point2::new(10.5, 10.4));
    harness.frame();

    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 0);
    assert!(harness.shows("Click to end the line, Escape to stop"));
}

#[test]
fn drawing_a_line_adds_its_points_and_the_line_in_one_undoable_step() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    assert!(harness.shows("Click the start of the line"));

    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows("Click to end the line, Escape to stop"));
    harness.click_at(Point2::new(40.0, 25.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 1);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 2);
    assert_eq!(sketch.constraints().len(), 0);
    let (start, end) = sketch.line_endpoints(lines[0]).unwrap();
    assert!(near(start, Point2::new(10.0, 10.0)), "{start}");
    assert!(near(end, Point2::new(40.0, 25.0)), "{end}");
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.sketch(feature).entities().len(), 0);
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
}

#[test]
fn a_nearly_level_line_is_made_horizontal() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(40.0, 11.0));
    assert!(harness.shows("Horizontal"));
    harness.click_at(Point2::new(40.0, 11.0));

    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal"),
        vec![Constraint::Horizontal(line)]
    );
    harness.settle();
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.y - end.y).abs() < 1e-12, "{start} {end}");
    assert!(near(end, Point2::new(40.0, 10.0)), "{end}");
    assert!(harness.shows("3 degrees of freedom left"));
}

fn unit_cross(sketch: &Sketch, a: EntityId, b: EntityId) -> (f64, f64) {
    let (a_start, a_end) = sketch.line_endpoints(a).unwrap();
    let (b_start, b_end) = sketch.line_endpoints(b).unwrap();
    let a = (a_end - a_start).normalize();
    let b = (b_end - b_start).normalize();
    (a.perp_dot(b), a.dot(b))
}

#[test]
fn a_line_drawn_nearly_parallel_to_a_slanted_line_is_kept_parallel_in_one_undoable_step() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(5.0, 40.0));
    harness.click_at(Point2::new(35.0, 60.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let slanted = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(40.0, 30.4));
    assert!(harness.shows(&format!("Parallel to Line {slanted}")));
    harness.click_at(Point2::new(40.0, 30.4));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let drawn = lines[1];
    assert_eq!(
        constraints_of_kind(sketch, "Parallel"),
        vec![Constraint::Parallel(drawn, slanted)]
    );
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    harness.settle();
    let (cross, _) = unit_cross(&harness.shown(feature), slanted, drawn);
    assert!(cross.abs() < 1e-9, "{cross}");
    assert!(harness.shows("7 degrees of freedom left"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line"), vec![slanted]);
    assert!(constraints_of_kind(sketch, "Parallel").is_empty());
    assert_eq!(entities_of_kind(sketch, "Point").len(), 2);
}

#[test]
fn a_chained_line_turned_nearly_square_is_kept_perpendicular_to_the_last() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    let first = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.point_at(Point2::new(20.0, 60.4));
    assert!(harness.shows(&format!("Perpendicular to Line {first}")));
    harness.click_at(Point2::new(20.0, 60.4));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let second = lines[1];
    let (_, first_end) = line_ends(sketch, first);
    let (second_start, _) = line_ends(sketch, second);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );
    assert_eq!(
        constraints_of_kind(sketch, "Perpendicular"),
        vec![Constraint::Perpendicular(second, first)]
    );
    assert!(constraints_of_kind(sketch, "Parallel").is_empty());
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
    harness.settle();
    let (_, dot) = unit_cross(&harness.shown(feature), first, second);
    assert!(dot.abs() < 1e-9, "{dot}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line"), vec![first]);
    assert!(constraints_of_kind(sketch, "Perpendicular").is_empty());
}

#[test]
fn a_line_started_on_the_origin_is_joined_to_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.point_at(Point2::new(0.4, 0.3));
    assert!(harness.shows("Origin"));
    harness.click_at(Point2::new(0.4, 0.3));
    harness.click_at(Point2::new(20.0, 30.0));

    let sketch = harness.sketch(feature);
    let line = entities_of_kind(sketch, "Line")[0];
    let (start, _) = line_ends(sketch, line);
    assert_eq!(sketch.point(start), Some(Point2::ZERO));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(start, EntityId::ORIGIN)]
    );
}

#[test]
fn a_selected_dimension_is_disabled_into_a_reference_that_shows_the_measured_value() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(37.0, 0.0));
    let (start, end) = match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    };
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let length = sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        })
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    let label = Pickable::SketchConstraint {
        feature,
        constraint: length,
    };
    harness.select([label]);
    harness.frame();
    assert!(harness.shows("37 mm"));

    harness.click_button("Disable");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(!sketch.is_active(length));
    assert!(harness.shows("(37 mm)"));
    assert_eq!(
        sketch.describe_constraint(length),
        format!(
            "{} (disabled)",
            sketch.describe(sketch.constraint(length).unwrap())
        )
    );

    harness.click_button("Enable");
    harness.settle();
    assert!(harness.sketch(feature).is_active(length));
    assert!(!harness.shows("(37 mm)"));
}

#[test]
fn a_dimension_of_geometry_already_determined_is_added_as_a_reference() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(37.0, 0.0));
    let (start, end) = match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    };
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        })
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    harness.select(entity_pickables(feature, &[start, end]));

    harness.key(Key::X, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("already fully determined"))
    );
    harness.settle();

    let sketch = harness.sketch(feature);
    let (added, constraint) = sketch
        .constraints()
        .find(|(_, constraint)| matches!(constraint, Constraint::HorizontalDistance { .. }))
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    assert_eq!(
        constraint,
        Constraint::HorizontalDistance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        }
    );
    assert!(!sketch.is_active(added));
    assert!(
        harness
            .model
            .settled_solution(feature)
            .is_some_and(|solution| solution.redundancies().is_empty())
    );
    assert_eq!(
        harness.model.undo_label(),
        Some("Add reference Horizontal distance")
    );
    assert_ne!(
        harness.focused(),
        Some(annotations::field_id(feature, added))
    );
}

#[test]
fn a_point_placed_where_two_lines_cross_is_held_on_both() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(70.0, 50.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.click_at(Point2::new(10.0, 40.0));
    harness.click_at(Point2::new(50.0, 20.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    let lines = entities_of_kind(harness.sketch(feature), "Line");
    let [first, second] = lines[..] else {
        panic!("two lines are drawn");
    };

    let along = 30.0 / (2.0 / 3.0 + 0.5);
    let crossing = Point2::new(10.0 + along, 40.0 - 0.5 * along);
    harness.use_tool(Key::P);
    harness.point_at(Point2::new(crossing.x + 0.3, crossing.y + 0.4));
    assert!(harness.shows(&format!("Crossing of Line {first} and Line {second}")));
    harness.click_at(Point2::new(crossing.x + 0.3, crossing.y + 0.4));
    harness.settle();

    let sketch = harness.sketch(feature);
    let point = *entities_of_kind(sketch, "Point").last().unwrap();
    assert!(near(sketch.point(point).unwrap(), crossing));
    let held: Vec<EntityId> = constraints_of_kind(sketch, "Coincident")
        .into_iter()
        .filter_map(|constraint| match constraint {
            Constraint::Coincident(a, b) if a == point => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(held, vec![first, second]);
}

#[test]
fn snapping_can_be_turned_off_for_good_and_back_on() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert!(harness.workspace.viewport.snapping());

    run_from_palette(&mut harness, "snapping");
    harness.frame();
    assert!(!harness.workspace.viewport.snapping());
    harness.use_tool(Key::L);
    harness.point_at(Point2::new(50.3, 10.4));
    assert!(!harness.shows("On Point"));
    harness.click_at(Point2::new(50.3, 10.4));
    harness.click_at(Point2::new(50.3, 40.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let (start, _) = line_ends(sketch, lines[1]);
    assert!(near(sketch.point(start).unwrap(), Point2::new(50.3, 10.4)));
    assert!(constraints_of_kind(sketch, "Coincident").is_empty());

    run_from_palette(&mut harness, "snapping");
    harness.frame();
    assert!(harness.workspace.viewport.snapping());
}

#[test]
fn a_point_placed_at_the_centre_of_a_rectangle_is_held_there_by_symmetry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 30.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    harness.use_tool(Key::P);
    harness.point_at(Point2::new(30.4, 20.3));
    assert!(harness.shows_containing("Centre of the outline of Line"));
    harness.click_at(Point2::new(30.4, 20.3));
    harness.settle();

    let sketch = harness.sketch(feature);
    let symmetric = constraints_of_kind(sketch, "Symmetric");
    let [Constraint::Symmetric { about, .. }] = symmetric[..] else {
        panic!("one symmetry about the centre is expected");
    };
    assert!(near(sketch.point(about).unwrap(), Point2::new(30.0, 20.0)));
}

#[test]
fn a_point_placed_at_the_middle_of_an_arc_is_held_at_its_midpoint() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    let center = Point2::new(20.0, 20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    harness.point_at(center + Vector2::new(7.0, 7.0));
    harness.click_at(center + Vector2::new(0.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    let arc = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let middle = center + Vector2::from_angle(std::f64::consts::FRAC_PI_4) * 10.0;

    harness.use_tool(Key::P);
    harness.point_at(middle + Vector2::new(0.3, -0.2));
    assert!(harness.shows(&format!("Midpoint of Arc {arc}")));
    harness.click_at(middle + Vector2::new(0.3, -0.2));
    harness.settle();

    let sketch = harness.sketch(feature);
    let midpoints = constraints_of_kind(sketch, "Midpoint");
    let [Constraint::Midpoint { point, curve }] = midpoints[..] else {
        panic!("one midpoint constraint is expected");
    };
    assert_eq!(curve, arc);
    assert!(near(sketch.point(point).unwrap(), middle));
}

#[test]
fn a_line_started_at_the_middle_of_another_is_held_at_its_midpoint() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(50.0, 10.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    harness.use_tool(Key::L);
    harness.point_at(Point2::new(30.4, 10.5));
    let base = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert!(harness.shows(&format!("Midpoint of Line {base}")));
    harness.click_at(Point2::new(30.4, 10.5));
    harness.click_at(Point2::new(30.0, 40.0));
    harness.settle();

    let sketch = harness.sketch(feature);
    let midpoints = constraints_of_kind(sketch, "Midpoint");
    let [Constraint::Midpoint { point, curve }] = midpoints[..] else {
        panic!("one midpoint constraint is expected");
    };
    assert_eq!(curve, base);
    assert!(near(sketch.point(point).unwrap(), Point2::new(30.0, 10.0)));
}

#[test]
fn a_point_dropped_on_a_line_stays_on_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 40.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();
    harness.use_tool(Key::P);
    assert!(harness.shows("Click to place a point"));
    harness.point_at(Point2::new(30.5, 30.0));
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert!(harness.shows(&format!("On Line {line}")));
    harness.click_at(Point2::new(30.5, 30.0));

    let sketch = harness.sketch(feature);
    let point = *entities_of_kind(sketch, "Point").last().unwrap();
    let position = sketch.point(point).unwrap();
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(
        (end - start).perp_dot(position - start).abs() < 1e-9,
        "{position}"
    );
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(point, line)]
    );
    assert_eq!(harness.model.undo_label(), Some("Draw point"));
}

#[test]
fn chained_lines_are_joined_and_clicking_the_last_point_again_stops() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 25.0));
    harness.click_at(Point2::new(20.0, 40.0));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 4);
    let (_, first_end) = line_ends(sketch, lines[0]);
    let (second_start, _) = line_ends(sketch, lines[1]);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );
    assert_eq!(sketch.point(second_start), sketch.point(first_end));

    harness.point_at(Point2::new(20.3, 40.2));
    assert!(harness.shows("Stop here"));
    harness.click_at(Point2::new(20.3, 40.2));
    assert!(harness.shows("Click the start of the line"));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
}

#[test]
fn a_line_chain_goes_on_as_a_tangent_arc_and_back_to_lines_without_starting_over() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));

    harness.use_tool(Key::T);
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    harness.click_at(Point2::new(60.0, 30.0));
    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("the first line stays");
    };
    let [arc] = entities_of_kind(sketch, "Arc")[..] else {
        panic!("the arc continues the line");
    };
    assert_eq!(
        constraints_of_kind(sketch, "Tangent"),
        vec![Constraint::Tangent(line, arc)]
    );

    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.click_at(Point2::new(60.0, 60.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 2);

    harness.use_tool(Key::T);
    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert_eq!(entities_of_kind(harness.sketch(feature), "Arc").len(), 1);
    harness.click_at(Point2::new(70.0, 50.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Arc").len(), 2);
}

#[test]
fn backspace_and_undo_step_a_line_chain_back_one_line_at_a_time() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 25.0));
    harness.click_at(Point2::new(20.0, 40.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 2);

    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows("Click to end the line, Escape to stop"));

    harness.click_at(Point2::new(60.0, 30.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 2);
    let (_, first_end) = line_ends(sketch, lines[0]);
    let (second_start, _) = line_ends(sketch, lines[1]);
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(second_start, first_end)]
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
    assert!(harness.shows("Click to end the line, Escape to stop"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 0);
    assert!(harness.shows("Click to end the line, Escape to stop"));

    harness.click_at(Point2::new(30.0, 10.0));
    let sketch = harness.sketch(feature);
    let [line] = entities_of_kind(sketch, "Line")[..] else {
        panic!("one line should have been drawn from the first click");
    };
    let (start, _) = line_ends(sketch, line);
    let anchor = sketch.point(start).unwrap();
    assert!(anchor.distance(Point2::new(10.0, 10.0)) < 1e-3, "{anchor}");
}

#[test]
fn double_clicking_a_curve_selects_the_chain_it_belongs_to() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let a = Point2::new(10.0, 10.0);
    let b = Point2::new(40.0, 10.0);
    let c = Point2::new(40.0, 30.0);
    let first = sketch.add_line(a, b);
    let second = sketch.add_line(b, c);
    let (_, joint) = line_ends(&sketch, first);
    let (joined, _) = line_ends(&sketch, second);
    sketch
        .add_constraint(Constraint::Coincident(joint, joined))
        .unwrap();
    let apart = sketch.add_line(Point2::new(10.0, 50.0), Point2::new(40.0, 50.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let pick = |entity| Pickable::SketchEntity { feature, entity };

    let position = harness.hover_pickable(Plane::XY, Point2::new(25.0, 10.0), pick(first));
    harness.press(position);
    harness.press(position);
    harness.frame();

    let selected: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    assert_eq!(selected.len(), 2, "{selected:?}");
    assert!(selected.contains(&pick(first)) && selected.contains(&pick(second)));

    let position = harness.hover_pickable(Plane::XY, Point2::new(25.0, 50.0), pick(apart));
    harness.press(position);
    harness.press(position);
    harness.frame();

    let alone: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    assert_eq!(alone, [pick(apart)]);
}

#[test]
fn a_line_chain_stops_when_it_closes_on_its_start() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    harness.click_at(Point2::new(25.0, 35.0));
    harness.click_at(Point2::new(10.2, 10.1));
    assert!(harness.shows("Click the start of the line"));

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 3);
    let (first_start, _) = line_ends(sketch, lines[0]);
    let (_, last_end) = line_ends(sketch, lines[2]);
    assert!(
        constraints_of_kind(sketch, "Coincident")
            .contains(&Constraint::Coincident(last_end, first_start))
    );

    harness.click_at(Point2::new(60.0, 10.0));
    harness.click_at(Point2::new(60.0, 40.0));
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 4);
    assert!(!harness.shows("Click the start of the line"));
}

#[test]
fn a_rectangle_is_four_joined_lines_with_four_degrees_of_freedom() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.shows("Click the opposite corner"));
    harness.click_at(Point2::new(40.0, 30.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4);
    assert_eq!(entities_of_kind(sketch, "Point").len(), 8);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Vertical").len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));
    harness.settle();
    assert!(harness.shows("4 degrees of freedom left"));
    assert!(harness.shows("Click the rectangle's first corner"));
}

#[test]
fn circles_and_arcs_get_the_drawn_geometry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::C);
    assert!(harness.shows("Click the circle's centre"));
    harness.click_at(Point2::new(20.0, 20.0));
    assert!(harness.shows("Click a point on the circle"));
    harness.click_at(Point2::new(30.0, 20.0));
    let circle = entities_of_kind(harness.sketch(feature), "Circle")[0];
    let (center, radius) = harness.sketch(feature).circle(circle).unwrap();
    assert!(near(center, Point2::new(20.0, 20.0)), "{center}");
    assert!((radius - 10.0).abs() < DRAWN, "{radius}");
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));

    harness.use_tool(Key::A);
    assert!(harness.shows("Click the arc's centre"));
    let center = Point2::new(-20.0, 20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    for step in [Vector2::new(7.0, 7.0), Vector2::new(0.0, 10.0)] {
        harness.point_at(center + step);
    }
    harness.click_at(center + Vector2::new(-4.0, 3.0));
    let turned = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let arc = harness.sketch(feature).arc(turned).unwrap();
    assert!((arc.radius - 10.0).abs() < DRAWN);
    assert!(arc.start_angle.abs() < DRAWN);
    let expected = 3f64.atan2(-4.0);
    assert!((arc.sweep - expected).abs() < DRAWN, "{}", arc.sweep);

    let center = Point2::new(-20.0, -20.0);
    harness.click_at(center);
    harness.click_at(center + Vector2::new(10.0, 0.0));
    for step in [Vector2::new(7.0, -7.0), Vector2::new(0.0, -10.0)] {
        harness.point_at(center + step);
    }
    harness.click_at(center + Vector2::new(-4.0, -3.0));
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let swept = harness.sketch(feature).arc(arcs[1]).unwrap();
    assert!((swept.sweep - expected).abs() < DRAWN, "{}", swept.sweep);
    let Some(Entity::Arc { end, .. }) = harness.sketch(feature).entity(arcs[1]) else {
        panic!("expected an arc");
    };
    assert!(near(
        harness.sketch(feature).point(*end).unwrap(),
        center + Vector2::new(10.0, 0.0)
    ));
    assert_eq!(harness.model.undo_label(), Some("Draw arc"));
}

fn arc_end(sketch: &Sketch, arc: EntityId) -> EntityId {
    let Some(Entity::Arc { end, .. }) = sketch.entity(arc) else {
        panic!("expected an arc");
    };
    *end
}

fn joins(sketch: &Sketch, point: EntityId, other: EntityId) -> bool {
    constraints_of_kind(sketch, "Coincident")
        .iter()
        .any(|constraint| {
            matches!(constraint, Constraint::Coincident(a, b)
            if (*a == point && *b == other) || (*a == other && *b == point))
        })
}

#[test]
fn an_arc_end_and_a_circle_rim_join_only_what_they_land_on() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "46, 20");
    type_point(&mut harness, "46, 60");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    let on_line = format!("On {}", harness.sketch(feature).entity_label(line));
    harness.use_tool(Key::P);
    type_point(&mut harness, "47.5, 47");
    let lone = harness
        .sketch(feature)
        .entities()
        .find(
            |(_, entity)| matches!(entity, Entity::Point(at) if near(*at, Point2::new(47.5, 47.0))),
        )
        .map(|(id, _)| id)
        .expect("the lone point was placed");
    let lone_label = format!("On {}", harness.sketch(feature).entity_label(lone));

    harness.use_tool(Key::A);
    type_point(&mut harness, "40, 40");
    type_point(&mut harness, "50, 40");
    harness.point_at(Point2::new(47.0, 46.5));
    let near_lone_on_arc = harness.shows(&lone_label);
    harness.point_at(Point2::new(46.1, 48.1));
    let near_crossing = harness.shows(&on_line);
    harness.click_at(Point2::new(46.1, 48.1));
    harness.settle();
    let arc = entities_of_kind(harness.sketch(feature), "Arc")[0];
    let crossing_end = arc_end(harness.sketch(feature), arc);

    type_point(&mut harness, "40, 40");
    type_point(&mut harness, "50, 40");
    harness.point_at(Point2::new(47.0, 46.0));
    type_point(&mut harness, "47.5, 47");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let projected_end = arc_end(harness.sketch(feature), arcs[1]);
    let projected = harness.sketch(feature).point(projected_end).unwrap();

    harness.use_tool(Key::C);
    type_point(&mut harness, "60, 30");
    harness.point_at(Point2::new(46.2, 30.0));
    let rim_on_line = harness.shows(&on_line);
    harness.click_at(Point2::new(46.2, 30.0));
    harness.settle();
    let circle = entities_of_kind(harness.sketch(feature), "Circle")[0];

    assert!(!near_lone_on_arc);
    assert!(near_crossing);
    assert!(joins(harness.sketch(feature), crossing_end, line));
    assert!(near(
        harness.shown(feature).point(crossing_end).unwrap(),
        Point2::new(46.0, 48.0)
    ));
    assert!(!joins(harness.sketch(feature), projected_end, lone));
    assert!((projected.distance(Point2::new(40.0, 40.0)) - 10.0).abs() < DRAWN);
    assert!(!rim_on_line);
    assert!(!joins(harness.sketch(feature), line, circle));
    assert!(
        constraints_of_kind(harness.sketch(feature), "Coincident")
            .iter()
            .all(|constraint| !constraint.entities().contains(&circle))
    );
}

#[test]
fn q_switches_the_selected_curves_or_new_drawing_to_construction_geometry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let construction = |harness: &Harness| {
        harness
            .workspace
            .editing
            .active()
            .is_some_and(|active| active.construction)
    };
    let dashed = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .lines()
            .filter(|line| matches!(line.stroke, caditor_render::Stroke::Dashed { .. }))
            .count()
    };
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "20, 0");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let ordinary = entities_of_kind(harness.sketch(feature), "Line")[0];

    harness.key(Key::Q, Modifiers::NONE);
    harness.frame();
    let drawing_construction = construction(&harness);
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 10");
    type_point(&mut harness, "20, 10");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let guide = entities_of_kind(harness.sketch(feature), "Line")[1];
    harness.key(Key::Q, Modifiers::NONE);
    harness.frame();

    assert!(drawing_construction);
    assert!(!construction(&harness));
    assert!(harness.sketch(feature).is_construction(guide));
    assert!(!harness.sketch(feature).is_construction(ordinary));
    assert_eq!(dashed(&mut harness), 1);

    let line = |entity| Pickable::SketchEntity { feature, entity };
    harness.select([line(ordinary)]);
    harness.key(Key::Q, Modifiers::NONE);
    harness.settle();
    assert!(harness.sketch(feature).is_construction(ordinary));
    assert_eq!(dashed(&mut harness), 2);

    harness.select([line(ordinary), line(guide)]);
    harness.key(Key::Q, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.sketch(feature).construction().len(), 0);
    assert!(!construction(&harness));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert_eq!(harness.sketch(feature).construction().len(), 2);

    harness.select([]);
    harness.click_button("Construction");
    assert!(construction(&harness));
}

#[test]
fn a_typed_arc_goes_the_shorter_way_and_x_sends_it_the_long_way() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::A);
    assert!(offer(&harness, Command::ReverseArc).availability.is_err());
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    assert!(harness.shows_hint("X: the other way round   The arc follows your sweep around the centre, a typed end the shorter way   Esc: cancel the arc   Type x, y or length < angle for an exact point"));
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let short = harness.sketch(feature).arc(arcs[0]).unwrap();
    assert!((short.sweep - FRAC_PI_2).abs() < DRAWN, "{}", short.sweep);

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    harness.key(Key::X, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let long = harness.sketch(feature).arc(arcs[1]).unwrap();
    assert!(
        (long.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        long.sweep
    );
    let Some(Entity::Arc { start, .. }) = harness.sketch(feature).entity(arcs[1]) else {
        panic!("expected an arc");
    };
    assert!(near(
        harness.sketch(feature).point(*start).unwrap(),
        Point2::new(10.0, 0.0)
    ));

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    run_from_palette(&mut harness, "reverse the arc");
    type_point(&mut harness, "0, -10 mm");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let reversed = harness.sketch(feature).arc(arcs[2]).unwrap();
    assert!(
        (reversed.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        reversed.sweep
    );

    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "10 mm, 0");
    type_point(&mut harness, "-10 mm, 0");
    let arcs = entities_of_kind(harness.sketch(feature), "Arc");
    let half = harness.sketch(feature).arc(arcs[3]).unwrap();
    assert!((half.sweep - PI).abs() < DRAWN, "{}", half.sweep);
    assert!(half.start_angle.abs() < DRAWN, "{}", half.start_angle);
}

fn line_lengths(sketch: &Sketch) -> Vec<f64> {
    entities_of_kind(sketch, "Line")
        .into_iter()
        .map(|line| {
            let (start, end) = sketch.line_endpoints(line).unwrap();
            start.distance(end)
        })
        .collect()
}

#[test]
fn a_three_point_arc_runs_from_its_start_through_the_third_point_to_its_end() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool_with(Key::A, Modifiers::ALT);
    assert_eq!(harness.tool(), Some(Tool::ThreePointArc));
    assert!(harness.shows("Click where the arc starts"));
    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where the arc ends"));
    type_point(&mut harness, "-10, 0");
    assert!(harness.shows("Click a point the arc passes through"));
    type_point(&mut harness, "30, 0");
    assert!(harness.shows(Refusal::ArcInLine.reason()));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(1.0, -5.0));
    type_point(&mut harness, "6, -8");

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert_eq!(arcs.len(), 1);
    let arc = sketch.arc(arcs[0]).unwrap();
    assert!(near(arc.center, Point2::ZERO), "{}", arc.center);
    assert!((arc.radius - 10.0).abs() < DRAWN);
    assert!((arc.sweep - PI).abs() < DRAWN, "{}", arc.sweep);
    assert!(
        (arc.start_angle.abs() - PI).abs() < DRAWN,
        "{}",
        arc.start_angle
    );
    assert_eq!(harness.model.undo_label(), Some("Draw 3-point arc"));
    assert!(harness.shows("Click where the arc starts"));
}

#[test]
fn tangent_arcs_continue_smoothly_from_a_line_and_from_each_other() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "20, 0");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    let continue_line = format!("Continue {}", harness.sketch(feature).entity_label(line));

    harness.use_tool(Key::T);
    assert!(harness.shows("Click the end of a line, arc or spline to continue from"));
    type_point(&mut harness, "40, 40");
    let refused = harness.shows(Refusal::TangentStart.reason());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(20.0, 0.0));
    let offered = harness.shows(&continue_line);
    type_point(&mut harness, "20, 0");
    type_point(&mut harness, "30, 10");
    type_point(&mut harness, "40, 20");
    assert!(harness.shows("Click where the arc ends, Escape to stop"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.settle();

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert!(refused);
    assert!(offered);
    assert_eq!(arcs.len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    let first = harness.shown(feature).arc(arcs[0]).unwrap();
    let second = harness.shown(feature).arc(arcs[1]).unwrap();
    assert!(
        near(first.center, Point2::new(20.0, 10.0)),
        "{}",
        first.center
    );
    assert!((first.sweep - FRAC_PI_2).abs() < DRAWN, "{}", first.sweep);
    assert!(
        near(second.center, Point2::new(40.0, 10.0)),
        "{}",
        second.center
    );
    assert!((second.sweep - FRAC_PI_2).abs() < DRAWN, "{}", second.sweep);
    assert!(harness.shows("6 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw tangent arc"));
    assert!(harness.shows("Click the end of a line, arc or spline to continue from"));
}

#[test]
fn a_slot_has_round_ends_of_one_radius_tangent_to_its_sides() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::U);
    assert!(harness.shows("Click the centre of the slot's first end"));
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "30, 10");
    type_point(&mut harness, "@0, 0");
    assert!(harness.shows(Refusal::SlotWidth.reason()));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.point_at(Point2::new(20.0, 20.0));
    type_point(&mut harness, "@-5, 15");
    harness.settle();

    let sketch = harness.sketch(feature);
    let arcs = entities_of_kind(sketch, "Arc");
    assert_eq!(arcs.len(), 2);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    let radius = 10f64.hypot(30.0) / 2.0;
    for arc in &arcs {
        let arc = harness.shown(feature).arc(*arc).unwrap();
        assert!((arc.radius - radius).abs() < DRAWN, "{}", arc.radius);
        assert!((arc.sweep - PI).abs() < DRAWN, "{}", arc.sweep);
    }
    for length in line_lengths(&harness.shown(feature)) {
        assert!((length - 10f64.hypot(30.0)).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("3 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw slot"));

    harness.click_at(Point2::new(5.0, -40.0));
    harness.click_at(Point2::new(35.0, -39.9));
    harness.click_at(Point2::new(15.0, -35.0));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("7 degrees of freedom left"));

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the slots were extruded");
    let slot_area = |length: f64, radius: f64| length * 2.0 * radius + PI * radius * radius;
    let area = slot_area(10f64.hypot(30.0), radius) + slot_area(30.0, 5.0);
    let volume = harness.body_volume(extrude);
    assert!((volume / (10.0 * area) - 1.0).abs() < 1e-2, "{volume}");
}

#[test]
fn a_polygon_is_regular_with_as_many_sides_as_chosen() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    assert!(offer(&harness, Command::MoreSides).availability.is_err());
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the hexagon's centre"));
    harness.key(Key::CloseBracket, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Click the heptagon's centre"));
    for _ in 0..4 {
        harness.key(Key::OpenBracket, Modifiers::NONE);
        harness.frame();
    }
    harness.frame();
    assert!(harness.shows("Click the triangle's centre"));
    assert!(offer(&harness, Command::FewerSides).availability.is_err());
    run_from_palette(&mut harness, "another side");
    harness.frame();
    assert!(harness.shows("Click the square's centre"));

    harness.use_tool(Key::L);
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the square's centre"));
    type_point(&mut harness, "5, 5");
    assert!(harness.shows("Click a corner of the square"));
    type_point(&mut harness, "15, 5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    let circles = entities_of_kind(sketch, "Circle");
    assert_eq!(lines.len(), 4);
    assert_eq!(circles.len(), 1);
    assert!(sketch.is_construction(circles[0]));
    assert!(lines.iter().all(|line| !sketch.is_construction(*line)));
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 8);
    for length in line_lengths(&harness.shown(feature)) {
        assert!((length - 10.0 * 2f64.sqrt()).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw square"));

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the square was extruded");
    assert!((harness.body_volume(extrude) - 2000.0).abs() < 1.0);
}

#[test]
fn a_polygons_sides_are_typed_as_a_count_or_scrubbed_with_shift_and_the_pointer() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::G);
    harness.click_at(Point2::new(20.0, 20.0));
    harness.point_at(Point2::new(35.0, 20.0));
    assert!(harness.shows("R 15.00 mm   6 sides"));

    type_point(&mut harness, "9 sides");
    harness.point_at(Point2::new(35.0, 20.0));
    assert!(harness.shows("R 15.00 mm   9 sides"));

    type_point(&mut harness, "2 sides");
    assert!(harness.shows("A polygon needs at least three sides"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    type_point(&mut harness, "65 sides");
    assert!(harness.shows("A polygon has at most 64 sides"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    harness.point_at(Point2::new(35.0, 20.0));
    let start = harness.on_screen(Point2::new(35.0, 20.0));
    harness.hold(Modifiers::SHIFT);
    harness.point_at(Point2::new(35.0, 20.0));
    for (offset, sides) in [(48.0, 11), (-72.0, 6), (-96.0, 5), (150.0, 15)] {
        harness
            .events
            .push(Event::PointerMoved(start + egui::vec2(offset, 0.0)));
        harness.frame();
        assert!(
            harness.shows(&format!("R 15.00 mm   {sides} sides")),
            "{offset}"
        );
    }
    harness.hold(Modifiers::NONE);
    harness.point_at(Point2::new(30.0, 20.0));
    assert!(harness.shows("R 10.00 mm   15 sides"));
}

fn refused(harness: &mut Harness, text: &str, refusal: Refusal) {
    type_point(harness, text);
    assert!(harness.shows(refusal.reason()), "{text}");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
}

fn with_type_hint(hint: &str, keys: &str) -> String {
    format!("{hint}   {keys}   Type x, y or length < angle for an exact point")
}

fn radii(sketch: &Sketch, kind: &str) -> Vec<f64> {
    let mut radii: Vec<f64> = entities_of_kind(sketch, kind)
        .into_iter()
        .map(|curve| match kind {
            "Circle" => sketch.circle(curve).unwrap().1,
            _ => sketch.arc(curve).unwrap().radius,
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    radii
}

fn close(found: &[f64], expected: &[f64]) -> bool {
    found.len() == expected.len()
        && found
            .iter()
            .zip(expected)
            .all(|(found, expected)| (found - expected).abs() < DRAWN)
}

#[test]
fn pressing_a_shape_key_again_cycles_its_ways_of_drawing_and_each_is_remembered() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();

    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    assert!(harness.shows_hint(&with_type_hint(
        "Rectangle from two corners   R: from its centre",
        "Esc: back to Select"
    )));
    harness.use_tool(Key::R);
    assert_eq!(harness.tool(), Some(Tool::Rectangle));
    assert!(harness.shows("Click the rectangle's centre"));
    assert!(harness.shows_hint(&with_type_hint(
        "Rectangle from its centre   R: from three points",
        "Esc: back to Select"
    )));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click where the rectangle's first side starts"));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's first corner"));
    harness.use_tool(Key::R);
    harness.use_tool(Key::L);
    harness.use_tool(Key::C);
    assert!(harness.shows("Click the circle's centre"));
    harness.use_tool(Key::R);
    assert!(harness.shows("Click the rectangle's centre"));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Rectangle),
        Some(ShapeMode::Rectangle(RectangleMode::Center))
    );

    harness.use_tool(Key::U);
    harness.click_at(Point2::new(10.0, 10.0));
    assert!(harness.workspace.viewport.is_drawing());
    harness.use_tool(Key::U);
    assert!(!harness.workspace.viewport.is_drawing());
    assert!(harness.shows("Click the slot's centre"));
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);
    assert!(harness.shows("Click where a side of the hexagon starts"));
    harness.use_tool(Key::G);
    assert!(harness.shows("Click the hexagon's centre"));
    assert!(harness.shows_hint(&with_type_hint(
        "Polygon from its centre and a corner   G: from its centre and a side's middle   ] or \
         [: more or fewer sides   Shift: move sideways to set the sides",
        "Esc: back to Select"
    )));
    assert!(harness.shows("6 sides: set the sides"));
}

#[test]
fn every_way_of_drawing_is_a_command_in_the_palette_and_in_a_menu_on_its_button() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    for mode in ShapeMode::ALL {
        assert!(
            offer(&harness, Command::ShapeMode(mode))
                .availability
                .is_ok()
        );
    }

    run_from_palette(&mut harness, "draw circle through three points");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Circle));
    assert!(harness.shows("Click a first point on the circle"));
    harness.hover("Circle");
    assert!(
        harness.shows("Draw a circle through three points on it (C)\nC again: from its centre")
    );

    harness.hover_button(&sketch_toolbar::modes_label(Tool::Slot));
    assert!(harness.shows(&sketch_toolbar::modes_label(Tool::Slot)));
    assert!(!harness.shows(
        "Draw a slot from the centres of its round ends and its width (U)\nU again: from its centre"
    ));
    harness.hover("Slot");
    assert!(harness.shows(
        "Draw a slot from the centres of its round ends and its width (U)\nU again: from its centre"
    ));
    harness.click_button(&sketch_toolbar::modes_label(Tool::Slot));
    assert!(harness.shows("From the centres of its ends"));
    assert!(harness.shows("From its centre"));
    harness.click("Along an arc");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Slot));
    assert!(harness.shows("Click the centre of the arc the slot follows"));
    assert!(!harness.shows("From the centres of its ends"));
    assert_eq!(
        harness.workspace.editing.modes().of(Tool::Circle),
        Some(ShapeMode::Circle(CircleMode::ThreePoints))
    );

    harness.select([]);
    harness.frame();
    assert_readable(&harness, "The sketch bar with its mode menus");

    harness.click("Sketch");
    harness.hover("Ways to draw shapes");
    harness.click("Draw polygon from one side");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Polygon));
    assert!(harness.shows("Click where a side of the hexagon starts"));
}

#[test]
fn a_rectangle_from_its_centre_stays_centred_on_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click a corner of the rectangle"));
    refused(&mut harness, "15, 0", Refusal::Rectangle);
    type_point(&mut harness, "15, 10");
    harness.settle();

    let sketch = harness.sketch(feature);
    let mut lengths = line_lengths(&harness.shown(feature));
    lengths.sort_by(f64::total_cmp);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Horizontal").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Vertical").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 5);
    assert!(close(&lengths, &[20.0, 20.0, 30.0, 30.0]), "{lengths:?}");
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));
    assert!(harness.shows("Click the rectangle's centre"));
}

#[test]
fn a_rectangle_from_three_points_turns_with_its_first_side_and_stays_square() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);
    harness.use_tool(Key::R);

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where its first side ends"));
    refused(&mut harness, "10, 0", Refusal::RectangleSide);
    type_point(&mut harness, "40, 40");
    assert!(harness.shows("Click to set the rectangle's width"));
    refused(&mut harness, "@3, 4", Refusal::RectangleWidth);
    type_point(&mut harness, "@-8, 6");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let corners: Vec<Point2> = entities_of_kind(&shown, "Line")
        .into_iter()
        .map(|line| shown.line_endpoints(line).unwrap().0)
        .collect();
    let expected = [
        Point2::new(10.0, 0.0),
        Point2::new(40.0, 40.0),
        Point2::new(32.0, 46.0),
        Point2::new(2.0, 6.0),
    ];
    assert_eq!(constraints_of_kind(sketch, "Perpendicular").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Parallel").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 4);
    assert_eq!(corners.len(), expected.len());
    for (corner, expected) in corners.iter().zip(expected) {
        assert!(near(*corner, expected), "{corner} is not {expected}");
    }
    assert!(harness.shows("5 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw rectangle"));

    harness.click_at(Point2::new(10.0, -30.0));
    harness.click_at(Point2::new(40.0, -29.9));
    harness.click_at(Point2::new(30.0, -20.0));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("9 degrees of freedom left"));
}

#[test]
fn a_circle_through_the_ends_of_a_diameter_is_centred_between_them() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);
    type_point(&mut harness, "10, 10");
    type_point(&mut harness, "30, 10");
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);

    assert!(harness.shows("Click one end of the circle's diameter"));
    type_point(&mut harness, "10, 10");
    assert!(harness.shows("Click the other end of the diameter"));
    refused(&mut harness, "10, 10", Refusal::CircleDiameter);
    type_point(&mut harness, "30, 10");
    harness.settle();
    let joined = harness.shows("4 degrees of freedom left");
    type_point(&mut harness, "50, 0");
    type_point(&mut harness, "50, 30");
    harness.settle();

    let sketch = harness.sketch(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let (center, radius) = sketch.circle(circles[0]).unwrap();
    let (free_center, free_radius) = sketch.circle(circles[1]).unwrap();
    assert!(joined);
    assert!(near(center, Point2::new(20.0, 10.0)), "{center}");
    assert!((radius - 10.0).abs() < DRAWN);
    assert!(near(free_center, Point2::new(50.0, 15.0)), "{free_center}");
    assert!((free_radius - 15.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 1);
    assert!(harness.shows("7 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));
}

#[test]
fn a_circle_through_three_points_passes_through_each() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::P);
    type_point(&mut harness, "0, 10");
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);
    harness.use_tool(Key::C);

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click a second point on the circle"));
    refused(&mut harness, "10, 0", Refusal::CircleInLine);
    type_point(&mut harness, "0, 10");
    assert!(harness.shows("Click a third point on the circle"));
    refused(&mut harness, "-10, 20", Refusal::CircleInLine);
    type_point(&mut harness, "-6, -8");
    harness.settle();

    let sketch = harness.sketch(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let (center, radius) = harness.shown(feature).circle(circles[0]).unwrap();
    assert_eq!(circles.len(), 1);
    assert!(near(center, Point2::ZERO), "{center}");
    assert!((radius - 10.0).abs() < DRAWN, "{radius}");
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 1);
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw circle"));
    assert!(harness.shows("Click a first point on the circle"));
}

#[test]
fn a_polygon_sized_by_the_middle_of_a_side_keeps_a_circle_inside_touching_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::G);
    harness.use_tool(Key::G);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click the middle of a side of the hexagon"));
    refused(&mut harness, "0, 0", Refusal::PolygonSideMiddle);
    type_point(&mut harness, "0, -5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let circles = entities_of_kind(sketch, "Circle");
    let across_corners = 5.0 / (PI / 6.0).cos();
    assert_eq!(entities_of_kind(sketch, "Line").len(), 6);
    assert!(circles.iter().all(|circle| sketch.is_construction(*circle)));
    assert!(close(&radii(&shown, "Circle"), &[5.0, across_corners]));
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 5);
    for line in entities_of_kind(&shown, "Line") {
        let (start, end) = shown.line_endpoints(line).unwrap();
        assert!((start.midpoint(end).length() - 5.0).abs() < DRAWN);
    }
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw hexagon"));
}

#[test]
fn a_polygon_drawn_from_one_side_lies_to_its_left() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    for _ in 0..3 {
        harness.use_tool(Key::G);
    }
    harness.key(Key::OpenBracket, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "10, 0");
    assert!(harness.shows("Click where that side of the pentagon ends"));
    refused(&mut harness, "10, 0", Refusal::PolygonSide);
    type_point(&mut harness, "20, 0");
    harness.settle();

    let shown = harness.shown(feature);
    let (center, _) = shown.circle(entities_of_kind(&shown, "Circle")[0]).unwrap();
    assert_eq!(entities_of_kind(&shown, "Line").len(), 5);
    assert!(
        center.y > 0.0 && (center.x - 15.0).abs() < DRAWN,
        "{center}"
    );
    for length in line_lengths(&shown) {
        assert!((length - 10.0).abs() < DRAWN, "{length}");
    }
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw pentagon"));

    harness.click_at(Point2::new(40.0, -40.0));
    harness.click_at(Point2::new(55.0, -39.95));
    harness.settle();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(harness.shows("7 degrees of freedom left"));
}

#[test]
fn a_slot_from_its_centre_stays_symmetric_about_it() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::U);
    harness.use_tool(Key::U);

    type_point(&mut harness, "0, 0");
    assert!(harness.shows("Click the centre of one of the slot's ends"));
    refused(&mut harness, "0, 0", Refusal::SlotLength);
    type_point(&mut harness, "20, 0");
    assert!(harness.shows("Click to set the slot's width"));
    refused(&mut harness, "@0, 0", Refusal::SlotWidth);
    type_point(&mut harness, "@0, 5");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let centers: Vec<Point2> = entities_of_kind(&shown, "Arc")
        .into_iter()
        .map(|arc| shown.arc(arc).unwrap().center)
        .collect();
    assert!(close(&radii(&shown, "Arc"), &[5.0, 5.0]));
    assert!(
        centers
            .iter()
            .any(|center| near(*center, Point2::new(-20.0, 0.0)))
    );
    assert!(
        centers
            .iter()
            .any(|center| near(*center, Point2::new(20.0, 0.0)))
    );
    for length in line_lengths(&shown) {
        assert!((length - 40.0).abs() < DRAWN, "{length}");
    }
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 1);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    assert!(harness.shows("3 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw slot"));
}

#[test]
fn an_arc_slot_follows_its_arc_with_round_ends_tangent_to_both_sides() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    for _ in 0..3 {
        harness.use_tool(Key::U);
    }

    type_point(&mut harness, "0, 0");
    refused(&mut harness, "0, 0", Refusal::ArcSlotRadius);
    type_point(&mut harness, "20, 0");
    assert!(harness.shows("Click the centre of the slot's other end"));
    refused(&mut harness, "20, 0", Refusal::ArcSlotSweep);
    type_point(&mut harness, "0, 20");
    assert!(harness.shows("Click to set the slot's width"));
    refused(&mut harness, "@0, 0", Refusal::ArcSlotWidth);
    refused(&mut harness, "@0, 21", Refusal::ArcSlotWidth);
    type_point(&mut harness, "@0, 3");
    harness.settle();

    let sketch = harness.sketch(feature);
    let shown = harness.shown(feature);
    let outer = entities_of_kind(&shown, "Arc")
        .into_iter()
        .filter_map(|arc| shown.arc(arc))
        .find(|arc| (arc.radius - 23.0).abs() < DRAWN)
        .expect("the slot has an outer arc");
    assert!(close(&radii(&shown, "Arc"), &[3.0, 3.0, 17.0, 23.0]));
    assert!((outer.sweep - FRAC_PI_2).abs() < DRAWN, "{}", outer.sweep);
    assert!(outer.start_angle.abs() < DRAWN, "{}", outer.start_angle);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 4);
    assert_eq!(constraints_of_kind(sketch, "Coincident").len(), 5);
    assert!(harness.shows("4 degrees of freedom left"));
    assert_eq!(harness.model.undo_label(), Some("Draw arc slot"));

    type_point(&mut harness, "100, 0");
    type_point(&mut harness, "120, 0");
    harness.key(Key::X, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "100, 20");
    type_point(&mut harness, "@0, 3");
    harness.settle();
    let shown = harness.shown(feature);
    let long_way = entities_of_kind(&shown, "Arc")
        .into_iter()
        .filter_map(|arc| shown.arc(arc))
        .find(|arc| (arc.radius - 23.0).abs() < DRAWN && arc.center.x > 50.0)
        .expect("the reversed slot has an outer arc");
    assert!(
        (long_way.sweep - 3.0 * FRAC_PI_2).abs() < DRAWN,
        "{}",
        long_way.sweep
    );

    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the arc slots were extruded");
    let slot_area = |sweep: f64| sweep * (23.0 * 23.0 - 17.0 * 17.0) / 2.0 + PI * 9.0;
    let area = slot_area(FRAC_PI_2) + slot_area(3.0 * FRAC_PI_2);
    let volume = harness.body_volume(extrude);
    assert!((volume / (10.0 * area) - 1.0).abs() < 1e-2, "{volume}");
}

#[test]
fn a_typed_point_can_be_a_length_and_angle_or_a_length_toward_the_pointer() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "12");
    let alone_first = harness.shows(
        "Type x, y such as 10, 20, or a length and an angle such as 25 < 30; a length alone \
         needs a placed point to measure from",
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    type_point(&mut harness, "0, 0");
    type_point(&mut harness, "@10 < 90");
    type_point(&mut harness, "20 < 0");
    harness.point_at(Point2::new(20.0, 30.0));
    type_point(&mut harness, "5");
    type_point(&mut harness, "@4 < (pi / 2) rad");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let ends: Vec<Point2> = entities_of_kind(sketch, "Line")
        .into_iter()
        .map(|line| sketch.line_endpoints(line).unwrap().1)
        .collect();
    let expected = [
        Point2::new(0.0, 10.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 9.0),
    ];
    assert!(alone_first);
    assert_eq!(ends.len(), expected.len());
    for (end, expected) in ends.iter().zip(expected) {
        assert!(near(*end, expected), "{end} is not {expected}");
    }
}

#[test]
fn a_spline_takes_clicked_control_points_until_enter() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::S);
    assert!(harness.shows("Click the spline's first control point"));
    let clicked = [
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 30.0),
        Point2::new(30.0, 10.0),
        Point2::new(40.0, 30.0),
    ];
    for point in clicked {
        harness.click_at(point);
    }
    harness.click_at(Point2::new(45.0, -30.0));
    harness.key(Key::Backspace, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Click the next control point"));
    assert_eq!(harness.sketch(feature).entities().len(), 0);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let spline = entities_of_kind(sketch, "Spline")[0];
    let Some(Entity::Spline { control_points }) = sketch.entity(spline) else {
        panic!("expected a spline");
    };
    assert_eq!(control_points.len(), 4);
    for (id, expected) in control_points.iter().zip(clicked) {
        assert!(near(sketch.point(*id).unwrap(), expected));
    }
    assert_eq!(harness.model.undo_label(), Some("Draw spline"));
    assert!(harness.shows("Click the spline's first control point"));
}

#[test]
fn escape_cancels_the_shape_in_progress_then_returns_to_select() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let before = harness.document().clone();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.point_at(Point2::new(30.0, 30.0));
    assert!(harness.shows("Click the opposite corner"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(*harness.document(), before);
    assert_eq!(harness.tool(), Some(Tool::Rectangle));
    assert!(harness.shows("Click the rectangle's first corner"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
    assert_eq!(harness.editing(), Some(feature));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.editing(), None);
    assert_eq!(*harness.document(), before);
}

#[test]
fn tool_buttons_name_their_shortcuts() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.hover("Line");
    assert!(harness.shows("Draw connected lines, one click per corner (L)"));
    harness.click("Circle");
    assert_eq!(harness.tool(), Some(Tool::Circle));
    harness.hover_button("Parallel");
    assert!(harness.shows("Make lines parallel. Select two or more lines (Shift+P)"));
}

fn sketch_bar(harness: &Harness) -> Rect {
    egui::containers::panel::PanelState::load(&harness.context, Id::new("sketch-toolbar"))
        .expect("the sketch bar is shown")
        .outer_rect
}

fn sketch_bar_problems(harness: &Harness, visible: Rect) -> Vec<String> {
    let bar = sketch_bar(harness);
    let texts: Vec<&(String, Rect)> = harness
        .texts
        .iter()
        .zip(&harness.text_clips)
        .filter(|((_, rect), clip)| {
            bar.contains(rect.center()) && clip.intersect(*rect).is_positive()
        })
        .map(|(text, _)| text)
        .collect();
    let overlapping = texts.iter().enumerate().flat_map(|(index, (first, a))| {
        texts[index + 1..]
            .iter()
            .filter(move |(_, b)| a.shrink(0.5).intersects(b.shrink(0.5)))
            .map(move |(second, _)| format!("{first:?} overlaps {second:?}"))
    });
    let outside = texts
        .iter()
        .filter(|(_, rect)| !visible.expand(0.5).contains_rect(*rect))
        .map(|(shown, _)| format!("{shown:?} is cut off"));
    overlapping.chain(outside).collect()
}

fn lowest_in(harness: &Harness, area: Rect, label: &str) -> Pos2 {
    harness
        .texts
        .iter()
        .filter(|(shown, rect)| shown == label && area.contains(rect.center()))
        .map(|(_, rect)| rect.center())
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap_or_else(|| panic!("'{label}' is not in {area:?}"))
}

fn in_sketch_bar(harness: &Harness, label: &str) -> Pos2 {
    lowest_in(harness, sketch_bar(harness), label)
}

fn ribbon(harness: &Harness) -> Rect {
    egui::containers::panel::PanelState::load(&harness.context, Id::new("toolbar"))
        .expect("the ribbon is shown")
        .outer_rect
}

const RIBBON_GROUPS: [(&str, &[&str]); 7] = [
    ("History", &["Undo", "Redo"]),
    ("Sketch", &[toolbar::NEW_SKETCH_LABEL]),
    ("Solid", &["Extrude", "Revolve", "Hole"]),
    (
        "Modify",
        &[
            "Fillet",
            "Chamfer",
            "Shell",
            "Combine",
            "Move body",
            "Mirror body",
            "Scale body",
        ],
    ),
    ("Pattern", &["Linear pattern", "Circular pattern"]),
    ("Reference", &[toolbar::PLANE_LABEL, toolbar::AXIS_LABEL]),
    (
        "Inspect",
        &[toolbar::MEASURE_LABEL, toolbar::INTERFERENCE_LABEL],
    ),
];

fn ribbon_layout(harness: &Harness) -> Vec<Pos2> {
    let bar = ribbon(harness);
    RIBBON_GROUPS
        .iter()
        .flat_map(|(caption, buttons)| std::iter::once(caption).chain(buttons.iter()))
        .map(|label| lowest_in(harness, bar, label))
        .collect()
}

#[test]
fn every_ribbon_group_is_captioned_under_its_buttons() {
    let mut harness = Harness::new();
    harness.frame();

    let bar = ribbon(&harness);
    for (caption, buttons) in RIBBON_GROUPS {
        let below = lowest_in(&harness, bar, caption);
        let rects: Vec<Rect> = buttons
            .iter()
            .map(|button| harness.button_rect(button))
            .collect();
        let left = rects
            .iter()
            .map(|rect| rect.left())
            .fold(f32::MAX, f32::min);
        let right = rects
            .iter()
            .map(|rect| rect.right())
            .fold(f32::MIN, f32::max);

        assert!(below.x > left && below.x < right, "{caption}");
        assert!(
            rects.iter().all(|rect| below.y > rect.bottom()),
            "{caption}"
        );
    }
    harness.hover("Undo");
    assert_eq!(harness.count_shown("Undo"), 2);
    assert!(harness.shows("Nothing to undo"));
}

#[test]
fn the_ribbon_keeps_its_height_and_groups_while_choosing_a_plane() {
    let mut harness = Harness::new();
    harness.frame();
    let bar = ribbon(&harness);
    let layout = ribbon_layout(&harness);

    harness.click(toolbar::NEW_SKETCH_LABEL);
    harness.frame();
    assert!(harness.workspace.editing.is_choosing_plane());
    assert_eq!(ribbon(&harness), bar);
    assert_eq!(ribbon_layout(&harness), layout);

    harness.click(toolbar::NEW_SKETCH_LABEL);
    harness.frame();
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert_eq!(ribbon(&harness), bar);
}

#[test]
fn the_arc_button_offers_each_way_to_draw_an_arc_and_keeps_the_last_chosen() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    let bar = sketch_bar(&harness);

    harness.click_button(sketch_toolbar::ARC_WAYS_LABEL);
    for tool in sketch_toolbar::ARC_TOOLS {
        assert!(harness.shows(tool.label()), "{tool:?}");
    }
    harness.click("Tangent arc");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::TangentArc));

    harness.use_tool(Key::L);
    assert_eq!(harness.tool(), Some(Tool::Line));
    harness.click_button(sketch_toolbar::ARC_LABEL);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    assert!(!harness.shows("3-point arc"));

    harness.use_tool_with(Key::A, Modifiers::ALT);
    assert_eq!(harness.tool(), Some(Tool::ThreePointArc));
    harness.use_tool(Key::A);
    assert_eq!(harness.tool(), Some(Tool::Arc));
    harness.use_tool(Key::T);
    assert_eq!(harness.tool(), Some(Tool::TangentArc));
    assert_eq!(sketch_bar(&harness), bar);
}

fn sketch_bar_buttons() -> Vec<String> {
    Tool::ALL
        .into_iter()
        .filter(|tool| !sketch_toolbar::ARC_TOOLS.contains(tool))
        .map(Tool::label)
        .chain(ConstraintTool::ALL.map(ConstraintTool::label))
        .chain([
            sketch_toolbar::ARC_LABEL,
            sketch_toolbar::ARC_WAYS_LABEL,
            sketch_toolbar::CONSTRUCTION_LABEL,
            sketch_toolbar::MOVE_LABEL,
            sketch_toolbar::SELECT_ALL_LABEL,
            sketch_toolbar::DELETE_LABEL,
            sketch_toolbar::FINISH_LABEL,
        ])
        .map(str::to_owned)
        .chain(
            [Tool::Rectangle, Tool::Circle, Tool::Polygon, Tool::Slot]
                .map(sketch_toolbar::modes_label),
        )
        .collect()
}

#[test]
fn the_canvas_says_how_far_apart_the_grid_lines_are_and_names_it_for_screen_readers() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    let shown: Vec<&str> = harness
        .texts
        .iter()
        .map(|(text, _)| text.as_str())
        .filter(|text| text.starts_with("Grid "))
        .collect();
    let [label] = shown.as_slice() else {
        panic!("the grid spacing should show once: {shown:?}");
    };

    assert!(label.ends_with(" mm"), "{label}");
    assert!(
        harness
            .accessible
            .iter()
            .any(|(_, node)| { node.role() == Role::Label && node.value() == Some(label) })
    );
}

#[test]
fn at_200_percent_the_side_panels_leave_the_view_a_usable_width() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.workspace.measure.toggle();
    harness.workspace.interference.toggle();
    for _ in 0..4 {
        harness.frame();
    }

    let window = SCREEN.width() / 2.0;
    let view = harness.workspace.viewport.rect().unwrap().width();

    assert!(harness.workspace.measure.open && harness.workspace.interference.open);
    assert!(view >= window * 0.39, "{view} of {window}");
}

#[test]
fn the_sketch_bar_fits_one_row_wraps_at_200_percent_and_names_every_button() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let base = edit_base_sketch(&mut harness);
    harness.frame();
    let line = entities_of_kind(harness.sketch(base), "Line")[0];

    let row = in_sketch_bar(&harness, "Draw").y;
    let bar = sketch_bar(&harness);
    let finish = harness.position_of(sketch_toolbar::FINISH_LABEL);
    assert!(harness.shows("Editing Base sketch"));
    assert_eq!(sketch_bar_problems(&harness, SCREEN), Vec::<String>::new());
    for caption in ["Select", "Modify", "Constrain", "Dimension"] {
        assert!(
            (in_sketch_bar(&harness, caption).y - row).abs() < 0.5,
            "{caption}"
        );
    }
    assert!(harness.position_of("Select").y < row);
    assert!(finish.x > in_sketch_bar(&harness, "Dimension").x);
    assert!(finish.y < row);
    assert_readable(&harness, "The sketch bar");
    for name in sketch_bar_buttons() {
        assert!(harness.accessible_named(Role::Button, &name), "{name}");
    }

    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);
    harness.frame();
    harness.frame();
    assert_eq!(sketch_bar(&harness), bar);
    let delete = harness.button_rect(sketch_toolbar::DELETE_LABEL);
    assert!(bar.contains_rect(delete));

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    harness.frame();
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);
    assert_eq!(sketch_bar_problems(&harness, visible), Vec::<String>::new());
    assert!(harness.button_rect("Parallel").top() > harness.button_rect("Point").bottom());
    assert!(!harness.shows("Constrain"));
    assert!(!harness.shows("Inspect"));
    assert!(harness.shows(sketch_toolbar::FINISH_LABEL));
    assert_readable(&harness, "The sketch bar at 200%");
    for name in sketch_bar_buttons() {
        assert!(harness.accessible_named(Role::Button, &name), "{name}");
    }
}

#[test]
fn the_sketch_bar_selects_everything_and_moves_it_to_a_typed_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let pickable = Pickable::SketchEntity {
        feature,
        entity: line,
    };

    harness.hover_button(sketch_toolbar::MOVE_LABEL);
    assert!(harness.shows(
        "Move the selected geometry to a typed position, or by a typed offset. Select sketch \
         geometry to move it (M)"
    ));
    harness.click_button(sketch_toolbar::SELECT_ALL_LABEL);
    harness.frame();
    assert!(harness.workspace.viewport.selection().contains(pickable));

    harness.click_button(sketch_toolbar::MOVE_LABEL);
    harness.frame();
    assert!(harness.shows(typed_point::MOVE_LABEL));
}

fn rectangle(sketch: &mut Sketch, min: Point2, max: Point2) {
    let corners = [
        min,
        Point2::new(max.x, min.y),
        max,
        Point2::new(min.x, max.y),
    ];
    for (index, corner) in corners.iter().enumerate() {
        sketch.add_line(*corner, corners[(index + 1) % 4]);
    }
}

fn distance_of(harness: &Harness, feature: FeatureId) -> String {
    let SolidFeature::Extrude(extrude) = harness.solid(feature) else {
        panic!("expected an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: caditor_document::ExtrudeEnd::Distance(distance),
        ..
    } = &extrude.extent
    else {
        panic!("expected a one-sided extent");
    };
    harness.document().expression_text(distance)
}

#[test]
fn extruding_a_drawn_rectangle_makes_a_shaded_body_that_follows_its_distance() {
    let mut harness = Harness::new();
    let sketch = harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();

    harness.click("Extrude");
    harness.settle();

    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the extrusion was created");
    assert_eq!(harness.model.undo_label(), Some("Create Extrude 1"));
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    assert_eq!(harness.solid(extrude).sketch(), sketch);
    assert_eq!(harness.solid(extrude).operation(), BodyOperation::NewBody);
    assert!((harness.body_volume(extrude) - 6000.0).abs() < 1.0);
    assert!(harness.shows("Click regions of the sketch to include or leave them out"));

    let built = harness.built_with_meshes(1);
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert_eq!(built.scene.translucent_meshes[0].mesh.face_count(), 6);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha < 1.0 && face.pick.is_some())
    );
    let pickables: Vec<Pickable> = built.picks.pickables().collect();
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Face { .. }))
            .count(),
        6
    );
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Edge { .. }))
            .count(),
        12
    );
    assert_eq!(
        pickables
            .iter()
            .filter(|pickable| matches!(pickable, Pickable::Region { .. }))
            .count(),
        1
    );
    assert!(built.everything.max().z >= 10.0);

    let field = Id::new(("solid-field", "distance", extrude));
    harness.type_into_field(field, "25 mm");
    assert_eq!(distance_of(&harness, extrude), "25 mm");
    harness.settle();
    assert!((harness.body_volume(extrude) - 15000.0).abs() < 1.0);

    harness.type_into_field(field, "-5 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO_OR_REVERSE));
    assert_eq!(distance_of(&harness, extrude), "25 mm");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(distance_of(&harness, extrude), "10 mm");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let closed = harness.built();
    assert!(
        closed
            .picks
            .pickables()
            .all(|pickable| !matches!(pickable, Pickable::Region { .. }))
    );
    assert_eq!(closed.scene.meshes.len(), 1);
    assert!(closed.scene.translucent_meshes.is_empty());
}

#[test]
fn a_flat_rectangle_is_refused_with_the_reason() {
    let mut harness = Harness::new();
    let sketch = harness.draw_on_new_sketch();

    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 10.0));
    harness.settle();

    assert_eq!(harness.sketch(sketch).entities().len(), 0);
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("A rectangle needs its corners apart in both directions.")
    );
}

#[test]
fn both_distances_of_a_two_sided_extrusion_must_be_above_zero() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();

    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.click("One side");
    harness.click("Two sides");
    harness.settle();
    let SolidFeature::Extrude(extruded) = harness.solid(extrude) else {
        panic!("expected an extrusion");
    };
    assert!(matches!(extruded.extent, ExtrudeExtent::TwoSides { .. }));
    let before = harness.solid(extrude).clone();
    harness.type_into_field(Id::new(("solid-field", "forward", extrude)), "0 mm");
    let refused_forward = harness.shows(crate::feature_fields::ABOVE_ZERO);
    harness.type_into_field(Id::new(("solid-field", "backward", extrude)), "-3 mm");
    let refused_backward = harness.shows(crate::feature_fields::ABOVE_ZERO);

    assert!(refused_forward);
    assert!(refused_backward);
    assert_eq!(harness.solid(extrude), &before);
}

#[test]
fn clicking_a_hole_region_adds_it_and_a_face_names_the_feature_that_made_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    rectangle(
        &mut sketch,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 20.0),
    );
    let sketch = harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    assert_eq!(harness.solid(extrude).sketch(), sketch);
    assert!((harness.body_volume(extrude) - 15000.0).abs() < 1.0);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let hole = harness
        .built()
        .picks
        .pickables()
        .filter_map(|pickable| match pickable {
            Pickable::Region { region, .. } => Some(region),
            _ => None,
        })
        .find(|region| {
            let RegionChoice::All = harness.solid(extrude).regions() else {
                return false;
            };
            let regions = crate::selection::swept_regions(
                harness.document(),
                harness.model.evaluation(),
                extrude,
            )
            .unwrap()
            .1;
            regions
                .iter()
                .any(|candidate| candidate.region.key() == *region && !candidate.even_depth)
        })
        .expect("the hole is a region");
    harness.click_pickable(
        Plane::XY,
        Point2::new(15.0, 15.0),
        Pickable::Region {
            feature: extrude,
            region: hole,
        },
    );
    harness.settle();

    let RegionChoice::Chosen(references) = harness.solid(extrude).regions() else {
        panic!("the regions were not chosen");
    };
    assert_eq!(references.len(), 2);
    assert!(
        references
            .iter()
            .all(|reference| !reference.boundary().is_empty() && reference.anchor().is_some())
    );
    assert_eq!(
        harness.model.undo_label(),
        Some("Choose regions of Extrude 1")
    );
    assert!((harness.body_volume(extrude) - 16000.0).abs() < 1.0);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable");
    harness.select([top]);
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    let Pickable::Face { body, .. } = top else {
        panic!("expected a face");
    };
    assert_eq!(body, extrude);
}

#[test]
fn revolving_about_a_selected_line_uses_it_as_the_axis() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XZ);
    rectangle(&mut sketch, Point2::new(10.0, 0.0), Point2::new(20.0, 10.0));
    let axis = sketch.add_line(Point2::new(0.0, -5.0), Point2::new(0.0, 15.0));
    let sketch = harness.add_sketch(sketch);
    harness.select([Pickable::SketchEntity {
        feature: sketch,
        entity: axis,
    }]);

    harness.click("Revolve");
    harness.settle();

    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolution is open");
    assert_eq!(harness.solid(revolve).axis_line(), Some(axis));
    let expected = std::f64::consts::PI * (20.0f64.powi(2) - 10.0f64.powi(2)) * 10.0;
    assert!((harness.body_volume(revolve) - expected).abs() / expected < 0.01);
    assert!(harness.shows("Axis"));
}

fn extruded_plate(harness: &mut Harness) -> (FeatureId, Pickable) {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable");
    (extrude, top)
}

fn attached_body(harness: &Harness, sketch: FeatureId) -> Option<FeatureId> {
    harness
        .document()
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body())
}

fn plane_height(harness: &Harness, sketch: FeatureId) -> f64 {
    harness.shown(sketch).plane().origin().z
}

#[test]
fn the_constraint_tools_work_out_their_candidates_when_the_selection_or_sketch_changes_not_every_frame()
 {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    let second = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(10.0, 6.0));
    let id = harness.add_sketch(sketch);
    harness.settle();
    harness.perform(Action::Editing(EditingCommand::Enter(id)));
    let pick = |entity| Pickable::SketchEntity {
        feature: id,
        entity,
    };
    harness.select([pick(first), pick(second)]);
    harness.frame();
    let computed = harness.workspace.panels.constraint_offers.computations();
    assert!(computed > 0);

    for _ in 0..5 {
        harness.frame();
    }
    assert_eq!(
        harness.workspace.panels.constraint_offers.computations(),
        computed
    );

    harness.select([pick(first)]);
    harness.frame();
    assert_eq!(
        harness.workspace.panels.constraint_offers.computations(),
        computed + 1
    );
}

#[test]
fn what_the_selection_offers_is_worked_out_when_it_or_the_model_changes_not_every_frame() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    let computed = harness.workspace.selection_offers.computations();

    for _ in 0..5 {
        harness.frame();
    }
    assert_eq!(harness.workspace.selection_offers.computations(), computed);
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    harness.hover("Shell");
    assert!(harness.shows_containing(&format!(
        "{} (1 face open)",
        crate::shell_tools::DESCRIPTION
    )));

    harness.select([]);
    harness.frame();
    assert_eq!(
        harness.workspace.selection_offers.computations(),
        computed + 1
    );
    assert!(harness.shows("Nothing selected"));

    let before = harness.workspace.selection_offers.computations();
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.workspace.selection_offers.computations() > before);
}

#[test]
fn a_large_selection_is_counted_whole_but_described_and_measured_only_in_part() {
    let mut harness = Harness::new();
    let mut lines = Sketch::new(Plane::XY);
    let entities: Vec<EntityId> = (0..20)
        .map(|index| {
            let y = f64::from(index) * 5.0;
            lines.add_line(Point2::new(0.0, y), Point2::new(10.0, y))
        })
        .collect();
    let feature = harness.add_sketch(lines);
    harness.frame();

    harness.select(entities.iter().map(|entity| Pickable::SketchEntity {
        feature,
        entity: *entity,
    }));
    harness.frame();
    let offers = harness
        .workspace
        .selection_offers
        .refresh(&harness.model, harness.workspace.viewport.selection())
        .clone();

    assert_eq!(offers.selected, 20);
    assert_eq!(offers.described.len(), crate::offers::MAX_DESCRIBED);
    assert!(harness.shows("20 items selected"));
}

#[test]
fn a_use_selected_offer_is_worked_out_once_per_selection_and_model_change() {
    let harness = Harness::new();
    let ctx = egui::Context::default();
    let feature = feature_named(&harness, "Base sketch");
    let reference = (feature, crate::reference_picking::Slot::MirrorPlane);
    let mut computed = 0;
    let mut offer = |selection: &crate::selection::Selection| {
        crate::feature_fields::offered_change(&ctx, &harness.model, selection, reference, || {
            computed += 1;
            Err("nothing usable".to_owned())
        })
    };
    let mut selection = crate::selection::Selection::default();
    selection.replace_with(Pickable::Origin);

    let first = offer(&selection);
    let again = offer(&selection);
    selection.clear();
    let cleared = offer(&selection);

    assert_eq!(first, Err("nothing usable".to_owned()));
    assert_eq!(again, first);
    assert_eq!(cleared, first);
    assert_eq!(computed, 2);
}

#[test]
fn the_interference_report_is_rebuilt_only_when_its_inputs_or_findings_change() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    add_peg(&mut harness);
    let selection = crate::selection::Selection::default();
    let mut interference = crate::interference::Interference::default();

    let mut report = interference
        .refresh(&harness.model, &selection, None)
        .expect("the first refresh reports");
    let deadline = Instant::now() + FILE_TIMEOUT;
    while report.is_checking() {
        assert!(Instant::now() < deadline, "the check never finished");
        std::thread::sleep(Duration::from_millis(2));
        if let Some(newer) = interference.refresh(&harness.model, &selection, None) {
            report = newer;
        }
    }

    assert_eq!(report.checked(), 1);
    assert!(
        interference
            .refresh(&harness.model, &selection, None)
            .is_none()
    );
    assert!(
        interference
            .refresh(&harness.model, &selection, None)
            .is_none()
    );
}

#[test]
fn a_file_dialog_left_open_says_so_and_can_be_stopped_from_the_window() {
    let mut harness = Harness::new();
    *harness.dialogs.held.lock() = Some(Vec::new());

    harness.perform(Action::File(FileCommand::Import { into: None }));
    harness.frame();
    let waiting = harness.shows("Waiting for the file dialog…");
    let blocked = harness.files.is_blocking();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let freed = !harness.files.is_blocking() && !harness.files.is_importing();
    let late = harness.dialogs.held.lock().take().unwrap_or_default();
    for respond in late {
        respond(Ok(Some(PathBuf::from("/tmp/late.step"))));
    }
    harness.frame();
    harness.frame();

    assert!(waiting);
    assert!(blocked);
    assert!(freed);
    assert!(!harness.files.is_picking());
    assert!(!harness.files.is_importing());
    assert!(!harness.shows("Waiting for the file dialog…"));
}

#[test]
fn a_sketch_started_on_a_selected_face_follows_it_when_the_body_changes() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.hover("New sketch");
    assert!(harness.shows_containing(
        "Start a sketch on the selected face; it follows the face when the model changes"
    ));
    harness.click("New sketch");
    harness.settle();

    let sketch = harness.editing().expect("the new sketch is edited");
    assert_eq!(harness.model.undo_label(), Some("Create Sketch 1"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
    assert!(harness.shows("Lies on Extrude 1 end face"));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    let boss = harness
        .workspace
        .editing
        .solid()
        .expect("the second extrusion is open");
    assert_eq!(
        harness.solid(boss).operation(),
        BodyOperation::Remove(extrude)
    );
    assert!((harness.body_volume(extrude) - (16000.0 - 2000.0)).abs() < 1.0);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(Id::new(("solid-field", "distance", extrude)), "25 mm");
    harness.settle();
    assert_eq!(plane_height(&harness, sketch), 25.0);
    assert!((harness.body_volume(extrude) - (40000.0 - 2000.0)).abs() < 1.0);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_open_cut_shows_the_body_solid_and_only_the_material_it_removes_see_through() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("New sketch");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    let cut = harness.workspace.editing.solid().expect("the cut is open");
    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(extrude)
    );

    let open = harness.built_with_overlays(1);

    assert_eq!(open.scene.meshes.len(), 1);
    assert!(open.scene.translucent_meshes.is_empty());
    assert!(
        open.scene.meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha >= 1.0 && face.pick.is_some())
    );
    assert_eq!(open.scene.overlay_meshes[0].mesh.face_count(), 6);
    assert!(
        open.scene.overlay_meshes[0]
            .faces
            .iter()
            .all(|face| face.color.alpha < 1.0 && face.pick.is_none())
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let closed = harness.built();
    assert!(closed.scene.overlay_meshes.is_empty());
    assert_eq!(closed.scene.meshes.len(), 1);
}

fn plate_on_screen(harness: &Harness) -> (Pos2, Pos2) {
    let viewport = &harness.workspace.viewport;
    let corners: Vec<Pos2> = [0.0, 10.0]
        .into_iter()
        .flat_map(|height| {
            let plane =
                Plane::from_frame(Point3::new(0.0, 0.0, height), Vector3::Z, Vector3::X).unwrap();
            [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]
                .map(|(x, y)| viewport.screen_position(plane, Point2::new(x, y)).unwrap())
        })
        .collect();
    let low = corners
        .iter()
        .fold(Pos2::new(f32::MAX, f32::MAX), |low, corner| {
            low.min(*corner)
        });
    let high = corners
        .iter()
        .fold(Pos2::new(f32::MIN, f32::MIN), |high, corner| {
            high.max(*corner)
        });
    (low, high)
}

fn selected_kinds(harness: &Harness) -> (usize, usize, usize) {
    let selection = harness.workspace.viewport.selection();
    let count = |test: fn(&Pickable) -> bool| selection.iter().filter(&test).count();
    (
        count(|pickable| matches!(pickable, Pickable::Face { .. })),
        count(|pickable| matches!(pickable, Pickable::Edge { .. })),
        count(|pickable| matches!(pickable, Pickable::Vertex { .. })),
    )
}

#[test]
fn a_box_dragged_over_the_model_selects_what_it_holds_or_touches_by_the_filter() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);

    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (3, 0, 0));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (0, 12, 0));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Vertices);
    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (0, 0, 8));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Everything);
    let middle = harness
        .workspace
        .viewport
        .screen_position(
            Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
            Point2::new(20.0, 20.0),
        )
        .unwrap();
    drag_screen(
        &mut harness,
        middle + egui::vec2(5.0, 5.0),
        middle - egui::vec2(5.0, 5.0),
    );
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![top]
    );
}

#[test]
fn a_display_style_hides_the_faces_or_the_edges_but_keeps_what_is_left_pickable() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.frame();
    let edge_alphas = |harness: &mut Harness| -> Vec<f32> {
        let built = harness.built();
        let edges: Vec<_> = built
            .picks
            .pickables()
            .filter(|pickable| matches!(pickable, Pickable::Edge { .. }))
            .filter_map(|pickable| built.picks.id_of(pickable))
            .collect();
        built
            .scene
            .batches
            .iter()
            .flat_map(|batch| batch.lines.iter())
            .filter(|line| line.pick.is_some_and(|pick| edges.contains(&pick)))
            .map(|line| line.color.alpha)
            .collect()
    };
    let faces_pickable = |harness: &mut Harness| {
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| matches!(pickable, Pickable::Face { .. }))
    };
    assert_eq!(
        harness.workspace.viewport.style(),
        DisplayStyle::ShadedWithEdges
    );
    assert_eq!(harness.built().scene.meshes.len(), 1);
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));

    run_from_palette(&mut harness, "shaded without edges");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Shaded);
    assert_eq!(harness.built().scene.meshes.len(), 1);
    let hidden = edge_alphas(&mut harness);
    assert!(!hidden.is_empty() && hidden.iter().all(|alpha| *alpha == 0.0));
    assert!(faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "wireframe");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Wireframe);
    assert!(harness.built().scene.meshes.is_empty());
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));
    assert!(!faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::XRay);
    let built = harness.built();
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| face.pick.is_none() && face.color.alpha < 1.0)
    );
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));
    assert!(!faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "hidden lines removed");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::HiddenLine);
    let built = harness.built();
    assert!(built.scene.meshes.is_empty());
    assert!(built.scene.translucent_meshes.is_empty());
    assert_eq!(built.scene.flat_meshes.len(), 1);
    let first = built.scene.flat_meshes[0].faces[0].color;
    assert!(
        built.scene.flat_meshes[0]
            .faces
            .iter()
            .all(|face| face.color == first && face.pick.is_some())
    );
    assert!(first.red > 0.9 && first.green > 0.9 && first.blue > 0.9);
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha == 1.0));
    assert!(faces_pickable(&mut harness));
}

#[test]
fn a_body_is_renamed_selected_whole_and_removed_from_the_palette() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.frame();

    run_from_palette(&mut harness, "rename body");
    harness.frame();
    let focused = harness.focused() == Some(crate::body_appearance::name_field_id(plate));
    harness.type_text("Base plate");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let named = harness.document().body_name(plate) == Some("Base plate");
    let listed = harness.shows("Base plate");

    harness.select([top]);
    run_from_palette(&mut harness, "select the whole body");
    harness.frame();
    let faces = harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == plate))
        .count();

    run_from_palette(&mut harness, "remove body");
    harness.settle();
    let removed = harness.model.evaluation().body(plate).is_none();
    let told = harness.shows_containing("Removed Base plate with Remove 1");
    harness.perform(Action::Undo);
    harness.settle();

    assert!(focused);
    assert!(named);
    assert!(listed);
    assert_eq!(faces, 6);
    assert!(removed);
    assert!(told);
    assert!(harness.model.evaluation().body(plate).is_some());
}

#[test]
fn copy_body_makes_a_placed_copy_beside_the_original() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let volume = harness.body_volume(plate);
    harness.select([top]);
    harness.frame();

    run_from_palette(&mut harness, "copy body");
    harness.settle();
    let copy = harness.workspace.editing.solid().expect("the copy is open");
    harness.type_into_field(Id::new(("move-field", "offset", 1usize, copy)), "60 mm");
    harness.settle();

    let feature = harness.document().feature(copy).unwrap();
    assert_eq!(feature.name, "Copy 1");
    assert!(feature.makes_body());
    assert!(harness.shows(crate::move_panel::MAKE_A_COPY));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!((harness.body_volume(copy) - volume).abs() < 1e-6 * volume);
    let bounds = harness
        .model
        .evaluation()
        .body(copy)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().y - 60.0).abs() < 1e-6);
    let original = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(original.min().y.abs() < 1e-6);
}

#[test]
fn the_3d_view_tells_screen_readers_what_it_shows() {
    let mut harness = Harness::new();
    let (_, _) = extruded_plate(&mut harness);
    harness.context.enable_accesskit();
    harness.frame();
    let shown = harness.view_description().unwrap_or_default();

    let sketch = harness
        .document()
        .features()
        .find(|feature| feature.name == "Base sketch")
        .map(caditor_document::Feature::id)
        .unwrap();
    harness.edit(sketch);
    harness.frame();
    let editing = harness.view_description().unwrap_or_default();

    assert!(shown.starts_with("1 body shown: Extrude 1"), "{shown}");
    assert!(shown.contains("sketch"), "{shown}");
    assert!(editing.starts_with("Editing Base sketch: "), "{editing}");
    assert!(editing.contains("constraint"), "{editing}");
}

#[test]
fn a_selection_filter_makes_clicks_skip_everything_but_one_kind() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let edge = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .expect("an edge is pickable");
    harness.select([]);

    run_from_palette(&mut harness, "select edges only");
    harness.frame();
    assert_eq!(harness.workspace.viewport.filter(), SelectionFilter::Edges);
    assert!(harness.shows("Selecting edges only"));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.click_pickable(Plane::XY, Point2::new(0.0, 0.0), edge);
    assert!(harness.workspace.viewport.selection().contains(edge));

    harness.click("Selecting edges only");
    harness.frame();
    assert_eq!(
        harness.workspace.viewport.filter(),
        SelectionFilter::Everything
    );
    assert!(!harness.shows("Selecting edges only"));

    harness.select([]);
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().contains(top));
}

#[test]
fn an_extrusion_takes_a_start_offset_from_its_panel_and_zero_clears_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.settle();
    let start_of = |harness: &Harness| harness.solid(extrude).clone();
    let start = |solid: SolidFeature| match solid {
        SolidFeature::Extrude(extrude) => extrude.start,
        SolidFeature::Revolve(_) => panic!("expected an extrusion"),
    };
    let lowest = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(extrude)
            .and_then(|result| result.solid())
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| bounds.min().z)
            .unwrap()
    };
    assert!(harness.shows("Start offset"));
    assert_eq!(start(start_of(&harness)), None);
    assert!(lowest(&harness).abs() < 1e-9);

    harness.type_into_field(Id::new(("solid-field", "start", extrude)), "6 mm");
    harness.settle();
    assert_eq!(
        start(start_of(&harness)).and_then(|start| start
            .distance()
            .map(|e| harness.document().expression_text(e))),
        Some("6 mm".to_owned())
    );
    assert!((lowest(&harness) - 6.0).abs() < 1e-9);

    harness.type_into_field(Id::new(("solid-field", "start", extrude)), "0 mm");
    harness.settle();
    assert_eq!(start(start_of(&harness)), None);
    assert!(lowest(&harness).abs() < 1e-9);
}

#[test]
fn a_click_waits_for_the_pick_under_the_cursor_rather_than_using_an_old_one() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.hover_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.frame();

    harness.picks_held = true;
    let beside = harness
        .workspace
        .viewport
        .screen_position(Plane::XY, Point2::new(-30.0, 20.0))
        .unwrap();
    harness.events.push(Event::PointerMoved(beside));
    harness.press(beside);
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.picks_held = false;
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    assert!(harness.workspace.viewport.selection().contains(top));
    harness.picks_held = true;
    harness.events.push(Event::PointerMoved(beside));
    harness.press(beside);
    harness.picks_held = false;
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn clicking_a_flat_face_while_choosing_a_plane_starts_a_sketch_on_it() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([]);
    harness.click("New sketch");
    assert!(harness.workspace.editing.is_choosing_plane());
    assert!(harness.shows("Click a plane or a flat face to sketch on"));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    let sketch = harness.editing().expect("the new sketch is edited");
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
}

#[test]
fn the_tree_places_a_sketch_on_the_selected_face_and_detaches_it() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Add sketch");
    let mut loose = Sketch::new(Plane::XY);
    rectangle(&mut loose, Point2::new(5.0, 5.0), Point2::new(15.0, 15.0));
    let sketch = transaction.add_feature("Loose", FeatureKind::from(loose));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    harness.select([top]);
    harness.click_button("Show details of Loose");
    harness.click("Place on selected face");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Place Loose on a face"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);
    assert!(harness.shows("Lies on Extrude 1 end face"));

    harness.click("Detach");
    harness.settle();
    assert_eq!(
        harness.model.undo_label(),
        Some("Detach Loose from its face")
    );
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(harness.sketch(sketch).plane().origin().z, 10.0);
    assert!(!harness.shows("Lies on Extrude 1 end face"));

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(plane_height(&harness, sketch), 0.0);
}

fn top_edge_along_x(harness: &Harness, body: FeatureId, y: f64) -> caditor_kernel::EdgeName {
    let solid = harness.model.evaluation().body(body).unwrap();
    solid
        .edges()
        .find(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            (middle - caditor_geometry::Point3::new(20.0, y, 10.0)).length() < 1e-6
        })
        .map(|(_, edge)| edge.name())
        .expect("the plate has that top edge")
}

fn removed_about(harness: &Harness, body: FeatureId, expected: f64) -> bool {
    let removed = 16000.0 - harness.body_volume(body);
    (removed - expected).abs() < 0.1 * expected
}

fn blend_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Blend {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.blend())
        .unwrap()
}

#[test]
fn an_open_fillet_listing_long_edge_names_keeps_the_side_panel_width() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let view = harness
        .workspace
        .viewport
        .rect()
        .expect("the view is shown");

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    harness.let_animations_finish();
    let opened = harness
        .workspace
        .viewport
        .rect()
        .expect("the view is shown");

    assert!(harness.workspace.editing.solid().is_some());
    assert!(
        (opened.min.x - view.min.x).abs() < 0.5,
        "{view:?} became {opened:?}"
    );
}

#[test]
fn a_fillet_starts_from_the_selected_edge_and_takes_more_edges_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let back = top_edge_along_x(&harness, plate, 40.0);
    let spandrel = |radius: f64| (1.0 - std::f64::consts::PI / 4.0) * radius * radius;

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    let fillet = harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open");
    assert_eq!(harness.model.undo_label(), Some("Create Fillet 1"));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);
    assert!(removed_about(&harness, plate, 40.0 * spandrel(1.0)));
    assert!(harness.shows("Click edges to add them or leave them out"));
    assert!(harness.shows("Radius"));

    let built = harness.built_with_meshes(1);
    assert_eq!(built.scene.meshes.len(), 1);
    assert_eq!(built.scene.meshes[0].mesh.face_count(), 6);
    let blend_edges = built
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::BlendEdge { .. }))
        .count();
    assert_eq!(blend_edges, 12);

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::BlendEdge {
            feature: fillet,
            edge: back,
        },
    );
    harness.settle();
    assert_eq!(blend_of(&harness, fillet).edges.len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Add an edge to Fillet 1"));
    assert!(removed_about(&harness, plate, 80.0 * spandrel(1.0)));

    harness.type_into_field(Id::new(("blend-size", fillet)), "2 mm");
    harness.settle();
    assert!(removed_about(&harness, plate, 80.0 * spandrel(2.0)));

    harness.type_into_field(Id::new(("blend-size", fillet)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(fillet));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::BlendEdge {
            feature: fillet,
            edge: front,
        },
    );
    harness.settle();
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Leave an edge out of Fillet 1")
    );

    harness.click_leftmost("Fillet");
    harness.click_leftmost("Chamfer");
    harness.settle();
    assert_eq!(
        blend_of(&harness, fillet).kind,
        caditor_document::BlendKind::Chamfer
    );
    assert!(removed_about(&harness, plate, 40.0 * 2.0));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    let chamfer_face = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Fillet 1 face"
        })
        .expect("the chamfer face is pickable and named after its feature");
    assert!(matches!(chamfer_face, Pickable::Face { .. }));
}

#[test]
fn enter_confirms_a_fillet_whose_row_then_closes_and_choosing_in_the_view_keeps_the_selection() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let back = top_edge_along_x(&harness, plate, 40.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    let fillet = harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open");

    assert!(harness.shows("Radius"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert!(!harness.shows("Radius"));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 1);

    harness.select([Pickable::Edge {
        body: plate,
        edge: back,
    }]);
    harness.click_button("Show details of Fillet 1");
    harness.frame();
    harness.click("Choose in the view");
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), Some(fillet));
    assert_eq!(blend_of(&harness, fillet).edges.len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some("Add the selected edges to Fillet 1")
    );
    assert!(removed_about(
        &harness,
        plate,
        80.0 * (1.0 - std::f64::consts::PI / 4.0)
    ));
}

#[test]
fn a_fillet_lists_an_edge_split_by_an_earlier_cut_as_its_pieces() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Fillet");
    harness.settle();
    let fillet = harness
        .workspace
        .editing
        .solid()
        .expect("the fillet is open");
    let whole = crate::bodies::describe_edge(
        harness.document(),
        crate::bodies::input(harness.model.evaluation(), fillet).unwrap(),
        front,
    );
    let listed_whole = harness.shows(&whole);

    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut slot = Sketch::new(top);
    rectangle(&mut slot, Point2::new(18.0, -5.0), Point2::new(22.0, 45.0));
    let last = harness.document().features().len() + 1;
    let mut transaction = harness.document().transaction("Cut a slot");
    let sketch = transaction.add_feature("Slot sketch", FeatureKind::from(slot));
    transaction.add_feature(
        "Slot",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), true),
            operation: BodyOperation::Remove(plate),
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    transaction.edit(Edit::MoveFeature {
        id: fillet,
        index: last,
    });
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let input = crate::bodies::input(harness.model.evaluation(), fillet).unwrap();
    let pieces = blend_of(&harness, fillet).resolutions(&input.solid);
    let [caditor_document::Resolution::Pieces(pieces)] = pieces.as_slice() else {
        panic!("the slot splits the chosen edge in two: {pieces:?}");
    };
    let split = format!(
        "{}, split into 2 pieces",
        crate::bodies::describe_edge_id(harness.document(), input, pieces[0])
    );

    assert!(listed_whole);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert_eq!(pieces.len(), 2);
    assert!(harness.shows(&split), "{split} is listed");
    assert!(!harness.shows("An edge that is no longer there"));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.shows(&whole));
    assert!(!harness.shows(&split));
}

fn add_peg(harness: &mut Harness) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(
        &mut outline,
        Point2::new(30.0, 10.0),
        Point2::new(60.0, 30.0),
    );
    let mut transaction = harness.document().transaction("Add a peg");
    let sketch = transaction.add_feature("Peg sketch", FeatureKind::from(outline));
    let peg = transaction.add_feature(
        "Peg",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("5 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    peg
}

#[test]
fn two_bodies_are_combined_from_the_selection_and_the_panel_changes_how() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let peg_top = pickable_described(&mut harness, "Peg › Peg end face");
    let plate_volume = 16000.0;
    let peg_volume = 30.0 * 20.0 * 5.0;
    let overlap = 10.0 * 20.0 * 5.0;

    harness.select([top]);
    harness.hover("Combine");
    assert!(harness.shows_containing("Select faces or edges of two bodies"));

    harness.select([top, peg_top]);
    harness.click("Combine");
    harness.settle();
    let combine = harness
        .workspace
        .editing
        .solid()
        .expect("the combine is open");
    assert_eq!(harness.model.undo_label(), Some("Create Combine 1"));
    let definition = harness
        .document()
        .feature(combine)
        .unwrap()
        .kind
        .combine()
        .unwrap()
        .clone();
    assert_eq!((definition.body, definition.tool), (plate, peg));
    assert!(harness.shows("Operation"));
    assert!(harness.shows("Target body"));
    assert!(harness.shows("Tool body"));
    assert!((harness.body_volume(plate) - (plate_volume + peg_volume - overlap)).abs() < 100.0);
    assert!(harness.model.evaluation().body_result(peg).is_none());

    harness.click_button("Cut");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Combine 1"));
    assert!((harness.body_volume(plate) - (plate_volume - overlap)).abs() < 100.0);

    harness.click_button("Intersect");
    harness.settle();
    assert!((harness.body_volume(plate) - overlap).abs() < 100.0);

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.model.evaluation().body_result(peg).is_some());
    assert!((harness.body_volume(plate) - plate_volume).abs() < 100.0);
}

fn combine_nearly_touching_blocks(harness: &mut Harness) -> FeatureId {
    let mut transaction = harness.document().transaction("Nearly touching blocks");
    let mut bodies = Vec::new();
    let mut plate = Sketch::new(Plane::XY);
    rectangle(&mut plate, Point2::new(0.0, 0.0), Point2::new(20.0, 10.0));
    plate.add_circle(Point2::new(10.0, 5.0), 2.5);
    let mut peg = Sketch::new(Plane::XY);
    peg.add_circle(Point2::new(10.0000015, 5.0), 2.5);
    for (name, outline) in [("Plate", plate), ("Peg", peg)] {
        let sketch = transaction.add_feature(format!("{name} sketch"), FeatureKind::from(outline));
        bodies.push(transaction.add_feature(
            name,
            FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
                sketch,
                regions: RegionChoice::All,
                extent: ExtrudeExtent::one_side(Expression::parse_stored("4 mm").unwrap(), false),
                operation: BodyOperation::NewBody,
                start: None,
                other_bodies: Vec::new(),
            })),
        ));
    }
    let combine = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(caditor_document::Combine {
            body: bodies[0],
            tool: bodies[1],
            operation: caditor_document::CombineOperation::Join,
        }),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    combine
}

#[test]
fn a_combine_failing_where_faces_nearly_touch_is_marked_and_shown_in_the_view() {
    let mut harness = Harness::new();

    let combine = combine_nearly_touching_blocks(&mut harness);

    let Some(caditor_document::FeatureState::Failed(error)) = harness
        .model
        .evaluation()
        .feature(combine)
        .map(|status| status.state.clone())
    else {
        panic!("the nearly touching blocks should not combine");
    };
    let place = error.place.expect("the failure has a place");
    let before = harness.workspace.viewport.viewpoint().target;
    assert!(harness.shows(&error.reason), "{}", error.reason);
    assert!(harness.shows("Combine 1 failed here"));
    assert!(before.distance(place) > 1.0);

    harness.click("Show where");
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let after = harness.workspace.viewport.viewpoint();
    assert!(after.target.distance(place) < 1e-6, "{after:?} {place:?}");
    assert!(harness.shows("Combine 1 failed here"));
}

#[test]
fn a_combine_of_a_third_body_or_none_is_refused_with_what_to_select() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.click("Combine");
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert!(
        harness
            .document()
            .features()
            .all(|feature| feature.kind.combine().is_none())
    );
}

#[test]
fn a_body_is_moved_by_distances_and_turns_typed_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    let movement = harness.workspace.editing.solid().expect("the move is open");
    assert_eq!(harness.model.undo_label(), Some("Create Move body 1"));
    assert!(harness.shows("Move along X"));
    assert!(harness.shows("Turn about Z"));

    harness.type_into_field(Id::new(("move-field", "offset", 0usize, movement)), "5 mm");
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!((bounds.min().x - 5.0).abs() < 1e-6, "{:?}", bounds.min());
    assert_eq!(harness.model.undo_label(), Some("Edit Move body 1"));

    harness.type_into_field(Id::new(("move-field", "turn", 2usize, movement)), "90 deg");
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(
        (bounds.min().x - (-40.0 + 5.0)).abs() < 1e-6,
        "{:?}",
        bounds.min()
    );

    harness.type_into_field(Id::new(("move-field", "offset", 1usize, movement)), "5 deg");
    assert!(harness.shows_containing("length"));
    assert_eq!(harness.workspace.editing.solid(), Some(movement));

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    assert!(bounds.min().x.abs() < 1e-6);
}

#[test]
fn a_hole_is_drilled_at_the_points_of_a_sketch_and_its_panel_changes_the_style_and_sizes() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_point(Point2::new(20.0, 20.0));
    let sketch = harness.add_sketch(sketch);
    harness.select([]);

    harness.hover("Hole");
    assert!(harness.shows_containing(crate::hole_tools::DESCRIPTION));
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    let definition = harness
        .document()
        .feature(hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone();
    assert_eq!((definition.sketch, definition.body), (sketch, plate));
    assert!(harness.document().feature(sketch).unwrap().hidden);
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));
    assert!(harness.shows("Diameter"));
    assert!(harness.shows("Depth"));

    harness.type_into_field(Id::new(("hole-field", "diameter", hole)), "10 mm");
    harness.settle();
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 25.0 * 10.0
    ));

    harness.type_into_field(Id::new(("hole-field", "depth", hole)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(hole));

    harness.click("Plain");
    harness.settle();
    harness.click("Countersink");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Hole 1"));
    assert!(harness.shows("Countersink angle"));
    assert_eq!(harness.model.evaluation().failed_count(), 1);
    harness.type_into_field(
        Id::new(("hole-field", "countersink-diameter", hole)),
        "14 mm",
    );
    harness.click("Through all");
    harness.settle();
    assert_eq!(rows_named(&harness, "Depth").len(), 1);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let label = harness.model.undo_label().map(str::to_owned);
    harness.type_into_field(
        Id::new(("hole-field", "countersink-angle", hole)),
        "180 deg",
    );
    assert!(harness.shows("Enter an angle above 0° and up to 179°"));
    assert_eq!(harness.model.undo_label().map(str::to_owned), label);

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 25.0 * 10.0
    ));
}

fn plate_bounds(
    harness: &Harness,
    plate: FeatureId,
) -> (caditor_geometry::Point3, caditor_geometry::Point3) {
    let bounds = harness
        .model
        .evaluation()
        .body(plate)
        .unwrap()
        .bounding_box()
        .unwrap();
    (bounds.min(), bounds.max())
}

#[test]
fn a_body_is_mirrored_across_a_plane_and_keeps_or_leaves_out_its_original_from_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Mirror body");
    harness.settle();
    let mirror = harness
        .workspace
        .editing
        .solid()
        .expect("the mirror is open");
    assert_eq!(harness.model.undo_label(), Some("Create Mirror body 1"));
    assert!(harness.shows("Mirror across"));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 40.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6);

    harness.click(mirror_panel::KEEP_ORIGINAL);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Mirror body 1"));
    assert!(volume_about(&harness, plate, 16000.0));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 40.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XY plane");
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.z + 10.0).abs() < 1e-6 && high.z.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.select([top]);
    run_from_palette(&mut harness, "mirror across selected");
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.z - 10.0).abs() < 1e-6 && (high.z - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    assert_eq!(harness.workspace.editing.solid(), Some(mirror));

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
}

#[test]
fn a_body_is_split_along_a_plane_into_two_bodies_from_the_panel() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(
        &mut sketch,
        Point2::new(-20.0, -20.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let plate = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Face { body, .. } if *body == plate))
        .expect("a face of the plate is pickable");

    harness.select([top]);
    harness.use_tool_with(Key::K, Modifiers::ALT);
    harness.settle();
    let split = harness
        .workspace
        .editing
        .solid()
        .expect("the split is open");
    assert_eq!(harness.model.undo_label(), Some("Create Split 1"));
    assert!(harness.shows("Split along"));
    assert!(harness.shows("Split-off body"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    let (low, high) = plate_bounds(&harness, plate);
    assert!(low.x.abs() < 1e-6 && (high.x - 20.0).abs() < 1e-6);
    let (low, high) = plate_bounds(&harness, split);
    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);
    assert!(volume_about(&harness, split, 8000.0));

    harness.click(split_panel::KEEP_OTHER_SIDE);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Split 1"));
    let (low, high) = plate_bounds(&harness, plate);
    assert!((low.x + 20.0).abs() < 1e-6 && high.x.abs() < 1e-6);

    choose(&mut harness, "The YZ plane", "The XZ plane");
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        low.y.abs() < 1e-6 && (high.y - 20.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );
    let (low, high) = plate_bounds(&harness, split);
    assert!(
        (low.y + 20.0).abs() < 1e-6 && high.y.abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_eq!(harness.built_with_meshes(2).scene.meshes.len(), 2);
}

fn extruded(harness: &mut Harness, min: Point2, max: Point2) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, min, max);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

#[test]
fn one_cut_removes_material_from_every_body_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let first = extruded(&mut harness, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let second = extruded(
        &mut harness,
        Point2::new(50.0, 0.0),
        Point2::new(90.0, 40.0),
    );
    choose(&mut harness, "Add to body", "New body");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(volume_about(&harness, second, 16000.0));

    let cut = extruded(
        &mut harness,
        Point2::new(30.0, 10.0),
        Point2::new(60.0, 20.0),
    );
    choose(&mut harness, "Add to body", "Remove from body");
    assert_eq!(
        harness.solid(cut).operation(),
        BodyOperation::Remove(second)
    );
    assert!(harness.shows(solid_panel::ALSO_CUTS));
    assert!(volume_about(&harness, second, 15000.0));
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");
    assert_eq!(harness.solid(cut).other_bodies(), [first]);
    assert!(volume_about(&harness, first, 15000.0));
    assert!(volume_about(&harness, second, 15000.0));
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.click_button("Stop cutting Extrude 1");
    harness.settle();
    assert!(harness.solid(cut).other_bodies().is_empty());
    assert!(volume_about(&harness, first, 16000.0));

    choose(&mut harness, solid_panel::ADD_CUT_BODY, "Extrude 1");
    choose(&mut harness, "Remove from body", "Add to body");
    assert!(harness.solid(cut).other_bodies().is_empty());
}

#[test]
fn a_body_is_scaled_by_a_factor_about_a_centre_typed_in_the_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.use_tool_with(Key::S, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let scale = harness
        .workspace
        .editing
        .solid()
        .expect("the scale is open");
    assert_eq!(harness.model.undo_label(), Some("Create Scale body 1"));
    assert!(harness.shows("Factor"));
    assert!(harness.shows("Centre X"));
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.type_into_field(Id::new(("scale-field", ("factor", 0usize), scale)), "0.5");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Scale body 1"));
    assert!(volume_about(&harness, plate, 16000.0 / 8.0));

    harness.type_into_field(Id::new(("scale-field", ("center", 0usize), scale)), "40 mm");
    harness.settle();
    let (low, high) = plate_bounds(&harness, plate);
    assert!(
        (low.x - 20.0).abs() < 1e-6 && (high.x - 40.0).abs() < 1e-6,
        "{low:?} {high:?}"
    );

    harness.type_into_field(Id::new(("scale-field", ("factor", 0usize), scale)), "-1");
    assert!(harness.shows_containing("above zero"));
    assert_eq!(harness.workspace.editing.solid(), Some(scale));

    for _ in 0..3 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    assert!(volume_about(&harness, plate, 16000.0));
}

fn shell_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Shell {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.shell())
        .unwrap()
}

fn pickable_described(harness: &mut Harness, text: &str) -> Pickable {
    let built = harness.built();
    let found = built
        .picks
        .pickables()
        .find(|pickable| pickable.describe(harness.document(), harness.model.evaluation()) == text);
    found.unwrap_or_else(|| panic!("nothing pickable is described as {text}"))
}

#[test]
fn a_shell_opens_the_selected_face_and_takes_more_faces_clicked_in_the_view() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face: top_key, .. } = top else {
        panic!("the top is a face");
    };

    harness.select([top]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    assert_eq!(harness.model.undo_label(), Some("Create Shell 1"));
    assert_eq!(shell_of(&harness, shell).open.len(), 1);
    assert!(removed_about(&harness, plate, 38.0 * 38.0 * 9.0));
    assert!(harness.shows("Click flat faces to open them or close them again"));
    assert!(harness.shows("Thickness"));

    let built = harness.built_with_meshes(1);
    assert_eq!(built.scene.meshes.len(), 1);
    let shell_faces = built
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::ShellFace { .. }))
        .count();
    assert_eq!(shell_faces, 6);

    let bottom = pickable_described(
        &mut harness,
        "Extrude 1 start face: click to open it in Shell 1 or close it again",
    );
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), bottom);
    harness.settle();
    assert_eq!(shell_of(&harness, shell).open.len(), 2);
    assert_eq!(harness.model.undo_label(), Some("Open a face of Shell 1"));
    assert!(removed_about(&harness, plate, 38.0 * 38.0 * 10.0));

    harness.type_into_field(Id::new(("shell-thickness", shell)), "2 mm");
    harness.settle();
    assert!(removed_about(&harness, plate, 36.0 * 36.0 * 10.0));

    harness.type_into_field(Id::new(("shell-thickness", shell)), "0 mm");
    assert!(harness.shows(crate::feature_fields::ABOVE_ZERO));
    assert_eq!(harness.workspace.editing.solid(), Some(shell));

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 20.0),
        Pickable::ShellFace {
            feature: shell,
            face: top_key,
        },
    );
    harness.settle();
    assert_eq!(shell_of(&harness, shell).open.len(), 1);
    assert_eq!(harness.model.undo_label(), Some("Close a face of Shell 1"));
    assert!(removed_about(&harness, plate, 36.0 * 36.0 * 8.0));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
    let inner = pickable_described(&mut harness, "Extrude 1 › Shell 1 inner face");
    assert!(matches!(inner, Pickable::Face { .. }));
}

#[test]
fn the_shell_button_needs_faces_of_a_body_and_opens_every_selected_one() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let built = harness.built();
    let side = built
        .picks
        .pickables()
        .find(|pickable| matches!(pickable, Pickable::Face { .. }) && *pickable != top)
        .expect("the plate has other faces");

    harness.select([]);
    harness.click("Shell");
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Shell 1"));

    harness.select([top, side]);
    harness.click("Shell");
    harness.settle();
    let shell = harness
        .workspace
        .editing
        .solid()
        .expect("the shell is open");
    assert_eq!(shell_of(&harness, shell).open.len(), 2);
    assert!(harness.body_volume(plate) < 16000.0 - 38.0 * 38.0 * 9.0);
}

fn pattern_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Pattern {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.pattern())
        .unwrap()
}

fn volume_about(harness: &Harness, body: FeatureId, expected: f64) -> bool {
    (harness.body_volume(body) - expected).abs() < 1e-3 * expected
}

#[test]
fn a_linear_pattern_repeats_the_body_and_takes_its_count_and_directions_from_the_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.hover("Linear pattern");
    assert!(harness.shows_containing(
        "Repeat the body of Extrude 1 along the X axis, or along an edge or axis you select first"
    ));
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Linear pattern 1"));
    assert_eq!(pattern_of(&harness, pattern).body, plate);
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));
    assert!(harness.shows("Direction"));
    assert!(harness.shows("Spacing"));

    harness.type_into_field(Id::new(("pattern-field", "count", pattern)), "4");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));

    assert_eq!(
        offer(&harness, Command::PatternSecondUseSelected).availability,
        Err("Select an axis, straight edge or round face made before this pattern".to_owned())
    );
    harness.select([Pickable::Axis(Axis::Y)]);
    run_from_palette(&mut harness, "pattern also along selected");
    harness.settle();
    let caditor_document::PatternKind::Linear { second, .. } = &pattern_of(&harness, pattern).kind
    else {
        panic!("the pattern stays linear");
    };
    assert!(second.is_some());
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));
    harness.perform(Action::Redo);
    harness.settle();
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));

    harness.type_into_field(Id::new(("pattern-field", "count", pattern)), "2.5");
    assert!(harness.shows("Enter a whole number of at least 1"));
    assert!(volume_about(&harness, plate, 8.0 * 16000.0));
}

#[test]
fn a_pattern_takes_its_directions_from_lists_in_its_panel() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    let directions = |harness: &Harness| match &pattern_of(harness, pattern).kind {
        caditor_document::PatternKind::Linear { first, second } => (
            first.axis.clone(),
            second.as_ref().map(|second| second.axis.clone()),
        ),
        caditor_document::PatternKind::Circular(_) => panic!("the pattern stays linear"),
    };

    open_combo(&mut harness, "Direction");
    harness.click_lowest("The Y axis");
    harness.settle();
    open_combo(&mut harness, "Second direction");
    harness.click_lowest("The X axis");
    harness.settle();

    assert_eq!(
        directions(&harness),
        (
            caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::Y),
            Some(caditor_document::AxisReference::Principal(
                caditor_document::PrincipalAxis::X
            ))
        )
    );

    open_combo(&mut harness, "Second direction");
    harness.click_lowest(crate::pattern_panel::NO_SECOND_DIRECTION);
    harness.settle();

    assert_eq!(
        directions(&harness),
        (
            caditor_document::AxisReference::Principal(caditor_document::PrincipalAxis::Y),
            None
        )
    );
}

#[test]
fn a_pattern_measures_first_to_last_and_leaves_out_the_copies_clicked_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);

    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    let spacing = |harness: &Harness| {
        let caditor_document::PatternKind::Linear { first, .. } =
            &pattern_of(harness, pattern).kind
        else {
            panic!("the pattern stays linear");
        };
        (first.measured, first.spacing.to_stored_text())
    };

    assert!(harness.shows(crate::pattern_panel::INSTANCES));
    assert!(harness.shows("Spacing"));
    assert_eq!(
        spacing(&harness),
        (
            caditor_document::LinearSpacing::BetweenCopies,
            "48 mm".to_owned()
        )
    );

    harness.click(crate::pattern_panel::MEASURED_OVERALL);
    harness.settle();

    assert!(harness.shows("Total length"));
    assert_eq!(
        spacing(&harness),
        (caditor_document::LinearSpacing::Total, "96 mm".to_owned())
    );
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));

    harness.click_button("Copy 2");
    harness.settle();

    assert!(pattern_of(&harness, pattern).is_skipped([2, 0]));
    assert!(harness.shows("1 copy is left out."));
    assert!(volume_about(&harness, plate, 2.0 * 16000.0));
    assert_eq!(harness.model.undo_label(), Some("Edit Linear pattern 1"));

    harness.perform(Action::Undo);
    harness.settle();

    assert!(pattern_of(&harness, pattern).skipped.is_empty());
    assert!(volume_about(&harness, plate, 3.0 * 16000.0));
}

fn datum_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Datum {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.datum())
        .unwrap()
}

fn datum_plane(harness: &Harness, feature: FeatureId) -> Plane {
    crate::datum_tools::result(harness.model.evaluation(), feature)
        .and_then(|result| result.plane())
        .expect("the datum plane has a position")
}

#[test]
fn a_datum_plane_carries_a_sketch_that_follows_its_offset() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the new plane is open");
    assert_eq!(harness.model.undo_label(), Some("Create Plane 1"));
    assert!(datum_of(&harness, plane).is_plane());
    assert_eq!(datum_plane(&harness, plane).origin().z, 10.0);
    assert!(harness.shows("Starts from"));
    assert!(harness.shows("The XY plane"));
    assert!(
        harness
            .shows("Select planes, faces, axes or edges for the feature's panel, or choose them in the view from it")
    );

    harness.type_into_field(Id::new(("datum-field", "offset", plane)), "25 mm");
    harness.settle();
    assert_eq!(datum_plane(&harness, plane).origin().z, 25.0);
    assert!(
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| pickable == Pickable::Datum(plane))
    );

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
    harness.select([Pickable::Datum(plane)]);
    harness.click("New sketch");
    harness.settle();
    let sketch = harness.editing().expect("the new sketch is edited");
    assert_eq!(plane_height(&harness, sketch), 25.0);
    let attachment = harness
        .document()
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .cloned();
    assert_eq!(
        attachment,
        Some(caditor_document::SketchAttachment::Datum(plane))
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let Some(caditor_document::Datum::Plane(mut changed)) = harness
        .document()
        .feature(plane)
        .and_then(|feature| feature.kind.datum())
        .cloned()
    else {
        panic!("the plane is a datum plane");
    };
    changed.offset = Expression::Measure(-5.0, Unit::Millimetre);
    harness.perform(Action::Apply(Transaction::single(
        "Move the plane",
        Edit::SetFeatureKind {
            id: plane,
            kind: FeatureKind::Datum(caditor_document::Datum::Plane(changed)),
        },
    )));
    harness.settle();
    assert_eq!(plane_height(&harness, sketch), -5.0);
}

#[test]
fn an_axis_from_a_selected_edge_turns_a_plane_and_a_revolve() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let edge = Pickable::Edge {
        body: plate,
        edge: front,
    };

    harness.select([top, edge]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    let tilted = datum_plane(&harness, plane);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!((tilted.normal().z.abs() - half).abs() < 1e-9, "{tilted:?}");
    assert!(
        tilted
            .signed_distance(caditor_geometry::Point3::new(7.0, 0.0, 10.0))
            .abs()
            < 1e-9
    );
    assert!(harness.shows("Turned about"));
    assert!(harness.shows("Angle"));

    harness.select([edge]);
    harness.click("Axis");
    harness.settle();
    let axis = harness.workspace.editing.solid().expect("the axis is open");
    assert_eq!(harness.model.undo_label(), Some("Create Axis 1"));
    let line = crate::datum_tools::result(harness.model.evaluation(), axis)
        .and_then(|result| result.axis())
        .expect("the axis has a position");
    assert!((line.direction().x.abs() - 1.0).abs() < 1e-9);

    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 30.0),
    );
    let section = harness.add_sketch(section);
    harness.select([Pickable::Datum(axis)]);
    harness.click("Revolve");
    harness.settle();
    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolution is open");
    let definition = harness.solid(revolve);
    assert_eq!(definition.sketch(), section);
    assert_eq!(
        definition
            .axis()
            .and_then(caditor_document::RevolveAxis::model),
        Some(&caditor_document::AxisReference::Datum(axis))
    );
    assert_eq!(
        harness
            .model
            .evaluation()
            .feature(revolve)
            .map(|status| &status.state),
        Some(&caditor_document::FeatureState::UpToDate)
    );
}

#[test]
fn the_command_palette_runs_what_fits_the_context_and_explains_the_rest() {
    let mut harness = Harness::new();
    harness.edit_width("50 mm");
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    assert!(harness.workspace.palette.is_open());

    harness.type_text("fillet");
    assert!(harness.shows("Fillet is not available: Select the edges of a body first."));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.palette.is_open());

    harness.replace_text("draw line");
    assert!(harness.shows("Draw line"));
    assert!(
        harness.shows("Draw line is not available here: it works only while a sketch is edited.")
    );
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.palette.is_open());
    assert_eq!(harness.tool(), None);

    harness.replace_text("zqzqzq");
    assert!(harness.shows("Nothing is called “zqzqzq”. Try another word, or fewer letters."));

    harness.replace_text("undo");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.expression_text("width"), "40 mm");

    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    harness.click("Search commands");
    assert!(harness.workspace.palette.is_open());
    harness.type_text("draw line");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert_eq!(harness.tool(), Some(Tool::Line));
}

#[test]
fn a_shortcut_recorded_in_the_editor_runs_its_command_and_is_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.edit_width("50 mm");
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    harness.click("Keyboard shortcuts…");
    assert!(harness.shows("Keyboard shortcuts"));
    harness.type_text("undo");
    assert_eq!(harness.count_shown("Redo"), 1);
    harness.click("Keyboard shortcuts");

    harness.click("Add…");
    assert!(harness.shows("Press the keys… (Esc cancels)"));
    harness.key(Key::U, Modifiers::ALT);
    harness.show_new_windows();
    assert!(
        harness
            .workspace
            .preferences
            .keymap
            .shortcuts(Command::Undo)
            .contains(&egui::KeyboardShortcut::new(Modifiers::ALT, Key::U))
    );
    assert!(harness.shows("U"));

    let shown_before = harness.count_shown("F");
    harness.click("Add…");
    harness.key(Key::F, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.shows("F is already used by Fit view. Use it for Undo instead?"));
    harness.click("Keep it where it is");
    assert_eq!(harness.count_shown("F"), shown_before);

    harness.click("Add…");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.shows(
        "Esc, Enter and Tab keep their meaning everywhere (back out, confirm, move between \
         fields), so they cannot be shortcuts."
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.shortcut_editor.is_none());
    assert!(harness.workspace.preferences_open);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.preferences_open);

    harness.key(Key::U, Modifiers::ALT);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
    harness.wait_until("the shortcut is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).texts("keys.edit.undo")
            == Some(vec!["Ctrl+Z".to_owned(), "Alt+U".to_owned()])
    });
    harness.hover("Redo");
    assert!(harness.shows("Redo Edit width (Ctrl+Shift+Z)"));
}

#[test]
fn resetting_all_shortcuts_asks_first_and_can_be_undone() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Bind(
            Command::Undo,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::U),
        ),
    )));
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    let bound = harness.workspace.preferences.keymap.clone();
    assert!(!bound.is_all_default());

    harness.click("Reset all shortcuts");
    assert!(harness.shows("Reset all shortcuts?"));
    assert_eq!(harness.workspace.preferences.keymap, bound);
    harness.click("Cancel");
    assert!(!harness.shows("Reset all shortcuts?"));
    assert_eq!(harness.workspace.preferences.keymap, bound);

    harness.click("Reset all shortcuts");
    harness.click("Reset all shortcuts");
    assert!(harness.workspace.preferences.keymap.is_all_default());
    assert!(harness.shows("Every shortcut is back to the one caditor starts with."));
    harness.click_lowest("Undo");
    assert_eq!(harness.workspace.preferences.keymap, bound);
    assert!(!harness.shows("Every shortcut is back to the one caditor starts with."));
    assert!(harness.workspace.shortcut_editor.is_some());
}

#[test]
fn the_palette_finds_features_and_parameters_and_goes_to_them() {
    let mut harness = Harness::new();
    let side = feature_named(&harness, "Side sketch");
    let height = harness.parameter("height");

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("side sk");
    assert!(harness.shows("Features"));
    assert!(harness.shows("Press Enter to select it in the feature tree."));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(harness.workspace.panels.selected, Some(side));

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("heig");
    assert!(harness.shows("Parameters"));
    assert!(harness.shows("width / 2"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.palette.is_open());
    assert_eq!(
        harness.focused(),
        Some(Focus::ParameterValue(height).field_id())
    );
}

#[test]
fn the_palette_lists_what_does_not_fit_the_context_last_with_the_reason() {
    let mut harness = Harness::new();
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text("trim");
    assert!(harness.shows("Trim sketch curves"));
    let entries = harness.workspace.palette.entries(
        &harness.workspace.last_offers,
        &harness.workspace.preferences.keymap,
        harness.model.document(),
    );
    let trim = entries
        .iter()
        .position(|entry| entry.choice == Choice::Command(Command::SketchTool(Tool::Trim)))
        .unwrap();
    let last_ready = entries
        .iter()
        .rposition(|entry| entry.state == State::Ready)
        .unwrap_or_default();
    assert!(trim > last_ready, "{entries:#?}");
    for _ in 0..trim {
        harness.key(Key::ArrowDown, Modifiers::NONE);
    }
    harness.frame();
    assert!(harness.shows(
        "Trim sketch curves is not available here: it works only while a sketch is edited."
    ));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.palette.is_open());
    assert_eq!(harness.tool(), None);
}

fn run_from_palette(harness: &mut Harness, query: &str) {
    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.type_text(query);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
}

fn type_point(harness: &mut Harness, text: &str) {
    harness.type_text(text);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();
}

#[test]
fn space_while_drawing_starts_a_line_at_the_highlighted_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let corner = sketch.add_point(Point2::new(10.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.use_tool(Key::L);
    let point = Pickable::SketchEntity {
        feature,
        entity: corner,
    };

    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(point) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(point));

    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();
    type_point(&mut harness, "@20 mm, 0");

    let lines = entities_of_kind(harness.sketch(feature), "Line");
    assert_eq!(lines.len(), 1);
    assert!(
        constraints_of_kind(harness.sketch(feature), "Coincident")
            .iter()
            .any(|constraint| matches!(
                constraint,
                Constraint::Coincident(a, b) if *a == corner || *b == corner
            ))
    );
    assert!(!harness.workspace.viewport.selection().contains(point));
}

#[test]
fn a_part_can_be_modelled_from_the_keyboard_alone() {
    let mut harness = Harness::new();
    run_from_palette(&mut harness, "new sketch");
    assert!(harness.workspace.editing.is_choosing_plane());

    let xy = Pickable::Plane(PrincipalPlane::Xy);
    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(xy) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(xy));
    assert!(harness.shows("XY plane"));
    harness.key(Key::Space, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();
    let sketch = harness
        .editing()
        .expect("a sketch on the XY plane is being edited");

    harness.use_tool(Key::R);
    type_point(&mut harness, "5, 5");
    assert!(harness.workspace.viewport.is_drawing());
    assert!(!harness.shows(typed_point::FIELD_LABEL));
    type_point(&mut harness, "5, nowhere");
    assert!(harness.shows(typed_point::FIELD_LABEL));
    assert!(
        harness
            .texts
            .iter()
            .any(|(shown, _)| shown.starts_with("y: "))
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.shows(typed_point::FIELD_LABEL));
    assert!(harness.workspace.viewport.is_drawing());
    type_point(&mut harness, "@2000 m, 5");
    assert!(harness.shows("Keep the point within 1000 m of the sketch's origin"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    type_point(&mut harness, "@20 mm, 0");
    assert!(harness.shows("A rectangle needs its corners apart in both directions"));
    assert!(entities_of_kind(harness.sketch(sketch), "Line").is_empty());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.viewport.is_drawing());
    type_point(&mut harness, "@20 mm, width / 4");
    assert_eq!(entities_of_kind(harness.sketch(sketch), "Line").len(), 4);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert_eq!(harness.editing(), None);

    run_from_palette(&mut harness, "extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the extrusion was created");
    assert!((harness.body_volume(extrude) - 2000.0).abs() < 1.0);

    harness.key(Key::Num2, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let forward = harness.workspace.viewport.viewpoint().forward();
    assert!(
        forward.dot(caditor_geometry::Vector3::NEG_Z) > 0.999,
        "{forward:?}"
    );
    harness.key(Key::ArrowUp, Modifiers::NONE);
    harness.frame();
    let turned = harness.workspace.viewport.viewpoint().forward();
    assert!(turned.dot(forward) < 0.9999);
}

#[test]
fn the_interface_scales_from_the_keyboard_and_high_contrast_changes_the_colours() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let normal_panel = harness.context.global_style().visuals.panel_fill;
    harness.key(Key::Plus, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    assert_eq!(harness.workspace.preferences.appearance.scale, 1.125);
    assert_eq!(harness.context.zoom_factor(), 1.125);
    harness.key(Key::Num0, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    assert_eq!(harness.context.zoom_factor(), 1.0);
    for _ in 0..3 {
        harness.key(Key::Minus, Modifiers::COMMAND);
        harness.frame();
    }
    assert_eq!(harness.workspace.preferences.appearance.scale, 0.75);
    harness.key(Key::Minus, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Make the interface smaller: The interface is at its smallest, 75%")
    );

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    let visible = SCREEN.size() / 2.0;
    for label in ["File", "Axis", "Up to date", "Features"] {
        let rect = harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1;
        assert!(
            rect.max.x <= visible.x && rect.max.y <= visible.y,
            "{label} at {rect:?}"
        );
    }

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.0),
    )));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Appearance");
    harness.click("High contrast");
    assert!(harness.workspace.preferences.appearance.high_contrast);
    let panel = harness.context.global_style().visuals.panel_fill;
    assert_ne!(panel, normal_panel);
    assert!(panel == Color32::BLACK || panel == Color32::WHITE);
    harness.wait_until("the appearance is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).flag("appearance.high_contrast")
            == Some(true)
    });
}

#[test]
fn a_first_run_welcomes_opens_a_sample_and_offers_tips_until_they_are_hidden() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    assert!(harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Tip"));
    harness.click("Flanged spool");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert!(
        harness
            .document()
            .features()
            .any(|feature| feature.name == "Spool")
    );
    assert!(!harness.model.is_dirty());
    assert_eq!(harness.model.undo_label(), None);
    assert!(
        harness.shows(
            &Hint::Navigate.text(&harness.workspace.preferences.keymap, InputMode::default())
        )
    );

    harness.click("Got it");
    assert!(
        harness.shows(
            &Hint::Palette.text(&harness.workspace.preferences.keymap, InputMode::default())
        )
    );
    harness.click("Hide tips");
    assert!(!harness.shows("Tip"));
    let config = dir.path().join("config");
    harness.wait_until("the tips are saved", |_| {
        let settings = caditor_file::Settings::load(&config);
        settings.flag("onboarding.hints") == Some(false)
            && settings.flag("onboarding.welcomed") == Some(true)
            && settings.texts("onboarding.dismissed_hints") == Some(vec!["navigate".to_owned()])
    });

    harness.command(FileCommand::New);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::RestoreHints,
    )));
    harness.settle();
    let keymap = harness.workspace.preferences.keymap.clone();
    assert!(harness.shows(&Hint::Start.text(&keymap, InputMode::default())));
    harness.draw_on_new_sketch();
    assert!(harness.shows(&Hint::Draw.text(&keymap, InputMode::default())));
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();
    assert!(harness.shows(&Hint::Constrain.text(&keymap, InputMode::default())));
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    assert!(harness.shows(&Hint::Sweep.text(&keymap, InputMode::default())));
    harness.click("Help");
    harness.click("Welcome and samples…");
    assert!(harness.shows("Welcome to caditor"));
    harness.click("Start with an empty model");
    assert!(!harness.shows("Welcome to caditor"));
    harness.click("Continue without saving");
    harness.settle();
    assert_eq!(harness.document().features().len(), 0);
    assert!(!harness.model.is_dirty());
}

fn open_menus(harness: &Harness) -> Vec<Rect> {
    harness.context.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .filter_map(|layer| memory.area_rect(layer.id))
            .collect()
    })
}

#[test]
fn every_menu_stays_on_screen_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);

    for menu in ["File", "Edit", "View", "Model", "Sketch", "Help"] {
        harness.click(menu);
        harness.frame();
        harness.frame();
        let menus = open_menus(&harness);

        assert!(!menus.is_empty(), "{menu} did not open");
        for rect in menus {
            assert!(
                visible.expand(0.5).contains_rect(rect),
                "{menu} reaches {rect:?}, past {visible:?}"
            );
        }

        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }

    harness.click("Model");
    harness.frame();

    assert!(harness.shows("Bodies"));

    harness.click("Bodies");
    harness.frame();
    harness.frame();

    assert!(harness.shows("Move body"));
    for rect in open_menus(&harness) {
        assert!(visible.expand(0.5).contains_rect(rect), "{rect:?}");
    }
}

#[test]
fn the_welcome_lists_recent_files_opens_one_and_clears_them() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut earlier = Harness::with_directories(Some(dir.path()));
    earlier.command(FileCommand::OpenPath(path.clone()));
    earlier.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(earlier.files.wait_for_jobs(FILE_TIMEOUT));
    drop(earlier);

    let mut harness = Harness::first_run(dir.path());
    harness.wait_until("the recent files are read", |harness| {
        !harness.files.recent().is_empty()
    });
    assert!(harness.shows("Welcome to caditor"));
    assert!(harness.shows("Recent files"));
    assert!(harness.shows("kept.caditor"));
    harness.click("kept.caditor");
    harness.wait_until("the recent file opens", |harness| {
        harness.model.path().is_some()
    });
    assert!(!harness.shows("Welcome to caditor"));

    harness.perform(Action::Preferences(PreferencesCommand::ShowWelcome));
    assert!(harness.shows("Recent files"));
    harness.click("Clear recent files");
    assert!(harness.files.recent().is_empty());
    assert!(harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Recent files"));
}

#[test]
fn starting_with_an_empty_model_from_the_welcome_replaces_an_opened_one() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    harness.click("Flanged spool");
    harness.settle();
    assert!(harness.document().features().len() > 0);

    harness.click("Help");
    harness.click("Welcome and samples…");
    harness.click("Start with an empty model");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert!(!harness.shows("Save changes"));
    assert_eq!(harness.document().features().len(), 0);

    harness.click("Help");
    harness.click("Welcome and samples…");
    harness.click("Start with an empty model");
    harness.settle();
    assert!(!harness.shows("Welcome to caditor"));
    assert_eq!(harness.model.undo_label(), None);
}

#[test]
fn about_shows_the_version_from_the_help_menu_and_the_palette() {
    let mut harness = Harness::new();
    harness.click("Help");
    harness.click("About caditor");
    assert!(harness.workspace.about_open);
    assert!(harness.shows(crate::about::VERSION));
    harness.click("Close");
    assert!(!harness.workspace.about_open);

    run_from_palette(&mut harness, "about");
    assert!(harness.workspace.about_open);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.about_open);
}

#[test]
fn recent_messages_keep_a_failed_save_after_a_later_notice_replaced_it() {
    let mut harness = Harness::new();
    harness.perform(Action::Inform(Notice::failure(
        "Saving “plate” failed: the disk is full.",
    )));
    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.frame();
    assert!(harness.shows("Exported 1 body."));
    assert!(!harness.shows("Saving “plate” failed: the disk is full."));

    harness.click("Help");
    harness.click(crate::messages::TITLE);

    assert!(harness.workspace.messages_open);
    assert!(harness.shows("Saving “plate” failed: the disk is full."));
    assert_eq!(harness.count_shown("Exported 1 body."), 2);
    assert!(!harness.shows(crate::messages::EMPTY));
    harness.click("Close");
    assert!(!harness.workspace.messages_open);

    assert_eq!(
        harness
            .model
            .recorded_notices()
            .take(2)
            .map(|recorded| recorded.notice.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Exported 1 body.",
            "Saving “plate” failed: the disk is full."
        ]
    );
}

#[test]
fn the_view_and_the_tool_prompt_are_named_for_screen_readers() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.frame();
    harness.frame();

    assert!(
        harness
            .accessible
            .iter()
            .any(|(_, node)| node.label() == Some("3D view"))
    );
    let prompt = harness.accessible.iter().find(|(_, node)| {
        node.role() == Role::Label
            && node
                .value()
                .is_some_and(|value| value.starts_with("Click the start of the line"))
    });
    let (_, prompt) = prompt.expect("the prompt is on the accessibility tree");
    assert_eq!(prompt.live(), Some(egui::accesskit::Live::Polite));
    assert!(prompt.value().unwrap().contains("Esc: back to Select"));
}

#[test]
fn a_cancelled_or_stopped_recompute_is_announced() {
    let mut harness = Harness::new();
    harness.settle();
    harness.context.enable_accesskit();
    let live_of = |harness: &Harness, text: &str| {
        harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
            .map(|(_, node)| node.live())
    };

    harness.model.set_status(RecomputeStatus::Cancelled);
    harness.frame();
    harness.frame();
    let cancelled = live_of(&harness, crate::status_bar::CANCELLED);
    harness.model.set_status(RecomputeStatus::Stopped);
    harness.frame();
    harness.frame();
    let stopped = live_of(&harness, crate::status_bar::STOPPED);

    assert_eq!(cancelled, Some(Some(egui::accesskit::Live::Polite)));
    assert_eq!(stopped, Some(Some(egui::accesskit::Live::Assertive)));
}

#[test]
fn notices_are_live_regions_so_a_screen_reader_announces_them() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.frame();
    let live_of = |harness: &Harness, text: &str| {
        harness
            .accessible
            .iter()
            .find(|(_, node)| node.role() == Role::Label && node.value() == Some(text))
            .map(|(_, node)| node.live())
    };

    harness.perform(Action::Inform(Notice::info("Exported 1 body.")));
    harness.frame();
    harness.frame();
    assert_eq!(
        live_of(&harness, "Exported 1 body."),
        Some(Some(egui::accesskit::Live::Polite))
    );

    harness.perform(Action::Inform(Notice::failure(
        "Saving failed: the disk is full.",
    )));
    harness.frame();
    harness.frame();
    assert_eq!(
        live_of(&harness, "Saving failed: the disk is full."),
        Some(Some(egui::accesskit::Live::Assertive))
    );
}

#[test]
fn recent_messages_open_from_the_palette_and_close_with_escape() {
    let mut harness = Harness::new();

    run_from_palette(&mut harness, "recent messages");
    assert!(harness.workspace.messages_open);
    assert!(harness.shows(crate::messages::TITLE));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.messages_open);
}

#[test]
fn the_undo_history_goes_back_and_forward_several_steps_at_once() {
    let mut harness = Harness::new();
    let before = harness.document().parameters().len();
    for name in ["first", "second", "third"] {
        let length = harness.model.length_unit().default_length(10.0);
        let mut transaction = harness.document().transaction(format!("Add {name}"));
        transaction.add_parameter(name.to_owned(), length);
        harness.perform(Action::Apply(transaction.finish()));
        harness.settle();
    }
    assert_eq!(harness.document().parameters().len(), before + 3);

    run_from_palette(&mut harness, "undo history");
    assert!(harness.workspace.undo_history_open);
    for label in ["Add first", "Add second", "Add third"] {
        assert!(harness.shows(label), "{label}");
    }

    harness.click("Add first");
    harness.settle();
    assert_eq!(harness.document().parameters().len(), before + 1);
    assert_eq!(
        harness
            .model
            .redo_steps()
            .map(Transaction::label)
            .collect::<Vec<_>>(),
        ["Add second", "Add third"]
    );
    assert!(harness.workspace.undo_history_open);
    harness.hover("Add third");
    assert!(harness.shows_containing("Parameters: third"));

    harness.click("Add third");
    harness.settle();
    assert_eq!(harness.document().parameters().len(), before + 3);
    assert_eq!(harness.model.redo_steps().count(), 0);

    harness.click("Close");
    assert!(!harness.workspace.undo_history_open);
}

fn sample_document() -> anyhow::Result<Document> {
    let mut document = Document::default();
    let mut transaction = document.transaction("Sample model");
    let width = transaction.parse("40 mm")?;
    transaction.add_parameter("width", width);
    let height = transaction.parse("width / 2")?;
    transaction.add_parameter("height", height);

    let base = dimensioned_line(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Constraint::Horizontal,
        transaction.parse("width")?,
    )?;
    transaction.add_feature("Base sketch", FeatureKind::from(base));
    let side = dimensioned_line(
        Plane::XZ,
        Point2::new(0.0, 20.0),
        Constraint::Vertical,
        transaction.parse("height")?,
    )?;
    transaction.add_feature("Side sketch", FeatureKind::from(side));

    document.apply(transaction.finish())?;
    Ok(document)
}

fn dimensioned_line(
    plane: Plane,
    end: Point2,
    direction: fn(EntityId) -> Constraint,
    value: Expression,
) -> anyhow::Result<Sketch> {
    let mut sketch = Sketch::new(plane);
    let line = sketch.add_line(Point2::ZERO, end);
    sketch.add_constraint(direction(line))?;
    if let Some((from, to)) = endpoints(&sketch, line) {
        sketch.add_constraint(Constraint::Distance { from, to, value })?;
    }
    Ok(sketch)
}

fn endpoints(sketch: &Sketch, line: EntityId) -> Option<(EntityId, EntityId)> {
    match sketch.entity(line)? {
        Entity::Line { start, end } => Some((*start, *end)),
        _ => None,
    }
}

#[test]
fn adding_a_feature_then_undoing_it_leaves_the_model_saved() {
    let mut harness = Harness::new();
    assert!(!harness.model.is_dirty());
    harness.add_sketch(Sketch::new(Plane::XY));
    assert!(harness.model.is_dirty());
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(!harness.model.is_dirty());
    assert_eq!(app::window_title(&harness.model), "Untitled — caditor");
}

#[test]
fn p_hides_the_principal_geometry_outside_a_sketch_and_places_points_inside_one() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();

    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.document().hidden_principal().next().is_some());

    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let shown_again = harness.document().hidden_principal().next().is_none();
    harness.edit(base);
    harness.key(Key::P, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(shown_again);
    assert_eq!(
        harness.workspace.editing.active().map(|active| active.tool),
        Some(Tool::Point)
    );
    assert!(harness.document().hidden_principal().next().is_none());
}

#[test]
fn holding_a_key_repeats_only_commands_that_should_repeat() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight().is_some() {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    let target = harness.workspace.viewport.keyboard_highlight().unwrap();
    for repeat in [false, true, true, true] {
        harness.events.push(Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        });
    }
    harness.frame();
    assert!(harness.workspace.viewport.selection().contains(target));
}

#[test]
fn command_shortcuts_work_while_a_button_has_focus() {
    let mut harness = Harness::new();
    harness.edit_width("45 mm");
    let button = egui::Id::new("focused button");
    harness
        .context
        .memory_mut(|memory| memory.request_focus(button));
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
}

#[test]
fn stepping_the_highlight_visits_a_datum_plane_once() {
    let mut harness = Harness::new();
    harness.click("Plane");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let mut visited = Vec::new();
    for _ in 0..40 {
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
        let Some(highlight) = harness.workspace.viewport.keyboard_highlight() else {
            continue;
        };
        if visited.first() == Some(&highlight) {
            break;
        }
        visited.push(highlight);
    }
    let datums = visited
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Datum(_)))
        .count();
    assert_eq!(datums, 1, "{visited:?}");
    let distinct: std::collections::BTreeSet<_> = visited.iter().collect();
    assert_eq!(distinct.len(), visited.len());
}

fn feature_names(harness: &Harness) -> Vec<String> {
    harness
        .document()
        .features()
        .map(|feature| feature.name.clone())
        .collect()
}

#[test]
fn a_feature_chosen_in_the_tree_is_moved_renamed_and_deleted_from_the_keyboard() {
    let mut harness = Harness::new();
    harness.click("Side sketch");
    harness.frame();
    run_from_palette(&mut harness, "move feature up");
    harness.settle();
    assert_eq!(feature_names(&harness), ["Side sketch", "Base sketch"]);

    harness.key(Key::F2, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.replace_text("Profile");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Profile", "Base sketch"]);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Base sketch"]);
    harness.key(Key::Delete, Modifiers::NONE);
    harness.settle();
    assert_eq!(feature_names(&harness), ["Base sketch"]);
}

#[test]
fn recent_models_notices_and_recompute_are_commands() {
    let dir = TempDir::new().unwrap();
    let root = canonical(&dir);
    let path = root.as_path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(root.as_path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    harness.command(FileCommand::New);
    harness.settle();
    assert_eq!(harness.model.path(), None);

    let recent = harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == Command::OpenRecent(RecentSlot::ALL[0]))
        .map(Offer::title);
    assert_eq!(
        recent.as_deref(),
        Some("Open the most recent model: kept.caditor")
    );
    run_from_palette(&mut harness, "kept");
    harness.wait_until("the recent model is open", |harness| {
        harness.model.path() == Some(path.as_path())
    });

    harness.perform(Action::Inform(Notice::info("Something to read.")));
    harness.frame();
    assert!(harness.model.notice().is_some());
    run_from_palette(&mut harness, "dismiss the notice");
    harness.frame();
    assert!(harness.model.notice().is_none());

    run_from_palette(&mut harness, "recompute the model");
    harness.settle();
    assert_eq!(harness.model.status(), RecomputeStatus::UpToDate);
}

#[test]
fn dialogs_fit_the_screen_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.frame();
    harness.show_new_windows();
    let visible = SCREEN.size() / 2.0;
    for label in ["Close", "Reset all shortcuts", "Keyboard shortcuts"] {
        let rect = harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1;
        assert!(
            rect.min.x >= 0.0
                && rect.min.y >= 0.0
                && rect.max.x <= visible.x
                && rect.max.y <= visible.y,
            "{label} at {rect:?}"
        );
    }
}

#[test]
fn a_curved_face_or_a_round_edge_says_why_it_cannot_be_used() {
    let mut harness = Harness::new();
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::ZERO, 10.0);
    harness.add_sketch(disc);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let pickables: Vec<Pickable> = harness.built().picks.pickables().collect();
    let side = pickables
        .iter()
        .copied()
        .find(|pickable| {
            pickable
                .describe(harness.document(), harness.model.evaluation())
                .contains("side")
        })
        .expect("the round side is pickable");
    let rim = pickables
        .iter()
        .copied()
        .find(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .expect("a rim is pickable");

    harness.select([]);
    harness.click("New sketch");
    harness.click_pickable(Plane::XY, Point2::new(10.0, 0.0), side);
    harness.settle();
    assert!(harness.workspace.editing.is_choosing_plane());
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("New sketch: The selected face is curved; sketches lie on planes and flat faces.")
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([rim]);
    let end = harness.document().features().len();
    let selection = harness.workspace.viewport.selection();
    assert_eq!(
        crate::datum_tools::plane_from_selection(&harness.model, selection, end),
        Err(crate::datum_tools::PLANE_CHOICES)
    );
    assert!(matches!(
        crate::datum_tools::point_from_selection(&harness.model, selection, end),
        Ok(caditor_document::DatumPoint {
            base: caditor_document::PointReference::Centre { .. },
            ..
        })
    ));
}

#[test]
fn dragging_a_speed_slider_applies_at_once_and_is_saved_when_released() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Navigation");
    let label = harness.position_of("Orbit speed");
    let value = harness.position_of("1.00");
    let start = Pos2::new(value.x - 60.0, label.y);
    let button = |pressed| Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    harness.events.push(Event::PointerMoved(start));
    harness.frame();
    harness.events.push(button(true));
    harness.frame();
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(Pos2::new(
            start.x + 10.0 * step as f32,
            start.y,
        )));
        harness.frame();
    }
    let dragged = harness.workspace.preferences.navigation.orbit_speed;
    assert!(dragged > 1.0, "{dragged}");
    let config = dir.path().join("config");
    assert_eq!(
        caditor_file::Settings::load(&config).number("navigation.orbit_speed"),
        None
    );

    harness.events.push(Event::PointerButton {
        pos: Pos2::new(start.x + 40.0, start.y),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let released = harness.workspace.preferences.navigation.orbit_speed;
    harness.wait_until("the orbit speed is saved", |_| {
        caditor_file::Settings::load(&config).number("navigation.orbit_speed") == Some(released)
    });
}

fn offer(harness: &Harness, command: Command) -> Offer {
    harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == command)
        .cloned()
        .unwrap_or_else(|| panic!("{command:?} is not offered"))
}

fn feature_named(harness: &Harness, name: &str) -> FeatureId {
    harness
        .document()
        .features()
        .find(|feature| feature.name == name)
        .map(Feature::id)
        .unwrap_or_else(|| panic!("there is no feature named {name}"))
}

#[test]
fn features_are_opened_and_finished_from_the_keyboard() {
    let mut harness = Harness::new();
    let side = feature_named(&harness, "Side sketch");
    assert!(offer(&harness, Command::EditFeature).availability.is_err());

    harness.click("Side sketch");
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.editing(), Some(side));
    run_from_palette(&mut harness, "finish sketch");
    assert_eq!(harness.editing(), None);

    let (extrude, _) = extruded_plate(&mut harness);
    assert!(offer(&harness, Command::CloseFeature).availability.is_err());
    harness.click("Extrude 1");
    assert_eq!(
        offer(&harness, Command::EditFeature).title(),
        "Edit feature: Extrude 1"
    );
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    harness.key(Key::E, Modifiers::NONE);
    harness.frame();
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Edit feature: Extrude 1 is already being edited")
    );

    run_from_palette(&mut harness, "finish editing feature");
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn a_sketch_is_placed_on_the_selected_face_and_detached_from_the_palette() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Add sketch");
    let mut loose = Sketch::new(Plane::XY);
    rectangle(&mut loose, Point2::new(5.0, 5.0), Point2::new(15.0, 15.0));
    let sketch = transaction.add_feature("Loose", FeatureKind::from(loose));
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    harness.click("Loose");
    assert_eq!(
        offer(&harness, Command::PlaceSketch).availability,
        Err("Select a datum plane or a flat face to place the sketch on".to_owned())
    );
    assert_eq!(
        offer(&harness, Command::DetachSketch).availability,
        Err("Loose does not lie on a face or datum plane".to_owned())
    );

    harness.select([top]);
    run_from_palette(&mut harness, "place sketch");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Place Loose on a face"));
    assert_eq!(attached_body(&harness, sketch), Some(extrude));
    assert_eq!(plane_height(&harness, sketch), 10.0);

    run_from_palette(&mut harness, "detach sketch");
    harness.settle();
    assert_eq!(
        harness.model.undo_label(),
        Some("Detach Loose from its face")
    );
    assert_eq!(attached_body(&harness, sketch), None);
    assert_eq!(harness.sketch(sketch).plane().origin().z, 10.0);
}

#[test]
fn a_revolve_and_a_datum_plane_take_the_selection_from_the_palette() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let edge = Pickable::Edge {
        body: plate,
        edge: top_edge_along_x(&harness, plate, 0.0),
    };

    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 30.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = harness
        .workspace
        .editing
        .solid()
        .expect("the revolve is open");
    assert_eq!(
        harness
            .solid(revolve)
            .axis()
            .and_then(caditor_document::RevolveAxis::model),
        None
    );
    harness.select([edge]);
    run_from_palette(&mut harness, "revolve about selected axis");
    harness.settle();
    assert!(
        harness
            .solid(revolve)
            .axis()
            .and_then(caditor_document::RevolveAxis::model)
            .is_some()
    );

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = harness
        .workspace
        .editing
        .solid()
        .expect("the plane is open");
    assert!(datum_plane(&harness, plane).normal().z.abs() > 0.999);
    assert_eq!(
        offer(&harness, Command::DatumUseSelected).availability,
        Err("Select a plane or flat face made before this plane".to_owned())
    );
    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    run_from_palette(&mut harness, "base datum on selection");
    harness.settle();
    assert!(datum_plane(&harness, plane).normal().y.abs() > 0.999);

    harness.select([edge]);
    run_from_palette(&mut harness, "turn datum plane about selected axis");
    harness.settle();
    let Some(caditor_document::Datum::Plane(turned)) = harness
        .document()
        .feature(plane)
        .and_then(|feature| feature.kind.datum())
    else {
        panic!("the plane is a datum plane");
    };
    assert!(turned.rotation.is_some());
}

#[test]
fn the_failed_pill_is_a_button_that_shows_the_first_failed_feature() {
    let mut harness = Harness::new();
    let height = harness.parameter("height");
    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();

    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    assert!(
        harness.accessible.iter().any(|(_, node)| {
            node.role() == Role::Button
                && node.label() == Some("1 feature failed")
                && node.live() == Some(egui::accesskit::Live::Assertive)
        }),
        "the failed pill is a live region once accessibility is on"
    );

    harness.click_button("1 feature failed");
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.workspace.panels.selected,
        Some(feature_named(&harness, "Side sketch"))
    );
}

#[test]
fn a_parameter_is_deleted_and_a_failed_feature_found_from_the_keyboard() {
    let mut harness = Harness::new();
    assert_eq!(
        offer(&harness, Command::DeleteParameter).availability,
        Err(
            "Click or tab into a parameter's name or expression in the Parameters section first"
                .to_owned()
        )
    );
    let width = harness.parameter("width");
    harness.focus(Focus::ParameterValue(width));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let offered = offer(&harness, Command::DeleteParameter);
    assert_eq!(offered.title(), "Delete parameter: width");
    assert!(offered.availability.is_ok());

    run_from_palette(&mut harness, "add parameter");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.document().parameter_named("parameter1").is_some());
    run_from_palette(&mut harness, "delete parameter");
    harness.settle();
    assert!(harness.document().parameter_named("parameter1").is_none());
    assert_eq!(harness.model.undo_label(), Some("Delete parameter1"));

    assert!(
        offer(&harness, Command::ShowFirstFailed)
            .availability
            .is_err()
    );
    let height = harness.parameter("height");
    harness.type_into(
        Focus::ParameterValue(height),
        "400 mm * 1 mm / (width - 30 mm)",
    );
    harness.type_into(Focus::ParameterValue(width), "30 mm");
    harness.settle();
    assert!(harness.shows("1 feature failed"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::F8, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.workspace.panels.selected,
        Some(feature_named(&harness, "Side sketch"))
    );
}

#[test]
fn a_used_parameter_is_deleted_by_writing_its_expression_into_its_uses() {
    let mut harness = Harness::new();
    assert!(!harness.shows(crate::icons::DELETE));
    harness.hover_button("Delete width");
    let explained = harness.shows_containing("Delete width and write 40 mm in its place")
        && harness.shows_containing("Used by height and Base sketch.");

    let mut transaction = harness.document().transaction("Add spare");
    transaction.add_parameter("spare", transaction.parse("3 mm").unwrap());
    harness.perform(Action::Apply(transaction.finish()));
    harness.frame();
    let spare_marked = harness.describes("spare is unused: nothing refers to it yet.");
    let width_marked = harness.describes("width is unused: nothing refers to it yet.");
    harness.click_button("Delete width");
    harness.settle();
    let document = harness.document();
    let height = document.parameter_named("height").unwrap();
    let base = feature_named(&harness, "Base sketch");
    let base_value = document
        .feature(base)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| sketch.constraints().find_map(|(_, held)| held.dimension()))
        .map(|value| document.expression_text(value));

    assert!(explained);
    assert!(spare_marked);
    assert!(!width_marked);
    assert!(document.parameter_named("width").is_none());
    assert_eq!(document.expression_text(&height.expression), "40 mm / 2");
    assert_eq!(base_value.as_deref(), Some("40 mm"));
    assert_eq!(harness.model.undo_label(), Some("Delete width"));
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Deleted width and wrote 40 mm into its 2 uses. Undo brings it back.")
    );
    assert_eq!(harness.model.evaluation().failed_count(), 0);

    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.document().parameter_named("width").is_some());
}

#[test]
fn parameters_are_reordered_and_noted_from_the_keyboard() {
    let mut harness = Harness::new();
    let order = |harness: &Harness| -> Vec<String> {
        harness
            .document()
            .parameters()
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect()
    };
    let height = harness.parameter("height");
    harness.focus(Focus::ParameterName(height));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let at_bottom = offer(&harness, Command::MoveParameterDown).availability;
    run_from_palette(&mut harness, "move parameter up");
    harness.frame();
    let moved = order(&harness);
    let at_top = offer(&harness, Command::MoveParameterUp).availability;
    run_from_palette(&mut harness, "parameter's note");
    harness.frame();
    let focused = harness.focused() == Some(Id::new("parameter-note-field"));
    harness.type_text("Half the width, for the side");
    harness.click(crate::parameter_table::SAVE_NOTE_LABEL);
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();

    assert_eq!(
        at_bottom,
        Err("height is already the last parameter".to_owned())
    );
    assert_eq!(moved, ["height", "width"]);
    assert_eq!(
        at_top,
        Err("height is already the first parameter".to_owned())
    );
    assert!(focused);
    assert_eq!(
        harness.document().parameter(height).unwrap().note,
        "Half the width, for the side"
    );
    assert!(harness.describes("Note on height: Half the width, for the side"));
    assert_eq!(harness.model.undo_label(), Some("Note on height"));
}

#[test]
fn model_properties_are_edited_in_a_dialog_as_one_undoable_change() {
    use caditor_document::ModelProperty;

    use crate::model_properties::{SAVE_LABEL, field_id};

    let mut harness = Harness::new();
    harness.context.enable_accesskit();

    run_from_palette(&mut harness, "model properties");
    harness.frame();
    let title_focused = harness.focused() == Some(field_id(ModelProperty::Title));
    let captioned = harness.captioned(Role::TextInput, "Part number");
    harness.type_text("Wall bracket");
    harness.type_into_field(field_id(ModelProperty::PartNumber), "BR-100");
    harness.type_into_field(field_id(ModelProperty::Notes), "Print it flat");
    harness.click(SAVE_LABEL);
    harness.frame();
    let closed = harness.workspace.model_properties.is_none();

    assert!(title_focused);
    assert!(captioned);
    assert!(closed);
    assert_eq!(harness.document().properties().title, "Wall bracket");
    assert_eq!(harness.document().properties().part_number, "BR-100");
    assert_eq!(harness.document().properties().notes, "Print it flat");
    assert_eq!(
        harness.model.undo_label(),
        Some(crate::model_properties::CHANGE_LABEL)
    );

    harness.perform(Action::Undo);
    assert!(harness.document().properties().is_empty());
}

#[test]
fn cancelling_the_model_properties_changes_nothing() {
    use caditor_document::ModelProperty;

    use crate::model_properties::{CANCEL_LABEL, field_id};

    let mut harness = Harness::new();
    let revision = harness.model.revision();

    harness.perform(Action::Preferences(
        crate::preferences::PreferencesCommand::ShowModelProperties,
    ));
    harness.frame();
    harness.type_into_field(field_id(ModelProperty::Revision), "B");
    harness.click(CANCEL_LABEL);
    harness.frame();

    assert!(harness.workspace.model_properties.is_none());
    assert!(harness.document().properties().is_empty());
    assert_eq!(harness.model.revision(), revision);
}

#[test]
fn the_feature_tree_is_filtered_by_name_from_the_keyboard() {
    let mut harness = Harness::new();
    let hidden_at_first = !harness.shows(crate::feature_tree::FILTER_HINT);

    harness.key(Key::F, Modifiers::COMMAND);
    harness.frame();
    let focused = harness.focused() == Some(Focus::TreeFilter.field_id());
    harness.type_text("SIDE");
    let side_only = harness.shows("Side sketch") && !harness.shows("Base sketch");
    harness.replace_text("bracket");
    let none_named = harness.shows("No feature is named like “bracket” or is of that kind.");
    harness.click(crate::feature_tree::CLEAR_FILTER_LABEL);
    harness.frame();

    assert!(hidden_at_first);
    assert!(focused);
    assert!(side_only);
    assert!(none_named);
    assert!(harness.shows("Side sketch") && harness.shows("Base sketch"));
    assert!(harness.workspace.panels.tree_filter.is_empty());
}

#[test]
fn the_feature_tree_filter_matches_kinds_as_well_as_names() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    for _ in 0..REVEAL_FRAMES {
        harness.frame();
    }
    harness.context.enable_accesskit();
    let row = |harness: &Harness, name: &str| {
        harness.accessible_named(Role::Button, &format!("More actions for {name}"))
    };

    harness.key(Key::F, Modifiers::COMMAND);
    harness.frame();
    harness.type_text("extrusion");
    harness.frame();
    let extrusions_only = row(&harness, "Extrude 1") && !row(&harness, "Base sketch");
    harness.replace_text("sketch");
    harness.frame();
    let sketches_only =
        row(&harness, "Base sketch") && row(&harness, "Side sketch") && !row(&harness, "Extrude 1");

    assert!(extrusions_only);
    assert!(sketches_only);
}

#[test]
fn tips_are_dismissed_and_hidden_from_the_palette() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    harness.click("Flanged spool");
    harness.settle();
    let keymap = harness.workspace.preferences.keymap.clone();
    assert!(harness.shows(&Hint::Navigate.text(&keymap, InputMode::default())));

    run_from_palette(&mut harness, "dismiss the tip");
    assert!(!harness.shows(&Hint::Navigate.text(&keymap, InputMode::default())));
    assert!(harness.shows(&Hint::Palette.text(&keymap, InputMode::default())));
    run_from_palette(&mut harness, "hide tips");
    assert!(!harness.shows("Tip"));
    assert!(!harness.workspace.preferences.onboarding.hints);
    assert!(offer(&harness, Command::DismissTip).availability.is_err());
}

fn assert_readable(harness: &Harness, screen: &str) {
    let unreadable = harness.unreadable_nodes();
    assert!(
        unreadable.is_empty(),
        "{screen} exposes glyphs or unnamed buttons: {unreadable:#?}"
    );
}

#[test]
fn icon_buttons_are_named_and_captions_label_their_fields_for_screen_readers() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    assert_readable(&harness, "The empty window");
    assert!(harness.accessible_named(Role::Button, "More actions for Side sketch"));
    assert!(harness.accessible_named(Role::Button, "Show details of Base sketch"));
    assert!(harness.accessible_named(Role::Button, "Edit Base sketch"));
    assert!(harness.accessible_named(Role::Button, "Add parameter"));
    assert!(harness.accessible_named(Role::Button, "Length unit: millimetres"));

    harness.click("File");
    assert_readable(&harness, "The File menu");
    assert!(harness.accessible_named(Role::Button, "Open sample"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.settle();
    assert_readable(&harness, "An open extrusion");
    assert!(harness.captioned(Role::TextInput, "Distance"));
    assert!(harness.captioned(Role::ComboBox, "Extent"));
    assert!(harness.captioned(Role::ComboBox, "Result"));

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    assert_readable(&harness, "An open datum plane");
    assert!(harness.captioned(Role::TextInput, "Offset"));
    assert!(harness.accessible_named(Role::Button, crate::feature_fields::CHOOSE_IN_VIEW));

    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.show_new_windows();
    assert_readable(&harness, "Preferences");
    for tab in PreferencesTab::ALL {
        assert!(harness.accessible_named(Role::Tab, tab.label()), "{tab:?}");
    }
    harness.click("Appearance");
    assert_readable(&harness, "The Appearance preferences");
    assert!(harness.accessible_named(Role::Button, "Make the interface smaller"));
    assert!(harness.accessible_named(Role::Button, "Make the interface larger"));
    harness.click("Navigation");
    assert!(harness.captioned(Role::Slider, "Orbit speed"));
    harness.click("Graphics");
    assert_readable(&harness, "The Graphics preferences");
    assert!(harness.captioned(Role::CheckBox, "Vsync"));
    assert!(harness.accessible_named(Role::Button, crate::graphics::COPY_DETAILS));

    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.show_new_windows();
    assert_readable(&harness, "The shortcut editor");
    assert!(harness.accessible_named(Role::Button, "Record a new shortcut for Undo"));
    assert!(harness.accessible_named(Role::Button, "Record a new shortcut for Redo"));
    assert!(harness.accessible_named(Role::Button, "Remove Ctrl+Z from Undo"));
    assert!(harness.accessible_named(
        Role::Button,
        "Reset Undo to the shortcut caditor starts with"
    ));
    assert!(!harness.accessible_named(Role::Button, "Add…"));
    assert!(!harness.accessible_named(Role::Button, "Reset"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    assert_readable(&harness, "The command palette");
    assert!(harness.accessible_named(Role::Button, "Fit view"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    let base = feature_named(&harness, "Base sketch");
    harness.edit(base);
    assert_readable(&harness, "An edited sketch");
    assert!(harness.accessible_named(Role::Button, "Horizontal"));
}

#[test]
fn the_welcome_dialog_and_tips_are_readable_by_screen_readers() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    assert!(harness.shows("Welcome to caditor"));
    assert_readable(&harness, "The welcome dialog");
    assert!(harness.accessible_named(Role::Button, "Close (Esc)"));
    assert!(harness.accessible_named(Role::Button, "Open the Flanged spool sample"));

    harness.click("Flanged spool");
    harness.settle();
    assert!(harness.shows("Got it"));
    assert_readable(&harness, "A tip");
}

fn drag_screen(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let position = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
}

#[test]
fn the_bars_and_the_parameter_grid_wrap_or_shrink_rather_than_overlap_at_200_percent() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();

    let document = harness.document().clone();
    let name = "Bracket for the front suspension, revised after the second test.caditor";
    harness
        .model
        .replace(document, Some(dir.path().join(name)), None, false);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    let notice = "The sketch could not be placed on the selected face, because the face is curved; \
                  choose a flat face or a plane";
    harness.perform(Action::Inform(Notice::error(notice)));
    harness.frame();
    harness.frame();
    let panel_id = Id::new("model");
    let panel = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    drag_screen(
        &mut harness,
        Pos2::new(panel.max.x - 1.0, 300.0),
        Pos2::new(60.0, 300.0),
    );
    let narrowed = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    for _ in 0..5 {
        harness.frame();
    }
    let settled = egui::containers::panel::PanelState::load(&harness.context, panel_id)
        .unwrap()
        .outer_rect;
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);
    let rect_of = |label: &str| {
        harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1
    };
    let line = rect_of(status_bar::UP_TO_DATE).height();
    let shown: Vec<(&str, Rect)> = harness
        .texts
        .iter()
        .zip(&harness.text_clips)
        .map(|((text, rect), clip)| (text.as_str(), clip.intersect(*rect)))
        .filter(|(_, rect)| rect.is_positive())
        .collect();
    let overlapping: Vec<(&str, &str)> = shown
        .iter()
        .enumerate()
        .flat_map(|(index, (first, a))| {
            shown[index + 1..]
                .iter()
                .filter(move |(_, b)| a.shrink(0.5).intersects(b.shrink(0.5)))
                .map(move |(second, _)| (*first, *second))
        })
        .collect();
    let outside: Vec<&str> = harness
        .texts
        .iter()
        .filter(|(_, rect)| !visible.expand(0.5).contains_rect(*rect))
        .map(|(shown, _)| shown.as_str())
        .collect();

    assert!(narrowed.width() < panel.width(), "{narrowed:?}");
    assert_eq!(settled, narrowed);
    assert!(overlapping.is_empty(), "{overlapping:#?}");
    assert!(outside.is_empty(), "{outside:#?}");
    assert!(rect_of(notice).height() > 1.5 * line, "the notice wraps");
    assert!(rect_of(name).max.x <= rect_of("Search commands").min.x);
    harness.button_rect("Delete width");
    let deletes: Vec<Rect> = ["width", "height"]
        .iter()
        .filter_map(|name| {
            let label = format!("Delete {name}");
            harness
                .accessible
                .iter()
                .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(&label))
                .find_map(|(_, node)| node.bounds())
        })
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
        .collect();
    assert!(!deletes.is_empty());
    assert!(deletes.iter().all(|rect| rect.max.x <= narrowed.max.x));
}

fn feature_order(harness: &Harness) -> Vec<String> {
    harness
        .document()
        .features()
        .map(|feature| feature.name.clone())
        .collect()
}

#[test]
fn parameters_are_added_renamed_given_expressions_and_deleted_in_their_table() {
    let mut harness = Harness::new();
    harness.click_beside(crate::icons::ADD, "Parameters");
    harness.frame();
    let added = harness.parameter("parameter1");
    let added_label = harness.model.undo_label().map(str::to_owned);
    let focused_on_name = harness.focused() == Some(Focus::ParameterName(added).field_id());
    harness.key(Key::A, Modifiers::COMMAND);
    harness.events.push(Event::Text("depth".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
    let renamed = harness.document().parameter(added).map(|p| p.name.clone());
    harness.type_into(Focus::ParameterValue(added), "width / 4");
    harness.settle();
    let quarter_shown = harness.shows("10 mm");
    let width = harness.parameter("width");
    harness.type_into(Focus::ParameterName(width), "span");
    harness.settle();
    let height_text = harness.expression_text("height");
    let depth_text = harness.expression_text("depth");
    harness.click_button("Delete depth");
    harness.settle();
    let deleted_label = harness.model.undo_label().map(str::to_owned);
    let deleted = harness.document().parameter(added).is_none();
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(added_label.as_deref(), Some("Add parameter1"));
    assert!(focused_on_name);
    assert_eq!(renamed.as_deref(), Some("depth"));
    assert!(quarter_shown);
    assert_eq!(height_text, "span / 2");
    assert_eq!(depth_text, "span / 4");
    assert!(harness.shows("span / 2"));
    assert_eq!(deleted_label.as_deref(), Some("Delete depth"));
    assert!(deleted);
    assert_eq!(harness.expression_text("depth"), "span / 4");
}

#[test]
fn features_move_up_and_down_from_their_menu() {
    let mut harness = Harness::new();
    harness.click_beside(crate::icons::MORE, "Side sketch");
    harness.click("Move up");
    harness.settle();
    let moved_up = feature_order(&harness);
    let up_label = harness.model.undo_label().map(str::to_owned);
    harness.click_beside(crate::icons::MORE, "Side sketch");
    let first_cannot_rise = harness.shows("Move up") && {
        harness.click("Move up");
        harness.settle();
        feature_order(&harness) == moved_up
    };
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click_beside(crate::icons::MORE, "Side sketch");
    harness.click("Move down");
    harness.settle();

    assert_eq!(moved_up, ["Side sketch", "Base sketch"]);
    assert_eq!(up_label.as_deref(), Some("Move up Side sketch"));
    assert!(first_cannot_rise);
    assert_eq!(feature_order(&harness), ["Base sketch", "Side sketch"]);
    assert_eq!(harness.model.undo_label(), Some("Move down Side sketch"));
}

fn open_solid(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a solid feature is open")
}

fn extrude_extent(harness: &Harness, feature: FeatureId) -> &'static str {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => match extrude.extent {
            ExtrudeExtent::OneSide { .. } => "one side",
            ExtrudeExtent::Symmetric { .. } => "symmetric",
            ExtrudeExtent::TwoSides { .. } => "two sides",
        },
        SolidFeature::Revolve(_) => "a revolve",
    }
}

fn choose(harness: &mut Harness, current: &str, option: &str) {
    harness.hold_still();
    harness.click_lowest(current);
    harness.click_lowest(option);
    harness.settle();
}

#[test]
fn a_solid_feature_changes_its_extent_result_body_sketch_and_axis_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut boss = Sketch::new(Plane::XY);
    rectangle(&mut boss, Point2::new(10.0, 10.0), Point2::new(20.0, 20.0));
    let boss = harness.add_sketch(boss);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let second = open_solid(&harness);
    let starts_adding = harness.solid(second).operation() == BodyOperation::Add(plate);

    choose(&mut harness, "One side", "Symmetric");
    let symmetric = extrude_extent(&harness, second);
    choose(&mut harness, "Symmetric", "Two sides");
    let two_sides = extrude_extent(&harness, second);
    choose(&mut harness, "Add to body", "Remove from body");
    let removing = harness.solid(second).operation();
    choose(&mut harness, "Remove from body", "New body");
    let separate = harness.solid(second).operation();
    choose(&mut harness, "Plate 1", "Plate");
    let sketch = harness.solid(second).sketch();
    choose(&mut harness, "Plate", "Plate 1");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let third = open_solid(&harness);
    let adds_to_last = harness.solid(third).operation() == BodyOperation::Add(second);
    harness.hold_still();
    let third_card_in_view = harness.shows("Body");
    choose(&mut harness, "Extrude 2", "Extrude 1");
    let retargeted = harness.solid(third).operation();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(50.0, 0.0),
        Point2::new(60.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    choose(&mut harness, "Vertical axis", "Horizontal axis");
    let axis = harness.solid(revolve).axis().cloned();
    choose(&mut harness, "Full turn", "Symmetric");
    let turn = match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.extent.clone(),
        SolidFeature::Extrude(_) => panic!("expected a revolve"),
    };

    assert!(starts_adding);
    assert_eq!(symmetric, "symmetric");
    assert_eq!(two_sides, "two sides");
    assert_eq!(removing, BodyOperation::Remove(plate));
    assert_eq!(separate, BodyOperation::NewBody);
    assert_eq!(sketch, feature_named(&harness, "Plate"));
    assert_eq!(harness.solid(second).sketch(), boss);
    assert!(adds_to_last);
    assert!(third_card_in_view);
    assert_eq!(retargeted, BodyOperation::Add(plate));
    assert_eq!(
        axis,
        Some(caditor_document::RevolveAxis::Sketch(
            caditor_sketch::Reference::HorizontalAxis.id()
        ))
    );
    assert!(matches!(
        turn,
        caditor_document::RevolveExtent::Symmetric { .. }
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

fn extent_of(harness: &Harness, feature: FeatureId) -> ExtrudeExtent {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => extrude.extent.clone(),
        SolidFeature::Revolve(_) => panic!("expected an extrusion"),
    }
}

fn open_combo(harness: &mut Harness, caption: &str) {
    if harness.accessible.is_empty() {
        harness.context.enable_accesskit();
        harness.frame();
    }
    harness.hold_still();
    let captions: Vec<NodeId> = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Label && node.value() == Some(caption))
        .map(|(id, _)| *id)
        .collect();
    let bounds = harness
        .accessible
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::ComboBox
                && node
                    .labelled_by()
                    .iter()
                    .any(|label| captions.contains(label))
        })
        .filter_map(|(_, node)| node.bounds())
        .max_by(|a, b| a.y0.total_cmp(&b.y0))
        .unwrap_or_else(|| panic!("no list captioned '{caption}'"));
    let center = Pos2::new(
        (0.5 * (bounds.x0 + bounds.x1)) as f32,
        (0.5 * (bounds.y0 + bounds.y1)) as f32,
    );
    harness.click_screen(center);
    harness.show_new_windows();
    harness.frame();
}

fn extrusion_above_plate(harness: &mut Harness, height: f64) -> FeatureId {
    let plane = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, height),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut profile = Sketch::new(plane);
    rectangle(
        &mut profile,
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 20.0),
    );
    harness.add_sketch(profile);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    open_solid(harness)
}

fn target_origin(extent: &ExtrudeExtent) -> Option<caditor_kernel::FaceOrigin> {
    let ExtrudeExtent::OneSide {
        end:
            caditor_document::ExtrudeEnd::UpToFace(caditor_document::PlaneReference::Face(attachment)),
        ..
    } = extent
    else {
        return None;
    };
    attachment.face.origin()
}

#[test]
fn an_extrusion_cuts_through_all_or_up_to_the_next_face_chosen_in_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let cut = extrusion_above_plate(&mut harness, 10.0);

    open_combo(&mut harness, "End");
    harness.hover("Through all");
    let explained = harness.shows(
        "Through all cuts into a body or intersects with it; choose Remove from body or Intersect \
         with body first",
    );
    open_combo(&mut harness, "End");
    harness.settle();
    let while_adding = extent_of(&harness, cut);
    choose(&mut harness, "Add to body", "Remove from body");
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();
    open_combo(&mut harness, "End");
    harness.click_lowest("Through all");
    harness.settle();
    let through = extent_of(&harness, cut);
    let through_volume = harness.body_volume(plate);
    choose(&mut harness, "Through all", "Up to next");
    let next = extent_of(&harness, cut);
    let next_volume = harness.body_volume(plate);
    harness.perform(Action::Undo);
    harness.settle();
    let undone = extent_of(&harness, cut);

    assert!(explained);
    assert!(matches!(
        while_adding,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::Distance(_),
            reversed: false
        }
    ));
    assert_eq!(
        through,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::ThroughAll,
            reversed: true
        }
    );
    assert!((through_volume - 15_000.0).abs() < 1.0);
    assert_eq!(
        next,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::UpToNext,
            reversed: true
        }
    );
    assert!((next_volume - 15_000.0).abs() < 1.0);
    assert_eq!(undone, through);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn an_extrusion_runs_up_to_a_face_chosen_in_the_view_and_chosen_again() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
    harness.settle();

    open_combo(&mut harness, "End");
    harness.click_lowest("Up to face");
    harness.settle();
    let asks_for_a_face = harness.shows("Click a flat face or plane to extrude up to.");
    let still_a_distance = extent_of(&harness, tower);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let picking_ended = harness.workspace.editing.picking().is_none();
    let reached = extent_of(&harness, tower);
    let volume = harness.body_volume(plate);
    let named = harness.shows("Extrude 1 end face");

    let bottom = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Extrude 1 start face"
        })
        .expect("the bottom face is pickable");
    harness.select([bottom]);
    harness.click_button("Use selected");
    harness.settle();
    let chosen_again = extent_of(&harness, tower);
    harness.select([top]);
    run_from_palette(&mut harness, "extrude up to selected");
    harness.settle();
    let from_palette = extent_of(&harness, tower);

    let plate_feature = plate.raw();
    assert!(asks_for_a_face);
    assert!(matches!(
        still_a_distance,
        ExtrudeExtent::OneSide {
            end: caditor_document::ExtrudeEnd::Distance(_),
            ..
        }
    ));
    assert!(picking_ended);
    assert_eq!(
        target_origin(&reached),
        Some(caditor_kernel::FaceOrigin::EndCap {
            feature: plate_feature
        })
    );
    assert!(matches!(
        reached,
        ExtrudeExtent::OneSide { reversed: true, .. }
    ));
    assert!((volume - 18_000.0).abs() < 1.0);
    assert!(named);
    assert_eq!(
        target_origin(&chosen_again),
        Some(caditor_kernel::FaceOrigin::StartCap {
            feature: plate_feature
        })
    );
    assert_eq!(from_palette, reached);
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

fn start_of_solid(harness: &Harness, feature: FeatureId) -> Option<caditor_document::SolidStart> {
    harness.solid(feature).start().cloned()
}

#[test]
fn an_extrusion_starts_at_a_face_chosen_in_the_view_and_goes_back_to_its_sketch_plane() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let tower = extrusion_above_plate(&mut harness, 30.0);
    let highest = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(plate)
            .and_then(|result| result.solid())
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| bounds.max().z)
            .unwrap()
    };

    open_combo(&mut harness, "Start");
    harness.click_lowest("Face or plane");
    harness.settle();
    let asks_for_a_face =
        harness.shows("Click a flat face or plane parallel to the sketch to start from.");
    let still_at_the_sketch = start_of_solid(&harness, tower);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let picking_ended = harness.workspace.editing.picking().is_none();
    let started = start_of_solid(&harness, tower);
    let from_the_face = highest(&harness);
    let named = harness.shows("Starts at");

    harness.click_button("Start at the sketch plane again");
    harness.settle();
    let cleared = start_of_solid(&harness, tower);
    let back_at_the_sketch = highest(&harness);

    assert!(asks_for_a_face);
    assert_eq!(still_at_the_sketch, None);
    assert!(picking_ended);
    assert!(named);
    assert!(matches!(
        started,
        Some(caditor_document::SolidStart::Plane(
            caditor_document::PlaneReference::Face(ref attachment)
        )) if attachment.body == plate
            && attachment.face.origin()
                == Some(caditor_kernel::FaceOrigin::EndCap { feature: plate.raw() })
    ));
    assert!((from_the_face - 20.0).abs() < 1e-9);
    assert_eq!(cleared, None);
    assert!((back_at_the_sketch - 40.0).abs() < 1e-9);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

#[test]
fn a_revolve_turns_by_two_angles_set_in_its_panel() {
    let mut harness = Harness::new();
    let mut section = Sketch::new(Plane::XZ);
    rectangle(
        &mut section,
        Point2::new(10.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    harness.add_sketch(section);
    harness.select([]);
    harness.click("Revolve");
    harness.settle();
    let revolve = open_solid(&harness);
    let full = PI * (400.0 - 100.0) * 10.0;

    choose(&mut harness, "Full turn", "Two angles");
    let both_ways = harness.body_volume(revolve);
    let field = Id::new(("solid-field", "forward-angle", revolve));
    harness.type_into_field(field, "350 deg");
    let refused = harness.shows(crate::feature_fields::TURNS_TOGETHER);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.type_into_field(field, "90 deg");
    harness.settle();
    let turned = match harness.solid(revolve) {
        SolidFeature::Revolve(revolve) => revolve.extent.clone(),
        SolidFeature::Extrude(_) => panic!("expected a revolve"),
    };

    assert!((both_ways - full * 210.0 / 360.0).abs() < 0.01 * full);
    assert!(refused);
    assert_eq!(
        turned,
        caditor_document::RevolveExtent::TwoSides {
            forward: Expression::Measure(90.0, Unit::Degree),
            backward: Expression::Measure(30.0, Unit::Degree),
        }
    );
    assert!((harness.body_volume(revolve) - full / 3.0).abs() < 0.01 * full);
}

fn datum_plane_of(harness: &Harness, feature: FeatureId) -> caditor_document::DatumPlane {
    match datum_of(harness, feature) {
        caditor_document::Datum::Plane(plane) => plane.clone(),
        other => panic!("expected a datum plane offset from another, found {other:?}"),
    }
}

#[test]
fn a_datum_takes_its_base_and_turn_from_the_selection_in_its_panel() {
    let mut harness = Harness::new();
    harness.select([]);
    harness.click("Plane");
    harness.settle();
    let plane = open_solid(&harness);
    harness.hold_still();

    harness.select([Pickable::Plane(PrincipalPlane::Xz)]);
    harness.click_beside("Use selected", "Starts from");
    harness.settle();
    let based = datum_plane(&harness, plane).normal();
    harness.select([Pickable::Axis(crate::selection::Axis::Z)]);
    harness.click_beside("Use selected", "Turned about");
    harness.settle();
    let turned = datum_plane_of(&harness, plane).rotation.is_some();
    harness.hold_still();
    harness.type_into_field(Id::new(("datum-field", "angle", plane)), "90 deg");
    harness.settle();
    let quarter = datum_plane(&harness, plane).normal();
    harness.click_button("Stop turning the plane");
    harness.settle();
    let unturned = datum_plane_of(&harness, plane).rotation.is_none();

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([
        Pickable::Plane(PrincipalPlane::Xy),
        Pickable::Plane(PrincipalPlane::Xz),
    ]);
    harness.click("Axis");
    harness.settle();
    let axis = open_solid(&harness);
    harness.hold_still();
    let meeting = matches!(
        datum_of(&harness, axis),
        caditor_document::Datum::Axis(caditor_document::DatumAxis::Intersection(..))
    );
    harness.select([Pickable::Axis(crate::selection::Axis::Z)]);
    harness.click_beside("Use selected", "Defined by");
    harness.settle();

    assert!(based.y.abs() > 0.999, "{based}");
    assert!(turned);
    assert!(quarter.x.abs() > 0.999, "{quarter}");
    assert!(unturned);
    assert!(datum_plane(&harness, plane).normal().y.abs() > 0.999);
    assert!(meeting);
    assert!(matches!(
        datum_of(&harness, axis),
        caditor_document::Datum::Axis(caditor_document::DatumAxis::Along(_))
    ));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}

fn hidden(harness: &Harness, feature: FeatureId) -> bool {
    harness.document().feature(feature).unwrap().hidden
}

#[test]
fn swept_sketches_hide_and_bodies_and_sketches_hide_and_show_again() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let sketch = feature_named(&harness, "Plate");
    let sketch_hidden_by_extrude = hidden(&harness, sketch);
    let pickables =
        |harness: &mut Harness| -> Vec<Pickable> { harness.built().picks.pickables().collect() };
    let sketch_pickable = pickables(&mut harness).iter().any(
        |pickable| matches!(pickable, Pickable::SketchEntity { feature, .. } if *feature == sketch),
    );

    harness.let_animations_finish();
    harness.click_beside(crate::icons::HIDE, "Plate");
    harness.settle();
    let shown_from_tree = !hidden(&harness, sketch);
    let show_label = harness.model.undo_label().map(str::to_owned);

    let face = pickables(&mut harness)
        .into_iter()
        .find(|pickable| matches!(pickable, Pickable::Face { .. }))
        .expect("the body has faces");
    harness.select([face]);
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let body_hidden = hidden(&harness, plate);
    let faces_left = pickables(&mut harness)
        .iter()
        .any(|pickable| matches!(pickable, Pickable::Face { .. }));
    let selection_cleared = harness.workspace.viewport.selection().is_empty();

    harness.key(Key::H, Modifiers::ALT);
    harness.settle();

    assert!(sketch_hidden_by_extrude);
    assert!(!sketch_pickable);
    assert!(shown_from_tree);
    assert_eq!(show_label.as_deref(), Some("Show Plate"));
    assert!(body_hidden);
    assert!(!faces_left);
    assert!(selection_cleared);
    assert!(!hidden(&harness, plate) && !hidden(&harness, sketch));
    assert_eq!(harness.model.undo_label(), Some("Show everything"));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(hidden(&harness, plate));
}

#[test]
fn hiding_others_keeps_only_the_selected_body_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let sketch = feature_named(&harness, "Plate");
    harness.click_beside(crate::icons::HIDE, "Plate");
    harness.settle();
    let sketch_shown = !hidden(&harness, sketch);

    harness.select([top]);
    harness.key(Key::H, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();

    assert!(sketch_shown);
    assert!(hidden(&harness, sketch));
    assert!(!hidden(&harness, plate));
    assert_eq!(
        harness.model.undo_label(),
        Some("Hide everything but Extrude 1")
    );
    harness.perform(Action::Undo);
    harness.settle();
    assert!(!hidden(&harness, sketch));
}

#[test]
fn looking_at_a_flat_face_turns_the_view_to_face_it_head_on() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.key(Key::V, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let toward_eye =
        harness.workspace.viewport.viewpoint().orientation * caditor_geometry::Vector3::Z;
    assert!(
        (toward_eye - caditor_geometry::Vector3::Z).length() < 1e-6,
        "{toward_eye}"
    );
}

fn selected_of(harness: &Harness, wanted: fn(&Pickable) -> bool) -> usize {
    harness
        .workspace
        .viewport
        .selection()
        .iter()
        .filter(wanted)
        .count()
}

fn is_edge(pickable: &Pickable) -> bool {
    matches!(pickable, Pickable::Edge { .. })
}

fn is_face(pickable: &Pickable) -> bool {
    matches!(pickable, Pickable::Face { .. })
}

#[test]
fn select_all_takes_every_face_or_edge_of_the_shown_bodies_by_the_selection_filter() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Faces);
    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_face), 6);
    assert_eq!(harness.workspace.viewport.selection().iter().count(), 6);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 12);
    assert_eq!(harness.workspace.viewport.selection().iter().count(), 12);
}

#[test]
fn select_all_without_a_kind_to_take_says_what_to_choose() {
    let mut harness = Harness::new();
    let (_, _) = extruded_plate(&mut harness);
    harness.select([]);

    harness.key(Key::A, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();

    harness.frame();

    assert!(harness.workspace.viewport.selection().is_empty());
    assert!(harness.shows_containing(crate::body_selection::NO_KIND_TO_SELECT));
}

#[test]
fn the_edges_around_a_face_replace_the_selected_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.key(Key::E, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 4);
    assert_eq!(selected_of(&harness, is_face), 0);
}

#[test]
fn tangent_edges_join_the_selection_along_smooth_joins_and_a_corner_says_there_are_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    sketch.add_arc(
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    sketch.add_line(Point2::new(20.0, 10.0), Point2::new(0.0, 10.0));
    sketch.add_arc(
        Point2::new(0.0, 5.0),
        Point2::new(0.0, 10.0),
        Point2::new(0.0, 0.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let edges: Vec<Pickable> = harness.built().picks.pickables().filter(is_edge).collect();
    let rim: Vec<Pickable> = edges
        .iter()
        .copied()
        .filter(|edge| {
            edge.describe(harness.document(), harness.model.evaluation())
                .contains("end face")
        })
        .collect();
    assert_eq!(rim.len(), 4);

    harness.select([rim[0]]);
    harness.key(Key::T, Modifiers::ALT);
    harness.frame();

    assert_eq!(selected_of(&harness, is_edge), 4);

    harness.key(Key::T, Modifiers::ALT);
    harness.frame();

    harness.frame();

    assert!(harness.shows_containing(crate::body_selection::NO_TANGENT_EDGES));
}

#[test]
fn tangent_faces_join_the_selection_around_rounded_ends_and_a_flat_end_says_there_are_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    sketch.add_arc(
        Point2::new(20.0, 5.0),
        Point2::new(20.0, 0.0),
        Point2::new(20.0, 10.0),
    );
    sketch.add_line(Point2::new(20.0, 10.0), Point2::new(0.0, 10.0));
    sketch.add_arc(
        Point2::new(0.0, 5.0),
        Point2::new(0.0, 10.0),
        Point2::new(0.0, 0.0),
    );
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let faces: Vec<Pickable> = harness.built().picks.pickables().filter(is_face).collect();
    let describe = |face: &Pickable| face.describe(harness.document(), harness.model.evaluation());
    let side = faces
        .iter()
        .copied()
        .find(|face| !describe(face).contains("end face"))
        .unwrap();
    let end = faces
        .iter()
        .copied()
        .find(|face| describe(face).contains("end face"))
        .unwrap();

    harness.select([side]);
    harness.key(Key::T, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    let around = selected_of(&harness, is_face);
    harness.select([end]);
    harness.key(Key::T, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    harness.frame();

    assert_eq!(faces.len(), 6);
    assert_eq!(around, 4);
    assert_eq!(selected_of(&harness, is_face), 1);
    assert!(harness.shows_containing(crate::body_selection::NO_TANGENT_FACES));
}

fn rows_named(harness: &Harness, label: &str) -> Vec<Rect> {
    let mut rows: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == label)
        .map(|(_, rect)| *rect)
        .collect();
    rows.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
    rows
}

#[test]
fn the_bodies_group_lists_every_body_and_hides_or_selects_one() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    harness.settle();

    assert!(harness.shows("Bodies (2)"));
    assert_eq!(rows_named(&harness, "Peg").len(), 1);
    harness.click("Bodies (2)");
    harness.settle();
    let rows = rows_named(&harness, "Peg");
    assert_eq!(rows.len(), 2);

    let listed = rows[1];
    harness.click_screen(listed.center());
    harness.settle();
    assert_eq!(harness.workspace.panels.selected, Some(peg));

    let eye = harness
        .texts
        .iter()
        .filter(|(shown, rect)| {
            shown == crate::icons::SHOW && (rect.center().y - listed.center().y).abs() < 4.0
        })
        .map(|(_, rect)| rect.center())
        .next()
        .expect("the listed body has an eye");
    harness.click_screen(eye);
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Hide Peg"));
    assert!(hidden(&harness, peg));
    assert!(!hidden(&harness, plate));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(!hidden(&harness, peg));
}

fn principal_pickables(harness: &mut Harness) -> Vec<Pickable> {
    harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| crate::visibility::principal(*pickable).is_some())
        .collect()
}

#[test]
fn principal_planes_axes_and_origin_hide_and_planes_return_while_choosing_one() {
    let mut harness = Harness::new();
    harness.settle();
    let shown_at_first = principal_pickables(&mut harness).len();

    harness.click_beside(crate::icons::SHOW, crate::principal_tree::GROUP_TITLE);
    harness.settle();
    let hidden_label = harness.model.undo_label().map(str::to_owned);
    let left_after_hiding = principal_pickables(&mut harness);

    harness.perform(Action::Editing(EditingCommand::NewSketch(None)));
    harness.settle();
    let offered_while_choosing = principal_pickables(&mut harness);
    harness.perform(Action::Editing(EditingCommand::CancelNewSketch));
    harness.settle();
    let left_after_choosing = principal_pickables(&mut harness).len();

    harness.key(Key::H, Modifiers::ALT);
    harness.settle();
    let shown_again = principal_pickables(&mut harness).len();
    let xy = Pickable::Plane(PrincipalPlane::Xy);
    harness.select([xy]);
    harness.key(Key::H, Modifiers::NONE);
    harness.settle();
    let after_hiding_xy = principal_pickables(&mut harness);

    assert_eq!(shown_at_first, 7);
    assert_eq!(
        hidden_label.as_deref(),
        Some("Hide principal planes, axes and origin")
    );
    assert!(left_after_hiding.is_empty());
    assert_eq!(
        offered_while_choosing,
        PrincipalPlane::ALL.map(Pickable::Plane).to_vec()
    );
    assert_eq!(left_after_choosing, 0);
    assert_eq!(shown_again, 7);
    assert_eq!(after_hiding_xy.len(), 6);
    assert!(!after_hiding_xy.contains(&xy));
    assert_eq!(harness.model.undo_label(), Some("Hide XY plane"));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn o_switches_the_view_to_orthographic_and_back_and_the_preference_remembers_it() {
    let mut harness = Harness::new();
    harness.settle();
    let view = |harness: &Harness| harness.workspace.viewport.current_view().unwrap();
    let before = view(&harness);

    harness.key(Key::O, Modifiers::NONE);
    harness.settle();
    let switched = view(&harness);
    let remembered = harness.workspace.preferences.navigation.projection;
    let offered = harness
        .workspace
        .last_offers
        .iter()
        .any(|offer| offer.command == Command::ToggleProjection);
    harness.key(Key::O, Modifiers::NONE);
    harness.settle();
    let back = view(&harness);

    assert!(!before.is_orthographic());
    assert!(switched.is_orthographic());
    assert_eq!(remembered, caditor_render::Projection::Orthographic);
    assert!(offered);
    assert_eq!(switched.viewpoint(), before.viewpoint());
    assert!(
        (switched.units_per_pixel_at(1.0) - before.units_per_pixel_at(before.viewpoint().distance))
            .abs()
            < 1e-9
    );
    assert!(!back.is_orthographic());
    assert_eq!(
        harness.workspace.preferences.navigation.projection,
        caditor_render::Projection::Perspective
    );
}

#[test]
fn an_orthographic_preference_survives_the_first_fit_and_o_switches_back() {
    let mut preferences = Preferences::default();
    preferences.onboarding = crate::onboarding::Onboarding::finished();
    preferences.navigation.projection = caditor_render::Projection::Orthographic;
    let mut harness = Harness::starting(
        None,
        sample_document().unwrap(),
        Workspace::with_preferences(preferences),
    );
    harness.settle();
    let started = harness.workspace.viewport.current_view().unwrap();

    harness.key(Key::O, Modifiers::NONE);
    harness.settle();

    assert!(started.is_orthographic());
    assert!(
        !harness
            .workspace
            .viewport
            .current_view()
            .unwrap()
            .is_orthographic()
    );
    assert_eq!(
        harness.workspace.preferences.navigation.projection,
        caditor_render::Projection::Perspective
    );
}

#[test]
fn a_suppressed_sketch_counts_toward_fitting_the_view_no_more() {
    let mut harness = Harness::new();
    harness.settle();
    let mut far = Sketch::new(Plane::XY);
    far.add_line(Point2::new(900.0, 900.0), Point2::new(1000.0, 1000.0));
    let sketch = harness.add_sketch(far);
    harness.settle();
    let reaching = harness.built().fit_all().max().x;

    harness.perform(Action::Apply(Transaction::single(
        "Suppress",
        Edit::SetFeatureSuppressed {
            id: sketch,
            suppressed: true,
        },
    )));
    harness.settle();

    assert!(reaching >= 1000.0, "{reaching}");
    assert!(harness.built().fit_all().max().x < 900.0);
}

#[test]
fn look_at_sketch_turns_the_view_square_on_to_the_edited_sketch_again() {
    let mut harness = Harness::new();
    let mut slanted = Sketch::new(Plane::XZ);
    slanted.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    edit_free_sketch(&mut harness, slanted);
    let facing = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();
    let orbit = Command::Camera(crate::commands::CameraMove::OrbitLeft).default_shortcuts()[0];
    for _ in 0..3 {
        harness.key(orbit.logical_key, orbit.modifiers);
    }
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let orbited = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();

    harness.key(Key::V, Modifiers::ALT | Modifiers::SHIFT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let back = *harness
        .workspace
        .viewport
        .current_view()
        .unwrap()
        .viewpoint();

    let turned = |viewpoint: caditor_render::Viewpoint| {
        viewpoint.orientation.angle_between(facing.orientation)
    };
    assert!(turned(orbited) > 0.1);
    assert!(turned(back) < 1e-6, "{}", turned(back));
}

fn edit_free_sketch(harness: &mut Harness, sketch: Sketch) -> FeatureId {
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    feature
}

fn drag_in_sketch(harness: &mut Harness, from: Pos2, to: Point2) {
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(to);
    for step in 1..=4 {
        let position = from + (target - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
}

#[test]
fn dragging_a_sketch_point_moves_it_as_its_constraints_allow_in_one_undoable_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 12.0));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let dragged = harness.sketch(feature).point(end).unwrap();
    assert!(
        dragged.distance(Point2::new(30.0, 12.0)) < 0.5,
        "{dragged:?}"
    );
    assert!(harness.sketch(feature).point(start).unwrap().length() < DRAWN);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(end)).as_str())
    );
    let shown = harness.shown(feature).point(end).unwrap();
    assert!(shown.distance(dragged) < DRAWN);

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.sketch(feature).point(end), before.point(end));
}

#[test]
fn a_point_dragged_onto_another_snaps_and_joins_it_in_the_same_change() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    let lone = sketch.add_point(Point2::new(30.0, 12.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.02, 12.02));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(
        sketch
            .constraints()
            .any(|(_, constraint)| *constraint == Constraint::Coincident(end, lone))
    );
    assert!(sketch.point(end).unwrap().distance(Point2::new(30.0, 12.0)) < 1e-6);
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Drag {}", before.entity_label(end)).as_str())
    );
}

#[test]
fn a_crowded_entity_shows_a_few_glyphs_and_counts_the_rest() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(60.0, 0.0));
    for index in 1..=7 {
        let y = f64::from(index) * 12.0;
        let other = sketch.add_line(Point2::new(0.0, y), Point2::new(60.0, y));
        sketch
            .add_constraint(Constraint::Equal(base, other))
            .unwrap();
    }
    edit_free_sketch(&mut harness, sketch);
    harness.settle();
    harness.frame();

    assert!(harness.shows("+4"));
    assert!(!harness.shows("+7"));

    harness.key(Key::G, Modifiers::ALT);
    harness.frame();
    harness.frame();
    assert!(!harness.workspace.viewport.glyphs_shown());
    assert!(!harness.shows("+4"));
    harness.key(Key::G, Modifiers::ALT);
    harness.frame();
    harness.frame();
    assert!(harness.shows("+4"));
}

#[test]
fn a_drag_in_a_conflicting_sketch_names_the_conflict() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    for constraint in [
        Constraint::Coincident(start, EntityId::ORIGIN),
        Constraint::Horizontal(line),
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::parse_stored("40 mm").unwrap(),
        },
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::parse_stored("50 mm").unwrap(),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    let conflict = harness.model.sketch_conflict(feature);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(Point2::new(30.0, 12.0));
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            grabbed + (target - grabbed) * (step as f32 / 4.0),
        ));
        harness.frame();
    }
    harness.wait_until("the cue shows", |harness| {
        harness.shows_containing(crate::viewport::DRAG_CONFLICT)
    });
    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.wait_until("the drag ends", |harness| !harness.model.drag_blocked());
    harness.frame();

    let conflict = conflict.expect("the sketch conflicts");
    assert!(conflict.contains("Distance"), "{conflict}");
    assert!(harness.shows_containing(&conflict));
}

#[test]
fn a_drag_the_constraints_cannot_follow_says_so_until_it_ends() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let (start, end) = line_ends(&sketch, line);
    for constraint in [
        Constraint::Coincident(start, EntityId::ORIGIN),
        Constraint::Horizontal(line),
        Constraint::Vertical(line),
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let feature = edit_free_sketch(&mut harness, sketch);

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(40.0, 0.0),
        Pickable::SketchEntity {
            feature,
            entity: end,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let target = harness.on_screen(Point2::new(30.0, 12.0));
    for step in 1..=4 {
        harness.events.push(Event::PointerMoved(
            grabbed + (target - grabbed) * (step as f32 / 4.0),
        ));
        harness.frame();
    }
    let cued = |harness: &Harness| {
        harness.shows(crate::viewport::DRAG_BLOCKED)
            || harness.shows_containing(crate::viewport::DRAG_CONFLICT)
    };
    harness.wait_until("the cue shows", cued);
    assert!(harness.model.drag_blocked());

    harness.events.push(Event::PointerButton {
        pos: target,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.wait_until("the drag ends", |harness| !harness.model.drag_blocked());
    harness.frame();
    assert!(!cued(&harness));
}

#[test]
fn dragging_across_empty_space_selects_what_the_box_takes_in() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let near = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let far = sketch.add_line(Point2::new(15.0, 30.0), Point2::new(60.0, 30.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let selected = |harness: &Harness| {
        let mut entities =
            crate::sketch_tools::selected_entities(harness.workspace.viewport.selection(), feature);
        entities.sort();
        entities
    };

    let corner = harness.on_screen(Point2::new(5.0, 25.0));
    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    drag_in_sketch(&mut harness, corner, Point2::new(25.0, 5.0));
    assert_eq!(selected(&harness), vec![near]);

    let corner = harness.on_screen(Point2::new(25.0, 5.0));
    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    drag_in_sketch(&mut harness, corner, Point2::new(5.0, 35.0));
    let mut both = vec![near, far];
    both.sort();
    assert_eq!(selected(&harness), both);
}

#[test]
fn selected_geometry_moves_by_a_typed_offset_from_the_keyboard() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(20.0, 10.0));
    let (start, end) = line_ends(&sketch, line);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.key(Key::A, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchEntity {
                feature,
                entity: line
            })
    );
    harness.key(Key::M, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows(typed_point::MOVE_LABEL));
    harness.type_text("@5, -4");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.wait_until("the move is committed", |harness| {
        harness.sketch(feature).point(start) != Some(Point2::new(10.0, 10.0))
    });

    let moved = harness.sketch(feature);
    assert!(moved.point(start).unwrap().distance(Point2::new(15.0, 6.0)) < DRAWN);
    assert!(moved.point(end).unwrap().distance(Point2::new(25.0, 6.0)) < DRAWN);
    assert!(!harness.shows(typed_point::MOVE_LABEL));
    let label = format!("Move {}", harness.sketch(feature).entity_label(line));
    assert_eq!(harness.model.undo_label(), Some(label.as_str()));
}

#[test]
fn escape_during_a_drag_puts_the_geometry_back_and_changes_nothing() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(20.0, 20.0), 10.0);
    let feature = edit_free_sketch(&mut harness, sketch);
    let revision = harness.model.revision();

    let grabbed = harness.hover_pickable(
        Plane::XY,
        Point2::new(30.0, 20.0),
        Pickable::SketchEntity {
            feature,
            entity: circle,
        },
    );
    harness.events.push(Event::PointerButton {
        pos: grabbed,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    let outside = harness.on_screen(Point2::new(35.0, 20.0));
    for step in 1..=4 {
        let position = grabbed + (outside - grabbed) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.wait_until("the dragged circle is shown", |harness| {
        harness
            .shown(feature)
            .circle(circle)
            .is_some_and(|(_, radius)| (radius - 15.0).abs() < 0.5)
    });
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: outside,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();

    assert_eq!(harness.model.revision(), revision);
    assert_eq!(harness.shown(feature).circle(circle).unwrap().1, 10.0);
    assert_eq!(harness.sketch(feature).circle(circle).unwrap().1, 10.0);
    assert_eq!(harness.editing(), Some(feature));
}

fn bar_row(harness: &Harness) -> f32 {
    harness.position_of("File").y
}

fn in_bar(harness: &Harness, label: &str) -> bool {
    let row = bar_row(harness);
    harness
        .texts
        .iter()
        .any(|(shown, rect)| shown == label && (rect.center().y - row).abs() < 12.0)
}

fn rightmost_in_bar(harness: &Harness) -> String {
    let row = bar_row(harness);
    harness
        .texts
        .iter()
        .filter(|(_, rect)| (rect.center().y - row).abs() < 12.0)
        .max_by(|a, b| a.1.center().x.total_cmp(&b.1.center().x))
        .map(|(shown, _)| shown.clone())
        .unwrap()
}

fn click_window_button(harness: &mut Harness, glyph: &str) {
    let row = bar_row(harness);
    let position = harness
        .texts
        .iter()
        .filter(|(shown, rect)| shown == glyph && (rect.center().y - row).abs() < 12.0)
        .map(|(_, rect)| rect.center())
        .max_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap_or_else(|| panic!("'{glyph}' is not in the title bar"));
    harness.click_screen(position);
    harness.show_new_windows();
}

fn empty_bar_spot(harness: &Harness) -> Pos2 {
    Pos2::new(480.0, bar_row(harness))
}

fn asked_window(harness: &Harness, command: &ViewportCommand) -> bool {
    harness.window_commands.contains(command)
}

#[test]
fn the_title_bar_buttons_minimize_maximize_restore_and_close_the_window() {
    let mut harness = Harness::new();

    assert_eq!(rightmost_in_bar(&harness), icons::CLOSE);
    click_window_button(&mut harness, icons::MINIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Minimized(true)));
    click_window_button(&mut harness, icons::MAXIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));

    harness.window.maximized = Some(true);
    harness.frame();
    assert!(in_bar(&harness, icons::RESTORE));
    assert!(!in_bar(&harness, icons::MAXIMIZE));
    click_window_button(&mut harness, icons::RESTORE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(false)));

    click_window_button(&mut harness, icons::CLOSE);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn the_window_buttons_are_wide_targets_a_control_tall_and_close_turns_red() {
    let mut harness = Harness::new();
    harness.frame();

    for name in [
        window_frame::MINIMIZE,
        window_frame::MAXIMIZE,
        window_frame::CLOSE,
    ] {
        let rect = harness.button_rect(name);
        assert_eq!(
            rect.size(),
            egui::vec2(40.0, appearance::CONTROL_HEIGHT),
            "{name}"
        );
    }
    harness.hover_button(window_frame::CLOSE);
    let tokens = appearance::tokens_for(&harness.context.global_style().visuals);
    assert_eq!(harness.color_of(icons::CLOSE), tokens.text_on_accent);
}

#[test]
fn the_top_right_corner_of_a_maximized_window_closes_it() {
    let mut harness = Harness::new();
    harness.window.maximized = Some(true);
    harness.frame();
    let close = harness.button_rect(window_frame::CLOSE);
    let corner = Pos2::new(SCREEN.max.x - 0.5, SCREEN.min.y + 0.5);

    harness.events.push(Event::PointerMoved(corner));
    harness.frame();
    let tokens = appearance::tokens_for(&harness.context.global_style().visuals);
    let highlighted = harness.color_of(icons::CLOSE) == tokens.text_on_accent;
    harness.click_screen(corner);

    assert!(
        close.max.x < corner.x && close.min.y > corner.y,
        "{close:?}"
    );
    assert!(highlighted);
    harness.wait_until("caditor quits", |harness| harness.files.should_quit());
}

#[test]
fn dragging_the_title_bar_moves_the_window_and_double_clicking_it_maximizes() {
    let mut harness = Harness::new();
    let spot = empty_bar_spot(&harness);

    harness.click_screen(spot);
    harness.press(spot);
    harness.frame();
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));
    assert!(!asked_window(&harness, &ViewportCommand::StartDrag));

    harness.window_commands.clear();
    harness.let_animations_finish();
    drag_screen(&mut harness, spot, spot + egui::vec2(80.0, 0.0));
    assert!(asked_window(&harness, &ViewportCommand::StartDrag));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn the_window_menu_switches_to_the_system_title_bar_and_back() {
    let mut harness = Harness::new();
    let spot = empty_bar_spot(&harness);

    harness.events.push(Event::PointerMoved(spot));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: spot,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.show_new_windows();
    assert!(harness.shows(window_frame::MINIMIZE));
    harness.click(window_frame::SYSTEM_TITLE_BAR);
    assert_eq!(harness.workspace.preferences.title_bar, TitleBar::System);
    assert!(asked_window(&harness, &ViewportCommand::Decorations(true)));
    assert!(!in_bar(&harness, icons::CLOSE));
    assert!(in_bar(&harness, menu_bar::SEARCH_LABEL));

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::TitleBar(TitleBar::BuiltIn),
    )));
    assert!(asked_window(&harness, &ViewportCommand::Decorations(false)));
    assert_eq!(rightmost_in_bar(&harness), icons::CLOSE);
}

#[test]
fn full_screen_toggles_from_the_keyboard_and_its_button_leaves_it() {
    let mut harness = Harness::new();

    harness.key(Key::F11, Modifiers::NONE);
    harness.frame();
    assert!(asked_window(&harness, &ViewportCommand::Fullscreen(true)));

    harness.window.fullscreen = Some(true);
    harness.window_commands.clear();
    harness.frame();
    assert!(!in_bar(&harness, icons::MAXIMIZE));
    assert!(!in_bar(&harness, icons::MINIMIZE));
    let maximize = harness
        .workspace
        .last_offers
        .iter()
        .find(|offer| offer.command == Command::MaximizeWindow)
        .unwrap();
    assert!(maximize.availability.is_err());
    click_window_button(&mut harness, icons::LEAVE_FULL_SCREEN);
    assert!(asked_window(&harness, &ViewportCommand::Fullscreen(false)));
}

#[test]
fn the_window_border_resizes_the_window_unless_it_is_maximized() {
    let mut harness = Harness::new();
    let right = Pos2::new(SCREEN.max.x - 1.0, 500.0);
    let corner = Pos2::new(SCREEN.max.x - 1.0, SCREEN.max.y - 1.0);

    harness.click_screen(right);
    harness.click_screen(corner);
    assert!(asked_window(
        &harness,
        &ViewportCommand::BeginResize(ResizeDirection::East)
    ));
    assert!(asked_window(
        &harness,
        &ViewportCommand::BeginResize(ResizeDirection::SouthEast)
    ));

    harness.window.maximized = Some(true);
    harness.window_commands.clear();
    harness.frame();
    harness.click_screen(right);
    assert!(
        !harness
            .window_commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::BeginResize(_)))
    );
}

#[test]
fn the_window_buttons_and_title_bar_still_work_while_a_dialog_is_open() {
    let mut harness = Harness::new();

    harness.perform(Action::Preferences(PreferencesCommand::Show));
    assert!(harness.workspace.preferences_open);
    click_window_button(&mut harness, icons::MAXIMIZE);
    assert!(asked_window(&harness, &ViewportCommand::Maximized(true)));
    let spot = empty_bar_spot(&harness);
    drag_screen(&mut harness, spot, spot + egui::vec2(80.0, 0.0));
    assert!(asked_window(&harness, &ViewportCommand::StartDrag));
    assert!(harness.workspace.preferences_open);
}

#[test]
fn the_model_title_shows_the_edited_sketch_and_saves_from_its_details() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    harness.edit_width("45 mm");
    harness.frame();
    assert!(in_bar(&harness, "Untitled"));
    assert!(in_bar(&harness, "Unsaved"));
    harness.click("Untitled");
    assert!(harness.shows("Unsaved changes"));
    assert!(!harness.shows(menu_bar::COPY_PATH));
    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click(&Command::Save.title());
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    assert!(in_bar(&harness, "plate.caditor"));
    assert!(!in_bar(&harness, "Unsaved"));

    harness.click("plate.caditor");
    assert!(harness.shows("All changes saved"));
    assert!(harness.shows(menu_bar::COPY_PATH));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let feature = harness.draw_on_new_sketch();
    let name = harness.document().feature(feature).unwrap().name.clone();
    assert!(in_bar(&harness, &name));
}

fn open_preferences(harness: &mut Harness) {
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
}

fn tab_after(harness: &mut Harness, key: Key, modifiers: Modifiers) -> PreferencesTab {
    harness.key(key, modifiers);
    harness.frame();
    harness.frame();
    harness.workspace.preferences_tab
}

fn focused_tab(harness: &Harness) -> Option<usize> {
    let focused = harness.focused()?;
    (0..PreferencesTab::ALL.len()).find(|index| widgets::tab_id("preferences", *index) == focused)
}

#[test]
fn preferences_tabs_switch_by_mouse_and_keyboard_and_the_last_one_stays_open() {
    let mut harness = Harness::new();
    open_preferences(&mut harness);
    assert!(harness.shows("Centimetres"));
    assert!(!harness.shows("Scroll up to zoom out"));

    harness.click("Navigation");
    assert_eq!(
        harness.workspace.preferences_tab,
        PreferencesTab::Navigation
    );
    assert!(harness.shows("Scroll up to zoom out"));
    assert!(!harness.shows("Centimetres"));

    let ctrl = Modifiers::COMMAND;
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl),
        PreferencesTab::Graphics
    );
    assert!(harness.shows("Anti-aliasing"));
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl),
        PreferencesTab::General
    );
    assert_eq!(
        tab_after(&mut harness, Key::Tab, ctrl_shift),
        PreferencesTab::Graphics
    );
    assert_eq!(
        tab_after(&mut harness, Key::PageUp, ctrl),
        PreferencesTab::Navigation
    );
    assert_eq!(
        tab_after(&mut harness, Key::PageDown, ctrl),
        PreferencesTab::Graphics
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.workspace.preferences_open);
    open_preferences(&mut harness);
    assert!(harness.shows("Anti-aliasing"));

    for _ in 0..12 {
        if focused_tab(&harness).is_some() {
            break;
        }
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(focused_tab(&harness), Some(0));
    assert_eq!(harness.workspace.preferences_tab, PreferencesTab::Graphics);
    let none = Modifiers::NONE;
    assert_eq!(
        tab_after(&mut harness, Key::ArrowRight, none),
        PreferencesTab::Appearance
    );
    assert_eq!(focused_tab(&harness), Some(1));
    assert_eq!(
        tab_after(&mut harness, Key::ArrowRight, none),
        PreferencesTab::Navigation
    );
    assert_eq!(
        tab_after(&mut harness, Key::ArrowLeft, none),
        PreferencesTab::Appearance
    );
    assert_eq!(
        tab_after(&mut harness, Key::End, none),
        PreferencesTab::Graphics
    );
    assert_eq!(
        tab_after(&mut harness, Key::Home, none),
        PreferencesTab::General
    );
    assert_eq!(
        tab_after(&mut harness, Key::ArrowLeft, none),
        PreferencesTab::Graphics
    );
    assert_eq!(focused_tab(&harness), Some(3));
    assert!(harness.shows("Anti-aliasing"));
}

#[test]
fn every_preferences_tab_fits_the_window_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    harness.workspace.hardware.adapter = Some(test_adapter());
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    open_preferences(&mut harness);
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);

    for tab in PreferencesTab::ALL {
        harness.click(tab.label());
        assert_eq!(harness.workspace.preferences_tab, tab);
        for label in PreferencesTab::ALL
            .map(PreferencesTab::label)
            .into_iter()
            .chain(["Restore defaults", "Close"])
        {
            let rect = harness
                .texts
                .iter()
                .find(|(shown, _)| shown == label)
                .unwrap_or_else(|| panic!("{label} is not on screen in {tab:?}"))
                .1;
            assert!(
                visible.contains_rect(rect),
                "{label} at {rect:?} in {tab:?}"
            );
        }
    }
}

fn test_adapter() -> GraphicsInfo {
    GraphicsInfo {
        adapter: "Test Adapter 9000".to_owned(),
        backend: "Vulkan".to_owned(),
        driver: "test driver 1.2".to_owned(),
        msaa_offered: vec![Msaa::Off, Msaa::X2, Msaa::X4],
        msaa: Msaa::X4,
        vsync_optional: false,
        vsync: true,
    }
}

#[test]
fn graphics_options_apply_at_once_are_saved_and_what_the_adapter_lacks_says_why() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.workspace.hardware = Hardware {
        adapter: Some(test_adapter()),
        refresh_rate: Some(60.0),
    };
    harness.workspace.preferences.graphics.msaa = Msaa::X8;
    open_preferences(&mut harness);
    harness.click("Graphics");

    assert!(harness.shows("Test Adapter 9000"));
    assert!(harness.shows("Vulkan"));
    assert!(harness.shows("test driver 1.2"));
    assert!(harness.shows("60 Hz"));
    assert!(harness.shows("This graphics adapter cannot draw 8× anti-aliasing, so 4× is used."));

    harness.click("4×");
    assert_eq!(harness.workspace.preferences.graphics.msaa, Msaa::X4);
    assert!(!harness.shows("This graphics adapter cannot draw 8× anti-aliasing, so 4× is used."));
    harness.click("8×");
    assert_eq!(harness.workspace.preferences.graphics.msaa, Msaa::X4);
    harness.hover("8×");
    assert!(harness.shows("This graphics adapter cannot smooth edges with 8 samples per pixel"));
    harness.click("Wait for the display");
    assert!(harness.workspace.preferences.graphics.vsync);

    harness.click("2×");
    harness.click("Enhanced");
    harness.click("Match the display");
    harness.click("120 fps");
    let graphics = harness.workspace.preferences.graphics;
    assert_eq!(graphics.msaa, Msaa::X2);
    assert_eq!(graphics.shading, Shading::Enhanced);
    assert_eq!(graphics.frame_limit, FrameLimit::Fps120);
    assert_eq!(graphics.render().msaa, Msaa::X2);
    assert_eq!(
        graphics.frame_interval(&harness.workspace.hardware),
        Some(Duration::from_secs_f64(1.0 / 120.0))
    );
    let config = dir.path().join("config");
    harness.wait_until("the graphics settings are saved", |_| {
        let saved = caditor_file::Settings::load(&config);
        saved.number("graphics.msaa") == Some(2.0)
            && saved.text("graphics.shading") == Some("enhanced")
            && saved.text("graphics.frame_limit") == Some("120")
    });

    harness.click("Restore defaults");
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(harness.shows("Restore the defaults of the Graphics tab?"));
    harness.click("Cancel");
    assert!(!harness.shows("Restore the defaults of the Graphics tab?"));
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(harness.workspace.preferences_open);

    harness.click("Restore defaults");
    harness.click("Restore defaults");
    assert_eq!(harness.workspace.preferences.graphics, Graphics::default());
    assert_eq!(harness.workspace.preferences_tab, PreferencesTab::Graphics);
    assert!(harness.shows("This tab is back to its defaults."));
    harness.click_lowest("Undo");
    assert_eq!(harness.workspace.preferences.graphics, graphics);
    assert!(!harness.shows("This tab is back to its defaults."));
    harness.wait_until("the undone settings are saved", |_| {
        caditor_file::Settings::load(&config).text("graphics.frame_limit") == Some("120")
    });
}

#[test]
fn coarse_curves_mesh_bodies_again_with_fewer_facets_and_apply_from_the_start() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(0.0, 0.0), 20.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    let body = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let facets = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .body_result(body)
            .and_then(|result| result.solid())
            .and_then(SolidResult::mesh)
            .map(|mesh| mesh.triangles().len())
            .expect("the body is meshed")
    };
    let smooth = facets(&harness);
    assert_eq!(harness.model.mesh_quality(), MeshQuality::SMOOTH);

    open_preferences(&mut harness);
    harness.click("Graphics");
    harness.click("Coarse");
    assert_eq!(
        harness.workspace.preferences.graphics.curves,
        CurveQuality::Coarse
    );
    assert_eq!(harness.model.mesh_quality(), MeshQuality::COARSE);
    harness.settle();
    let coarse = facets(&harness);
    assert!(coarse < smooth, "{coarse} facets coarse, {smooth} smooth");

    harness.click("Smooth");
    harness.settle();
    assert_eq!(facets(&harness), smooth);

    let mut stored = caditor_file::Settings::default();
    stored.set_text("graphics.curve_quality", "coarse");
    app::apply_preferences(&mut harness.model, &Preferences::from_settings(stored));
    assert_eq!(harness.model.mesh_quality(), MeshQuality::COARSE);
}

fn logo_loaded(harness: &Harness, size: u32) -> bool {
    harness
        .context
        .tex_manager()
        .read()
        .allocated()
        .any(|(_, texture)| texture.name == logo::texture_name(size))
}

fn file_menu_after_title_bar(harness: &mut Harness, title_bar: TitleBar) -> f32 {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::TitleBar(title_bar),
    )));
    harness.frame();
    harness.position_of("File").x
}

#[test]
fn the_about_dialog_shows_the_logo_beside_the_name_and_tagline() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();

    harness.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    harness.show_new_windows();

    assert!(harness.shows("Parametric CAD"));
    assert!(harness.shows(about::VERSION));
    assert!(logo_loaded(&harness, 64));
    assert_readable(&harness, "The About dialog");
}

#[test]
fn a_dialog_focuses_its_primary_action_and_enter_runs_it() {
    let mut harness = Harness::new();

    harness.perform(Action::Preferences(PreferencesCommand::ShowAbout));
    harness.show_new_windows();
    assert!(harness.shows("About caditor"));
    assert!(harness.focused().is_some());

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(!harness.shows("About caditor"));
}

#[test]
fn enter_acts_on_the_widget_holding_focus_in_a_dialog_not_on_its_primary_action() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.show_new_windows();
    assert!(harness.shows("Preferences"));

    harness.key(Key::Tab, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.shows("Preferences"));
}

#[test]
fn the_built_in_title_bar_leads_with_the_logo_and_the_system_one_leaves_it_out() {
    let mut harness = Harness::new();

    let without_logo = file_menu_after_title_bar(&mut harness, TitleBar::System);
    let with_logo = file_menu_after_title_bar(&mut harness, TitleBar::BuiltIn);

    assert!(with_logo > without_logo + 10.0);
    assert!(logo_loaded(&harness, 32));
}

fn vertex_at(harness: &Harness, body: FeatureId, at: caditor_geometry::Point3) -> Pickable {
    let vertex = harness
        .workspace
        .viewport
        .bodies()
        .get(body)
        .expect("the body is meshed")
        .vertices
        .iter()
        .find(|vertex| vertex.position.distance(at) < 1e-9)
        .unwrap_or_else(|| panic!("no vertex at {at:?}"));
    Pickable::Vertex {
        body,
        vertex: vertex.key,
    }
}

#[test]
fn two_picked_vertices_are_measured_in_the_panel_and_the_view() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let revision = harness.model.revision();
    let corner_point = caditor_geometry::Point3::ZERO;
    let far_point = caditor_geometry::Point3::new(40.0, 40.0, 10.0);
    let corner = vertex_at(&harness, body, corner_point);
    let far = vertex_at(&harness, body, far_point);

    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.measure.open);
    assert!(harness.shows(crate::measure_panel::TITLE));

    harness.click_pickable(Plane::XY, Point2::ZERO, corner);
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![corner]
    );
    let Pickable::Vertex { vertex: key, .. } = corner else {
        panic!("a vertex was picked");
    };
    let shown = crate::bodies::shown(harness.model.evaluation(), body).unwrap();
    let id = crate::bodies::find_vertex(shown, key).expect("the name resolves");
    assert_eq!(shown.names().vertex_name(id), Some(key.name));
    assert_eq!(shown.solid.vertex(id).unwrap().point(), corner_point);
    harness.wait_until("the vertex is measured", |harness| {
        harness.shows("0.000, 0.000, 0.000 mm")
    });

    harness.select([corner, far]);
    let expected = LengthUnit::Millimetre.measured_length(corner_point.distance(far_point));
    harness.wait_until("the distance is measured", |harness| {
        harness.shows(&expected)
    });

    assert!(
        harness.count_shown(&expected) >= 2,
        "the panel and the view"
    );
    assert!(harness.shows("Between them"));
    assert!(harness.shows("Along X"));
    assert_eq!(harness.count_shown("40.000 mm"), 2);
    assert!(harness.shows("10.000 mm"));
    assert_eq!(harness.model.revision(), revision);

    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    assert!(!harness.workspace.measure.open);
    assert!(!harness.shows(&expected));
}

fn painted_faces(harness: &mut Harness, colour: caditor_render::Color) -> usize {
    harness
        .built_with_meshes(1)
        .scene
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.faces)
        .filter(|face| face.color == colour)
        .count()
}

#[test]
fn a_body_drawn_half_see_through_goes_to_the_translucent_pass() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    harness.select([]);

    harness.click_lowest("Solid");
    harness.frame();
    harness.click("50%");
    harness.settle();
    let built = harness.built();

    assert_eq!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .opacity,
        Some(50)
    );
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| (face.color.alpha - 0.5).abs() < 1e-6 && face.pick.is_some())
    );
}

#[test]
fn selected_faces_take_a_colour_of_their_own_from_the_body_card() {
    use crate::body_appearance::{CLEAR_FACE_COLOURS, face_swatch_name};

    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let red = caditor_render::Color::from_rgb8(200, 64, 52);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    assert!(harness.shows("Colour the selected face"));
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click_button(&face_swatch_name("Red"));
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Change the face colours of Extrude 1")
    );
    let appearance = &harness.document().feature(plate).unwrap().appearance;
    assert_eq!(appearance.faces.len(), 1);
    assert_eq!(appearance.colour, None);
    harness.select([]);
    harness.frame();
    assert_eq!(painted_faces(&mut harness, red), 1);
    assert!(!harness.shows("Colour the selected face"));
    harness.click(CLEAR_FACE_COLOURS);
    harness.settle();
    assert!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .faces
            .is_empty()
    );
    assert_eq!(painted_faces(&mut harness, red), 0);
}

#[test]
fn a_body_takes_a_colour_and_a_material_whose_density_gives_its_mass() {
    use crate::body_appearance::{DENSITY_CAPTION, NO_MATERIAL};

    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let steel_blue = caditor_render::Color::from_rgb8(70, 130, 180);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    let opened = harness
        .workspace
        .panels
        .painting
        .map(|painting| painting.body);
    let listed = harness.shows(DENSITY_CAPTION) && harness.shows(NO_MATERIAL);
    harness.select([]);
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click_button("Steel blue");
    harness.settle();
    let colour_label = harness.model.undo_label().map(str::to_owned);
    let painted = painted_faces(&mut harness, steel_blue);

    assert_eq!(opened, Some(plate));
    assert!(listed);
    assert_eq!(
        colour_label.as_deref(),
        Some("Change the colour of Extrude 1")
    );
    assert_eq!(painted, 6);

    harness.type_into_field(Id::new(("body-density", plate)), "-1");
    harness.settle();
    let refused_label = harness.model.undo_label().map(str::to_owned);
    let refusal_shown = harness.shows("The density must be above zero, and -1 is not");
    harness.type_into_field(Id::new(("body-density", plate)), "7.85");
    harness.type_into_field(Id::new(("body-material-name", plate)), "Steel");
    harness.settle();
    harness.click(crate::toolbar::MEASURE_LABEL);
    harness.wait_until("the mass is shown", |harness| harness.shows("125.60 g"));

    assert_eq!(refused_label, colour_label);
    assert!(refusal_shown);
    assert_eq!(
        harness.model.undo_label(),
        Some("Change the material of Extrude 1")
    );
    assert!(harness.shows("Steel"));
    let appearance = &harness.document().feature(plate).unwrap().appearance;
    assert_eq!(appearance.material.as_deref(), Some("Steel"));

    harness.type_into_field(Id::new(("body-colour", plate)), "#f80");
    harness.settle();
    let typed = harness.document().feature(plate).unwrap().appearance.colour;
    assert_eq!(typed, Some(caditor_document::Rgb::new(255, 136, 0)));

    harness.click_button("Close the colour and material of Extrude 1");
    let closed = harness.workspace.panels.painting.is_none();
    for _ in 0..4 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    let unpainted = painted_faces(&mut harness, steel_blue);

    assert!(closed);
    assert!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .is_default()
    );
    assert_eq!(unpainted, 0);
}

fn add_block(harness: &mut Harness, name: &str, corners: [Point2; 2], height: &str) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(&mut outline, corners[0], corners[1]);
    let mut transaction = harness.document().transaction(format!("Add {name}"));
    let sketch = transaction.add_feature(format!("{name} sketch"), FeatureKind::from(outline));
    let block = transaction.add_feature(
        name,
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored(height).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    block
}

#[test]
fn the_interference_panel_finds_bodies_that_overlap_or_touch_and_shows_where() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    add_peg(&mut harness);
    add_block(
        &mut harness,
        "Block",
        [Point2::new(-20.0, 0.0), Point2::new(0.0, 10.0)],
        "5 mm",
    );
    let block_side = pickable_described(&mut harness, "Block › Block end face");
    let revision = harness.model.revision();

    harness.select([]);
    harness.click(crate::toolbar::INTERFERENCE_LABEL);
    harness.wait_until("every pair is checked", |harness| {
        harness.shows("1 pair overlaps and 1 pair touches.")
    });

    assert!(harness.workspace.interference.open);
    assert!(harness.shows(crate::interference_panel::EVERYTHING));
    assert!(harness.shows("Extrude 1 and Peg"));
    assert!(harness.shows("1000.0 mm³"));
    assert!(harness.shows("35.000, 20.000, 2.500 mm"));
    assert!(harness.shows("Extrude 1 and Block"));
    assert!(harness.shows("0.000, 5.000, 2.500 mm"));
    assert!(harness.shows("Extrude 1 and Block touch"));
    assert!(!harness.shows("Peg and Block"));
    assert_eq!(harness.model.revision(), revision);

    harness.click(crate::interference_panel::SHOW_PLACE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let target = harness.workspace.viewport.viewpoint().target;
    assert!(
        target.distance(caditor_geometry::Point3::new(35.0, 20.0, 2.5)) < 1e-6,
        "{target:?}"
    );
    assert!(harness.shows("Extrude 1 and Peg overlap"));

    harness.select([block_side]);
    harness.wait_until("the block is checked against the rest", |harness| {
        harness.shows("1 pair touches.")
    });
    assert!(harness.shows_containing("Block against every other body shown"));
    assert!(!harness.shows("Extrude 1 and Peg"));

    harness.select([top, block_side]);
    harness.wait_until("the two chosen bodies are checked", |harness| {
        harness.shows(crate::interference_panel::CHOSEN)
    });
    assert!(harness.shows("1 pair touches."));

    harness.click_button(crate::interference_panel::CLOSE);
    assert!(!harness.workspace.interference.open);
    assert!(!harness.shows("Extrude 1 and Peg overlap"));
}

#[test]
fn planes_axes_and_sketch_curves_are_measured() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
    let circle_sketch = harness.add_sketch(sketch);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();

    harness.select([Pickable::Plane(PrincipalPlane::Xy), top]);
    harness.wait_until("the gap between the planes is measured", |harness| {
        harness.shows("Gap between the planes")
    });
    let normal_shown = harness.shows("(0, 0, 1)");
    let gap_shown = harness.count_shown("10.000 mm") >= 1;
    harness.select([Pickable::Axis(crate::selection::Axis::Z), top]);
    harness.wait_until("the axis is measured against the face", |harness| {
        harness.shows("Angle to the plane")
    });
    harness.select([Pickable::SketchEntity {
        feature: circle_sketch,
        entity: circle,
    }]);
    harness.wait_until("the circle is measured", |harness| {
        harness.shows("Radius") && harness.shows("Diameter")
    });
    let radius_shown = harness.shows("5.000 mm") && harness.shows("10.000 mm");

    assert!(normal_shown);
    assert!(gap_shown);
    assert!(radius_shown);
    assert!(!harness.shows(crate::measure::UNMEASURABLE));
}

#[test]
fn the_measure_panel_shows_the_mass_properties_of_a_body_and_the_area_of_a_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let revision = harness.model.revision();

    harness.select([]);
    harness.click(crate::toolbar::MEASURE_LABEL);
    assert!(harness.workspace.measure.open);
    harness.wait_until("the mass properties are shown", |harness| {
        harness.shows("16000.0 mm³")
    });

    assert!(harness.shows(crate::measure_panel::MASS_TITLE));
    assert!(harness.shows("4800.00 mm²"));
    assert!(harness.shows("20.000, 20.000, 5.000 mm"));
    assert!(harness.shows("Extrude 1"));

    harness.select([top]);
    harness.wait_until("the face is measured", |harness| {
        harness.shows("1600.00 mm²")
    });
    assert!(harness.shows("Extrude 1 › Extrude 1 end face"));
    assert!(harness.shows("16000.0 mm³"));
    assert_eq!(harness.model.revision(), revision);

    harness.click_button(crate::measure_panel::CLOSE);
    assert!(!harness.workspace.measure.open);
}

fn crossed_line() -> (Sketch, EntityId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(60.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let left = sketch.add_line(Point2::new(20.0, -20.0), Point2::new(20.0, 20.0));
    let right = sketch.add_line(Point2::new(40.0, -20.0), Point2::new(40.0, 20.0));
    (sketch, line, left, right)
}

#[test]
fn trim_cuts_away_the_hovered_piece_between_its_crossings_in_one_undoable_step() {
    let mut harness = Harness::new();
    let (sketch, line, left, right) = crossed_line();
    let [line_label, left_label, right_label] =
        [line, left, right].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::K);
    assert_eq!(harness.tool(), Some(Tool::Trim));
    assert!(harness.shows(trimming::TRIM_PROMPT));
    harness.point_at(Point2::new(30.0, 0.0));
    assert!(harness.shows(&format!(
        "Trim {line_label} back to {left_label} and {right_label}"
    )));

    harness.click_at(Point2::new(30.0, 0.0));
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 4);
    let (start, end) = sketch.line_endpoints(line).unwrap();
    assert!(near(start, Point2::ZERO) && near(end, Point2::new(20.0, 0.0)));
    let piece = lines
        .into_iter()
        .find(|id| ![line, left, right].contains(id))
        .unwrap();
    let (piece_start, piece_end) = sketch.line_endpoints(piece).unwrap();
    assert!(near(piece_start, Point2::new(40.0, 0.0)));
    assert!(near(piece_end, Point2::new(60.0, 0.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Horizontal"),
        vec![Constraint::Horizontal(line), Constraint::Horizontal(piece)]
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Trim {line_label}").as_str())
    );
    assert_eq!(harness.tool(), Some(Tool::Trim));

    harness.settle();
    assert!(
        harness
            .model
            .evaluation()
            .feature(feature)
            .is_some_and(|status| { status.state == caditor_document::FeatureState::UpToDate })
    );
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn dragging_across_pieces_trims_each_one_crossed_in_one_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let post = sketch.add_line(Point2::new(30.0, -30.0), Point2::new(30.0, 30.0));
    let upper = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(60.0, 10.0));
    let lower = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(60.0, -10.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.click_button("Trim");
    assert_eq!(harness.tool(), Some(Tool::Trim));

    let from = harness.on_screen(Point2::new(15.0, 20.0));
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    drag_in_sketch(&mut harness, from, Point2::new(15.0, -20.0));

    let sketch = harness.sketch(feature);
    for line in [upper, lower] {
        let (start, end) = sketch.line_endpoints(line).unwrap();
        assert!(near(start, Point2::new(30.0, start.y)), "{start}");
        assert!(near(end, Point2::new(60.0, start.y)), "{end}");
    }
    let (bottom, top) = sketch.line_endpoints(post).unwrap();
    assert!(near(bottom, Point2::new(30.0, -30.0)) && near(top, Point2::new(30.0, 30.0)));
    assert_eq!(harness.model.undo_label(), Some("Trim 2 pieces"));
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn extend_reaches_the_curve_in_the_way_and_says_when_there_is_none() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let circle = sketch.add_circle(Point2::new(40.0, 0.0), 10.0);
    let [line_label, circle_label] = [line, circle].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::J);
    assert_eq!(harness.tool(), Some(Tool::Extend));
    assert!(harness.shows(trimming::EXTEND_PROMPT));
    harness.point_at(Point2::new(9.0, 0.0));
    assert!(harness.shows(&format!("Extend {line_label} to {circle_label}")));
    harness.click_at(Point2::new(9.0, 0.0));

    let sketch = harness.sketch(feature);
    let (_, end) = line_ends(sketch, line);
    assert!(near(sketch.point(end).unwrap(), Point2::new(30.0, 0.0)));
    assert_eq!(
        constraints_of_kind(sketch, "Coincident"),
        vec![Constraint::Coincident(end, circle)]
    );
    assert_eq!(
        harness.model.undo_label(),
        Some(format!("Extend {line_label}").as_str())
    );

    harness.settle();
    let before = harness.sketch(feature).clone();
    harness.click_at(Point2::new(1.0, 0.0));
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!("Extend: nothing lies beyond this end of {line_label} to extend it to.")
    );
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn trim_and_extend_act_on_the_keyboard_highlight() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    let cutter = sketch.add_line(Point2::new(10.0, -10.0), Point2::new(10.0, 10.0));
    let circle = sketch.add_circle(Point2::new(60.0, 0.0), 5.0);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 40.0),
        Point2::new(10.0, 50.0),
        Point2::new(20.0, 40.0),
    ]);
    let [line_label, cutter_label, circle_label, spline_label] =
        [line, cutter, circle, spline].map(|id| sketch.entity_label(id));
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "trim sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Trim));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Trim {line_label} back to {cutter_label}")));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    let (start, end) = harness.sketch(feature).line_endpoints(line).unwrap();
    assert!(near(start, Point2::new(10.0, 0.0)) && near(end, Point2::new(30.0, 0.0)));
    harness.settle();

    harness.click_at(Point2::new(10.0, 45.0));
    assert_eq!(
        harness.model.notice().unwrap().text,
        format!("Trim: {spline_label} cannot be trimmed; only lines, circles and arcs can.")
    );

    run_from_palette(&mut harness, "extend a line");
    assert_eq!(harness.tool(), Some(Tool::Extend));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Extend {line_label} to {circle_label}")));
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    let (_, end) = line_ends(harness.sketch(feature), line);
    assert!(near(
        harness.sketch(feature).point(end).unwrap(),
        Point2::new(55.0, 0.0)
    ));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

fn is_suppressed(harness: &Harness, name: &str) -> bool {
    harness
        .document()
        .feature(feature_named(harness, name))
        .is_some_and(|feature| feature.suppressed)
}

fn click_with(harness: &mut Harness, label: &str, modifiers: Modifiers) {
    let position = harness.position_of(label);
    harness.events.push(Event::ModifiersChanged(modifiers));
    harness.events.push(Event::PointerMoved(position));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
        harness.frame();
    }
    harness
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    harness.frame();
}

fn hold_drag(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=6 {
        let position = from + (to - from) * (step as f32 / 6.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.frame();
}

fn release_drag(harness: &mut Harness, at: Pos2) {
    harness.events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.settle();
}

fn upper_edge_of(harness: &Harness, label: &str) -> Pos2 {
    let (_, rect) = harness
        .texts
        .iter()
        .find(|(shown, _)| shown == label)
        .unwrap_or_else(|| panic!("'{label}' is not on screen"));
    Pos2::new(rect.center().x, rect.top() + 1.0)
}

#[test]
fn suppressing_a_sketch_fails_its_extrusion_until_the_callout_unsuppresses_it() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let volume = harness.body_volume(extrude);

    harness.click_beside(icons::MORE, "Plate");
    harness.click("Suppress");
    harness.settle();
    let suppressed_label = harness.model.undo_label().map(str::to_owned);
    let failed = harness.shows("It uses Plate, which is suppressed.");
    let body_stale = harness.model.evaluation().is_stale(extrude);
    let muted = harness.color_of("Plate");
    harness.click_beside(icons::MORE, "Extrude 1");
    harness.click("Suppress");
    harness.settle();
    harness.click("Extrude 1");
    harness.key(Key::E, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let refused = harness.model.notice().map(|notice| notice.text.clone());
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(suppressed_label.as_deref(), Some("Suppress Plate"));
    assert!(failed);
    assert!(body_stale);
    assert_ne!(muted, harness.color_of("Side sketch"));
    assert_eq!(
        refused.as_deref(),
        Some("Edit feature: Extrude 1 is suppressed; unsuppress it to edit it")
    );
    assert!(is_suppressed(&harness, "Plate"));
    assert!(!is_suppressed(&harness, "Extrude 1"));

    harness.click("Unsuppress Plate");
    harness.settle();

    assert!(!is_suppressed(&harness, "Plate"));
    assert_eq!(harness.model.undo_label(), Some("Unsuppress Plate"));
    assert!(harness.shows("Up to date"));
    assert!((harness.body_volume(extrude) - volume).abs() < 1e-6);
}

#[test]
fn features_chosen_together_in_the_tree_are_suppressed_in_one_change_from_the_palette() {
    let mut harness = Harness::new();

    harness.click("Base sketch");
    click_with(&mut harness, "Side sketch", Modifiers::COMMAND);
    run_from_palette(&mut harness, "suppress or unsuppress");
    harness.settle();
    let both = is_suppressed(&harness, "Base sketch") && is_suppressed(&harness, "Side sketch");
    let label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();

    assert!(both);
    assert_eq!(label.as_deref(), Some("Suppress 2 features"));
    assert!(!is_suppressed(&harness, "Base sketch"));
    assert!(!is_suppressed(&harness, "Side sketch"));
}

#[test]
fn the_rollback_bar_moves_from_the_menu_and_keyboard_and_new_features_go_in_above_it() {
    let mut harness = Harness::new();

    harness.click_beside(icons::MORE, "Base sketch");
    harness.click("Roll back to here");
    harness.settle();
    let rolled_label = harness.model.undo_label().map(str::to_owned);
    let side_rolled_back = harness
        .document()
        .is_rolled_back(feature_named(&harness, "Side sketch"));
    let caption = harness.shows("1 feature rolled back");
    let sketch = harness.draw_on_new_sketch();
    let order = feature_names(&harness);
    harness.key(Key::ArrowDown, Modifiers::ALT);
    harness.settle();
    let at_end = harness.document().rollback_bar();
    harness.key(Key::ArrowUp, Modifiers::ALT);
    harness.settle();
    harness.key(Key::ArrowUp, Modifiers::ALT);
    harness.settle();
    let editing_after = harness.editing();
    let rolled_past_sketch = harness.document().is_rolled_back(sketch);
    run_from_palette(&mut harness, "roll to end");
    harness.settle();

    assert_eq!(rolled_label.as_deref(), Some("Roll back to Base sketch"));
    assert!(side_rolled_back);
    assert!(caption);
    assert_eq!(order, ["Base sketch", "Sketch 1", "Side sketch"]);
    assert_eq!(at_end, RollbackBar::AtEnd);
    assert!(rolled_past_sketch);
    assert_eq!(editing_after, None);
    assert_eq!(harness.document().rollback_bar(), RollbackBar::AtEnd);
    assert_eq!(harness.model.undo_label(), Some("Roll to end"));
    assert!(harness.unreadable_nodes().is_empty());
}

#[test]
fn dragging_a_feature_reorders_the_tree_and_a_refused_place_says_why_while_dragging() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    let from = harness.position_of("Extrude 1");
    let onto = upper_edge_of(&harness, "Plate");
    hold_drag(&mut harness, from, onto);
    let refusal = harness.shows("Extrude 1 cannot move above Plate, which it uses");
    release_drag(&mut harness, onto);
    let unchanged = feature_names(&harness);

    let from = harness.position_of("Side sketch");
    let onto = upper_edge_of(&harness, "Base sketch");
    hold_drag(&mut harness, from, onto);
    let allowed = !harness.shows("Side sketch cannot move above Base sketch, which it uses");
    release_drag(&mut harness, onto);

    assert!(refusal);
    assert_eq!(
        unchanged,
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
    assert!(allowed);
    assert_eq!(
        feature_names(&harness),
        ["Side sketch", "Base sketch", "Plate", "Extrude 1"]
    );
    assert_eq!(harness.model.undo_label(), Some("Move Side sketch"));
    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
}

#[test]
fn dragging_one_of_several_chosen_features_moves_them_all_together() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);

    harness.click("Base sketch");
    click_with(&mut harness, "Plate", Modifiers::COMMAND);
    let from = harness.position_of("Plate");
    let onto = upper_edge_of(&harness, "Base sketch");
    hold_drag(&mut harness, from, onto);
    let refused = harness.shows_containing("cannot move");
    release_drag(&mut harness, onto);

    assert!(!refused);
    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Plate", "Side sketch", "Extrude 1"]
    );
    assert_eq!(harness.model.undo_label(), Some("Move 2 features"));

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(
        feature_names(&harness),
        ["Base sketch", "Side sketch", "Plate", "Extrude 1"]
    );
}

#[test]
fn dragging_the_rollback_bar_shows_the_model_as_of_where_it_is_dropped() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    harness.let_animations_finish();

    let bar = harness
        .button_rect(crate::feature_tree::ROLLBACK_BAR_NAME)
        .center();
    let onto = upper_edge_of(&harness, "Plate");
    hold_drag(&mut harness, bar, onto);
    release_drag(&mut harness, onto);

    assert_eq!(
        harness.document().rollback_bar(),
        RollbackBar::Before(feature_named(&harness, "Plate"))
    );
    assert!(harness.model.evaluation().body(extrude).is_none());
    assert!(harness.shows("2 features rolled back"));
    assert_eq!(harness.model.undo_label(), Some("Move the rollback bar"));
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.model.evaluation().body(extrude).is_some());
}

#[test]
fn deleting_a_feature_others_use_asks_whether_to_take_or_keep_them() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let everything = feature_names(&harness);

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    let asked = harness.shows("Delete “Plate”?") && harness.shows("1 feature depends on “Plate”:");
    let listed = harness.shows("uses Plate");
    let with_dependents = harness.position_of(crate::feature_tree::DELETE_WITH_DEPENDENTS);
    let keep = harness.position_of(crate::feature_tree::KEEP_DEPENDENTS);
    let cancel = harness.position_of("Cancel");
    harness.click("Cancel");
    let cancelled = feature_names(&harness) == everything && !harness.shows("Delete “Plate”?");

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    harness.click(crate::feature_tree::KEEP_DEPENDENTS);
    harness.settle();
    let kept = feature_names(&harness);
    let failing = harness.shows("It uses a feature that no longer exists.");
    harness.perform(Action::Undo);
    harness.settle();

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    harness.click(crate::feature_tree::DELETE_WITH_DEPENDENTS);
    harness.settle();
    let taken = feature_names(&harness);
    let label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();

    assert!(asked);
    assert!(listed);
    assert!(with_dependents.x < keep.x && keep.x < cancel.x);
    assert!(cancelled);
    assert_eq!(kept, ["Base sketch", "Side sketch", "Extrude 1"]);
    assert!(failing);
    assert_eq!(taken, ["Base sketch", "Side sketch"]);
    assert_eq!(label.as_deref(), Some("Delete Plate and its dependents"));
    assert_eq!(feature_names(&harness), everything);
    assert!(harness.shows("Up to date"));
}

#[test]
fn a_click_on_a_feature_name_selects_it_without_opening_or_expanding_it() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    let side = feature_named(&harness, "Side sketch");

    harness.click("Side sketch");
    let chosen = harness.workspace.panels.chosen();
    let expanded = harness.shows(crate::feature_tree::DIMENSIONS_TITLE);
    click_with(&mut harness, "Base sketch", Modifiers::COMMAND);

    assert_eq!(chosen, vec![side]);
    assert!(!expanded);
    assert_eq!(harness.editing(), None);
    assert_eq!(harness.workspace.panels.chosen(), vec![side, base]);
    assert!(!harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn the_chevron_alone_shows_and_hides_a_feature_s_details() {
    let mut harness = Harness::new();

    harness.click_button("Show details of Side sketch");
    harness.let_animations_finish();
    let shown = harness.shows(crate::feature_tree::DIMENSIONS_TITLE);
    let chosen = harness.workspace.panels.chosen();
    harness.click_button("Hide details of Side sketch");
    harness.let_animations_finish();

    assert!(shown);
    assert!(chosen.is_empty());
    assert_eq!(harness.editing(), None);
    assert!(!harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn a_double_click_or_enter_on_a_feature_row_opens_it() {
    let mut harness = Harness::new();
    let base = feature_named(&harness, "Base sketch");
    let side = feature_named(&harness, "Side sketch");

    harness.double_click("Side sketch");
    harness.settle();
    let double_clicked = harness.editing();
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    harness
        .context
        .memory_mut(|memory| memory.request_focus(Id::new(("feature-row", base))));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();

    assert_eq!(double_clicked, Some(side));
    assert_eq!(harness.workspace.panels.chosen(), vec![base]);
    assert_eq!(harness.editing(), Some(base));
    assert!(harness.shows(crate::feature_tree::DIMENSIONS_TITLE));
}

#[test]
fn the_delete_dialog_starts_on_cancel_so_enter_deletes_nothing() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let everything = feature_names(&harness);

    harness.click("Plate");
    harness.key(Key::Delete, Modifiers::NONE);
    harness.show_new_windows();
    let asked = harness.shows("Delete “Plate”?");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();

    assert!(asked);
    assert!(!harness.shows("Delete “Plate”?"));
    assert_eq!(feature_names(&harness), everything);
    assert!(harness.workspace.panels.deleting.is_none());
}

#[test]
fn an_empty_tree_offers_a_new_sketch_and_the_samples() {
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    assert!(harness.shows(crate::feature_tree::EMPTY_TREE));
    harness.button_rect(crate::feature_tree::OPEN_SAMPLE_LABEL);
    let new_sketch = harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some("New sketch"))
        .filter_map(|(_, node)| node.bounds())
        .max_by(|a, b| a.y0.total_cmp(&b.y0))
        .map(|bounds| {
            Pos2::new(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            )
        })
        .expect("the empty tree has a New sketch button");

    harness.click_screen(new_sketch);
    harness.show_new_windows();
    let choosing = harness.workspace.editing.is_choosing_plane();
    harness.perform(Action::Editing(EditingCommand::CancelNewSketch));
    harness.click(crate::feature_tree::OPEN_SAMPLE_LABEL);

    assert!(choosing);
    assert!(harness.workspace.welcome_open);
}

#[test]
fn a_principal_plane_chosen_in_the_tree_is_selected_in_the_view() {
    let mut harness = Harness::new();
    harness.click("Side sketch");

    harness.click_button("Show details of Principal planes, axes and origin");
    harness.let_animations_finish();
    harness.click("XY plane");

    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::Plane(PrincipalPlane::Xy))
    );
    assert!(harness.workspace.panels.chosen().is_empty());
}

#[test]
fn the_measure_panel_keeps_the_last_readout_until_the_next_one_arrives() {
    let mut harness = Harness::new();
    let (body, _) = extruded_plate(&mut harness);
    let corner = vertex_at(&harness, body, caditor_geometry::Point3::ZERO);
    let far = vertex_at(
        &harness,
        body,
        caditor_geometry::Point3::new(40.0, 40.0, 10.0),
    );
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.select([far]);
    harness.wait_until("the far corner is measured", |harness| {
        harness.shows("40.000, 40.000, 10.000 mm")
    });

    let mut blank_frames = 0;
    harness.select([corner]);
    let deadline = Instant::now() + FILE_TIMEOUT;
    while !harness.shows("0.000, 0.000, 0.000 mm") {
        assert!(Instant::now() < deadline, "the corner is never measured");
        if !harness.shows("40.000, 40.000, 10.000 mm") {
            blank_frames += 1;
        }
        harness.frame();
    }

    assert_eq!(blank_frames, 0);
    assert!(!harness.shows(crate::measure_panel::MEASURING));
}

fn radii_of_circles(sketch: &Sketch) -> Vec<f64> {
    let mut radii: Vec<f64> = entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(_, radius)| radius)
        .collect();
    radii.sort_by(f64::total_cmp);
    radii
}

#[test]
fn offset_takes_the_clicked_chain_and_runs_where_the_pointer_is_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let bottom = entities_of_kind(&sketch, "Line")[0];
    let bottom_label = sketch.entity_label(bottom);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::W);
    assert_eq!(harness.tool(), Some(Tool::Offset));
    assert!(harness.shows(offsetting::CHOOSE_PROMPT));
    harness.point_at(Point2::new(20.0, 0.0));
    assert!(harness.shows(&format!("Take the chain of {bottom_label} to offset")));
    harness.click_at(Point2::new(20.0, 0.0));
    assert_eq!(
        sketch_tools::selected_entities(harness.workspace.viewport.selection(), feature).len(),
        4
    );
    assert!(harness.shows(offsetting::PROMPT));

    harness.point_at(Point2::new(20.0, -5.0));
    assert!(harness.shows("Offset 4 curves by 5 mm"));
    harness.click_at(Point2::new(20.0, -5.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 8);
    assert_eq!(constraints_of_kind(sketch, "Distance").len(), 4);
    let corners: Vec<Point2> = entities_of_kind(sketch, "Line")
        .into_iter()
        .filter_map(|line| sketch.line_endpoints(line))
        .map(|(start, _)| start)
        .collect();
    for expected in [
        Point2::new(-5.0, -5.0),
        Point2::new(45.0, -5.0),
        Point2::new(45.0, 25.0),
        Point2::new(-5.0, 25.0),
    ] {
        assert!(
            corners.iter().any(|corner| near(*corner, expected)),
            "{expected} in {corners:?}"
        );
    }
    assert_eq!(harness.model.undo_label(), Some(offsetting::TRANSACTION));
    assert_eq!(harness.tool(), Some(Tool::Offset));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn offset_by_a_typed_distance_needs_no_pointer_and_a_negative_one_goes_the_other_way() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let circle_label = sketch.entity_label(circle);
    let spline = sketch.add_spline(&[Point2::new(30.0, 0.0), Point2::new(40.0, 10.0)]);
    let spline_label = sketch.entity_label(spline);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, "offset sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Offset));
    type_point(&mut harness, "3");
    assert!(close(
        &radii_of_circles(harness.sketch(feature)),
        &[10.0, 13.0]
    ));
    assert_eq!(harness.model.undo_label(), Some(offsetting::TRANSACTION));

    type_point(&mut harness, "-2");
    assert!(close(
        &radii_of_circles(harness.sketch(feature)),
        &[8.0, 10.0, 13.0]
    ));

    type_point(&mut harness, "-12");
    assert!(harness.shows(&format!(
        "Offset: offsetting {circle_label} this far would shrink it to nothing."
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(radii_of_circles(harness.sketch(feature)).len(), 3);

    harness.select(entity_pickables(feature, &[spline]));
    harness.frame();
    assert!(harness.shows(&format!(
        "{spline_label} cannot be offset; only lines, arcs and circles can"
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

fn mirror_fixture() -> (Sketch, EntityId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let mirror = sketch.add_line(Point2::new(0.0, -30.0), Point2::new(0.0, 30.0));
    sketch.set_construction(mirror, true).unwrap();
    let line = sketch.add_line(Point2::new(5.0, 0.0), Point2::new(15.0, 10.0));
    let circle = sketch.add_circle(Point2::new(10.0, -10.0), 3.0);
    (sketch, mirror, line, circle)
}

#[test]
fn mirror_copies_the_selection_about_the_clicked_line_and_keeps_it_symmetric() {
    let mut harness = Harness::new();
    let (sketch, mirror, line, circle) = mirror_fixture();
    let mirror_label = sketch.entity_label(mirror);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::Y);
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    assert!(harness.shows(mirroring::SELECT_FIRST));
    harness.click_at(Point2::new(0.0, 20.0));
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Mirror: select the geometry to mirror."
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));

    harness.select(entity_pickables(feature, &[line, circle]));
    harness.use_tool(Key::Y);
    assert!(harness.shows(mirroring::PROMPT));
    harness.point_at(Point2::new(0.0, 20.0));
    assert!(harness.shows(&format!("Mirror 2 items about {mirror_label}")));
    harness.click_at(Point2::new(0.0, 20.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 3);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 2);
    let copy = entities_of_kind(sketch, "Line")
        .into_iter()
        .find(|id| ![mirror, line].contains(id))
        .unwrap();
    let (start, end) = sketch.line_endpoints(copy).unwrap();
    assert!(near(start, Point2::new(-5.0, 0.0)) && near(end, Point2::new(-15.0, 10.0)));
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    assert_eq!(harness.model.undo_label(), Some(mirroring::TRANSACTION));

    harness.settle();
    assert!(
        harness
            .model
            .evaluation()
            .feature(feature)
            .is_some_and(|status| status.state == caditor_document::FeatureState::UpToDate)
    );
}

#[test]
fn mirror_steps_through_lines_and_axes_from_the_keyboard() {
    let mut harness = Harness::new();
    let (sketch, _, line, circle) = mirror_fixture();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[line, circle]));
    run_from_palette(&mut harness, "mirror sketch geometry");
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Mirror 2 items about Horizontal axis"));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Mirror 2 items about Vertical axis"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let centres: Vec<Point2> = entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(centre, _)| centre)
        .collect();
    assert!(
        centres
            .iter()
            .any(|centre| near(*centre, Point2::new(-10.0, -10.0)))
    );
    assert!(
        constraints_of_kind(sketch, "Symmetric")
            .iter()
            .all(|constraint| matches!(
                constraint,
                Constraint::Symmetric { about, .. } if *about == EntityId::VERTICAL_AXIS
            ))
    );

    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

fn fillet_arc(sketch: &Sketch) -> Option<caditor_sketch::ArcGeometry> {
    entities_of_kind(sketch, "Arc")
        .first()
        .and_then(|arc| sketch.arc(*arc))
}

#[test]
fn a_sketch_fillet_rounds_the_clicked_corner_with_a_typed_radius_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lines = entities_of_kind(&sketch, "Line");
    let [first, second] = [lines[0], lines[1]].map(|line| sketch.entity_label(line));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::B);
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    assert!(harness.shows(filleting::CORNER_PROMPT));
    harness.point_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(&format!("Round the corner of {first} and {second}")));
    harness.click_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));

    type_point(&mut harness, "30");
    assert!(harness.shows(&format!(
        "Sketch fillet: the radius is too large for {second}."
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(fillet_arc(harness.sketch(feature)).is_none());

    type_point(&mut harness, "5");
    let sketch = harness.sketch(feature);
    let arc = fillet_arc(sketch).unwrap();
    assert!(near(arc.center, Point2::new(35.0, 5.0)));
    assert!((arc.radius - 5.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Radius").len(), 1);
    assert_eq!(harness.model.undo_label(), Some(filleting::TRANSACTION));
    assert!(harness.shows(filleting::CORNER_PROMPT));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_sketch_fillet_takes_its_radius_from_the_pointer_and_escape_backs_out_a_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.click_button("Sketch fillet");
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    harness.click_at(Point2::new(0.0, 20.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(filleting::CORNER_PROMPT));
    assert_eq!(harness.tool(), Some(Tool::Fillet));

    harness.click_at(Point2::new(40.0, 0.0));
    harness.point_at(Point2::new(38.0, 2.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    harness.click_at(Point2::new(38.0, 2.0));
    let arc = fillet_arc(harness.sketch(feature)).unwrap();
    let middle = arc.point_at(arc.start_angle + arc.sweep / 2.0);
    assert!(middle.distance(Point2::new(38.0, 2.0)) < 0.05, "{middle}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_sketch_fillet_is_chosen_typed_and_placed_from_the_keyboard() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lines = entities_of_kind(&sketch, "Line");
    let [first, second] = [lines[0], lines[1]].map(|line| sketch.entity_label(line));
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "fillet a sketch corner");
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Round the corner of {first} and {second}")));
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    type_point(&mut harness, "4");

    let arc = fillet_arc(harness.sketch(feature)).unwrap();
    assert!((arc.radius - 4.0).abs() < DRAWN);
    assert_eq!(harness.model.undo_label(), Some(filleting::TRANSACTION));
}

#[test]
fn the_modify_tools_name_their_keys_and_what_they_do() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    for (name, keys) in [("Offset", "W"), ("Mirror", "Y"), ("Sketch fillet", "B")] {
        harness.hover_button(name);
        assert!(
            harness
                .texts
                .iter()
                .any(|(text, _)| text.ends_with(&format!("({keys})"))),
            "{name}"
        );
    }
}

fn view_cube_node(harness: &Harness) -> Option<(NodeId, String)> {
    harness
        .accessible
        .iter()
        .find(|(_, node)| {
            node.role() == Role::Button
                && node
                    .label()
                    .is_some_and(|label| label.starts_with(view_cube::NAME))
        })
        .and_then(|(id, node)| Some((*id, node.label()?.to_owned())))
}

#[test]
fn the_view_cube_names_its_view_and_steps_to_a_neighbour_from_the_keyboard() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();

    let (cube, name) = view_cube_node(&harness).expect("the view cube is named");
    assert_eq!(name, "View cube: View from top, front and right");
    assert!(harness.accessible_named(Role::Button, "Fit all"));
    assert!(harness.shows("Fit all"));

    harness.events.push(Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_node: cube,
            target_tree: egui::accesskit::TreeId::ROOT,
            data: None,
        },
    ));
    harness.frame();
    harness.frame();
    harness.key(Key::ArrowRight, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();

    let looking_from = harness.workspace.viewport.viewpoint().orientation * Vector3::Z;
    let expected = Vector3::new(1.0, 0.0, 1.0).normalize();
    assert!(looking_from.distance(expected) < 1e-6, "{looking_from}");
    let (_, name) = view_cube_node(&harness).expect("the view cube is named");
    assert_eq!(name, "View cube: View from top and right");
}

fn bracket_profile(harness: &mut Harness) -> FeatureId {
    let session = harness.model.session();
    harness.command(FileCommand::OpenSample(crate::samples::Sample::Bracket));
    if harness.shows("Continue Without Saving") {
        harness.click("Continue Without Saving");
    }
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();
    let profile = harness
        .document()
        .features()
        .find(|feature| feature.name == "Bracket profile")
        .map(|feature| feature.id())
        .expect("the bracket has its profile sketch");
    harness.edit(profile);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
    profile
}

#[test]
fn constraint_glyphs_keep_clear_of_dimension_labels_in_the_bracket_profile() {
    let mut harness = Harness::new();
    bracket_profile(&mut harness);
    let view = harness.workspace.viewport.rect().unwrap();
    let in_view = |rect: &Rect| view.contains_rect(*rect);

    let labels: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(text, rect)| text.contains(" = ") && in_view(rect))
        .map(|(_, rect)| rect.expand2(canvas::PADDING))
        .collect();
    let glyphs: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(text, rect)| (text == "H" || text == "V") && in_view(rect))
        .map(|(_, rect)| Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)))
        .collect();

    assert_eq!(labels.len(), 4);
    assert_eq!(glyphs.len(), 6);
    for glyph in &glyphs {
        for label in &labels {
            assert!(!glyph.intersects(*label), "{glyph:?} overlaps {label:?}");
        }
    }
}

#[test]
fn the_prompt_and_key_hints_wrap_clear_of_the_view_cube_in_a_small_view() {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.5),
    )));
    harness.frame();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    harness.frame();

    let view = harness.workspace.viewport.rect().unwrap();
    let cube = view_cube::area(view);
    let prompt = "Click the rectangle's first corner";
    let keys = with_type_hint(
        "Rectangle from two corners   R: from its centre",
        "Esc: back to Select",
    );
    assert!(harness.shows(prompt));
    assert!(harness.shows_hint(&keys));
    let hinted: Vec<&str> = std::iter::once(prompt)
        .chain(canvas::hint_texts(&keys))
        .collect();
    let rows: Vec<f32> = harness
        .texts
        .iter()
        .filter(|(text, _)| hinted.contains(&text.as_str()))
        .map(|(text, rect)| {
            assert!(!rect.intersects(cube), "'{text}' runs under the view cube");
            assert!(
                view.contains_rect(*rect),
                "'{text}' leaves the view {view:?} at {rect:?}"
            );
            rect.top()
        })
        .collect();
    let first = rows.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(rows.iter().any(|top| *top > first + 30.0), "{rows:?}");
}

fn chosen_plate() -> (Document, FeatureId, FeatureId) {
    let mut outline = Sketch::new(Plane::XY);
    let corners = [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)];
    for index in 0..4 {
        let (from, to) = (corners[index], corners[(index + 1) % 4]);
        outline.add_line(Point2::new(from.0, from.1), Point2::new(to.0, to.1));
    }
    let regions = caditor_document::sketch_regions(&outline)
        .unwrap()
        .iter()
        .map(|region| caditor_kernel::RegionReference::capture(region, region.anchor()))
        .collect();
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let sketch = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::Chosen(regions),
            extent: ExtrudeExtent::one_side(Expression::Measure(5.0, Unit::Millimetre), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, base)
}

#[test]
fn a_region_matched_again_after_a_hole_is_drawn_warns_until_its_references_are_updated() {
    let (document, sketch, base) = chosen_plate();
    let mut harness = Harness::starting(None, document, Workspace::new());
    harness.context.enable_accesskit();
    let mut transaction = harness.document().transaction("Draw a hole");
    let center = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(20.0, 15.0)));
    transaction.add_sketch_entity(
        sketch,
        Entity::Circle {
            center,
            radius: 5.0,
        },
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let reason = "After an upstream change, a chosen region of Outline was matched to the most \
                  similar geometry.";

    let warned = harness.shows(reason);
    let hinted = harness.accessible.iter().any(|(_, node)| {
        node.value()
            == Some("Some of what it uses changed and was matched to the most similar geometry")
    });
    harness.click("Update references");
    harness.settle();

    assert!(warned);
    assert!(hinted);
    assert_eq!(
        harness.model.undo_label(),
        Some("Update references of Base")
    );
    assert!(!harness.shows(reason));
    assert!((harness.body_volume(base) - (6000.0 - 125.0 * PI)).abs() < 1.0);

    harness.perform(Action::Undo);
    harness.settle();
    harness.click("Base");
    run_from_palette(&mut harness, "update references");
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Update references of Base")
    );
    assert!(!harness.shows(reason));
}

fn viewport_centre(harness: &Harness) -> Pos2 {
    harness.workspace.viewport.rect().unwrap().center()
}

fn laptop_harness() -> Harness {
    let mut harness = Harness::new();
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::InputMode(InputMode::Laptop),
    )));
    harness.frame();
    harness
        .events
        .push(Event::PointerMoved(viewport_centre(&harness)));
    harness.frame();
    harness
}

fn scroll_in_view(harness: &mut Harness, delta: egui::Vec2) {
    harness.events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers: harness.held,
    });
    for _ in 0..8 {
        harness.frame();
    }
}

#[test]
fn the_wheel_zooms_in_caditor_mode_but_two_fingers_orbit_in_laptop_mode() {
    let mut harness = Harness::new();
    harness
        .events
        .push(Event::PointerMoved(viewport_centre(&harness)));
    harness.frame();
    let before = harness.workspace.viewport.viewpoint();

    scroll_in_view(&mut harness, egui::vec2(0.0, 60.0));

    let zoomed = harness.workspace.viewport.viewpoint();
    assert_ne!(zoomed.distance, before.distance);
    assert!(zoomed.forward().dot(before.forward()) > 0.99999);

    let mut harness = laptop_harness();
    let before = harness.workspace.viewport.viewpoint();

    scroll_in_view(&mut harness, egui::vec2(40.0, 0.0));

    let orbited = harness.workspace.viewport.viewpoint();
    assert!((orbited.distance - before.distance).abs() < 1e-6 * before.distance);
    assert!(orbited.forward().dot(before.forward()) < 0.9999);
}

#[test]
fn alt_and_two_fingers_pan_and_a_pinch_zooms_in_laptop_mode() {
    let mut harness = laptop_harness();
    let before = harness.workspace.viewport.viewpoint();

    harness.hold(Modifiers::ALT);
    scroll_in_view(&mut harness, egui::vec2(30.0, 20.0));
    harness.hold(Modifiers::NONE);

    let panned = harness.workspace.viewport.viewpoint();
    assert!(panned.target.distance(before.target) > 1e-6);
    assert!(panned.forward().dot(before.forward()) > 0.99999);

    harness.events.push(Event::Zoom(1.5));
    harness.frame();

    let pinched = harness.workspace.viewport.viewpoint();
    assert!(pinched.distance < panned.distance);
}

#[test]
fn alt_drag_orbits_in_laptop_mode_without_selecting_anything() {
    let mut harness = laptop_harness();
    let centre = viewport_centre(&harness);
    let before = harness.workspace.viewport.viewpoint();

    harness.hold(Modifiers::ALT);
    drag_screen(&mut harness, centre, centre + egui::vec2(60.0, 10.0));
    harness.hold(Modifiers::NONE);

    let orbited = harness.workspace.viewport.viewpoint();
    assert!(orbited.forward().dot(before.forward()) < 0.9999);
    assert!(harness.workspace.viewport.selection().is_empty());
}

#[test]
fn the_input_mode_is_chosen_in_preferences_and_remembered() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Navigation");
    assert!(harness.shows("Input mode"));

    harness.click(InputMode::Laptop.label());

    assert_eq!(
        harness.workspace.preferences.navigation.input_mode,
        InputMode::Laptop
    );
    harness.wait_until("the input mode is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).text("navigation.input_mode")
            == Some("laptop")
    });
    let stored =
        Preferences::from_settings(caditor_file::Settings::load(&dir.path().join("config")));
    assert_eq!(stored.navigation.input_mode, InputMode::Laptop);
    assert_eq!(
        Hint::Navigate.text(&stored.keymap, InputMode::Laptop),
        format!(
            "{} Double-click a face to open the feature that made it, and change any value in \
             the feature tree or in Parameters.",
            InputMode::Laptop.navigation_tip()
        )
    );
}

#[test]
fn the_project_tool_brings_a_face_outline_into_the_sketch_and_follows_the_keyboard() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = harness.draw_on_new_sketch();

    harness.use_tool_with(Key::P, Modifiers::ALT);
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { body, .. } if *body == extrude)
                && pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable while projecting");

    assert_eq!(harness.tool(), Some(Tool::Project));
    assert!(harness.shows(
        "Click an edge, corner or face of a body, or a curve of another sketch, to project it"
    ));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let projected: Vec<EntityId> = harness.sketch(sketch).projected().collect();
    let shown = harness.shown(sketch);
    let lines: Vec<EntityId> = projected
        .iter()
        .copied()
        .filter(|id| matches!(shown.entity(*id), Some(Entity::Line { .. })))
        .collect();

    assert_eq!(lines.len(), 4);
    assert!(
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Project the edges of"))
    );
    for line in &lines {
        let (start, end) = shown.line_endpoints(*line).unwrap();
        for end in [start, end] {
            assert!(
                [0.0, 40.0].iter().any(|edge| (end.x - edge).abs() < 1e-9)
                    && [0.0, 40.0].iter().any(|edge| (end.y - edge).abs() < 1e-9),
                "{end} is not a corner of the plate"
            );
        }
    }

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    assert!(harness.shows_containing("already projected into it"));
    assert_eq!(harness.sketch(sketch).projected().count(), projected.len());

    harness.key(Key::N, Modifiers::NONE);
    harness.frame();

    assert!(harness.shows_containing("Project "));
}

#[test]
fn a_hole_at_a_circle_can_take_the_circle_s_diameter_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let top = Plane::from_frame(
        caditor_geometry::Point3::new(0.0, 0.0, 10.0),
        caditor_geometry::Vector3::Z,
        caditor_geometry::Vector3::X,
    )
    .unwrap();
    let mut sketch = Sketch::new(top);
    sketch.add_circle(Point2::new(20.0, 20.0), 4.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 3.0 * 3.0 * 10.0
    ));
    assert!(harness.shows("Sized by"));

    harness.click(crate::hole_panel::SIZED_BY_CIRCLES);
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).sizing,
        caditor_document::HoleSizing::Circles
    );
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 4.0 * 4.0 * 10.0
    ));
}

fn open_hole(harness: &Harness, hole: FeatureId) -> caditor_document::Hole {
    harness
        .document()
        .feature(hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone()
}

#[test]
fn a_hole_is_drilled_on_a_selected_face_and_takes_a_metric_size_fit_and_slot_from_its_panel() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);

    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let definition = open_hole(&harness, hole);
    let sketch = harness.sketch(definition.sketch).clone();

    assert_eq!(harness.model.undo_label(), Some("Create Hole 1"));
    assert_eq!(definition.body, plate);
    assert_eq!(
        sketch
            .entities()
            .map(|(_, entity)| entity.clone())
            .collect::<Vec<_>>(),
        vec![Entity::Point(Point2::new(20.0, 20.0))]
    );
    assert!(attached_body(&harness, definition.sketch) == Some(plate));
    assert!(harness.shows_containing("Hole 1 is drilled in the middle of the face"));
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 9.0 * 10.0
    ));

    choose(&mut harness, crate::hole_panel::CUSTOM_SIZE, "M3");

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "3.4 mm"
    );
    assert!(removed_about(
        &harness,
        plate,
        std::f64::consts::PI * 1.7 * 1.7 * 10.0
    ));
    assert!(harness.shows("Normal"));

    harness.click("Normal");
    harness.click("Tapped");
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "2.5 mm"
    );
    assert!(harness.shows_containing("Thread M3 × 0.5"));

    harness.click("Tapped");
    harness.click("Fine");
    harness.settle();

    assert_eq!(
        open_hole(&harness, hole).diameter.to_stored_text(),
        "2.65 mm"
    );
    assert!(harness.shows_containing("Thread M3 × 0.35"));

    harness.click("Plain");
    harness.click("Counterbore");
    harness.settle();

    assert!(matches!(
        open_hole(&harness, hole).style,
        caditor_document::HoleStyle::Counterbore { ref diameter, .. } if diameter.to_stored_text() == "6.5 mm"
    ));

    harness.type_into_field(Id::new(("hole-field", "diameter", hole)), "2.6 mm");
    harness.settle();

    assert_eq!(open_hole(&harness, hole).standard, None);
    assert!(!harness.shows("Tapped"));

    harness.click("Round");
    harness.click("Slot");
    harness.settle();

    assert!(matches!(
        open_hole(&harness, hole).shape,
        caditor_document::HoleShape::Slot { .. }
    ));
    assert!(harness.shows("Slot length"));
    assert!(harness.shows("Slot angle"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
}
