use caditor_document::{
    BodyOperation, ExtrudeExtent, FeatureId, FeatureKind, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;

use super::{
    CAMERA_SETTLE, Harness, add_peg, choose, combine_nearly_touching_blocks, extruded_plate,
    pickable_described, rectangle,
};
use crate::model::Action;

#[test]
fn two_bodies_are_combined_from_the_selection_and_the_panel_changes_how() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let peg_top = pickable_described(&mut harness, "Peg › Peg end face");
    let plate_volume = 16000.0;
    let peg_volume = 30.0 * 20.0 * 5.0;
    let overlap = 10.0 * 20.0 * 5.0;

    harness.select([top]);
    harness.hover("Combine");
    assert!(harness.shows_containing("Select faces or edges of two bodies"));

    harness.select([top, peg_top]);
    harness.click("Combine");
    harness.settle();
    let combine = harness
        .workspace
        .editing
        .solid()
        .expect("the combine is open");
    assert_eq!(harness.model.undo_label(), Some("Create Combine 1"));
    let definition = harness
        .document()
        .feature(combine)
        .unwrap()
        .kind
        .combine()
        .unwrap()
        .clone();
    assert_eq!((definition.body, definition.tool), (plate, peg));
    assert!(harness.shows("Operation"));
    assert!(harness.shows("Target body"));
    assert!(harness.shows("Tool body"));
    assert!((harness.body_volume(plate) - (plate_volume + peg_volume - overlap)).abs() < 100.0);
    assert!(harness.model.evaluation().body_result(peg).is_none());

    harness.click_button("Cut");
    harness.settle();
    assert_eq!(harness.model.undo_label(), Some("Edit Combine 1"));
    assert!((harness.body_volume(plate) - (plate_volume - overlap)).abs() < 100.0);

    harness.click_button("Intersect");
    harness.settle();
    assert!((harness.body_volume(plate) - overlap).abs() < 100.0);

    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.perform(Action::Undo);
    harness.settle();
    assert!(harness.model.evaluation().body_result(peg).is_some());
    assert!((harness.body_volume(plate) - plate_volume).abs() < 100.0);
}

fn add_post(harness: &mut Harness) -> FeatureId {
    let mut outline = Sketch::new(Plane::XY);
    rectangle(&mut outline, Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    let mut transaction = harness.document().transaction("Add a post");
    let sketch = transaction.add_feature("Post sketch", FeatureKind::from(outline));
    let post = transaction.add_feature(
        "Post",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::parse_stored("5 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    post
}

#[test]
fn a_combine_takes_several_tool_bodies_and_can_keep_them() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    let post = add_post(&mut harness);
    let peg_top = pickable_described(&mut harness, "Peg › Peg end face");
    let plate_volume = 16000.0;
    let peg_cut = 10.0 * 20.0 * 5.0;
    let post_cut = 10.0 * 10.0 * 5.0;

    harness.select([top, peg_top]);
    harness.click("Combine");
    harness.settle();
    let combine = harness
        .workspace
        .editing
        .solid()
        .expect("the combine is open");
    harness.click_button("Cut");
    harness.settle();
    assert!(harness.shows(crate::combine_panel::KEEP_TOOL));
    assert!(harness.shows(crate::combine_panel::ADD_TOOL_BODY));

    choose(&mut harness, crate::combine_panel::ADD_TOOL_BODY, "Post");
    let definition = harness
        .document()
        .feature(combine)
        .unwrap()
        .kind
        .combine()
        .unwrap()
        .clone();
    assert_eq!(definition.more_tools, vec![post]);
    assert!(!definition.keep_tool);
    assert!(harness.shows(crate::combine_panel::ALSO_COMBINES));
    assert!(!harness.shows(crate::combine_panel::ADD_TOOL_BODY));
    assert!(harness.model.evaluation().body_result(peg).is_none());
    assert!(harness.model.evaluation().body_result(post).is_none());
    assert!(
        (harness.body_volume(plate) - (plate_volume - peg_cut - post_cut)).abs() < 100.0,
        "{}",
        harness.body_volume(plate)
    );

    harness.click_lowest(crate::combine_panel::KEEP_TOOL);
    harness.settle();
    let definition = harness
        .document()
        .feature(combine)
        .unwrap()
        .kind
        .combine()
        .unwrap()
        .clone();
    assert!(definition.keep_tool);
    assert!(harness.model.evaluation().body_result(peg).is_some());
    assert!(harness.model.evaluation().body_result(post).is_some());
    assert!((harness.body_volume(plate) - (plate_volume - peg_cut - post_cut)).abs() < 100.0);

    harness.click_button("Stop using Post");
    harness.settle();
    let definition = harness
        .document()
        .feature(combine)
        .unwrap()
        .kind
        .combine()
        .unwrap()
        .clone();
    assert!(definition.more_tools.is_empty());
    assert!((harness.body_volume(plate) - (plate_volume - peg_cut)).abs() < 100.0);
}

#[test]
fn a_combine_failing_where_faces_nearly_touch_is_marked_and_shown_in_the_view() {
    let mut harness = Harness::new();

    let combine = combine_nearly_touching_blocks(&mut harness);

    let Some(caditor_document::FeatureState::Failed(error)) = harness
        .model
        .evaluation()
        .feature(combine)
        .map(|status| status.state.clone())
    else {
        panic!("the nearly touching blocks should not combine");
    };
    let place = error.place.expect("the failure has a place");
    let before = harness.workspace.viewport.viewpoint().target;
    assert!(harness.shows(&error.reason()), "{}", error.reason());
    assert!(harness.shows("Combine 1 failed here"));
    assert!(before.distance(place) > 1.0);

    harness.click("Show where");
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();

    let after = harness.workspace.viewport.viewpoint();
    assert!(after.target.distance(place) < 1e-6, "{after:?} {place:?}");
    assert!(harness.shows("Combine 1 failed here"));
}

#[test]
fn a_combine_of_a_third_body_or_none_is_refused_with_what_to_select() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);

    harness.click("Combine");
    harness.settle();

    assert_eq!(harness.workspace.editing.solid(), None);
    assert!(
        harness
            .document()
            .features()
            .all(|feature| feature.kind.combine().is_none())
    );
}
