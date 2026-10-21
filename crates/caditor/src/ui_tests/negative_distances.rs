use caditor_document::{
    BodyOperation, Edit, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureKind, SolidFeature,
    Transaction,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::Point2;
use egui::{Id, Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, draft_volume, extruded_plate, length_value};
use crate::{
    feature_fields::{ABOVE_ZERO, NOT_ZERO_OR_REVERSE},
    model::{Action, NoticeKind},
    solid_tools,
};

const PLATE_VOLUME: f64 = 16_000.0;
const FOOTPRINT: f64 = 200.0;

fn distance_field(extrude: FeatureId) -> Id {
    Id::unique(("solid-field", "distance", extrude))
}

fn extrusion(harness: &Harness, feature: FeatureId) -> &Extrude {
    match harness.solid(feature) {
        SolidFeature::Extrude(extrude) => extrude,
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => panic!("expected an extrusion"),
    }
}

fn reversed(extrude: &Extrude) -> bool {
    match extrude.extent {
        ExtrudeExtent::OneSide { reversed, .. } => reversed,
        ExtrudeExtent::Symmetric { .. } | ExtrudeExtent::TwoSides { .. } => {
            panic!("expected a one-sided extent")
        }
    }
}

fn distance(extrude: &Extrude) -> &Expression {
    match &extrude.extent {
        ExtrudeExtent::OneSide {
            end: ExtrudeEnd::Distance(distance),
            ..
        } => distance,
        _ => panic!("expected a one-sided distance"),
    }
}

fn distance_in_millimetres(harness: &Harness, feature: FeatureId) -> f64 {
    length_value(harness, distance(extrusion(harness, feature)))
}

fn notice_text(harness: &Harness) -> String {
    harness
        .model
        .notice()
        .map(|notice| notice.text.clone())
        .unwrap_or_default()
}

fn open_cut_on_plate(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    harness.click("New sketch");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    harness.click("Extrude");
    harness.settle();
    let cut = harness.workspace.editing.solid().expect("the cut is open");
    (plate, cut)
}

fn set_distance(harness: &mut Harness, extrude: FeatureId, text: &str) {
    harness.type_into_field(distance_field(extrude), text);
    harness.settle();
}

fn become_one_sided(
    harness: &mut Harness,
    extrude: FeatureId,
    operation: BodyOperation,
    reversed: bool,
) {
    let mut changed = extrusion(harness, extrude).clone();
    let end = ExtrudeEnd::Distance(distance(&changed).clone());
    changed.extent = ExtrudeExtent::OneSide { end, reversed };
    let SolidFeature::Extrude(changed) =
        solid_tools::with_operation(&SolidFeature::Extrude(changed), operation)
    else {
        panic!("an extrusion stays an extrusion");
    };
    harness.perform(Action::Apply(Transaction::single(
        "Set up the extrusion",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(changed)),
        },
    )));
    harness.settle();
}

fn growing_from_the_top(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, cut) = open_cut_on_plate(harness);
    set_distance(harness, cut, "10 mm");
    become_one_sided(harness, cut, BodyOperation::Add(plate), false);
    (plate, cut)
}

fn cutting_from_the_top(harness: &mut Harness) -> (FeatureId, FeatureId) {
    let (plate, cut) = open_cut_on_plate(harness);
    set_distance(harness, cut, "5 mm");
    (plate, cut)
}

fn add_parameter(
    harness: &mut Harness,
    name: &str,
    value: &str,
) -> caditor_expression::ParameterId {
    let document = harness.document();
    let expression = Expression::parse(value, &|name| {
        document
            .parameter_named(name)
            .map(|parameter| parameter.id())
    })
    .unwrap();
    let mut transaction = document.transaction(format!("Add {name}"));
    let id = transaction.add_parameter(name, expression);
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    id
}

#[test]
fn a_negative_distance_on_an_add_reverses_it_and_cuts_into_the_body_in_one_undoable_change() {
    let mut harness = Harness::new();
    let (plate, extrude) = growing_from_the_top(&mut harness);
    let setup = harness.model.undo_label().map(str::to_owned);

    assert!((harness.body_volume(plate) - (PLATE_VOLUME + 10.0 * FOOTPRINT)).abs() < 1.0);

    set_distance(&mut harness, extrude, "-5 mm");
    let flipped = extrusion(&harness, extrude).clone();

    assert_eq!(flipped.operation, BodyOperation::Remove(plate));
    assert!(reversed(&flipped));
    assert_eq!(distance_in_millimetres(&harness, extrude), 5.0);
    assert!((harness.body_volume(plate) - (PLATE_VOLUME - 5.0 * FOOTPRINT)).abs() < 1.0);
    assert_eq!(harness.model.undo_label(), Some("Edit Extrude 2"));
    assert_eq!(
        notice_text(&harness),
        "Extrude 2: reversed and switched to Remove; a negative distance cuts the other way"
    );
    assert_eq!(
        harness.model.notice().map(|notice| notice.kind),
        Some(NoticeKind::Info)
    );

    harness.perform(Action::Undo);
    harness.settle();
    let restored = extrusion(&harness, extrude);

    assert_eq!(restored.operation, BodyOperation::Add(plate));
    assert!(!reversed(restored));
    assert_eq!(distance_in_millimetres(&harness, extrude), 10.0);
    assert!((harness.body_volume(plate) - (PLATE_VOLUME + 10.0 * FOOTPRINT)).abs() < 1.0);
    assert_eq!(harness.model.undo_label().map(str::to_owned), setup);
}

