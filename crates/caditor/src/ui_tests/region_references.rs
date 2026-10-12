use std::f64::consts::PI;

use caditor_document::{
    BodyOperation, Document, ExtrudeExtent, FeatureId, FeatureKind, RegionChoice, SolidFeature,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, Sketch};

use super::{Harness, offer, run_from_palette};
use crate::{app::Workspace, commands::Command, model::Action};

fn chosen_plate() -> (Document, FeatureId, FeatureId) {
    let mut outline = Sketch::new(Plane::XY);
    let corners = [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)];
    for index in 0..4 {
        let (from, to) = (corners[index], corners[(index + 1) % 4]);
        outline.add_line(Point2::new(from.0, from.1), Point2::new(to.0, to.1));
    }
    let regions = caditor_document::sketch_regions(&outline)
        .unwrap()
        .iter()
        .map(|region| caditor_kernel::RegionReference::capture(region, region.anchor()))
        .collect();
    let mut document = Document::default();
    let mut transaction = document.transaction("Build");
    let sketch = transaction.add_feature("Outline", FeatureKind::from(outline));
    let base = transaction.add_feature(
        "Base",
        FeatureKind::Solid(SolidFeature::Extrude(caditor_document::Extrude {
            sketch,
            regions: RegionChoice::Chosen(regions),
            extent: ExtrudeExtent::one_side(Expression::Measure(5.0, Unit::Millimetre), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    (document, sketch, base)
}

#[test]
fn a_region_matched_again_after_a_hole_is_drawn_warns_until_its_references_are_updated() {
    let (document, sketch, base) = chosen_plate();
    let mut harness = Harness::starting(None, document, Workspace::new());
    harness.context.enable_accesskit();
    let mut transaction = harness.document().transaction("Draw a hole");
    let center = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(20.0, 15.0)));
    transaction.add_sketch_entity(
        sketch,
        Entity::Circle {
            center,
            radius: 5.0,
        },
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let reason = "After an upstream change, a chosen region of Outline was matched to the most \
                  similar geometry.";

    let warned = harness.shows(reason);
    let hinted = harness.accessible.iter().any(|(_, node)| {
        node.value()
            == Some("Some of what it uses changed and was matched to the most similar geometry")
    });
    harness.click("Update references");
    harness.settle();

    assert!(warned);
    assert!(hinted);
    assert_eq!(
        harness.model.undo_label(),
        Some("Update references of Base")
    );
    assert!(!harness.shows(reason));
    assert!((harness.body_volume(base) - (6000.0 - 125.0 * PI)).abs() < 1.0);

    harness.perform(Action::Undo);
    harness.settle();
    harness.click("Base");
    run_from_palette(&mut harness, "update references");
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Update references of Base")
    );
    assert!(!harness.shows(reason));
}

#[test]
fn update_references_waits_for_the_evaluation_of_the_revision_it_would_change() {
    let (document, sketch, _) = chosen_plate();
    let mut harness = Harness::starting(None, document, Workspace::new());
    let mut transaction = harness.document().transaction("Draw a hole");
    let center = transaction.add_sketch_entity(sketch, Entity::Point(Point2::new(20.0, 15.0)));
    transaction.add_sketch_entity(
        sketch,
        Entity::Circle {
            center,
            radius: 5.0,
        },
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.click("Base");
    harness.frame();
    let offered = harness.shows("Update references");
    let available = offer(&harness, Command::UpdateReferences).availability;

    harness.model.hold_evaluations(true);
    harness.perform(Action::Undo);
    harness.frame();
    harness.frame();
    let offered_meanwhile = harness.shows("Update references");
    let available_meanwhile = offer(&harness, Command::UpdateReferences).availability;
    harness.model.hold_evaluations(false);
    harness.settle();

    assert!(offered);
    assert_eq!(available, Ok(()));
    assert!(!offered_meanwhile);
    assert_eq!(
        available_meanwhile,
        Err(crate::feature_tree::HEALING_AWAITS_RECOMPUTE.into())
    );
    assert_eq!(harness.model.redo_label(), Some("Draw a hole"));
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(!harness.shows("Update references"));
}
