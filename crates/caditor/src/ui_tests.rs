use std::{
    f64::consts::{FRAC_PI_2, PI},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{
    BodyOperation, Document, Edit, Editor, ExtrudeExtent, Feature, FeatureId, FeatureKind,
    RegionChoice, SolidFeature, SolidResult, Transaction,
};
use caditor_expression::{Expression, ParameterId, Unit};
use caditor_file::{ExportFormat, JournalEntry, Start, Storage, StorageConfig};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_kernel::MeshQuality;
use caditor_render::{GraphicsInfo, Msaa, Shading};
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
    annotations,
    app::{self, Workspace},
    canvas,
    commands::{Command, Offer, RecentSlot},
    drawing::Refusal,
    editing::{EditingCommand, Tool},
    export::ExportCommand,
    files::{Dialogs, FileCommand, Files, FilesConfig, Respond},
    graphics::{CurveQuality, FrameLimit, Graphics, Hardware},
    history::HistoryCommand,
    icons,
    import::{self, Placement},
    menu_bar,
    model::{Action, Model, Notice, RecomputeStatus, Services, WakerFactory},
    onboarding::Hint,
    panels::Focus,
    preferences::{PreferenceChange, Preferences, PreferencesCommand, PreferencesTab, TitleBar},
    scene,
    selection::{Pickable, PrincipalPlane},
    status_bar, typed_point,
    units::LengthUnit,
    widgets, window_frame,
};

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

#[derive(Clone, Default)]
struct ScriptedDialogs {
    answer: Arc<Mutex<Option<PathBuf>>>,
}

impl Dialogs for ScriptedDialogs {
    fn pick_model(&self, _directory: Option<PathBuf>, respond: Respond) {
        respond(self.answer.lock().clone());
    }

    fn pick_save_path(&self, _directory: Option<PathBuf>, _file_name: String, respond: Respond) {
        respond(self.answer.lock().clone());
    }

    fn pick_export_path(
        &self,
        _directory: Option<PathBuf>,
        _file_name: String,
        _format: ExportFormat,
        respond: Respond,
    ) {
        respond(self.answer.lock().clone());
    }

    fn pick_import(&self, _directory: Option<PathBuf>, respond: Respond) {
        respond(self.answer.lock().clone());
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
    text_colors: Vec<(String, Color32)>,
    time: f64,
    forced_hover: Option<(Pos2, Pickable)>,
    picks_held: bool,
    accessible: Vec<(NodeId, Node)>,
    window: ViewportInfo,
    window_commands: Vec<ViewportCommand>,
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
            text_colors: Vec::new(),
            time: 0.0,
            forced_hover: None,
            picks_held: false,
            accessible: Vec::new(),
            window: ViewportInfo::default(),
            window_commands: Vec::new(),
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
        output.textures_delta.clear();
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
        self.model
            .mesh_before(self.workspace.editing.context().solid);
        let built = self.workspace.viewport.build_scene(
            self.model.document(),
            self.model.evaluation(),
            self.model.display(),
            &self.workspace.editing,
        );
        self.answer_pick(&built);
        self.texts.clear();
        self.text_colors.clear();
        for clipped in output.shapes {
            let ClippedShape { shape, .. } = clipped;
            collect_texts(shape, &mut self.texts, &mut self.text_colors);
        }
    }