#[test]
fn a_negative_distance_on_a_remove_reverses_it_and_adds_to_the_body_it_was_cutting() {
    let mut harness = Harness::new();
    let (plate, extrude) = cutting_from_the_top(&mut harness);
    let cutting = extrusion(&harness, extrude);

    assert_eq!(cutting.operation, BodyOperation::Remove(plate));
    assert!(reversed(cutting));
    assert!((harness.body_volume(plate) - (PLATE_VOLUME - 5.0 * FOOTPRINT)).abs() < 1.0);

    set_distance(&mut harness, extrude, "-5 mm");
    let flipped = extrusion(&harness, extrude);

    assert_eq!(flipped.operation, BodyOperation::Add(plate));
    assert!(!reversed(flipped));
    assert_eq!(distance_in_millimetres(&harness, extrude), 5.0);
    assert!((harness.body_volume(plate) - (PLATE_VOLUME + 5.0 * FOOTPRINT)).abs() < 1.0);
    assert_eq!(
        notice_text(&harness),
        "Extrude 2: reversed and switched to Add; a negative distance adds the other way"
    );
}

#[test]
fn a_negative_distance_on_a_new_body_only_reverses_it() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Extrude");
    harness.settle();
    let extrude = harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open");
    become_one_sided(&mut harness, extrude, BodyOperation::NewBody, false);

    set_distance(&mut harness, extrude, "-5 mm");
    let flipped = extrusion(&harness, extrude);

    assert_eq!(flipped.operation, BodyOperation::NewBody);
    assert!(reversed(flipped));
    assert_eq!(distance_in_millimetres(&harness, extrude), 5.0);
    assert!(notice_text(&harness).ends_with("a negative distance goes the other way"));
    assert!(!notice_text(&harness).contains("switched"));
}

#[test]
fn a_negative_distance_on_an_intersection_only_reverses_it() {
    let mut harness = Harness::new();
    let (plate, extrude) = cutting_from_the_top(&mut harness);
    become_one_sided(&mut harness, extrude, BodyOperation::Intersect(plate), true);

    set_distance(&mut harness, extrude, "-5 mm");
    let flipped = extrusion(&harness, extrude);

    assert_eq!(flipped.operation, BodyOperation::Intersect(plate));
    assert!(!reversed(flipped));
    assert_eq!(distance_in_millimetres(&harness, extrude), 5.0);
}

#[test]
fn zero_and_a_negative_side_of_a_symmetric_or_two_sided_extrusion_are_still_refused() {
    let mut harness = Harness::new();
    let (_, extrude) = cutting_from_the_top(&mut harness);

    set_distance(&mut harness, extrude, "0 mm");
    assert!(harness.shows(NOT_ZERO_OR_REVERSE));
    assert_eq!(distance_in_millimetres(&harness, extrude), 5.0);

    let ten = Expression::measure(10.0, Unit::Millimetre);
    let mut symmetric = extrusion(&harness, extrude).clone();
    symmetric.extent = ExtrudeExtent::Symmetric {
        distance: ten.clone(),
    };
    harness.perform(Action::Apply(Transaction::single(
        "Make it symmetric",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(symmetric.clone())),
        },
    )));
    harness.settle();
    let before = harness.model.undo_label().map(str::to_owned);
    harness.type_into_field(distance_field(extrude), "-5 mm");

    assert!(harness.shows(ABOVE_ZERO));
    assert_eq!(extrusion(&harness, extrude), &symmetric);
    assert_eq!(harness.model.undo_label().map(str::to_owned), before);

    let mut two_sided = symmetric;
    two_sided.extent = ExtrudeExtent::two_sides(ten.clone(), ten);
    harness.perform(Action::Apply(Transaction::single(
        "Make it two-sided",
        Edit::SetFeatureKind {
            id: extrude,
            kind: FeatureKind::Solid(SolidFeature::Extrude(two_sided.clone())),
        },
    )));
    harness.settle();
    harness.type_into_field(Id::unique(("solid-field", "forward", extrude)), "-5 mm");

    assert!(harness.shows(ABOVE_ZERO));
    assert_eq!(extrusion(&harness, extrude), &two_sided);
}

