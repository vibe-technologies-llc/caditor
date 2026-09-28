use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{Document, Edit, Editor, Feature, FeatureId, FeatureKind, Transaction};
use caditor_expression::{Expression, ParameterId, Unit};
use caditor_file::{JournalEntry, Start, Storage, StorageConfig};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{
    Color32, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape,
    epaint::ClippedShape,
};
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::{
    annotations,
    app::{self, Workspace},
    editing::{EditingCommand, Tool},
    files::{Dialogs, FileCommand, Files, FilesConfig, Respond},
    model::{Action, Model, RecomputeStatus, Services, WakerFactory},
    panels::Focus,
    scene,
    selection::{Pickable, PrincipalPlane},
};

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1400.0, 1000.0));
const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_SECONDS: f64 = 0.05;
const ANIMATION_FRAMES: usize = 5;
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
}

impl Harness {
    fn new() -> Self {
        Self::with_directories(None)
    }

    fn with_directories(dir: Option<&Path>) -> Self {
        let document = crate::sample_document().unwrap();
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
        };
        let mut model = Model::new(document, services);
        let mut files = Files::new(config, Box::new(dialogs.clone()), no_wake());
        files.start(None, &mut model);
        let mut harness = Self {
            context: egui::Context::default(),
            model,
            files,
            dialogs,
            workspace: Workspace::new(),
            events: Vec::new(),
            texts: Vec::new(),
            text_colors: Vec::new(),
            time: 0.0,
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
        let input = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events: std::mem::take(&mut self.events),
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        self.model.poll();
        self.files.poll(&mut self.model);
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
        app::perform(
            actions,
            &mut self.model,
            &mut self.files,
            &mut self.workspace.editing,
        );
        self.workspace.viewport.build_scene(
            self.model.document(),
            self.model.evaluation(),
            &self.workspace.editing,
        );
        self.texts.clear();
        self.text_colors.clear();
        for clipped in output.shapes {
            let ClippedShape { shape, .. } = clipped;
            collect_texts(shape, &mut self.texts, &mut self.text_colors);
        }
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + RECOMPUTE_TIMEOUT;
        while matches!(self.model.status(), RecomputeStatus::Running { .. }) {
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
            &mut self.workspace.editing,
        );
        self.show_new_windows();
    }

    fn show_new_windows(&mut self) {
        self.frame();
        self.frame();
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
        scene::displayed_sketch(self.model.evaluation(), feature)
            .unwrap()
            .into_owned()
    }

    fn add_sketch(&mut self, sketch: Sketch) -> FeatureId {
        let mut transaction = self.document().transaction("Add sketch");
        let feature = transaction.add_feature("Plate", FeatureKind::Sketch(sketch));
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
        self.key(key, Modifiers::NONE);
        self.frame();
        self.frame();
    }

