use caditor_document::{
    Annotation, AnnotationId, DetailParent, DetailScale, DimensionMeasure, DrawingDimension,
    DrawnView, EdgePoint, EdgeSpot, MAX_DRAWINGS, Projection, ResolvedPlace, ScaleRatio,
    SectionPlace, SectionPlane, SheetPlan, StandardView,
};
use caditor_expression::Expression;
use caditor_geometry::{Point2, Vector2};
use caditor_kernel::EdgeName;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use tempfile::TempDir;

use super::{Harness, Wheel, drag_screen, extruded_plate, run_from_palette};
use crate::{
    commands::Command,
    drawings_panel::{
        ADD_DETAIL, ADD_SECTION, AT_OFFSET, CENTRE, DISTANCE, EXPORT, LAID_OUT, LEAVE_SHEET_VIEW,
        NEW, NO_ANNOTATIONS, NO_DETAILS, NO_SECTIONS, NONE_YET, RENAME, SHEET_IN_VIEW_NOTE,
        SHEET_TO_PANEL, SHEET_TO_VIEW, TEXT_OFFSET, WORKING, detail_field_id, offset_field_id,
    },
    model::Action,
    sheet_canvas::{MOVE_DOWN, MOVE_LEFT, RESET_POSITION, ZOOM_IN},
    sheet_tools::{DIMENSION_TOOL, NOTE_FIELD, PLACE_DIMENSION, PLACE_NOTE, SECOND_EDGE},
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
fn the_sheet_export_proposes_the_format_the_last_sheet_was_written_in() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    wait_for_views(&mut harness);

    harness.answer_dialog(Some(dir.path().join("first.svg")));
    harness.click(EXPORT);
    harness.wait_until("the SVG sheet is written", |harness| {
        dir.path().join("first.svg").exists() && !harness.files.is_blocking()
    });

    let first_proposed = harness.last_proposed_name();

    harness.answer_dialog(Some(dir.path().join("second")));
    harness.click(EXPORT);
    harness.wait_until("the second sheet is written", |harness| {
        dir.path().join("second.svg").exists() && !harness.files.is_blocking()
    });

    assert_eq!(first_proposed.as_deref(), Some("Drawing 1.pdf"));
    assert_eq!(
        harness.last_proposed_name().as_deref(),
        Some("Drawing 1.svg")
    );
    assert!(!dir.path().join("second.pdf").exists());
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
    let place = |harness: &Harness| {
        sections(harness)[0]
            .place
            .resolve(harness.model.parameters())
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

    assert_eq!(place(&harness), Ok(ResolvedPlace::Offset(20.0)));
    assert!(harness.shows(DISTANCE));

    harness.type_into_field(offset_field_id(drawing, 'A'), "10 mm");

    assert_eq!(place(&harness), Ok(ResolvedPlace::Offset(10.0)));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(place(&harness), Ok(ResolvedPlace::Offset(20.0)));

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

#[test]
fn a_section_offset_that_does_not_read_is_told_while_it_is_typed() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();

    let drawing = harness.document().drawings().drawings[0].id;

    wait_for_views(&mut harness);
    harness.click(ADD_SECTION);
    harness.frame();
    harness.click(AT_OFFSET);
    harness.frame();

    let document = harness.document();
    let unknown = document.parse("nothere").unwrap_err().to_string();
    let unfinished = document.parse("2 *").unwrap_err().to_string();

    harness.draft_into_field(offset_field_id(drawing, 'A'), "nothere");

    assert!(harness.shows(&unknown));

    harness.draft_into_field(offset_field_id(drawing, 'A'), "2 *");

    assert!(harness.shows(&unfinished));
    assert!(!harness.shows(&unknown));

    harness.draft_into_field(offset_field_id(drawing, 'A'), "5 mm");

    assert!(!harness.shows(&unfinished));
}

#[test]
fn a_section_offset_typed_as_a_parameter_follows_it_when_it_changes() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();

    let drawing = harness.document().drawings().drawings[0].id;

    wait_for_views(&mut harness);
    harness.click(ADD_SECTION);
    harness.frame();
    harness.click(AT_OFFSET);
    harness.frame();

    let document = harness.document();
    let mut transaction = document.transaction("Add cut");

    let cut = transaction.add_parameter("cut", Expression::number(12.0));

    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.type_into_field(offset_field_id(drawing, 'A'), "cut");

    let cut_at = |harness: &Harness| {
        let job = harness.workspace.drawings.job(&harness.model).ok()?;
        job.plan
            .views
            .iter()
            .find(|placed| placed.view == DrawnView::Section('A'))
            .and_then(|placed| job.views.iter().find(|(view, _)| view == placed))
            .and_then(|(_, outcome)| outcome.as_ref().as_ref().ok()?.section.clone())
            .map(|section| section.cutting.origin().x)
    };

    assert!(matches!(
        harness.document().drawing(drawing).unwrap().sections[0].place,
        SectionPlace::Offset(Expression::Parameter(id)) if id == cut
    ));

    harness.wait_until("the section is cut at the parameter", |harness| {
        cut_at(harness).is_some_and(|x| (x - 12.0).abs() < 1e-6)
    });

    let document = harness.document();
    let expression = document.parse("15 mm").unwrap();

    harness.perform(Action::Apply(caditor_document::Transaction::single(
        "Move cut",
        caditor_document::Edit::SetParameterExpression {
            id: cut,
            expression,
        },
    )));
    harness.settle();

    harness.wait_until("the section follows the parameter", |harness| {
        cut_at(harness).is_some_and(|x| (x - 15.0).abs() < 1e-6)
    });
}

