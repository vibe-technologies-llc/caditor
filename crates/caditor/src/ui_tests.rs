use std::time::{Duration, Instant};

use caditor_document::Document;
use caditor_expression::ParameterId;
use egui::{
    Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, epaint::ClippedShape,
};

use crate::{
    model::{Model, RecomputeStatus},
    panels::{self, Focus, PanelState},
    selection::Selection,
    toolbar,
};

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1400.0, 1000.0));
const RECOMPUTE_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_SECONDS: f64 = 0.05;
const ANIMATION_FRAMES: usize = 5;

struct Harness {
    context: egui::Context,
    model: Model,
    state: PanelState,
    selection: Selection,
    events: Vec<Event>,
    texts: Vec<(String, Rect)>,
    time: f64,
}

impl Harness {
    fn new() -> Self {
        let document = crate::sample_document().unwrap();
        let mut harness = Self {
            context: egui::Context::default(),
            model: Model::new(document, Box::new(|| Box::new(|| {}))),
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
        let Self {
            context,
            model,
            state,
            selection,
            ..
        } = self;
        let mut output = context.run_ui(input, |ui| {
            toolbar::show(ui, model, &mut actions);
            panels::show(ui, model, selection, state, &mut actions);
        });
        output.textures_delta.clear();
        for action in actions {
            self.model.perform(action);
        }
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
