use caditor_document::{FeatureId, RegionChoice};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Entity, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, attached_body, extruded_plate, open_extrude, open_hole, top_edge_along_x};
use crate::{editing::EditingCommand, model::Action, selection::Pickable};

#[test]
fn a_curved_face_or_a_round_edge_says_why_it_cannot_be_used() {
    let mut harness = Harness::new();
    let mut disc = Sketch::new(Plane::XY);
    disc.add_circle(Point2::ZERO, 10.0);
    harness.add_sketch(disc);
    harness.select([]);
    harness.click("Extrude");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let pickables: Vec<Pickable> = harness.built().picks.pickables().collect();
    let side = pickables
        .iter()
        .copied()
        .find(|pickable| {
            pickable
                .describe(harness.document(), harness.model.evaluation())
                .contains("side")
        })
        .expect("the round side is pickable");
    let rim = pickables
        .iter()
        .copied()
        .find(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .expect("a rim is pickable");

    harness.select([]);
    harness.click("New sketch");
    harness.click_pickable(Plane::XY, Point2::new(10.0, 0.0), side);
    harness.settle();
    assert!(harness.workspace.editing.is_choosing_plane());
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("New sketch: The selected face is curved; sketches lie on planes and flat faces.")
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    harness.select([rim]);
    let end = harness.document().features().len();
    let selection = harness.workspace.viewport.selection();
    assert!(matches!(
        crate::datum_tools::plane_from_selection(&harness.model, selection, end),
        Ok(caditor_document::Datum::PlaneThrough(
            caditor_document::PlaneThrough::SquareToCurve(_)
        ))
    ));
    assert!(matches!(
        crate::datum_tools::point_from_selection(&harness.model, selection, end),
        Ok(caditor_document::Datum::Point(
            caditor_document::DatumPoint {
                base: caditor_document::PointReference::Centre { .. },
                ..
            }
        ))
    ));
}

fn extrude_selected_face(harness: &mut Harness, face: Pickable) -> FeatureId {
    harness.select([face]);
    harness.click("Extrude");
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

#[test]
fn extrude_with_a_face_selected_extrudes_that_face_out_of_its_body_not_the_last_sketch() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let first_sketch = open_extrude(&harness, plate).sketch;

    let extrusion = extrude_selected_face(&mut harness, top);
    let definition = open_extrude(&harness, extrusion);
    let sketch = harness.sketch(definition.sketch).clone();

    assert_eq!(harness.model.undo_label(), Some("Create Extrude 2"));
    assert_ne!(definition.sketch, first_sketch);
    assert_eq!(
        definition.operation,
        caditor_document::BodyOperation::Add(plate)
    );
    assert_eq!(attached_body(&harness, definition.sketch), Some(plate));
    assert!(
        harness
            .document()
            .feature(definition.sketch)
            .unwrap()
            .hidden
    );
    assert_eq!(
        sketch
            .entities()
            .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
            .count(),
        4
    );
    assert!(harness.shows_containing("Extrude 2 extrudes the selected face out of its body"));
    assert!((harness.body_volume(plate) - 32000.0).abs() < 1.0);
}

#[test]
fn a_face_selected_while_another_feature_is_open_is_still_the_one_extruded_or_drilled() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(plate)));
    harness.settle();

    let extrusion = extrude_selected_face(&mut harness, top);
    let extruded_sketch = open_extrude(&harness, extrusion).sketch;
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.perform(Action::Editing(EditingCommand::OpenSolid(plate)));
    harness.settle();
    let side = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { .. })
                && pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 2 end face"
        })
        .expect("the extruded face is pickable");
    harness.select([side]);
    harness.click("Hole");
    harness.settle();
    let hole = harness.workspace.editing.solid().expect("the hole is open");

    assert_eq!(attached_body(&harness, extruded_sketch), Some(plate));
    assert_eq!(
        open_extrude(&harness, extrusion).operation,
        caditor_document::BodyOperation::Add(plate)
    );
    assert_eq!(
        attached_body(&harness, open_hole(&harness, hole).sketch),
        Some(plate)
    );
    assert_ne!(open_hole(&harness, hole).sketch, extruded_sketch);
}

#[test]
fn extrude_and_revolve_refuse_a_selection_holding_no_sketch_instead_of_taking_the_last_one() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let edge = top_edge_along_x(&harness, plate, 0.0);
    let features = harness.document().features().count();

    harness.select([Pickable::Edge { body: plate, edge }]);
    harness.click("Extrude");
    harness.settle();
    harness.select([top]);
    harness.click("Revolve");
    harness.settle();

    assert_eq!(harness.document().features().count(), features);
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn extruding_with_a_closed_shape_selected_sweeps_only_what_it_encloses() {
    let overlapping = || {
        let mut sketch = Sketch::new(Plane::XY);
        let sides: Vec<EntityId> = [
            (Point2::new(0.0, 0.0), Point2::new(30.0, 0.0)),
            (Point2::new(30.0, 0.0), Point2::new(30.0, 10.0)),
            (Point2::new(30.0, 10.0), Point2::new(0.0, 10.0)),
            (Point2::new(0.0, 10.0), Point2::new(0.0, 0.0)),
        ]
        .into_iter()
        .map(|(start, end)| sketch.add_line(start, end))
        .collect();
        let circle = sketch.add_circle(Point2::new(30.0, 0.0), 8.0);
        (sketch, sides, circle)
    };
    let extruded =
        |chosen: &dyn Fn(&[EntityId], EntityId) -> Vec<EntityId>| {
            let mut harness = Harness::new();
            let (sketch, sides, circle) = overlapping();
            let sketch = harness.add_sketch(sketch);
            harness.select(chosen(&sides, circle).into_iter().map(|entity| {
                Pickable::SketchEntity {
                    feature: sketch,
                    entity,
                }
            }));
            harness.click("Extrude");
            harness.settle();
            let extrusion = harness
                .workspace
                .editing
                .solid()
                .expect("the extrusion is open");
            let chosen = match harness.solid(extrusion).regions() {
                RegionChoice::Chosen(chosen) => chosen.len(),
                RegionChoice::All => 0,
            };
            (chosen, harness.body_volume(extrusion))
        };

    let rectangle = extruded(&|sides, _| sides.to_vec());
    let disc = extruded(&|_, circle| vec![circle]);
    let one_side = extruded(&|sides, _| vec![sides[0]]);

    assert_eq!(rectangle.0, 2);
    assert!((rectangle.1 - 300.0 * 10.0).abs() < 1.0, "{}", rectangle.1);
    assert_eq!(disc.0, 2);
    assert!(
        (disc.1 - std::f64::consts::PI * 64.0 * 10.0).abs() < 5.0,
        "{}",
        disc.1
    );
    assert_eq!(one_side.0, 0);
}

#[test]
fn a_chamfer_face_selected_for_extrude_grows_out_along_its_slant() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    harness.click("Chamfer");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let chamfered = harness.body_volume(plate);
    let chamfer_face = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation())
                == "Extrude 1 › Chamfer 1 face"
        })
        .expect("the chamfer face is pickable");

    let extrusion = extrude_selected_face(&mut harness, chamfer_face);
    let grown = harness.body_volume(plate) - chamfered;

    assert_eq!(
        open_extrude(&harness, extrusion).operation,
        caditor_document::BodyOperation::Add(plate)
    );
    assert!(
        (grown - 40.0 * std::f64::consts::SQRT_2 * 10.0).abs() < 1.0,
        "{grown}"
    );
}
