use caditor_document::{
    BlendKind, Edit, FeatureId, FeatureKind, PrimitiveShape, RevolveExtent, SolidFeature,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, click_with, distance_of, extruded_plate, primitive_of, rectangle,
    run_from_palette,
};
use crate::{
    feature_values::{PrimitiveSize, ValueSlot},
    model::Action,
    model_dimensions::{self, ShownShape},
    move_manipulator::Reach,
    selection::Pickable,
};

const WIDTH_LABEL: &str = "width = 40 mm";
const HEIGHT_LABEL: &str = "height = 20 mm";
const DISTANCE_LABEL: &str = "Distance 10 mm";
const HIGHLIGHT_STEPS: usize = 200;
const PAST_DOUBLE_CLICK_FRAMES: usize = 10;

fn show_dimensions(harness: &mut Harness) {
    run_from_palette(harness, "dimensions on the model");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    assert!(harness.workspace.viewport.dimensions_shown());
}

fn plate_with_distance_shown(harness: &mut Harness) -> FeatureId {
    let (extrude, top) = extruded_plate(harness);
    run_from_palette(harness, "fit view");
    show_dimensions(harness);
    harness.select([top]);
    harness.frame();
    harness.frame();
    extrude
}

fn distance_value(extrude: FeatureId) -> Pickable {
    Pickable::FeatureValue {
        feature: extrude,
        value: ValueSlot::Extrude(Reach::Only),
    }
}

fn enter(harness: &mut Harness, text: &str) {
    harness.events.push(Event::Text(text.to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
}

#[test]
fn show_dimensions_puts_the_shown_sketches_dimensions_on_the_model_and_edits_their_parameter() {
    let mut harness = Harness::new();
    harness.settle();
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    assert!(!harness.shows(WIDTH_LABEL));

    show_dimensions(&mut harness);
    assert!(harness.shows(WIDTH_LABEL));
    assert!(harness.shows(HEIGHT_LABEL));

    harness.double_click(WIDTH_LABEL);
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "width = 50 mm");
    assert_eq!(harness.expression_text("width"), "50 mm");
    assert!(harness.shows("width = 50 mm"));
    assert!(harness.shows("height = 25 mm"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert_eq!(harness.expression_text("width"), "40 mm");

    run_from_palette(&mut harness, "dimensions on the model");
    harness.frame();
    assert!(!harness.workspace.viewport.dimensions_shown());
    assert!(!harness.shows(WIDTH_LABEL));
}

#[test]
fn a_selected_extrusion_shows_its_distance_on_the_model_and_a_double_click_changes_it_in_one_step()
{
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    assert!(harness.shows(DISTANCE_LABEL));
    assert!(
        harness
            .workspace
            .viewport
            .model_labels()
            .contains(&distance_value(extrude))
    );

    harness.double_click(DISTANCE_LABEL);
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(model_dimensions::value_field_id(
            extrude,
            ValueSlot::Extrude(Reach::Only)
        ))
    );
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "25");
    assert_eq!(distance_of(&harness, extrude), "25");
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 1"));
    assert!(harness.shows("Distance 25 mm"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert_eq!(distance_of(&harness, extrude), "10 mm");
}

#[test]
fn a_distance_driven_by_a_parameter_reads_name_equals_value_and_edits_the_parameter() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let mut transaction = harness.document().transaction("Name the depth");
    let depth = transaction.parse("12 mm").unwrap();
    let id = transaction.add_parameter("depth", depth);
    let SolidFeature::Extrude(mut extrusion) = harness.solid(extrude).clone() else {
        panic!("expected an extrusion");
    };
    extrusion.extent = caditor_document::ExtrudeExtent::one_side(
        caditor_expression::Expression::Parameter(id),
        false,
    );
    transaction.edit(Edit::SetFeatureKind {
        id: extrude,
        kind: caditor_document::FeatureKind::Solid(SolidFeature::Extrude(extrusion)),
    });
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.frame();
    assert!(harness.shows("depth = 12 mm"));

    harness.double_click("depth = 12 mm");
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "depth = 30 mm");
    assert_eq!(harness.expression_text("depth"), "30 mm");
    assert_eq!(distance_of(&harness, extrude), "depth");
    assert!(harness.shows("depth = 30 mm"));
}

#[test]
fn the_highlight_keys_reach_a_features_value_and_enter_edits_it() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let target = distance_value(extrude);

    for _ in 0..HIGHLIGHT_STEPS {
        if harness.workspace.viewport.keyboard_highlight() == Some(target) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(
        harness.workspace.viewport.keyboard_highlight(),
        Some(target)
    );

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(model_dimensions::value_field_id(
            extrude,
            ValueSlot::Extrude(Reach::Only)
        ))
    );
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "18 mm");
    assert_eq!(distance_of(&harness, extrude), "18 mm");
}

#[test]
fn a_refused_value_keeps_the_field_open_and_changes_nothing() {
    let mut harness = Harness::new();
    let extrude = plate_with_distance_shown(&mut harness);
    let undo = harness.model.undo_label().map(str::to_owned);

    harness.double_click(DISTANCE_LABEL);
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "-4");
    assert_eq!(distance_of(&harness, extrude), "10 mm");
    assert!(harness.shows_containing("Enter a value above zero"));
    assert_eq!(harness.model.undo_label().map(str::to_owned), undo);
}

