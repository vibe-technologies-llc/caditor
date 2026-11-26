use caditor_document::{
    Annotation, CentreTarget, DimensionMeasure, DimensionTolerance, DrawnView, Edit, FeatureId,
    FeatureKind, Hole, HoleBottom, HoleDepth, HoleShape, HoleSizing, HoleStyle, StandardView,
    TappedThread, Transaction,
};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers};

use super::{
    Harness, Wheel,
    drawings::{click_sheet, drawn, plan, top_edge_middle, wait_for_views},
    extruded_plate, run_from_palette,
};
use crate::{
    commands::Command,
    drawings_panel::{NO_TOLERANCE, SHEET_IN_VIEW_NOTE, SHOW_DEVIATIONS, TEXT_OFFSET, TOLERANCE},
    sheet_tools::{CALLOUT_PLACE, PLACE_DIMENSION, SECOND_EDGE},
};

const HOLE_CENTRE: Point2 = Point2::new(20.0, 20.0);

fn hole_of(harness: &Harness, plate: FeatureId, diameter: &str) -> Hole {
    let document = harness.document();
    Hole {
        sketch: FeatureId::from_raw(0),
        body: plate,
        diameter: document.parse(diameter).unwrap(),
        depth: HoleDepth::ThroughAll,
        style: HoleStyle::Plain,
        reversed: true,
        shape: HoleShape::Round,
        standard: None,
        sizing: HoleSizing::Typed,
        bottom: HoleBottom::Flat,
        thread: TappedThread::default(),
    }
}

fn drilled_plate_in_view() -> (Harness, FeatureId) {
    let mut harness = Harness::new();

    let (plate, _) = extruded_plate(&mut harness);
    let mut sketch = Sketch::new(Plane::XY);

    sketch.add_point(HOLE_CENTRE);

    let hole = hole_of(&harness, plate, "6.6 mm");
    let mut transaction = harness.document().transaction("Drill");

    let sketch = transaction.add_feature("Hole sketch", FeatureKind::from(sketch));
    let feature = transaction.add_feature("Hole 1", FeatureKind::Hole(Hole { sketch, ..hole }));
    let finished = transaction.finish();

    harness.perform(crate::model::Action::Apply(finished));
    harness.settle();

    assert_eq!(harness.model.evaluation().failed_count(), 0);

    run_from_palette(&mut harness, "New drawing");
    harness.frame();
    wait_for_views(&mut harness);
    run_from_palette(&mut harness, Command::SheetInView.title().as_str());
    harness.frame();
    harness.frame();
    (harness, feature)
}

fn widened(harness: &mut Harness, hole: FeatureId, diameter: &str) {
    let mut changed = harness
        .document()
        .feature(hole)
        .unwrap()
        .kind
        .hole()
        .unwrap()
        .clone();

    changed.diameter = harness.document().parse(diameter).unwrap();

    let transaction = Transaction::single(
        "Widen",
        Edit::SetFeatureKind {
            id: hole,
            kind: FeatureKind::Hole(changed),
        },
    );

    harness.perform(crate::model::Action::Apply(transaction));
    harness.settle();
    wait_for_views(harness);
    harness.frame();
}

fn on_view(harness: &Harness, view: StandardView, point: Point2) -> Point2 {
    plan(harness)
        .placed(DrawnView::Standard(view))
        .unwrap()
        .on_sheet(point)
}

fn callout_text(harness: &Harness) -> Option<String> {
    let callout = drawn(harness).callouts.first().copied()?;
    let plan = plan(harness);

    let laid = plan.annotation(Annotation::Callout(callout.id))?;

    assert_eq!(laid.problem, None);
    Some(laid.content())
}

fn wheel_down(harness: &mut Harness, over: egui::Pos2) {
    harness.events.push(Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -100.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    harness.events.push(Event::PointerMoved(over));
    harness.frame();
    harness.frame();
}

#[test]
fn a_hole_is_called_out_from_the_palette_and_the_callout_follows_a_hole_edit() {
    let (mut harness, hole) = drilled_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingCallout.title().as_str());
    harness.frame();

    assert!(harness.workspace.drawings.canvas().tool().is_some());

    let rim = on_view(
        &harness,
        StandardView::Top,
        HOLE_CENTRE + Vector2::new(3.3, 0.0),
    );

    click_sheet(&mut harness, rim);

    assert!(harness.shows(CALLOUT_PLACE));

    let place = plan(&harness).view_area.min() + Vector2::new(30.0, 30.0);

    click_sheet(&mut harness, place);
    harness.frame();
    harness.frame();

    let callouts = drawn(&harness).callouts;

    assert_eq!(callouts.len(), 1);
    assert_eq!(callouts[0].feature, hole);
    assert_eq!(callout_text(&harness).as_deref(), Some("6.6"));

    widened(&mut harness, hole, "8 mm");

    assert_eq!(callout_text(&harness).as_deref(), Some("8"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert!(drawn(&harness).callouts.is_empty());
}

#[test]
fn centre_marks_are_added_to_the_picked_view_from_the_palette_once_each() {
    let (mut harness, _) = drilled_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingCentreMarks.title().as_str());
    harness.frame();

    assert!(drawn(&harness).centres.is_empty());

    let top = DrawnView::Standard(StandardView::Top);
    let inside_the_hole = on_view(&harness, StandardView::Top, HOLE_CENTRE);

    click_sheet(&mut harness, inside_the_hole);

    assert_eq!(harness.workspace.drawings.canvas().picked(), Some(top));

    run_from_palette(&mut harness, Command::DrawingCentreMarks.title().as_str());
    harness.frame();
    harness.frame();

    let centres = drawn(&harness).centres;

    assert_eq!(centres.len(), 1);
    assert_eq!(centres[0].view, top);
    assert!(matches!(centres[0].target, CentreTarget::Circle(_)));

    let laid = plan(&harness)
        .annotation(Annotation::Centre(centres[0].id))
        .cloned()
        .unwrap();

    assert_eq!(laid.problem, None);
    assert_eq!(laid.marks.len(), 2);

    run_from_palette(&mut harness, Command::DrawingCentreMarks.title().as_str());
    harness.frame();

    assert_eq!(drawn(&harness).centres.len(), 1);
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.starts_with("There is nothing more to mark"))
    );

    let front = DrawnView::Standard(StandardView::Front);
    let front_middle = plan(&harness).placed(front).unwrap().extent.center();

    click_sheet(&mut harness, front_middle);

    assert_eq!(harness.workspace.drawings.canvas().picked(), Some(front));

    run_from_palette(&mut harness, Command::DrawingCentreMarks.title().as_str());
    harness.frame();
    harness.frame();

    let centres = drawn(&harness).centres;

    assert_eq!(centres.len(), 2);
    assert!(matches!(centres[1].target, CentreTarget::Cylinder(_)));
    assert_eq!(centres[1].view, front);
}