#[test]
fn a_distance_written_as_a_negative_expression_keeps_its_parameters() {
    let mut harness = Harness::new();
    let (plate, extrude) = growing_from_the_top(&mut harness);
    let depth = add_parameter(&mut harness, "depth", "4 mm");
    let stock = add_parameter(&mut harness, "stock", "20 mm");

    set_distance(&mut harness, extrude, "-depth");
    let by_depth = extrusion(&harness, extrude).clone();

    assert_eq!(distance(&by_depth), &Expression::Parameter(depth));
    assert_eq!(by_depth.operation, BodyOperation::Remove(plate));
    assert!(reversed(&by_depth));
    assert!((harness.body_volume(plate) - (PLATE_VOLUME - 4.0 * FOOTPRINT)).abs() < 1.0);

    become_one_sided(&mut harness, extrude, BodyOperation::Add(plate), false);
    set_distance(&mut harness, extrude, "stock - 30 mm");
    let by_stock = extrusion(&harness, extrude).clone();

    assert!(matches!(distance(&by_stock), Expression::Negate(inner) if inner.uses(stock)));
    assert_eq!(by_stock.operation, BodyOperation::Remove(plate));
    assert!(reversed(&by_stock));
    assert_eq!(distance_in_millimetres(&harness, extrude), 10.0);
    assert!((harness.body_volume(plate) - (PLATE_VOLUME - 10.0 * FOOTPRINT)).abs() < 1.0);
}

#[test]
fn a_parameter_turning_negative_later_fails_the_feature_instead_of_flipping_it_again() {
    let mut harness = Harness::new();
    let (plate, extrude) = growing_from_the_top(&mut harness);
    let depth = add_parameter(&mut harness, "depth", "4 mm");
    set_distance(&mut harness, extrude, "-depth");
    let flipped = extrusion(&harness, extrude).clone();

    harness.perform(Action::Apply(Transaction::single(
        "Edit depth",
        Edit::SetParameterExpression {
            id: depth,
            expression: Expression::measure(-3.0, Unit::Millimetre),
        },
    )));
    harness.settle();

    assert_eq!(extrusion(&harness, extrude), &flipped);
    assert_eq!(flipped.operation, BodyOperation::Remove(plate));
    assert_eq!(harness.model.evaluation().failed_count(), 1);
}

#[test]
fn text_that_does_not_parse_yet_keeps_the_last_valid_preview_until_the_field_is_left() {
    let mut harness = Harness::new();
    let (plate, extrude) = growing_from_the_top(&mut harness);

    harness.draft_into_field(distance_field(extrude), "3 mm");
    harness.wait_until("the typed distance is previewed", |harness| {
        draft_volume(harness, plate).is_some_and(|volume| volume > PLATE_VOLUME)
    });
    let previewed = harness.model.draft_kind(extrude).cloned();

    harness.draft_into_field(distance_field(extrude), "3+");
    harness.frame();

    assert!(previewed.is_some());
    assert_eq!(harness.model.draft_kind(extrude).cloned(), previewed);
    assert!(draft_volume(&harness, plate).is_some_and(|volume| volume > PLATE_VOLUME));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.model.draft_evaluation().is_none());
}

#[test]
fn a_negative_distance_being_typed_is_previewed_flipped_before_it_is_entered() {
    let mut harness = Harness::new();
    let (plate, extrude) = growing_from_the_top(&mut harness);
    let before = harness.model.undo_label().map(str::to_owned);

    harness.draft_into_field(distance_field(extrude), "-5 mm");
    harness.wait_until("the typed distance is previewed", |harness| {
        draft_volume(harness, plate).is_some_and(|volume| volume < PLATE_VOLUME)
    });
    let Some(FeatureKind::Solid(SolidFeature::Extrude(drafted))) =
        harness.model.draft_kind(extrude).cloned()
    else {
        panic!("typing previews the extrusion");
    };

    assert_eq!(drafted.operation, BodyOperation::Remove(plate));
    assert!(reversed(&drafted));
    assert!(
        (draft_volume(&harness, plate).unwrap() - (PLATE_VOLUME - 5.0 * FOOTPRINT)).abs() < 1.0
    );
    assert_eq!(harness.model.undo_label().map(str::to_owned), before);
    assert_eq!(
        extrusion(&harness, extrude).operation,
        BodyOperation::Add(plate)
    );
    assert!(!notice_text(&harness).contains("switched"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.model.draft_evaluation().is_none());
    assert_eq!(
        extrusion(&harness, extrude).operation,
        BodyOperation::Add(plate)
    );
    assert!(!reversed(extrusion(&harness, extrude)));
}