#[test]
fn a_detail_view_is_added_in_the_panel_edited_undoably_clipped_to_its_circle_and_exported() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();

    let drawing = harness.document().drawings().drawings[0].id;

    wait_for_views(&mut harness);

    assert!(harness.shows(NO_DETAILS));

    harness.click(ADD_DETAIL);
    harness.frame();

    let details = |harness: &Harness| harness.document().drawing(drawing).unwrap().details.clone();

    assert_eq!(details(&harness).len(), 1);

    let detail = details(&harness)[0];

    assert_eq!(detail.letter, 'A');
    assert_eq!(detail.parent, DetailParent::Standard(StandardView::Front));
    assert_eq!(detail.scale, DetailScale::Enlarged);

    wait_for_views(&mut harness);

    assert!(harness.shows("Detail A"));

    let job = harness.workspace.drawings.job(&harness.model).unwrap();

    assert_eq!(job.views.len(), 5);

    let label = detail.label_at(job.plan.scale.factor());

    assert!(job.plan.texts.iter().any(|text| text.content == label));
    assert!(job.plan.texts.iter().any(|text| text.content == "A"));

    let (placed, outcome) = job
        .views
        .iter()
        .find(|(placed, _)| placed.view == DrawnView::Detail('A'))
        .unwrap();
    let lines = outcome.as_ref().as_ref().unwrap();
    let disc = lines.clip.unwrap();

    assert!(!lines.pieces.is_empty());
    assert!((placed.scale - detail.scale.ratio_at(job.plan.scale.factor()).factor()).abs() < 1e-9);

    for piece in &lines.pieces {
        for at in [piece.range.start(), piece.range.end()] {
            assert!((piece.curve.point(at) - disc.centre).length() <= disc.radius + 1e-6);
        }
    }

    harness.type_into_field(detail_field_id(drawing, 'A', "radius"), "2 mm");

    assert_eq!(details(&harness)[0].radius, 2.0);

    wait_for_views(&mut harness);
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(details(&harness)[0].radius, detail.radius);

    harness.key(Key::Y, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(details(&harness)[0].radius, 2.0);

    wait_for_views(&mut harness);

    harness.click(&DetailScale::Enlarged.label());
    harness.frame();
    harness.click(&ScaleRatio::new(10, 1).label());
    harness.frame();

    assert_eq!(
        details(&harness)[0].scale,
        DetailScale::Ratio(ScaleRatio::new(10, 1))
    );

    wait_for_views(&mut harness);

    harness.answer_dialog(Some(dir.path().join("detail")));
    harness.click(EXPORT);
    harness.wait_until("the sheet is written", |harness| {
        harness.model.notice().is_some_and(|notice| {
            notice
                .text
                .starts_with("Exported the drawing “Drawing 1” with 5 views to “detail.pdf”")
        })
    });

    harness.click_button("Delete detail A");
    harness.frame();

    assert!(details(&harness).is_empty());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(details(&harness).len(), 1);
}