    fn on_screen(&self, point: Point2) -> Pos2 {
        let feature = self.editing().expect("a sketch is being edited");
        let plane = self.sketch(feature).plane();
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

    fn error_color(&self) -> Color32 {
        self.context.global_style().visuals.error_fg_color
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

    harness.type_into(Focus::ParameterValue(height), "400 mm * 1 mm / width");
    assert_eq!(harness.expression_text("height"), "400 mm * 1 mm / width");
    harness.settle();
    assert!(harness.shows("10 mm"));

    harness.type_into(Focus::ParameterValue(width), "0 mm");
    harness.settle();
    assert!(harness.shows("⚑ 1 feature failed"));
    assert!(harness.shows("⚑ Side sketch"));
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
    assert!(harness.shows("height: There is no parameter named 'wdth'"));
    assert_eq!(harness.expression_text("height"), "400 mm * 1 mm / width");

    harness.focus(Focus::ParameterValue(height));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.shows("height: There is no parameter named 'wdth'"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
    harness.settle();
    assert!(harness.shows("Up to date"));
    assert_eq!(harness.model.undo_label(), Some("Edit height"));
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
fn closing_with_unsaved_changes_asks_first_and_the_title_marks_them() {
    let mut harness = Harness::new();
    assert_eq!(app::window_title(&harness.model), "Untitled — caditor");
    harness.command(FileCommand::Quit);
    assert!(harness.files.should_quit());

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
    assert!(harness.files.should_quit());
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
fn opening_a_damaged_file_reports_what_was_lost_and_keeps_the_original_on_save() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("damaged.caditor");
    let text = caditor_file::encode(&crate::sample_document().unwrap()).unwrap();
    let damaged: Vec<&str> = text
        .lines()
        .map(|line| {
            if line.contains("Side sketch") {
                "{\"feature\":"
            } else {
                line
            }
        })
        .collect();
    std::fs::write(&path, damaged.join("\n")).unwrap();

    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.command(FileCommand::OpenPath(path.clone()));
    harness.wait_until("the file is open", |harness| harness.model.path().is_some());
    assert!(harness.shows("Parts of “damaged.caditor” could not be read"));
    assert!(harness.shows("• Line 5 is damaged and was left out."));
    assert_eq!(harness.model.document().features().len(), 1);
    harness.click("OK");
    assert!(!harness.shows("Parts of “damaged.caditor” could not be read"));

    harness.command(FileCommand::Save);
    harness.wait_until("the model is saved", |harness| !harness.model.is_saving());
    let backup = dir.path().join("damaged.damaged.caditor");
    assert_eq!(std::fs::read_to_string(backup).unwrap(), damaged.join("\n"));
    assert!(harness.shows("Saved. The damaged original was kept as “damaged.damaged.caditor”."));
    assert!(caditor_file::load(&path).unwrap().issues.is_empty());
}

#[test]
fn unsaved_work_from_a_crash_is_offered_and_restored_with_its_history() {
    let dir = TempDir::new().unwrap();
    let base = crate::sample_document().unwrap();
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
    caditor_file::save(&crate::sample_document().unwrap(), &path, false).unwrap();
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
fn a_constraint_conflict_is_named_and_leads_to_the_newest_constraint() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().clone();
    let mut sketch = base.kind.sketch().unwrap().clone();
    let line = sketch
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let vertical = sketch.add_constraint(Constraint::Vertical(line)).unwrap();
    let replacement = Feature::new(base.id(), base.name.clone(), FeatureKind::Sketch(sketch));
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

    assert!(harness.shows("⚑ Base sketch"));
    assert!(harness.shows(
        "Vertical Line 2 conflicts with Horizontal Line 2 and Distance between Point 0 and \
         Point 1."
    ));
    harness.click("Go to Vertical Line 2");
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
    assert!(harness.shows("Click a plane to sketch on"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.workspace.editing.is_choosing_plane());
    assert!(!harness.shows("Click a plane to sketch on"));

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
    assert_eq!(harness.color_of("H"), Color32::from_rgb(86, 170, 255));

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
    assert_ne!(harness.color_of("H"), harness.error_color());
    assert_ne!(harness.color_of("width = 40 mm"), harness.error_color());
    let line = entities_of_kind(harness.sketch(base), "Line")[0];
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);

    harness.key(Key::V, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert!(harness.shows("Conflicting constraints"));
    for mark in ["H", "V", "width = 40 mm"] {
        assert_eq!(harness.color_of(mark), harness.error_color(), "{mark}");
    }
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
    assert!(harness.shows("Make two lines parallel. Select two lines (Shift+P)"));

    harness.key(Key::P, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Parallel: Select two lines"));
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
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
    assert!(harness.shows(
        "Vertical Line 2 conflicts with Horizontal Line 2 and Distance between Point 0 and \
         Point 1."
    ));

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
    harness.hover("∕ Line");
    assert!(harness.shows("Draw connected lines, one click per corner (L)"));
    harness.click("○ Circle");
    assert_eq!(harness.tool(), Some(Tool::Circle));
    harness.hover("Parallel");
    assert!(harness.shows("Make two lines parallel. Select two lines (Shift+P)"));
}
