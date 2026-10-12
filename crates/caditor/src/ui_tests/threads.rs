use std::fs;

use caditor_document::{
    FeatureId, FeatureKind, FeatureResult, HoleFit, HoleStandard, MetricSize, Thread, ThreadForm,
    ThreadLength,
};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, extruded_plate, extruded_shaft, open_hole, run_from_palette, thread_lines, thread_of,
};
use crate::{
    export::ExportCommand,
    files::FileCommand,
    hole_panel,
    model::{Action, THREADS_COSMETIC_OFF, THREADS_COSMETIC_ON},
    thread_panel, thread_tools,
};

const SWITCH: &str = "Show modelled threads as cosmetic";

fn notice_starts(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.starts_with(text))
}

fn threaded_shaft(harness: &mut Harness) -> FeatureId {
    let side = extruded_shaft(harness);
    harness.select([side]);
    harness.use_tool_with(Key::O, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let thread = harness
        .workspace
        .editing
        .solid()
        .expect("the thread is open");
    let depth = harness.document().parse("2 mm").unwrap();
    let short = Thread {
        length: ThreadLength::Depth(depth),
        ..thread_of(harness, thread).clone()
    };
    let change = thread_tools::edit(harness.document(), thread, short).unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();
    thread
}

fn close_feature(harness: &mut Harness) {
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
}

fn faces_cut_by(harness: &Harness, thread: FeatureId) -> usize {
    let body = thread_of(harness, thread).body;
    harness.model.evaluation().body(body).map_or(0, |solid| {
        solid
            .faces()
            .filter_map(|(_, face)| face.origin())
            .filter(|origin| origin.feature() == thread.raw())
            .count()
    })
}

#[test]
fn a_new_thread_is_modelled_and_the_switch_draws_it_cosmetic_until_turned_off() {
    let mut harness = Harness::new();
    run_from_palette(&mut harness, SWITCH);
    harness.settle();
    let switched_on = harness.model.threads_shown_cosmetic();
    let told_on = notice_starts(&harness, THREADS_COSMETIC_ON);

    let thread = threaded_shaft(&mut harness);
    let form = thread_of(&harness, thread).form;
    let says_shown_cosmetic = harness.shows(thread_panel::SHOWN_COSMETIC);
    close_feature(&mut harness);
    let drawn_while_cosmetic = !thread_lines(&mut harness).is_empty();
    let cut_while_cosmetic = faces_cut_by(&harness, thread);

    run_from_palette(&mut harness, SWITCH);
    harness.settle();
    let status = harness.model.evaluation().feature(thread).unwrap();
    let cut = status
        .result
        .as_deref()
        .and_then(FeatureResult::solid)
        .is_some();

    assert!(switched_on && told_on);
    assert_eq!(form, ThreadForm::Modelled);
    assert!(says_shown_cosmetic);
    assert!(drawn_while_cosmetic);
    assert_eq!(cut_while_cosmetic, 0);
    assert!(!harness.model.threads_shown_cosmetic());
    assert!(notice_starts(&harness, THREADS_COSMETIC_OFF));
    assert!(cut, "{:?}", status.state);
    assert!(faces_cut_by(&harness, thread) > 4);
    assert!(thread_lines(&mut harness).is_empty());
}

#[test]
fn the_form_row_of_a_thread_and_of_a_tapped_hole_is_one_undoable_change() {
    let mut harness = Harness::new();
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let thread = threaded_shaft(&mut harness);

    harness.click("Cosmetic");
    harness.settle();
    let made_cosmetic = thread_of(&harness, thread).form;
    let label = harness.model.undo_label().map(str::to_owned);
    let shown_note = harness.shows(thread_panel::SHOWN_COSMETIC);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    let undone = thread_of(&harness, thread).form;

    assert_eq!(made_cosmetic, ThreadForm::Cosmetic);
    assert!(label.is_some_and(|label| label.starts_with("Edit Thread on")));
    assert!(!shown_note);
    assert_eq!(undone, ThreadForm::Modelled);

    close_feature(&mut harness);
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");
    let tapped = crate::hole_tools::with_standard(
        &open_hole(&harness, hole),
        HoleStandard {
            size: MetricSize::M6,
            fit: HoleFit::Tapped,
        },
        crate::units::LengthUnit::Millimetre,
    );
    let change = crate::hole_tools::edit(harness.document(), hole, tapped).unwrap();
    harness.perform(Action::Apply(change));
    harness.settle();
    let tapped_form = open_hole(&harness, hole).thread.form;
    let offered = harness.shows(hole_panel::THREAD_FORM);

    harness.click("Cosmetic");
    harness.settle();
    let hole_cosmetic = open_hole(&harness, hole).thread.form;
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();

    assert_eq!(tapped_form, ThreadForm::Modelled);
    assert!(offered);
    assert_eq!(hole_cosmetic, ThreadForm::Cosmetic);
    assert_eq!(open_hole(&harness, hole).thread.form, ThreadForm::Modelled);
}

#[test]
fn an_export_cuts_modelled_threads_shown_as_cosmetic_unless_told_as_shown() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.perform(Action::ShowThreadsAsCosmetic(true));
    let thread = threaded_shaft(&mut harness);
    close_feature(&mut harness);
    let export = |harness: &mut Harness, name: &str| {
        harness.command(FileCommand::Export(ExportCommand::Show));
        harness.click("STEP");
        harness.answer_dialog(Some(dir.path().join(name)));
        harness.click("Export…");
        let written = format!("Exported 1 body to “{name}.step”");
        harness.wait_until("the export is written", |harness| {
            notice_starts(harness, &written)
        });
        let text = fs::read_to_string(dir.path().join(format!("{name}.step"))).unwrap();
        text.matches("ADVANCED_FACE").count()
    };

    let modelled_faces = export(&mut harness, "modelled");
    harness.command(FileCommand::Export(ExportCommand::Show));
    let offered = harness.shows("Threads") && harness.shows("As shown");
    harness.command(FileCommand::Export(ExportCommand::Hide));
    harness.command(FileCommand::Export(ExportCommand::SetThreads(
        crate::export::ExportThreads::AsShown,
    )));
    let shown_faces = export(&mut harness, "shown");

    assert!(offered);
    assert_eq!(faces_cut_by(&harness, thread), 0);
    assert!(
        modelled_faces > shown_faces + 4,
        "{modelled_faces} faces modelled, {shown_faces} as shown"
    );
    assert!(matches!(
        harness
            .document()
            .feature(thread)
            .map(|feature| &feature.kind),
        Some(FeatureKind::Thread(_))
    ));
}
