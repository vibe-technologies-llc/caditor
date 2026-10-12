use caditor_document::FeatureKind;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::{Entity, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{CAMERA_SETTLE, Harness, extruded_plate};
use crate::{editing::Tool, model::Action, selection::Pickable};

#[test]
fn the_project_tool_brings_a_face_outline_into_the_sketch_and_follows_the_keyboard() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = harness.draw_on_new_sketch();

    harness.use_tool_with(Key::P, Modifiers::ALT);
    let top = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            matches!(pickable, Pickable::Face { body, .. } if *body == extrude)
                && pickable.describe(harness.document(), harness.model.evaluation())
                    == "Extrude 1 › Extrude 1 end face"
        })
        .expect("the top face is pickable while projecting");

    assert_eq!(harness.tool(), Some(Tool::Project));
    assert!(harness.shows(
        "Click an edge, corner or face of a body, or a curve of another sketch, to project it"
    ));

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();
    let projected: Vec<EntityId> = harness.sketch(sketch).projected().collect();
    let shown = harness.shown(sketch);
    let lines: Vec<EntityId> = projected
        .iter()
        .copied()
        .filter(|id| matches!(shown.entity(*id), Some(Entity::Line { .. })))
        .collect();

    assert_eq!(lines.len(), 4);
    assert!(
        harness
            .model
            .undo_label()
            .is_some_and(|label| label.starts_with("Project the edges of"))
    );
    for line in &lines {
        let (start, end) = shown.line_endpoints(*line).unwrap();
        for end in [start, end] {
            assert!(
                [0.0, 40.0].iter().any(|edge| (end.x - edge).abs() < 1e-9)
                    && [0.0, 40.0].iter().any(|edge| (end.y - edge).abs() < 1e-9),
                "{end} is not a corner of the plate"
            );
        }
    }

    harness.click_pickable(Plane::XY, Point2::new(20.0, 20.0), top);
    harness.settle();

    assert!(harness.shows_containing("already projected into it"));
    assert_eq!(harness.sketch(sketch).projected().count(), projected.len());

    harness.key(Key::N, Modifiers::NONE);
    harness.frame();

    assert!(harness.shows_containing("Project "));
}

#[test]
fn the_intersect_tool_draws_where_the_sketch_cuts_a_face_the_whole_body_or_a_datum_plane() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    let mut transaction = harness.document().transaction("Datum");
    let datum = transaction.add_feature(
        "Plane 1",
        FeatureKind::Datum(caditor_document::Datum::Plane(
            caditor_document::DatumPlane {
                base: caditor_document::PlaneReference::Principal(
                    caditor_document::PrincipalPlane::Yz,
                ),
                rotation: None,
                offset: Expression::Measure(10.0, Unit::Millimetre),
            },
        )),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    let across = Plane::from_frame(Point3::new(0.0, 20.0, 0.0), -Vector3::Y, Vector3::X).unwrap();
    let sketch = harness.add_sketch(Sketch::new(across));
    harness.edit(sketch);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let lines = |harness: &Harness| {
        let shown = harness.shown(sketch);
        harness
            .sketch(sketch)
            .projected()
            .filter(|id| matches!(shown.entity(*id), Some(Entity::Line { .. })))
            .count()
    };

    harness.use_tool_with(Key::I, Modifiers::ALT);

    assert_eq!(harness.tool(), Some(Tool::Intersect));
    assert!(harness.shows_containing("Shift-click for its whole body"));
    assert!(
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| pickable == Pickable::Datum(datum))
    );

    harness.click_pickable(across, Point2::new(20.0, 5.0), top);
    harness.settle();

    assert_eq!(lines(&harness), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Intersect Extrude 1 end face")
    );

    harness.click_pickable(across, Point2::new(20.0, 5.0), top);
    harness.settle();

    assert!(harness.shows_containing(crate::projecting::ALREADY_INTERSECTED));

    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(top) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(top));
    harness.key(Key::Space, Modifiers::SHIFT);
    harness.frame();
    harness.settle();

    assert_eq!(lines(&harness), 4);
    assert_eq!(harness.model.undo_label(), Some("Intersect Extrude 1"));

    harness.click_pickable(across, Point2::new(10.0, 5.0), Pickable::Datum(datum));
    harness.settle();
    let along = harness
        .sketch(sketch)
        .projected()
        .find(|id| harness.sketch(sketch).is_construction(*id))
        .expect("the datum plane's cut is a construction line");
    let (start, end) = harness.shown(sketch).line_endpoints(along).unwrap();

    assert_eq!(lines(&harness), 5);
    assert!((across.to_world(start).x - 10.0).abs() < 1e-9);
    assert!((across.to_world(end).x - 10.0).abs() < 1e-9);
}

#[test]
fn the_intersect_tool_draws_where_a_principal_plane_crosses_the_sketch() {
    let mut harness = Harness::new();
    let across = Plane::from_frame(Point3::new(0.0, 20.0, 0.0), -Vector3::Y, Vector3::X).unwrap();
    let sketch = harness.add_sketch(Sketch::new(across));
    harness.edit(sketch);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let principal = |harness: &mut Harness, plane: caditor_document::PrincipalPlane| {
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| pickable == Pickable::Plane(plane))
    };

    assert!(!principal(
        &mut harness,
        caditor_document::PrincipalPlane::Xy
    ));

    harness.use_tool_with(Key::I, Modifiers::ALT);

    assert!(principal(
        &mut harness,
        caditor_document::PrincipalPlane::Xy
    ));
    assert!(principal(
        &mut harness,
        caditor_document::PrincipalPlane::Yz
    ));
    assert!(!principal(
        &mut harness,
        caditor_document::PrincipalPlane::Xz
    ));

    let level = Pickable::Plane(caditor_document::PrincipalPlane::Xy);
    harness.click_pickable(across, Point2::new(5.0, 0.0), level);
    harness.settle();
    let along = harness
        .sketch(sketch)
        .projected()
        .find(|id| harness.sketch(sketch).is_construction(*id))
        .expect("the principal plane's cut is a construction line");
    let (start, end) = harness.shown(sketch).line_endpoints(along).unwrap();

    assert_eq!(harness.model.undo_label(), Some("Intersect the XY plane"));
    assert!(across.to_world(start).z.abs() < 1e-9 && across.to_world(end).z.abs() < 1e-9);

    harness.click_pickable(across, Point2::new(5.0, 0.0), level);
    harness.settle();

    assert!(harness.shows_containing(crate::projecting::ALREADY_INTERSECTED));
}
