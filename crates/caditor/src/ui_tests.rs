use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{
    BodyOperation, Document, ExtrudeExtent, Feature, FeatureId, FeatureKind, RegionChoice,
    SolidFeature, SolidResult,
};
use caditor_expression::{Expression, ParameterId};
use caditor_file::{ExportFormat, PixelRows, StorageConfig};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_render::{Background, ImageError};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{
    Color32, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape,
    ViewportCommand, ViewportId, ViewportIdMap, ViewportInfo,
    accesskit::{Node, NodeId, Role},
    epaint::ClippedShape,
};
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::{
    app::{self, Workspace},
    canvas,
    commands::{Command, Offer},
    drawing::Refusal,
    editing::{EditingCommand, Tool},
    files::{Dialogs, FileCommand, Files, FilesConfig, Respond},
    icons,
    image_export::ReadPixels,
    logo,
    model::{Action, Model, RecomputeStatus, Services, WakerFactory},
    move_manipulator::Handle,
    panels::Focus,
    preferences::{InputMode, PreferenceChange, Preferences, PreferencesCommand},
    scene,
    selection::{Pickable, PrincipalPlane},
    version_preview,
};

mod accessibility;
mod analysis;
mod background_work;
mod blend_feet;
mod blends;
mod bodies;
mod body_appearance;
mod body_finishes;
mod body_groups;
mod body_tools;
mod box_selection;
mod cancel_and_parameters;
mod canvas_guidance;
mod canvas_overlays;
mod click_picks;
mod coils;
mod combine;
mod command_palette;
mod completion;
mod configuration_files;
mod configurations;
mod coordinate_systems;
mod curve_pattern_handles;
mod custom_colours;
mod datums;
mod delete_face;
mod dialogs;
mod dimension_labels;
mod display_styles;
mod drawing_import;
mod exporting;
mod extrude_from_selection;
mod extrusion_basics;
mod extrusion_detection;
mod face_tools;
mod far_cuts;
mod feature_handles;
mod feature_panels;
mod feature_tree;
mod feature_tree_choice;
mod feature_tree_marks;
mod files_and_history;
mod frame_costs;
mod group_menus;
mod handle_drags;
mod helices;
mod hole_feet;
mod hole_highlights;
mod hole_place_handles;
mod holes;
mod image_handles;
mod import_costs;
mod import_jobs;
mod imported_bodies;
mod interface_fit;
mod keyboard_driving;
mod laptop_input;
mod large_interface;
mod large_sketch_rows;
mod lofts;
mod measure;
mod model_dimensions;
mod model_properties_and_views;
mod negative_distances;
mod notices;
mod opening_and_saving;
mod paint_selection;
mod palette_and_notices;
mod panel_fields;
mod parameter_files;
mod parameters;
mod paste_picks;
mod path_sweeps;
mod pattern_grid;
mod patterns;
mod pick_list;
mod pipes;
mod plane_selectors;
mod preference_details;
mod preference_groups;
mod preferences;
mod primitive_creation;
mod primitive_feet;
mod primitives;
mod principal_geometry;
mod queued_actions;
mod queued_clicks;
mod reach_through;
mod recovery;
mod reference_images;
mod region_references;
mod remembered_starts;
mod repeating_and_growing;
mod ribbon_and_sketch_bar;
mod ribs;
mod scale_model;
mod screenshots;
mod sections;
mod selecting_alike;
mod selection_filters;
mod selection_readouts;
mod selection_sets;
mod selection_targets;
mod separate_pieces;
mod shaped_sweeps;
mod shell_and_offset;
mod shell_walls;
mod sketch_blend_curves;
mod sketch_body_picks;
mod sketch_body_snaps;
mod sketch_breaks;
mod sketch_chain_arcs;
mod sketch_chamfers;
mod sketch_conics;
mod sketch_constraints;
mod sketch_dimensions;
mod sketch_dragging;
mod sketch_drawing;
mod sketch_driven_patterns;
mod sketch_ellipses;
mod sketch_first_dimension;
mod sketch_free;
mod sketch_gears;
mod sketch_inference;
mod sketch_keys;
mod sketch_modify;
mod sketch_modify_status;
mod sketch_patterns;
mod sketch_planes;
mod sketch_projection;
mod sketch_regions;
mod sketch_rim_snaps;
mod sketch_sessions;
mod sketch_shapes;
mod sketch_snapping;
mod sketch_symmetry;
mod sketch_tangent_circles;
mod sketch_tidying;
mod sketch_tracking;
mod sketch_transform;
mod sketch_trim_extend;
mod solid_panels;
mod stale_sketch_results;
mod start_card;
mod storage_worker;
mod svg_import;
mod templates;
mod themes;
mod thread_features;
mod threads;
mod title_bar;
mod turn_snaps;
mod typed_values;
mod undo_history;
mod up_to_next_handles;
mod upload_indicator;
mod user_guide;
mod view_framing;
mod view_menu;
mod view_navigation;
mod welcome_and_about;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1400.0, 1000.0));
const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(60);
const FRAME_SECONDS: f64 = 0.05;
const ANIMATION_FRAMES: usize = 5;
const WHEEL_TRIES: usize = 40;
const WHEEL_STEP: f32 = 60.0;