    fn answer_pick(&mut self, built: &scene::BuiltScene) {
        if self.picks_held {
            return;
        }
        let Some(cursor) = self
            .workspace
            .viewport
            .request(built, true)
            .and_then(|request| request.pick_at)
        else {
            return;
        };
        let hits = self
            .forced_hover
            .and_then(|(_, pickable)| built.picks.id_of(pickable))
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

    fn shows(&self, text: &str) -> bool {
        self.texts.iter().any(|(shown, _)| shown == text)
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

    fn built(&mut self) -> scene::BuiltScene {
        self.workspace.viewport.build_scene(
            self.model.document(),
            self.model.evaluation(),
            self.model.display(),
            &self.workspace.editing,
        )
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
        let built = self.built();
        self.forced_hover = Some((position, pickable));
        self.workspace.viewport.hover_through_pick(&built, pickable);
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
    assert_eq!(harness.color_of("Side sketch"), harness.error_color());
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
    harness.click("Close Without Saving");
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
        Some(std::fs::canonicalize(&existing).unwrap().as_path())
    );
    assert_eq!(
        caditor_file::load(&existing).unwrap().document,
        *harness.model.document()
    );
}

#[test]
fn save_as_refuses_a_model_open_in_another_window() {
    let dir = TempDir::new().unwrap();
    let other_path = dir.path().join("other.caditor");
    caditor_file::save(&Document::default(), &other_path, false).unwrap();
    let other = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.path().join("recovery")),
        },
        Start {
            file: Some(other_path.clone()),
            loaded_with_problems: false,
            base: Document::default(),
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
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.edit_width("45 mm");
    let untitled_journals = std::fs::read_dir(dir.path().join("recovery"))
        .unwrap()
        .count();
    assert_eq!(untitled_journals, 1);

    harness.answer_dialog(Some(dir.path().join("bracket")));
    harness.command(FileCommand::Quit);
    harness.click("Save As…");
    harness.wait_until("the model is saved", |harness| harness.files.should_quit());

    let path = dir.path().join("bracket.caditor");
    let saved = caditor_file::load(&path).unwrap();
    assert!(saved.issues.is_empty());
    assert_eq!(saved.document, *harness.model.document());
    assert_eq!(
        std::fs::read_dir(dir.path().join("recovery"))
            .unwrap()
            .count(),
        0
    );
    assert!(!dir.path().join(".bracket.caditor.journal").exists());
    let recent = caditor_file::RecentFiles::load(&dir.path().join("state"));
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
    harness.click("Close");
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
    harness.wait_until("the report is shown", |harness| {
        harness.shows("Imported “hole.dxf” into bracket")
    });
    assert_eq!(harness.sketch(sketch).entities().len(), before + 2);
    assert_eq!(harness.document().features().len(), features + 1);
    assert!(harness.shows(
        "The drawing does not say which unit it uses, so its numbers were read as millimetres."
    ));
    harness.click("OK");
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
    harness.click("OK");

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
    let plan = import::plan_drawing(harness.model.base(), &square, None, drawing);

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
    let Placement::Stale(drawing) = placement else {
        panic!("a drawing planned before the edit was placed on the changed model");
    };
    assert_eq!(harness.document().features().len(), features);

    let plan = import::plan_drawing(harness.model.base(), &square, None, drawing);
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
fn dropped_drawings_are_imported_one_after_another_and_a_dropped_model_opens() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let square = dir.path().join("square.dxf");
    write_drawing(
        &square,
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n\
         10\n0\n20\n0\n10\n30\n20\n0\n10\n30\n20\n30\n10\n0\n20\n30\n",
    );
    let hole = dir.path().join("hole.dxf");
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

    let path = dir.path().join("plate.CADITOR");
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
    harness.click("Continue Without Saving");
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
    if harness.shows("Continue Without Saving") {
        harness.click("Continue Without Saving");
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
    assert!(harness.shows("Versions of “plate.caditor”"));
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
    harness.click("OK");
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
    let path = dir.path().join("plate.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let journal = dir.path().join(".plate.caditor.journal");
    std::fs::write(&journal, b"\x89CJL\r\n\x1a\n\xff\xff\xff\xff").unwrap();

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Unsaved changes to “plate.caditor” could not be recovered"));
    let kept: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".unreadable"))
        .collect();
    assert_eq!(kept.len(), 1);
    let note = caditor_file::describe_set_aside(&dir.path().join(&kept[0]));
    assert!(harness.shows(&note), "{note}");
    harness.click("OK");
    assert!(!harness.model.is_dirty());
}

#[test]
fn unsaved_work_from_a_crash_is_offered_and_restored_with_its_history() {
    let dir = TempDir::new().unwrap();
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
            recovery_dir: Some(dir.path().join("recovery")),
        },
        Start {
            file: None,
            loaded_with_problems: false,
            base,
            entries: vec![JournalEntry::Apply(change)],
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    assert!(crashed.flusher().flush(FILE_TIMEOUT));
    assert!(crashed.close(false).wait(FILE_TIMEOUT));

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
    assert_eq!(
        std::fs::read_dir(dir.path().join("recovery"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn a_missing_file_is_reported_and_dropped_from_recent_files_and_reopening_is_harmless() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert_eq!(harness.files.recent(), std::slice::from_ref(&path));

    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is checked", |harness| {
        harness.shows("“kept.caditor” is already open.")
    });

    std::fs::rename(&path, dir.path().join("moved.caditor")).unwrap();
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
    if harness.shows("Continue Without Saving") {
        harness.click("Continue Without Saving");
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

    assert_eq!(harness.color_of("Base sketch"), harness.error_color());
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
    harness.click("Horizontal");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Add Horizontal"));
    assert!(harness.shows("3 degrees of freedom left"));
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.y - end.y).abs() < 1e-9, "{start} {end}");

    harness.key(Key::H, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
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

    harness.click("Parallel");
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
    harness.hover("Parallel");
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

    harness.click("Diameter");
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

    harness.click("Parallel");
    harness.click("Equal");
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

    harness.click("Coincident");
    harness.settle();
    harness.select(entity_pickables(feature, &[spline, line]));
    harness.click("Tangent");
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
    harness.point_at(Point2::new(25.5, 25.0));
    let line = entities_of_kind(harness.sketch(feature), "Line")[0];
    assert!(harness.shows(&format!("On Line {line}")));
    harness.click_at(Point2::new(25.5, 25.0));

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
            .lines
            .iter()
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
    harness.click("Construction");
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
    assert!(harness.shows("X: the other way round   The arc follows your sweep around the centre, a typed end the shorter way   Esc: cancel the arc   Type x, y or length < angle for an exact point"));
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
    harness.hover("Parallel");
    assert!(harness.shows("Make lines parallel. Select two or more lines (Shift+P)"));
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
    let ExtrudeExtent::OneSide { distance, .. } = &extrude.extent else {
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

    let built = harness.built();
    assert_eq!(built.scene.meshes.len(), 1);
    assert_eq!(built.scene.meshes[0].mesh.face_count(), 6);
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
    assert!(harness.shows("Enter a value above zero. Use Reversed to go the other way"));
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
    assert!(
        harness
            .built()
            .picks
            .pickables()
            .all(|pickable| !matches!(pickable, Pickable::Region { .. }))
    );
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
    let refused_forward = harness.shows("Enter a distance above zero");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.type_into_field(Id::new(("solid-field", "backward", extrude)), "-3 mm");
    let refused_backward = harness.shows("Enter a distance above zero");

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

    let RegionChoice::Chosen(keys) = harness.solid(extrude).regions() else {
        panic!("the regions were not chosen");
    };
    assert_eq!(keys.len(), 2);
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
    assert!(harness.shows(&format!(
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
fn a_sketch_started_on_a_selected_face_follows_it_when_the_body_changes() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.hover("New sketch");
    assert!(
        harness.shows(
            "Start a sketch on the selected face; it follows the face when the model changes"
        )
    );
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
    assert_eq!(harness.solid(boss).operation(), BodyOperation::Add(extrude));
    assert!((harness.body_volume(extrude) - (16000.0 + 2000.0)).abs() < 1.0);

    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.type_into_field(Id::new(("solid-field", "distance", extrude)), "25 mm");
    harness.settle();
    assert_eq!(plane_height(&harness, sketch), 25.0);
    assert!((harness.body_volume(extrude) - (40000.0 + 2000.0)).abs() < 1.0);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
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
    harness.click("Loose");
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

    let built = harness.built();
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
    assert!(harness.shows("Enter a radius above zero"));
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
            extent: ExtrudeExtent::OneSide {
                distance: Expression::parse_stored("2 mm").unwrap(),
                reversed: true,
            },
            operation: BodyOperation::Remove(plate),
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

    let built = harness.built();
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
    assert!(harness.shows("Enter a thickness above zero"));
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
            .shows("Select planes, faces, axes or edges, then use them from the feature's panel")
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
    assert!(
        harness
            .shows("No command matches. Commands that do not fit what you are doing are left out.")
    );

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
    assert!(harness.shows("Keyboard Shortcuts"));
    harness.type_text("undo");
    assert_eq!(harness.count_shown("Redo"), 1);
    harness.click("Keyboard Shortcuts");

    harness.click("Add…");
    assert!(harness.shows("Press the keys… (Esc cancels)"));
    harness.key(Key::U, Modifiers::ALT);
    harness.show_new_windows();
    assert!(harness.shows("Alt+U"));

    harness.click("Add…");
    harness.key(Key::F, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.shows("F is already used by Fit view. Use it for Undo instead?"));
    harness.click("Keep it where it is");
    assert!(!harness.shows("F"));

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
    assert!(harness.shows(&Hint::Navigate.text(&harness.workspace.preferences.keymap)));

    harness.click("Got it");
    assert!(harness.shows(&Hint::Palette.text(&harness.workspace.preferences.keymap)));
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
    assert!(harness.shows(&Hint::Start.text(&keymap)));
    harness.draw_on_new_sketch();
    assert!(harness.shows(&Hint::Draw.text(&keymap)));
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(40.0, 30.0));
    harness.settle();
    assert!(harness.shows(&Hint::Constrain.text(&keymap)));
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    assert!(harness.shows(&Hint::Sweep.text(&keymap)));
    harness.click("Help");
    harness.click("Welcome and samples…");
    assert!(harness.shows("Welcome to caditor"));
    harness.click("Start with an empty model");
    assert!(!harness.shows("Welcome to caditor"));
    harness.click("Continue Without Saving");
    harness.settle();
    assert_eq!(harness.document().features().len(), 0);
    assert!(!harness.model.is_dirty());
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
    let path = dir.path().join("kept.caditor");
    caditor_file::save(&sample_document().unwrap(), &path, false).unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
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
    for label in ["Close", "Reset all shortcuts", "Keyboard Shortcuts"] {
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
    assert_eq!(
        crate::datum_tools::plane_from_selection(
            &harness.model,
            harness.workspace.viewport.selection(),
            end
        ),
        Err("The selected edge is not straight, so it gives no axis")
    );
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
    let refused = offer(&harness, Command::DeleteParameter);
    assert_eq!(refused.title(), "Delete parameter: width");
    assert!(refused.availability.is_err());

    run_from_palette(&mut harness, "add parameter");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.document().parameter_named("parameter").is_some());
    run_from_palette(&mut harness, "delete parameter");
    harness.settle();
    assert!(harness.document().parameter_named("parameter").is_none());
    assert_eq!(harness.model.undo_label(), Some("Delete parameter"));

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
fn a_parameter_that_cannot_be_deleted_names_what_uses_it() {
    let mut harness = Harness::new();
    let (_, trash) = harness
        .texts
        .iter()
        .find(|(shown, _)| shown == crate::icons::DELETE)
        .expect("the first parameter has a delete button")
        .clone();
    harness.events.push(Event::PointerMoved(trash.center()));
    for _ in 0..TOOLTIP_FRAMES {
        harness.frame();
    }

    assert!(harness.shows("width is used by height and Base sketch. Remove those uses first."));
}

#[test]
fn tips_are_dismissed_and_hidden_from_the_palette() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::first_run(dir.path());
    harness.frame();
    harness.click("Flanged spool");
    harness.settle();
    let keymap = harness.workspace.preferences.keymap.clone();
    assert!(harness.shows(&Hint::Navigate.text(&keymap)));

    run_from_palette(&mut harness, "dismiss the tip");
    assert!(!harness.shows(&Hint::Navigate.text(&keymap)));
    assert!(harness.shows(&Hint::Palette.text(&keymap)));
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
    assert!(harness.accessible_named(Role::Button, "Edit sketch (Base sketch)"));
    assert!(harness.accessible_named(Role::Button, "Add parameter"));
    assert!(harness.accessible_named(Role::Button, "Length unit: millimetres"));

    harness.click("File");
    assert_readable(&harness, "The File menu");
    assert!(harness.accessible_named(Role::Button, "Open Sample"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.settle();
    assert_readable(&harness, "An open extrusion");
    assert!(harness.captioned(Role::TextInput, "Distance"));
    assert!(harness.captioned(Role::ComboBox, "Extent"));

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    assert_readable(&harness, "An open datum plane");
    assert!(harness.captioned(Role::TextInput, "Offset"));
    assert!(harness.accessible_named(Role::Button, "Use selected"));

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
    let mut harness = Harness::new();
    let document = harness.document().clone();
    let name = "Bracket for the front suspension, revised after the second test.caditor";
    harness
        .model
        .replace(document, Some(PathBuf::from(name)), false);
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
    let overlapping: Vec<(&str, &str)> = harness
        .texts
        .iter()
        .enumerate()
        .flat_map(|(index, (first, a))| {
            harness.texts[index + 1..]
                .iter()
                .filter(move |(_, b)| a.shrink(0.5).intersects(b.shrink(0.5)))
                .map(move |(second, _)| (first.as_str(), second.as_str()))
        })
        .collect();
    let outside: Vec<&str> = harness
        .texts
        .iter()
        .filter(|(_, rect)| !visible.expand(0.5).contains_rect(*rect))
        .map(|(shown, _)| shown.as_str())
        .collect();
    let deletes: Vec<Rect> = harness
        .texts
        .iter()
        .filter(|(shown, _)| shown == crate::icons::DELETE)
        .map(|(_, rect)| *rect)
        .collect();

    assert!(narrowed.width() < panel.width(), "{narrowed:?}");
    assert_eq!(settled, narrowed);
    assert!(overlapping.is_empty(), "{overlapping:#?}");
    assert!(outside.is_empty(), "{outside:#?}");
    assert!(rect_of(notice).height() > 1.5 * line, "the notice wraps");
    assert!(rect_of(name).max.x <= rect_of("Search commands").min.x);
    assert_eq!(deletes.len(), 2);
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
    let added = harness.parameter("parameter");
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
    harness.click_beside(crate::icons::DELETE, "depth");
    harness.settle();
    let deleted_label = harness.model.undo_label().map(str::to_owned);
    let deleted = harness.document().parameter(added).is_none();
    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(added_label.as_deref(), Some("Add parameter"));
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

fn datum_plane_of(harness: &Harness, feature: FeatureId) -> caditor_document::DatumPlane {
    match datum_of(harness, feature) {
        caditor_document::Datum::Plane(plane) => plane.clone(),
        caditor_document::Datum::Axis(_) => panic!("expected a datum plane"),
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
    harness.click_beside(crate::icons::REMOVE, "Turned about");
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
    harness.click_beside("Use selected", "Where");
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
    drag_in_sketch(&mut harness, grabbed, Point2::new(30.0, 20.0));
    harness.wait_until("the drag is committed", |harness| {
        harness.sketch(feature).point(end) != before.point(end)
    });
    harness.settle();

    let dragged = harness.sketch(feature).point(end).unwrap();
    assert!(
        dragged.distance(Point2::new(30.0, 20.0)) < 0.5,
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
    assert_eq!(harness.workspace.preferences.graphics, Graphics::default());
    assert_eq!(harness.workspace.preferences_tab, PreferencesTab::Graphics);
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
