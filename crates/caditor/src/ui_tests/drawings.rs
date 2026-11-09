use caditor_document::{Projection, StandardView};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, extruded_plate, run_from_palette};
use crate::drawings_panel::{EXPORT, NONE_YET, WORKING, ZOOM_IN};

fn wait_for_views(harness: &mut Harness) {
    harness.wait_until("the drawing's views are worked out", |harness| {
        harness.workspace.drawings.is_ready() && !harness.shows(WORKING)
    });
}

#[test]
fn a_drawing_is_made_from_the_palette_changed_undoably_and_exported_as_a_pdf_sheet() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "Drawings");
    harness.frame();
    assert!(harness.workspace.drawings.open);
    assert!(harness.shows(NONE_YET));

    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    let drawings = harness.document().drawings().drawings.clone();
    assert_eq!(drawings.len(), 1);
    let drawing = drawings[0].id;
    assert!(harness.shows("Drawing 1"));
    wait_for_views(&mut harness);
    assert!(harness.shows(ZOOM_IN));

    harness.click(Projection::ThirdAngle.name());
    harness.frame();
    let projection = |harness: &Harness| harness.document().drawing(drawing).unwrap().projection;
    assert_eq!(projection(&harness), Projection::ThirdAngle);
    harness.click(StandardView::Left.name());
    harness.frame();
    assert!(
        harness
            .document()
            .drawing(drawing)
            .unwrap()
            .views
            .contains(&StandardView::Left)
    );
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(projection(&harness), Projection::FirstAngle);
    wait_for_views(&mut harness);

    harness.answer_dialog(Some(dir.path().join("plate")));
    harness.click(EXPORT);
    harness.wait_until("the sheet is written", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Exported the drawing “Drawing 1” with 4 views to “plate.pdf”")
        })
    });
    let pdf = std::fs::read(dir.path().join("plate.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();
    assert!(harness.document().drawings().is_empty());
    assert!(harness.shows(NONE_YET));
}