fn open_feature(harness: &Harness) -> FeatureId {
    harness
        .workspace
        .editing
        .solid()
        .expect("a feature is open")
}

fn close_feature(harness: &mut Harness) {
    harness.select([]);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
}

fn choose_row(harness: &mut Harness, row: &str) {
    click_with(harness, row, Modifiers::NONE);
    for _ in 0..PAST_DOUBLE_CLICK_FRAMES {
        harness.frame();
    }
}

fn shape_of(harness: &Harness, feature: FeatureId, value: ValueSlot) -> Option<ShownShape> {
    harness
        .workspace
        .viewport
        .model_dimensions()
        .shape_of(Pickable::FeatureValue { feature, value })
}

#[test]
fn a_revolves_angle_is_drawn_as_an_arc_about_its_axis_and_edited_in_place() {
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
    let revolve = open_feature(&harness);
    close_feature(&mut harness);
    let SolidFeature::Revolve(mut turned) = harness.solid(revolve).clone() else {
        panic!("expected a revolve");
    };
    turned.extent = RevolveExtent::OneSide {
        angle: Expression::measure(90.0, Unit::Degree),
        reversed: false,
    };
    let mut transaction = harness.document().transaction("Turn a quarter");
    transaction.edit(Edit::SetFeatureKind {
        id: revolve,
        kind: FeatureKind::Solid(SolidFeature::Revolve(turned)),
    });
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    run_from_palette(&mut harness, "fit view");
    show_dimensions(&mut harness);
    harness.select([]);
    choose_row(&mut harness, "Revolve 1");

    let label = "Angle 90°";
    assert!(harness.shows(label));
    assert!(matches!(
        shape_of(&harness, revolve, ValueSlot::RevolveAngle),
        Some(ShownShape::Path(points)) if points > 2
    ));

    harness.double_click(label);
    harness.frame();
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "120");
    assert_eq!(harness.model.undo_label(), Some("Edit Revolve 1"));
    assert!(harness.shows("Angle 120°"));
}

#[test]
fn a_holes_diameter_spans_its_rim_and_its_depth_runs_along_its_axis() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = open_feature(&harness);
    close_feature(&mut harness);
    run_from_palette(&mut harness, "fit view");
    show_dimensions(&mut harness);
    harness.select([]);
    choose_row(&mut harness, "Hole 1");

    assert_eq!(
        shape_of(&harness, hole, ValueSlot::HoleDiameter),
        Some(ShownShape::Path(2))
    );
    assert_eq!(
        shape_of(&harness, hole, ValueSlot::HoleDepth),
        Some(ShownShape::Path(2))
    );
    assert!(harness.shows_containing("Diameter"));
    assert!(harness.shows_containing("Depth"));
}

#[test]
fn a_fillets_radius_is_a_leader_to_its_round_face() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click_tool(BlendKind::Fillet.title());
    harness.settle();
    let fillet = open_feature(&harness);
    close_feature(&mut harness);
    run_from_palette(&mut harness, "fit view");
    show_dimensions(&mut harness);
    harness.select([]);
    choose_row(&mut harness, "Fillet 1");

    assert_eq!(
        shape_of(&harness, fillet, ValueSlot::BlendSize),
        Some(ShownShape::Leader)
    );
    assert!(harness.shows_containing("Radius"));
}

#[test]
fn a_boxs_sizes_are_offered_on_the_model_and_a_double_click_changes_one() {
    let mut harness = Harness::new();
    harness.select([]);
    run_from_palette(&mut harness, "Box");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let made = open_feature(&harness);
    close_feature(&mut harness);
    run_from_palette(&mut harness, "fit view");
    show_dimensions(&mut harness);
    harness.select([]);
    choose_row(&mut harness, "Box 1");

    let length = ValueSlot::Primitive(PrimitiveSize::Length);
    assert_eq!(shape_of(&harness, made, length), Some(ShownShape::Stacked));
    assert!(harness.shows("Length 20 mm"));

    harness.double_click("Length 20 mm");
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(model_dimensions::value_field_id(made, length))
    );
    harness.key(Key::A, Modifiers::COMMAND);
    enter(&mut harness, "45");
    assert_eq!(harness.model.undo_label(), Some("Edit Box 1"));
    assert!(matches!(
        &primitive_of(&harness, made).shape,
        PrimitiveShape::Box { length, .. } if harness.document().expression_text(length) == "45"
    ));
}

#[test]
fn labels_of_two_sketches_are_thinned_together_so_they_never_overlap() {
    let mut harness = Harness::new();
    harness.settle();
    let base = harness
        .document()
        .features()
        .find(|feature| feature.kind.sketch().is_some())
        .map(caditor_document::Feature::id)
        .expect("the model has a sketch");
    let copy = harness.sketch(base).clone();
    let twin = harness.add_sketch(copy);
    run_from_palette(&mut harness, "fit view");
    show_dimensions(&mut harness);
    harness.frame();

    let rects = harness
        .workspace
        .viewport
        .model_dimensions()
        .sketch_label_rects();
    let of = |feature: FeatureId| {
        rects
            .iter()
            .filter(move |(owner, _)| *owner == feature)
            .map(|(_, rect)| *rect)
    };
    assert!(of(base).next().is_some());
    assert!(
        of(base)
            .all(|first| of(twin)
                .all(|second| first.intersect(second).area() * 2.0 <= second.area()))
    );
}
