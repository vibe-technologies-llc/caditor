use caditor_document::{DrawnView, Projection, SectionPlace, SectionPlane, StandardView};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{Harness, extruded_plate, run_from_palette};
use crate::drawings_panel::{
    ADD_SECTION, AT_OFFSET, CENTRE, DISTANCE, EXPORT, NO_SECTIONS, NONE_YET, WORKING, ZOOM_IN,
    offset_field_id,
};

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

#[test]
fn a_section_view_is_added_in_the_panel_moved_undoably_and_exported_with_the_sheet() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    let drawing = harness.document().drawings().drawings[0].id;
    wait_for_views(&mut harness);
    assert!(harness.shows(NO_SECTIONS));

    harness.click(ADD_SECTION);
    harness.frame();
    let sections = |harness: &Harness| {
        harness
            .document()
            .drawing(drawing)
            .unwrap()
            .sections
            .clone()
    };
    assert_eq!(sections(&harness).len(), 1);
    assert_eq!(sections(&harness)[0].letter, 'A');
    wait_for_views(&mut harness);
    assert!(harness.shows("Section A\u{2013}A"));
    let job = harness.workspace.drawings.job(&harness.model).unwrap();
    assert_eq!(job.views.len(), 5);
    assert_eq!(job.plan.cutting.len(), 1);
    assert!(
        job.plan
            .texts
            .iter()
            .any(|text| text.content == "A\u{2013}A")
    );
    let section = job
        .views
        .iter()
        .find(|(placed, _)| placed.view == DrawnView::Section('A'))
        .unwrap();
    let cutting = section
        .1
        .as_ref()
        .as_ref()
        .unwrap()
        .section
        .clone()
        .unwrap();
    assert!((cutting.cutting.origin().x - 20.0).abs() < 1e-6);
    assert!(!cutting.hatching.is_empty());

    harness.click(AT_OFFSET);
    harness.frame();
    assert_eq!(sections(&harness)[0].place, SectionPlace::Offset(20.0));
    assert!(harness.shows(DISTANCE));
    harness.type_into_field(offset_field_id(drawing, 'A'), "10 mm");
    assert_eq!(sections(&harness)[0].place, SectionPlace::Offset(10.0));
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(sections(&harness)[0].place, SectionPlace::Offset(20.0));
    harness.click(SectionPlane::Xy.name());
    harness.frame();
    assert_eq!(sections(&harness)[0].plane, SectionPlane::Xy);
    wait_for_views(&mut harness);
    assert!(harness.shows_containing(
        "The section view A\u{2013}A could not be drawn: its cutting plane passes through none"
    ));
    harness.click(CENTRE);
    harness.frame();
    assert_eq!(sections(&harness)[0].place, SectionPlace::Centre);
    wait_for_views(&mut harness);

    harness.answer_dialog(Some(dir.path().join("cut")));
    harness.click(EXPORT);
    harness.wait_until("the sheet is written", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Exported the drawing “Drawing 1” with 5 views to “cut.pdf”")
        })
    });

    harness.click_button("Delete section A\u{2013}A");
    harness.frame();
    assert!(sections(&harness).is_empty());
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(sections(&harness).len(), 1);
}