#[derive(Debug, Clone, Copy)]
enum Wheel {
    Up,
    Down,
}

impl Wheel {
    fn delta(self) -> f32 {
        match self {
            Self::Up => WHEEL_STEP,
            Self::Down => -WHEEL_STEP,
        }
    }
}
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
    asked_in: Arc<Mutex<Vec<Option<PathBuf>>>>,
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

    fn respond(&self, directory: Option<PathBuf>, respond: Respond) {
        self.asked_in.lock().push(directory);
        if let Some(held) = self.held.lock().as_mut() {
            held.push(respond);
            return;
        }
        respond(self.reply());
    }
}

impl Dialogs for ScriptedDialogs {
    fn pick_model(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_save_path(&self, directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_template(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_template_path(&self, directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_export_path(
        &self,
        directory: Option<PathBuf>,
        _file_name: String,
        _format: ExportFormat,
        respond: Respond,
    ) {
        self.respond(directory, respond);
    }

    fn pick_import(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_drawing_path(
        &self,
        _title: &str,
        directory: Option<PathBuf>,
        _file_name: String,
        respond: Respond,
    ) {
        self.respond(directory, respond);
    }

    fn pick_image_path(&self, directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_parameters_path(
        &self,
        directory: Option<PathBuf>,
        _file_name: String,
        respond: Respond,
    ) {
        self.respond(directory, respond);
    }

    fn pick_parameters(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_configurations_path(
        &self,
        directory: Option<PathBuf>,
        _file_name: String,
        respond: Respond,
    ) {
        self.respond(directory, respond);
    }

    fn pick_configurations(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_picture(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
    }

    fn pick_backdrop(&self, directory: Option<PathBuf>, respond: Respond) {
        self.respond(directory, respond);
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
    renders_held: bool,
    textures: crate::overlay::TextureMirror,
    painted: Option<Painted>,
    hovered_files: Vec<egui::HoveredFile>,
    clipboard: Option<String>,
    repaint_after: Duration,
}

struct Painted {
    shapes: Vec<ClippedShape>,
    pixels_per_point: f32,
}

struct SolidRows {
    width: u32,
    height: u32,
    pixels: Option<Vec<u8>>,
    sent: Vec<u8>,
}

impl PixelRows for SolidRows {
    type Error = ImageError;

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn next_rows(&mut self) -> Option<Result<&[u8], ImageError>> {
        self.sent = self.pixels.take()?;
        Some(Ok(&self.sent))
    }
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
        Self::starting_with(dir, document, workspace, |_| {})
    }

    fn starting_with(
        dir: Option<&Path>,
        document: Document,
        workspace: Workspace,
        prepare: impl FnOnce(&mut Files),
    ) -> Self {
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
        prepare(&mut files);
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
            renders_held: false,
            textures: crate::overlay::TextureMirror::default(),
            painted: None,
            hovered_files: Vec::new(),
            clipboard: None,
            repaint_after: Duration::MAX,
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
        let shapes = self.pass();
        self.texts.clear();
        self.text_clips.clear();
        self.text_colors.clear();
        for clipped in shapes {
            let ClippedShape { shape, clip_rect } = clipped;
            collect_texts(shape, &mut self.texts, &mut self.text_colors);
            self.text_clips.resize(self.texts.len(), clip_rect);
        }
    }

    fn pass(&mut self) -> Vec<ClippedShape> {
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
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.clipboard = Some(text.clone());
            }
        }
        if let Some(root) = output.viewport_output.get_mut(&ViewportId::ROOT) {
            self.repaint_after = root.repaint_delay;
            if root.commands.contains(&ViewportCommand::RequestPaste)
                && let Some(text) = self.clipboard.clone()
            {
                self.events.push(Event::Paste(text));
            }
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
        if self.model.is_checking_constraints() {
            self.model.finish_checking_constraints();
            self.workspace.editing.sync(&self.model);
        }
        self.render_image();
        self.render_thumbnail();
        self.render_preview();
        self.model
            .mesh_before(self.workspace.editing.context().solid);
        self.model.request_regions(self.workspace.editing.feature());
        self.workspace
            .viewport
            .build_scene(&self.model, &self.workspace.editing);
        self.answer_pick();
        output.shapes
    }

    fn render_image(&mut self) {
        if self.renders_held {
            return;
        }
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
                Ok(Box::new(SolidRows {
                    width,
                    height,
                    pixels: Some(texel.repeat(width as usize * height as usize)),
                    sent: Vec::new(),
                }))
            }
        };
        self.files.image_rendered(pixels, &mut self.model);
    }

    fn render_thumbnail(&mut self) {
        if self.renders_held {
            return;
        }
        let Some(job) = self.files.thumbnail_job() else {
            return;
        };
        let (width, height) = (job.size.width, job.size.height);
        self.files.thumbnail_rendered(Some(Box::new(SolidRows {
            width,
            height,
            pixels: Some([90, 120, 200, 255].repeat(width as usize * height as usize)),
            sent: Vec::new(),
        })));
    }

    fn render_preview(&mut self) {
        let Some(picture) = self.files.preview_picture() else {
            return;
        };
        let size = version_preview::PICTURE_SIZE;
        assert!(picture.pixels_per_point > 0.0);
        self.files.preview_rendered(Ok(Box::new(SolidRows {
            width: size.width,
            height: size.height,
            pixels: Some([27, 28, 31, 255].repeat(size.width as usize * size.height as usize)),
            sent: Vec::new(),
        })));
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
        let Some(picks) = self
            .workspace
            .viewport
            .build_scene(&self.model, &self.workspace.editing)
            .map(|built| Arc::clone(&built.picks))
        else {
            return;
        };
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
        loop {
            while self.computing() {
                assert!(Instant::now() < deadline, "the recompute did not finish");
                self.model.poll();
                std::thread::yield_now();
            }
            self.frame();
            self.frame();
            if !self.computing() {
                return;
            }
        }
    }

    fn computing(&self) -> bool {
        matches!(self.model.status(), RecomputeStatus::Running { .. })
            || self.model.bodies_pending()
            || self.model.masses_pending()
            || self.model.pictures().pending()
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

    fn hold_dialogs(&self) {
        *self.dialogs.held.lock() = Some(Vec::new());
    }

    fn answer_held_dialogs(&self, path: Option<PathBuf>) {
        let held = self.dialogs.held.lock().take().unwrap_or_default();
        for respond in held {
            respond(Ok(path.clone()));
        }
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

    fn expand_keys(&mut self) {
        self.workspace.viewport.show_more_keys(true);
        self.frame();
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
        if self.computing() {
            self.settle();
        }
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

    fn click_tool(&mut self, label: &str) {
        if self.shows(label) {
            self.click(label);
        } else {
            self.click_button(label);
        }
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

    fn fully_shown(&self, label: &str) -> bool {
        self.texts
            .iter()
            .zip(&self.text_clips)
            .any(|((shown, rect), clip)| shown == label && clip.expand(0.5).contains_rect(*rect))
    }

    fn wheel_until_shown(&mut self, over: Pos2, label: &str, wheel: Wheel) {
        self.events.push(Event::PointerMoved(over));
        self.frame();
        for _ in 0..WHEEL_TRIES {
            if self.fully_shown(label) {
                return;
            }
            self.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, wheel.delta()),
                phase: egui::TouchPhase::Move,
                modifiers: self.held,
            });
            self.let_animations_finish();
        }
        panic!("'{label}' never came into view");
    }

    fn panel_rect(&self, id: &str) -> Rect {
        egui::containers::panel::PanelState::load(&self.context, Id::new(id))
            .unwrap_or_else(|| panic!("the {id} panel is not shown"))
            .outer_rect
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

    fn draft_into_field(&mut self, id: Id, text: &str) {
        self.context.memory_mut(|memory| memory.request_focus(id));
        self.frame();
        self.key(Key::A, Modifiers::COMMAND);
        self.events.push(Event::Text(text.to_owned()));
        self.frame();
        self.frame();
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

    fn read_screen(&mut self) {
        self.context.enable_accesskit();
        self.frame();
        self.context.disable_accesskit();
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

fn write_drawing(path: &Path, units: Option<i64>, entities: &str) {
    let header = units.map_or_else(String::new, |units| {
        format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n")
    });
    let text = format!("{header}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n");
    std::fs::write(path, text).unwrap();
}

fn square_drawing(path: &Path) {
    write_drawing(
        path,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
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

fn damage_chunk(bytes: &[u8], chunk: usize) -> Vec<u8> {
    let starts: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, window)| *window == b"CDCH")
        .map(|(start, _)| start)
        .collect();
    let end = starts.get(chunk + 1).copied().unwrap_or(bytes.len());
    let mut damaged = bytes.to_vec();
    damaged[end - 1] ^= 0x55;
    damaged
}

fn fail_a_frame(harness: &mut Harness) -> app::AfterFailedFrame {
    app::after_failed_frame(
        &mut harness.workspace,
        &mut harness.model,
        &mut harness.files,
    )
}

fn line_ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    }
}

fn only_constraint(sketch: &Sketch) -> (caditor_sketch::ConstraintId, Constraint) {
    let mut constraints = sketch.constraints();
    let (id, constraint) = constraints.next().unwrap();
    assert!(constraints.next().is_none());
    (id, constraint.clone())
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

fn set_high_contrast(harness: &mut Harness, on: bool) {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::HighContrast(on),
    )));
    harness.frame();
    harness.frame();
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

fn sketch_entity(feature: FeatureId, entity: EntityId) -> Pickable {
    Pickable::SketchEntity { feature, entity }
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

fn refused(harness: &mut Harness, text: &str, refusal: Refusal) {
    type_point(harness, text);
    assert!(harness.shows(refusal.reason()), "{text}");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
}

fn with_type_hint(hint: &str, keys: &str) -> String {
    format!("{hint}   {keys}   x,y or length<angle: type an exact point")
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

fn extruded_plate(harness: &mut Harness) -> (FeatureId, Pickable) {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 40.0));
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click_tool("Extrude");
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

fn drag_path(harness: &mut Harness, path: &[Pos2]) {
    let (Some(first), Some(last)) = (path.first().copied(), path.last().copied()) else {
        return;
    };
    harness.events.push(Event::PointerMoved(first));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: first,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for position in path.iter().skip(1) {
        harness.events.push(Event::PointerMoved(*position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: last,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
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

fn draft_volume(harness: &Harness, body: FeatureId) -> Option<f64> {
    let result = harness.model.draft_evaluation()?.body_result(body)?;
    Some(result.solid()?.mesh()?.mass_properties().volume)
}

fn drag_value_arrow(
    harness: &mut Harness,
    measured: crate::value_gauges::Measured,
    along: f64,
) -> f64 {
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let from = harness
        .workspace
        .viewport
        .handle_position(Handle::Value(measured), 0.0)
        .expect("the value arrow is shown");
    let to = harness
        .workspace
        .viewport
        .handle_position(Handle::Value(measured), along)
        .unwrap();
    let step = harness.workspace.viewport.manipulator_step().unwrap();
    drag_screen(harness, from, to);
    harness.frame();
    harness.settle();
    step
}

fn length_value(harness: &Harness, expression: &caditor_expression::Expression) -> f64 {
    harness
        .model
        .parameters()
        .evaluate_expression(expression)
        .unwrap()
        .value
}

fn opened_move(harness: &mut Harness, top: Pickable) -> FeatureId {
    harness.select([top]);
    harness.click("Move body");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.workspace.editing.solid().expect("the move is open")
}

fn move_of(harness: &Harness, movement: FeatureId) -> caditor_document::Move {
    match harness
        .document()
        .feature(movement)
        .map(|feature| feature.kind.clone())
    {
        Some(FeatureKind::Move(moved)) => moved,
        other => panic!("the move is still a move: {other:?}"),
    }
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
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    peg
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
                taper: None,
                wall: None,
                direction: None,
            })),
        ));
    }
    let combine = transaction.add_feature(
        "Combine 1",
        FeatureKind::Combine(caditor_document::Combine::new(
            bodies[0],
            bodies[1],
            caditor_document::CombineOperation::Join,
        )),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    combine
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

fn offset_of(harness: &Harness, feature: FeatureId) -> &caditor_document::OffsetFace {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.offset_face())
        .unwrap()
}

fn primitive_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Primitive {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.primitive())
        .unwrap()
}

fn body_centre(harness: &Harness, body: FeatureId) -> Point3 {
    let bounds = harness
        .model
        .evaluation()
        .body(body)
        .and_then(|solid| solid.bounding_box())
        .unwrap();
    (bounds.min() + bounds.max()) / 2.0
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

fn datum_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Datum {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.datum())
        .unwrap()
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

fn feature_names(harness: &Harness) -> Vec<String> {
    harness
        .document()
        .features()
        .map(|feature| feature.name.clone())
        .collect()
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

fn assert_readable(harness: &Harness, screen: &str) {
    let unreadable = harness.unreadable_nodes();
    assert!(
        unreadable.is_empty(),
        "{screen} exposes glyphs or unnamed buttons: {unreadable:#?}"
    );
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

fn open_solid(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a solid feature is open")
}

fn choose(harness: &mut Harness, current: &str, option: &str) {
    harness.hold_still();
    harness.click_lowest(current);
    harness.click_lowest(option);
    harness.settle();
}

fn open_combo(harness: &mut Harness, caption: &str) {
    if harness.computing() {
        harness.settle();
    }
    if harness.accessible.is_empty() {
        harness.context.enable_accesskit();
        harness.frame();
    }
    harness.hold_still();
    let mut center = combo_centre(harness, caption);
    harness.events.push(Event::PointerMoved(center));
    harness.hold_still();
    for _ in 0..STILL_FRAMES {
        let settled = combo_centre(harness, caption);
        if settled == center {
            break;
        }
        center = settled;
        harness.events.push(Event::PointerMoved(center));
        harness.hold_still();
    }
    harness.click_screen(center);
    harness.show_new_windows();
    harness.frame();
}

fn combo_centre(harness: &Harness, caption: &str) -> Pos2 {
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
    Pos2::new(
        (0.5 * (bounds.x0 + bounds.x1)) as f32,
        (0.5 * (bounds.y0 + bounds.y1)) as f32,
    )
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

fn start_of_solid(harness: &Harness, feature: FeatureId) -> Option<caditor_document::SolidStart> {
    harness.solid(feature).start().cloned()
}

fn datum_plane_of(harness: &Harness, feature: FeatureId) -> caditor_document::DatumPlane {
    match datum_of(harness, feature) {
        caditor_document::Datum::Plane(plane) => plane.clone(),
        other => panic!("expected a datum plane offset from another, found {other:?}"),
    }
}

fn hidden(harness: &Harness, feature: FeatureId) -> bool {
    harness.document().feature(feature).unwrap().hidden
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
    for step in 1..=4 {
        let target = harness.on_screen(to);
        let position = from + (target - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: harness.on_screen(to),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
}

fn bar_row(harness: &Harness) -> f32 {
    harness.position_of("File").y
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

fn logo_loaded(harness: &Harness, size: u32) -> bool {
    harness
        .context
        .tex_manager()
        .read()
        .allocated()
        .any(|(_, texture)| texture.name == logo::texture_name(size))
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

fn painted_faces(harness: &mut Harness, colour: caditor_render::Color) -> usize {
    harness
        .built_with_meshes(1)
        .scene
        .meshes
        .iter()
        .flat_map(|mesh| mesh.faces.iter())
        .filter(|face| face.color == colour)
        .count()
}

fn add_block(harness: &mut Harness, name: &str, corners: [Point2; 2], height: &str) -> FeatureId {
    let mut transaction = harness.document().transaction(format!("Add {name}"));
    let block = block_into(&mut transaction, name, corners, height);
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    block
}

fn block_into(
    transaction: &mut caditor_document::TransactionBuilder<'_>,
    name: &str,
    corners: [Point2; 2],
    height: &str,
) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(&mut outline, corners[0], corners[1]);
    let sketch = transaction.add_feature(format!("{name} sketch"), FeatureKind::from(outline));
    transaction.add_feature(
        name,
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored(height).unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    )
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

fn radii_of_circles(sketch: &Sketch) -> Vec<f64> {
    let mut radii: Vec<f64> = entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(_, radius)| radius)
        .collect();
    radii.sort_by(f64::total_cmp);
    radii
}

fn mirror_fixture() -> (Sketch, EntityId, EntityId, EntityId) {
    let mut sketch = Sketch::new(Plane::XY);
    let mirror = sketch.add_line(Point2::new(0.0, -30.0), Point2::new(0.0, 30.0));
    sketch.set_construction(mirror, true).unwrap();
    let line = sketch.add_line(Point2::new(5.0, 0.0), Point2::new(15.0, 10.0));
    let circle = sketch.add_circle(Point2::new(10.0, -10.0), 3.0);
    (sketch, mirror, line, circle)
}

fn fillet_arc(sketch: &Sketch) -> Option<caditor_sketch::ArcGeometry> {
    entities_of_kind(sketch, "Arc")
        .first()
        .and_then(|arc| sketch.arc(*arc))
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

fn open_extrude(harness: &Harness, feature: FeatureId) -> caditor_document::Extrude {
    match harness
        .document()
        .feature(feature)
        .unwrap()
        .kind
        .solid()
        .unwrap()
    {
        caditor_document::SolidFeature::Extrude(extrude) => extrude.clone(),
        caditor_document::SolidFeature::Revolve(_) | caditor_document::SolidFeature::Sweep(_) => {
            panic!("the feature is a revolve")
        }
    }
}

fn plate_edge_at(harness: &Harness, body: FeatureId, middle: Point3) -> caditor_kernel::EdgeName {
    let solid = harness.model.evaluation().body(body).unwrap();
    solid
        .edges()
        .find(|(_, edge)| (edge.curve().point(edge.interval().middle()) - middle).length() < 1e-6)
        .map(|(_, edge)| edge.name())
        .expect("the plate has that edge")
}

fn placed_holes(
    harness: &Harness,
    hole: FeatureId,
) -> Vec<(crate::hole_placement::PlacedHole, Point3)> {
    let definition = open_hole(harness, hole);
    let plane = crate::scene::sketch_plane(
        harness.document(),
        harness.model.evaluation(),
        definition.sketch,
    )
    .expect("the hole's sketch has a plane");
    crate::hole_placement::read(harness.document(), &definition)
        .expect("the hole's sketch is a placement")
        .holes
        .into_iter()
        .map(|placed| {
            let at =
                crate::hole_placement::solved_point(&harness.model, definition.sketch, &placed);
            (placed, plane.to_world(at))
        })
        .collect()
}

fn open_hole_on(harness: &mut Harness, face: Pickable) -> FeatureId {
    harness.select([face]);
    harness.click("Hole");
    harness.settle();
    harness.workspace.editing.solid().expect("the hole is open")
}

fn extruded_shaft(harness: &mut Harness) -> Pickable {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_circle(Point2::new(20.0, 20.0), 4.0);
    harness.add_sketch(sketch);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable
                    .describe(harness.document(), harness.model.evaluation())
                    .contains(" side ")
        })
        .expect("the round side is pickable")
}

fn thread_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Thread {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.thread())
        .unwrap()
}

fn thread_lines(harness: &mut Harness) -> Vec<caditor_render::Line> {
    let thread = crate::scene_palette::STANDARD.thread;
    harness
        .built()
        .scene
        .lines()
        .filter(|line| line.color == thread)
        .collect()
}
