use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{Document, Editor};
use caditor_expression::ParameterId;
use caditor_file::{JournalEntry, Start, Storage, StorageConfig};
use egui::{
    Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, epaint::ClippedShape,
};
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::{
    app,
    files::{self, Dialogs, FileCommand, Files, FilesConfig, Respond},
    model::{Action, Model, RecomputeStatus, Services, WakerFactory},
    panels::{self, Focus, PanelState},
    selection::Selection,
    toolbar,
};

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1400.0, 1000.0));
const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_SECONDS: f64 = 0.05;
const ANIMATION_FRAMES: usize = 5;
const FILE_TIMEOUT: Duration = Duration::from_secs(10);

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
    state: PanelState,
    selection: Selection,
    events: Vec<Event>,
    texts: Vec<(String, Rect)>,
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
            state: PanelState::default(),
            selection: Selection::default(),
            events: Vec::new(),
            texts: Vec::new(),
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
        let Self {
            context,
            model,
            files,
            state,
            selection,
            ..
        } = self;
        let mut output = context.run_ui(input, |ui| {
            toolbar::show(ui, model, files, &mut actions);
            panels::show(ui, model, selection, state, &mut actions);
            files::show(ui, model, files, &mut actions);
        });
        output.textures_delta.clear();
        app::perform(actions, &mut self.model, &mut self.files);
        self.texts.clear();
        for clipped in output.shapes {
            let ClippedShape { shape, .. } = clipped;
            collect_texts(shape, &mut self.texts);
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
        app::perform(
            vec![Action::File(command)],
            &mut self.model,
            &mut self.files,
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
        self.state.request_focus(focus);
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

    fn click(&mut self, label: &str) {
        let (_, rect) = self
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("'{label}' is not on screen"))
            .clone();
        let position = rect.center();
        self.events.push(Event::PointerMoved(position));
        self.frame();
        for pressed in [true, false] {
            self.events.push(Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.frame();
        }
        self.show_new_windows();
    }
}

fn collect_texts(shape: Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            texts.push((text.galley.text().to_owned(), rect));
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_texts(shape, texts);
            }
        }
        _ => {}
    }
}

#[test]
fn editing_parameters_breaking_a_feature_and_undoing_it_works_through_the_panels() {
    let mut harness = Harness::new();
    assert!(harness.shows("✔ Up to date"));
    let width = harness.parameter("width");
    let height = harness.parameter("height");

    harness.type_into(Focus::ParameterValue(height), "400 mm * 1 mm / width");
    assert_eq!(harness.expression_text("height"), "400 mm * 1 mm / width");
    harness.settle();
    assert!(harness.shows("10 mm"));

    harness.type_into(Focus::ParameterValue(width), "0 mm");
    harness.settle();
    assert!(harness.shows("⚠ 1 feature failed"));
    assert!(harness.shows("⚠ Side sketch"));
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
    assert!(harness.shows("✔ Up to date"));
    assert_eq!(harness.model.undo_label(), Some("Edit height"));
}

#[test]
fn a_dimension_edited_in_the_tree_is_undoable_and_rejects_the_wrong_kind() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let caditor_document::FeatureKind::Sketch(sketch) =
        &harness.document().feature(base).unwrap().kind;
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
