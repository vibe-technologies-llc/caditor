use caditor_document::FeatureId;
use caditor_kernel::FaceOrigin;
use egui::{Key, Modifiers};

use super::{Harness, extruded_plate, top_edge_along_x};
use crate::{bodies, selection::Pickable};

fn made_by(harness: &mut Harness, maker: FeatureId, wanted: fn(FaceOrigin) -> bool) -> Pickable {
    let built = harness.built();
    built
        .picks
        .pickables()
        .find(|pickable| {
            let Pickable::Face { body, face } = pickable else {
                return false;
            };
            bodies::shown(harness.model.evaluation(), *body)
                .and_then(|shown| bodies::face_origin(shown, *face))
                .is_some_and(|origin| origin.feature() == maker.raw() && wanted(origin))
        })
        .expect("the feature made a pickable face")
}

fn close_open_feature(harness: &mut Harness) -> FeatureId {
    let feature = harness
        .workspace
        .editing
        .solid()
        .expect("the feature is open");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.settle();
    harness.select([]);
    feature
}

fn marked_rows(harness: &mut Harness, face: Pickable, rows: [FeatureId; 2]) -> [bool; 2] {
    harness.select([face]);
    let panels = &harness.workspace.panels;
    rows.map(|row| panels.reads_selected(row))
}

#[test]
fn a_face_of_a_hole_marks_the_hole_row_and_not_the_extrusion_it_drills() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("Hole");
    harness.settle();
    let hole = close_open_feature(&mut harness);
    let bore = made_by(&mut harness, hole, |_| true);

    let marked = marked_rows(&mut harness, bore, [hole, extrude]);

    assert_eq!(marked, [true, false]);
    assert!(harness.workspace.panels.reads_body_selected(extrude));
}

#[test]
fn a_face_of_a_fillet_marks_the_fillet_row_and_not_the_extrusion_it_rounds() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let edge = top_edge_along_x(&harness, extrude, 0.0);
    harness.select([Pickable::Edge {
        body: extrude,
        edge,
    }]);
    harness.click("Fillet");
    harness.settle();
    let fillet = close_open_feature(&mut harness);
    let rounded = made_by(&mut harness, fillet, |origin| {
        matches!(origin, FaceOrigin::Fillet { .. })
    });

    let marked = marked_rows(&mut harness, rounded, [fillet, extrude]);

    assert_eq!(marked, [true, false]);
    assert!(harness.workspace.panels.reads_body_selected(extrude));
}

#[test]
fn a_face_of_a_chamfer_marks_the_chamfer_row_and_not_the_extrusion_it_cuts() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let edge = top_edge_along_x(&harness, extrude, 0.0);
    harness.select([Pickable::Edge {
        body: extrude,
        edge,
    }]);
    harness.click("Chamfer");
    harness.settle();
    let chamfer = close_open_feature(&mut harness);
    let cut = made_by(&mut harness, chamfer, |origin| {
        matches!(origin, FaceOrigin::Chamfer { .. })
    });

    let marked = marked_rows(&mut harness, cut, [chamfer, extrude]);

    assert_eq!(marked, [true, false]);
    assert!(harness.workspace.panels.reads_body_selected(extrude));
}

#[test]
fn a_face_of_the_extrusion_itself_still_marks_the_extrusion_row() {
    let mut harness = Harness::new();
    let (extrude, top) = extruded_plate(&mut harness);

    harness.select([top]);

    assert!(harness.workspace.panels.reads_selected(extrude));
    assert!(harness.workspace.panels.reads_body_selected(extrude));
}