#[test]
fn renaming_selects_the_name_keeps_a_refused_one_and_the_sheet_reads_its_zoom() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "Drawings");
    harness.frame();
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    wait_for_views(&mut harness);

    harness.click(RENAME);
    harness.frame();
    harness.type_text("drawing 2");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    let names: Vec<String> = harness
        .document()
        .drawings()
        .drawings
        .iter()
        .map(|drawing| drawing.name.clone())
        .collect();

    assert_eq!(names, ["Drawing 1", "Drawing 2"]);
    assert!(harness.shows("drawing 2"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click(ZOOM_IN);
    harness.frame();

    assert!(harness.shows("125%"));

    for _ in names.len()..MAX_DRAWINGS {
        let (transaction, _) = harness.document().adding_drawing(Vec::new()).unwrap();

        harness.perform(Action::Apply(transaction));
    }
    harness.frame();
    harness.read_screen();

    assert_eq!(harness.document().drawings().drawings.len(), MAX_DRAWINGS);
    assert_eq!(harness.button_enabled(NEW), Some(false));
}

fn drawn_plate_in_view() -> Harness {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    wait_for_views(&mut harness);
    run_from_palette(&mut harness, Command::SheetInView.title().as_str());
    harness.frame();
    harness.frame();
    harness
}

fn plan(harness: &Harness) -> SheetPlan {
    harness
        .workspace
        .drawings
        .plan(&harness.model)
        .expect("a drawing is shown")
}

fn centre_of(harness: &Harness, view: DrawnView) -> Point2 {
    plan(harness).placed(view).unwrap().extent.center()
}

fn on_screen(harness: &Harness, point: Point2) -> Pos2 {
    harness
        .workspace
        .drawings
        .canvas()
        .screen_of(point)
        .expect("the sheet has been drawn")
}

fn screen_centre(harness: &Harness, view: DrawnView) -> Pos2 {
    on_screen(harness, centre_of(harness, view))
}

fn drag_by(harness: &mut Harness, from: Pos2, by: egui::Vec2) {
    drag_screen(harness, from, from + by);
    harness.frame();
}

fn positions(harness: &Harness) -> Vec<DrawnView> {
    harness.document().drawings().drawings[0]
        .positions
        .keys()
        .copied()
        .collect()
}

fn near(left: Vector2, right: Vector2, within: f64) -> bool {
    (left - right).length() <= within
}

fn scroll_to_the_preview(harness: &mut Harness) {
    let over_the_panel = harness.position_of(NO_SECTIONS);

    harness.events.push(Event::PointerMoved(over_the_panel));
    harness.frame();
    for _ in 0..40 {
        harness.events.push(Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.settle();
}

fn press_with(harness: &mut Harness, at: Pos2, button: PointerButton) {
    harness.events.push(Event::PointerMoved(at));
    harness.frame();
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.frame();
}

#[test]
fn the_sheet_is_shown_across_the_view_and_escape_or_its_buttons_bring_the_model_back() {
    let mut harness = drawn_plate_in_view();

    let area = harness.workspace.viewport.rect().unwrap();
    let in_view = plan(&harness);
    let front = DrawnView::Standard(StandardView::Front);
    let sheet_corner = on_screen(&harness, Point2::new(in_view.width, in_view.height));

    assert!(harness.workspace.drawings.sheet_in_view());
    assert!(harness.shows(LEAVE_SHEET_VIEW));
    assert!(harness.shows(SHEET_IN_VIEW_NOTE));
    assert!(area.contains(screen_centre(&harness, front)));
    assert!(sheet_corner.x - on_screen(&harness, Point2::ZERO).x > area.width() * 0.5);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(!harness.workspace.drawings.sheet_in_view());
    assert!(harness.workspace.drawings.open);

    scroll_to_the_preview(&mut harness);

    assert!(harness.shows(SHEET_TO_VIEW));

    harness.click_button(SHEET_TO_VIEW);
    harness.frame();

    assert!(harness.workspace.drawings.sheet_in_view());

    harness.click_button(SHEET_TO_PANEL);
    harness.frame();

    assert!(!harness.workspace.drawings.sheet_in_view());

    run_from_palette(&mut harness, Command::SheetInView.title().as_str());
    harness.frame();
    harness.click_button(LEAVE_SHEET_VIEW);
    harness.frame();

    assert!(!harness.workspace.drawings.sheet_in_view());
}

#[test]
fn a_view_dragged_on_the_sheet_moves_undoably_and_a_projected_one_only_along_its_line() {
    let mut harness = drawn_plate_in_view();

    let isometric = DrawnView::Standard(StandardView::Isometric);
    let front = DrawnView::Standard(StandardView::Front);
    let top = DrawnView::Standard(StandardView::Top);
    let before = plan(&harness);
    let placed = |plan: &SheetPlan, view| *plan.placed(view).unwrap();

    assert!(harness.shows(LAID_OUT));

    let from = screen_centre(&harness, isometric);

    drag_by(&mut harness, from, vec2(-60.0, 40.0));

    let moved = plan(&harness);
    let shift = placed(&moved, isometric).origin - placed(&before, isometric).origin;

    assert_eq!(positions(&harness), [isometric]);
    assert!(shift.x < -1.0 && shift.y < -1.0, "{shift:?}");
    assert_eq!(placed(&moved, front), placed(&before, front));
    assert!(harness.shows("Isometric view, placed by hand"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert!(positions(&harness).is_empty());

    let from = screen_centre(&harness, top);

    drag_by(&mut harness, from, vec2(50.0, 30.0));

    let dropped = plan(&harness);
    let top_shift = placed(&dropped, top).origin - placed(&before, top).origin;

    assert_eq!(positions(&harness), [top]);
    assert!(near(Vector2::new(top_shift.x, 0.0), Vector2::ZERO, 1e-9));
    assert!(top_shift.y < -1.0);
    assert!((placed(&dropped, front).origin.x - placed(&dropped, top).origin.x).abs() < 1e-9);

    let from = screen_centre(&harness, front);

    drag_by(&mut harness, from, vec2(30.0, 0.0));

    let carried = plan(&harness);
    let front_shift = placed(&carried, front).origin - placed(&dropped, front).origin;
    let top_followed = placed(&carried, top).origin - placed(&dropped, top).origin;

    assert!(front_shift.x > 1.0);
    assert!(near(top_followed, front_shift, 1e-6));

    harness.click_button("Reset the position of front view");
    harness.frame();
    harness.frame();

    assert_eq!(positions(&harness), [top]);
    assert_eq!(placed(&plan(&harness), front), placed(&before, front));
}

#[test]
fn a_picked_view_is_nudged_by_the_arrow_keys_and_reset_from_its_menu() {
    let mut harness = drawn_plate_in_view();

    let isometric = DrawnView::Standard(StandardView::Isometric);
    let before = centre_of(&harness, isometric);
    let at = screen_centre(&harness, isometric);

    press_with(&mut harness, at, PointerButton::Primary);

    assert_eq!(
        harness.workspace.drawings.canvas().picked(),
        Some(isometric)
    );

    harness.key(Key::ArrowRight, Modifiers::NONE);
    harness.frame();
    harness.key(Key::ArrowUp, Modifiers::SHIFT);
    harness.frame();
    harness.frame();

    let nudged = centre_of(&harness, isometric) - before;

    assert!(
        near(nudged, Vector2::new(1.0, 10.0), 1e-6),
        "{nudged:?} {:?}",
        positions(&harness)
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert_eq!(harness.workspace.drawings.canvas().picked(), None);
    assert!(harness.workspace.drawings.sheet_in_view());

    harness.key(Key::ArrowRight, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(near(
        centre_of(&harness, isometric) - before,
        Vector2::new(1.0, 10.0),
        1e-6
    ));

    harness.read_screen();

    assert_eq!(harness.button_enabled(MOVE_LEFT), Some(false));

    let top = DrawnView::Standard(StandardView::Top);
    let top_before = centre_of(&harness, top);
    let at = screen_centre(&harness, top);

    press_with(&mut harness, at, PointerButton::Primary);
    harness.read_screen();

    assert_eq!(harness.button_enabled(MOVE_LEFT), Some(false));
    assert_eq!(harness.button_enabled(MOVE_DOWN), Some(true));

    harness.click_button(MOVE_DOWN);
    harness.frame();
    harness.frame();

    assert!(near(
        centre_of(&harness, top) - top_before,
        Vector2::new(0.0, -10.0),
        1e-6
    ));

    let at = screen_centre(&harness, isometric);

    press_with(&mut harness, at, PointerButton::Secondary);
    harness.read_screen();
    harness.click_button(RESET_POSITION);
    harness.frame();
    harness.frame();

    assert_eq!(positions(&harness), [top]);
    assert!(near(
        centre_of(&harness, isometric) - before,
        Vector2::ZERO,
        1e-9
    ));
}

#[test]
fn a_detail_circle_is_dragged_and_its_rim_resized_undoably_on_the_sheet() {
    let mut harness = drawn_plate_in_view();

    harness.click(ADD_DETAIL);
    harness.frame();
    wait_for_views(&mut harness);
    harness.frame();

    let detail = |harness: &Harness| harness.document().drawings().drawings[0].details[0];
    let start = detail(&harness);
    let parent = *plan(&harness).placed(start.parent.drawn()).unwrap();

    let centre = parent.on_sheet(start.centre);
    let rim = parent.on_sheet(start.centre + Vector2::new(start.radius, 0.0));

    let inside = on_screen(&harness, centre);

    drag_by(&mut harness, inside, vec2(20.0, -10.0));

    let moved = detail(&harness);

    assert!(moved.centre.x > start.centre.x && moved.centre.y > start.centre.y);
    assert_eq!(moved.radius, start.radius);
    assert!(positions(&harness).is_empty());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert_eq!(detail(&harness), start);

    let edge = on_screen(&harness, rim);

    drag_by(&mut harness, edge, vec2(25.0, 0.0));

    let resized = detail(&harness);

    assert!(resized.radius > start.radius);
    assert_eq!(resized.centre, start.centre);

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();

    assert_eq!(detail(&harness), start);
}

#[test]
fn a_view_is_dragged_in_the_panel_preview_too() {
    let mut harness = Harness::new();

    extruded_plate(&mut harness);
    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    wait_for_views(&mut harness);

    let isometric = DrawnView::Standard(StandardView::Isometric);

    scroll_to_the_preview(&mut harness);

    let before = centre_of(&harness, isometric);
    let from = screen_centre(&harness, isometric);

    assert!(harness.workspace.drawings.open);
    assert!(!harness.workspace.drawings.sheet_in_view());

    drag_by(&mut harness, from, vec2(-20.0, 0.0));

    assert_eq!(positions(&harness), [isometric]);
    assert!(centre_of(&harness, isometric).x < before.x);
}

fn top_edge_middle(harness: &Harness) -> Point2 {
    let front = plan(harness)
        .placed(DrawnView::Standard(StandardView::Front))
        .unwrap()
        .extent;
    Point2::new(front.center().x, front.max().y)
}

fn click_sheet(harness: &mut Harness, at: Point2) {
    let on = on_screen(harness, at);

    press_with(harness, on, PointerButton::Primary);
}

fn drawn(harness: &Harness) -> caditor_document::Drawing {
    harness.document().drawings().drawings[0].clone()
}

#[test]
fn a_dimension_is_added_by_two_edge_clicks_and_a_place_then_nudged_deleted_and_undone() {
    let mut harness = drawn_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingDimension.title().as_str());
    harness.frame();

    assert!(harness.workspace.drawings.canvas().tool().is_some());

    let edge = top_edge_middle(&harness);

    click_sheet(&mut harness, edge);

    assert!(harness.shows(SECOND_EDGE));

    click_sheet(&mut harness, edge + Vector2::new(2.0, 0.0));

    assert!(harness.shows(PLACE_DIMENSION));

    click_sheet(&mut harness, edge + Vector2::new(0.0, 12.0));
    harness.frame();
    harness.frame();

    let dimensions = drawn(&harness).dimensions;
    let id = AnnotationId::from_raw(0);
    let laid = plan(&harness)
        .annotation(Annotation::Dimension(id))
        .cloned()
        .unwrap();

    assert_eq!(dimensions.len(), 1);
    assert!(matches!(
        dimensions[0].measure,
        DimensionMeasure::Linear {
            from: EdgePoint {
                spot: EdgeSpot::Start,
                ..
            },
            to: EdgePoint {
                spot: EdgeSpot::End,
                ..
            },
            ..
        }
    ));
    assert_eq!(laid.text.content, "40");
    assert_eq!(laid.problem, None);
    assert!(laid.text.at.y > edge.y);
    assert_eq!(
        harness.workspace.drawings.canvas().picked_annotation(),
        Some(Annotation::Dimension(id))
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    assert!(harness.workspace.drawings.canvas().tool().is_none());
    assert!(harness.workspace.drawings.sheet_in_view());

    let at = on_screen(&harness, laid.text.at);

    press_with(&mut harness, at, PointerButton::Primary);

    let before = drawn(&harness).dimensions[0].place;

    harness.key(Key::ArrowUp, Modifiers::SHIFT);
    harness.frame();
    harness.frame();

    assert!(drawn(&harness).dimensions[0].place.y > before.y);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(drawn(&harness).dimensions.is_empty());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert_eq!(drawn(&harness).dimensions.len(), 1);
}

#[test]
fn a_note_is_placed_typed_and_pointed_at_an_edge() {
    let mut harness = drawn_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingNote.title().as_str());
    harness.frame();

    let corner = plan(&harness).view_area.min() + Vector2::new(30.0, 30.0);

    click_sheet(&mut harness, corner);
    harness.frame();

    harness.read_screen();

    assert!(harness.has_button(PLACE_NOTE));

    harness.type_text("Break all edges");

    let edge = top_edge_middle(&harness);

    click_sheet(&mut harness, edge);
    harness.click_button(PLACE_NOTE);
    harness.frame();
    harness.frame();

    harness.read_screen();

    let notes = drawn(&harness).notes;

    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "Break all edges");
    assert!(notes[0].leader.is_some());
    assert!(!harness.has_button(PLACE_NOTE));
    assert!(!harness.shows(NOTE_FIELD));

    let laid = plan(&harness)
        .annotation(Annotation::Note(notes[0].id))
        .cloned()
        .unwrap();

    assert!(!laid.marks.is_empty());
    assert_eq!(laid.problem, None);
}

#[test]
fn a_dimension_whose_edge_is_gone_says_why_in_the_panel_and_is_deleted_there() {
    let mut harness = drawn_plate_in_view();

    let id = drawn(&harness).id;
    let lost = EdgeName::from_digest(0xdead_beef);
    let change = harness
        .document()
        .changing_drawing(id, "Add dimension", |drawing| {
            let id = drawing.new_annotation_id();

            drawing.dimensions.push(DrawingDimension {
                id,
                view: DrawnView::Standard(StandardView::Front),
                measure: DimensionMeasure::Diameter(lost),
                place: Point2::new(5.0, 5.0),
                decimals: 2,
                text_offset: 0.0,
            });
        })
        .unwrap();

    harness.perform(Action::Apply(change));
    harness.frame();
    harness.frame();

    let laid = plan(&harness)
        .annotation(Annotation::Dimension(AnnotationId::from_raw(0)))
        .cloned()
        .unwrap();

    assert!(laid.problem.is_some());
    assert_eq!(laid.text.content, "\u{2300}?");

    let over_the_panel = harness.position_of(SHEET_IN_VIEW_NOTE);

    harness.wheel_until_shown(over_the_panel, TEXT_OFFSET, Wheel::Down);

    harness.read_screen();

    assert!(harness.shows_containing("Dimension 1 lost what it measures"));
    assert!(harness.has_button("Re-attach dimension 1"));

    harness.click_button("Delete dimension 1");
    harness.frame();
    harness.frame();

    harness.read_screen();

    assert!(drawn(&harness).dimensions.is_empty());
    assert!(harness.shows(NO_ANNOTATIONS));
    assert!(harness.has_button(DIMENSION_TOOL));
}