#[test]
fn a_dimension_takes_an_iso_fit_from_its_panel_row_and_shows_its_deviations() {
    let (mut harness, _) = drilled_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingDimension.title().as_str());
    harness.frame();

    let edge = top_edge_middle(&harness);

    click_sheet(&mut harness, edge);

    assert!(harness.shows(SECOND_EDGE));

    click_sheet(&mut harness, edge + Vector2::new(2.0, 0.0));

    assert!(harness.shows(PLACE_DIMENSION));

    click_sheet(&mut harness, edge + Vector2::new(0.0, 12.0));
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    let over_the_panel = harness.position_of(SHEET_IN_VIEW_NOTE);

    harness.wheel_until_shown(over_the_panel, TOLERANCE, Wheel::Down);
    harness.click(NO_TOLERANCE);
    harness.frame();
    harness.click("ISO 286 fit");
    harness.frame();
    harness.frame();

    let dimension = drawn(&harness).dimensions[0];
    let laid = plan(&harness)
        .annotation(Annotation::Dimension(dimension.id))
        .cloned()
        .unwrap();

    assert!(matches!(
        dimension.tolerance,
        Some(DimensionTolerance::Fit {
            deviations: false,
            ..
        })
    ));
    assert_eq!(laid.content(), "40 H7");

    harness.wheel_until_shown(over_the_panel, SHOW_DEVIATIONS, Wheel::Down);
    harness.click(SHOW_DEVIATIONS);
    harness.frame();
    harness.frame();

    let laid = plan(&harness)
        .annotation(Annotation::Dimension(dimension.id))
        .cloned()
        .unwrap();

    assert_eq!(laid.content(), "40 H7 +0.025 0");

    let fit_note = "H7 at 40 mm runs from +0.025 to 0 mm";

    harness.wheel_until_shown(over_the_panel, TEXT_OFFSET, Wheel::Down);
    for _ in 0..10 {
        if harness.shows_containing(fit_note) {
            break;
        }
        wheel_down(&mut harness, over_the_panel);
    }

    assert!(harness.shows_containing(fit_note));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.frame();

    assert!(matches!(
        drawn(&harness).dimensions[0].tolerance,
        Some(DimensionTolerance::Fit {
            deviations: false,
            ..
        })
    ));
}

fn dimension_text(harness: &Harness) -> Option<String> {
    let dimension = drawn(harness).dimensions.first().copied()?;
    let plan = plan(harness);

    let laid = plan.annotation(Annotation::Dimension(dimension.id))?;

    assert_eq!(laid.problem, None);
    Some(laid.content())
}

#[test]
fn a_hole_seen_from_the_side_is_dimensioned_across_its_outline_and_follows_a_hole_edit() {
    let (mut harness, hole) = drilled_plate_in_view();

    run_from_palette(&mut harness, Command::DrawingDimension.title().as_str());
    harness.frame();

    let front = plan(&harness)
        .placed(DrawnView::Standard(StandardView::Front))
        .unwrap()
        .extent;
    let outline = |harness: &Harness, x: f64| {
        let on_sheet = on_view(harness, StandardView::Front, Point2::new(x, 0.0));
        Point2::new(on_sheet.x, front.center().y)
    };
    let (left, right) = (outline(&harness, 16.7), outline(&harness, 23.3));

    click_sheet(&mut harness, left);

    assert!(harness.shows(SECOND_EDGE));

    click_sheet(&mut harness, right);

    assert!(harness.shows(PLACE_DIMENSION));

    click_sheet(
        &mut harness,
        Point2::new(front.center().x, front.max().y + 12.0),
    );
    harness.frame();
    harness.frame();

    let dimensions = drawn(&harness).dimensions;

    assert_eq!(dimensions.len(), 1);
    assert!(matches!(
        dimensions[0].measure,
        DimensionMeasure::CylinderDiameter(_)
    ));
    assert_eq!(dimension_text(&harness).as_deref(), Some("\u{2300}6.6"));

    widened(&mut harness, hole, "8 mm");

    assert_eq!(dimension_text(&harness).as_deref(), Some("\u{2300}8"));
}
