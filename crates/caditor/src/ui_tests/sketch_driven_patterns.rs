use caditor_document::{CurvePatternPath, CurveSpacing, PatternKind};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;

use super::{
    Harness, extruded_plate, pattern_of, run_from_palette, top_edge_along_x, volume_about,
};
use crate::{
    model::Action,
    pattern_panel,
    selection::{Axis, Pickable, PrincipalPlane},
    sketch_pattern_tools,
};

#[test]
fn patterns_along_a_sketch_curve_and_at_its_points_come_from_the_palette_and_their_panels() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    line.add_line(Point2::new(0.0, 0.0), Point2::new(200.0, 0.0));
    let path = harness.add_sketch(line);

    harness.select([]);
    run_from_palette(&mut harness, "curve pattern");
    harness.settle();
    harness.let_animations_finish();
    let along = harness
        .workspace
        .editing
        .solid()
        .expect("the curve pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Curve pattern 1"));
    let PatternKind::Curve(curve) = &pattern_of(&harness, along).kind else {
        panic!("a curve pattern follows a curve");
    };
    assert_eq!(curve.path.sketch(), Some(path));
    assert_eq!(pattern_of(&harness, along).body, plate);
    assert!(volume_about(&harness, plate, 4.0 * 16000.0));
    assert!(harness.shows(pattern_panel::CURVE));
    assert!(harness.shows(pattern_panel::CURVE_HINT));

    harness.click(pattern_panel::BY_DISTANCE);
    harness.settle();
    let PatternKind::Curve(curve) = &pattern_of(&harness, along).kind else {
        panic!("the pattern stays along the curve");
    };
    assert_eq!(curve.measured, CurveSpacing::Distance);
    assert!(harness.shows("Spacing"));
    assert!(volume_about(&harness, plate, 28000.0));

    let mut points = Sketch::new(Plane::XY);
    points.add_point(Point2::new(100.0, 100.0));
    points.add_point(Point2::new(-100.0, 100.0));
    let spots = harness.add_sketch(points);
    harness.select([]);
    run_from_palette(&mut harness, "point pattern");
    harness.settle();
    let at_points = harness
        .workspace
        .editing
        .solid()
        .expect("the point pattern is open");
    assert_eq!(harness.model.undo_label(), Some("Create Point pattern 1"));
    assert_eq!(pattern_of(&harness, at_points).kind.sketch(), Some(spots));
    assert!(harness.shows(pattern_panel::BASE_POINT));
    assert!(volume_about(&harness, plate, 3.0 * 28000.0));

    harness.perform(Action::Undo);
    harness.settle();
    assert!(volume_about(&harness, plate, 28000.0));
}

#[test]
fn sketch_driven_patterns_refuse_a_selection_naming_no_body_and_no_sketch() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    let mut line = Sketch::new(Plane::XY);
    line.add_line(Point2::new(0.0, 0.0), Point2::new(200.0, 0.0));
    harness.add_sketch(line);
    let mut points = Sketch::new(Plane::XY);
    points.add_point(Point2::new(100.0, 100.0));
    harness.add_sketch(points);
    let features = harness.document().features().count();

    for (selected, query) in [
        (Pickable::Axis(Axis::Y), "curve pattern"),
        (Pickable::Plane(PrincipalPlane::Xz), "curve pattern"),
        (Pickable::Axis(Axis::Y), "point pattern"),
        (Pickable::Plane(PrincipalPlane::Xz), "point pattern"),
    ] {
        harness.select([selected]);
        run_from_palette(&mut harness, query);
        harness.settle();

        assert_eq!(harness.document().features().count(), features);
        assert!(harness.workspace.editing.solid().is_none());
        assert!(
            harness
                .model
                .notice()
                .is_some_and(|notice| notice.text.contains("Select a face, edge or vertex")),
            "{query} with {selected:?}"
        );
    }
}

#[test]
fn a_curve_pattern_follows_the_selected_model_edges_and_its_panel_names_them() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let front = top_edge_along_x(&harness, plate, 0.0);
    let back = top_edge_along_x(&harness, plate, 40.0);

    harness.select([Pickable::Edge {
        body: plate,
        edge: front,
    }]);
    run_from_palette(&mut harness, "curve pattern");
    harness.settle();
    let along = harness
        .workspace
        .editing
        .solid()
        .expect("the curve pattern is open");
    let pattern = pattern_of(&harness, along).clone();
    let PatternKind::Curve(curve) = &pattern.kind else {
        panic!("a curve pattern follows a curve");
    };

    assert_eq!(harness.model.undo_label(), Some("Create Curve pattern 1"));
    assert_eq!(pattern.body, plate);
    assert_eq!(curve.path.body(), Some(plate));
    assert_eq!(curve.path.edges().len(), 1);
    assert!(volume_about(&harness, plate, 32000.0));
    assert!(harness.shows(pattern_panel::EDGES));
    assert!(harness.shows(pattern_panel::FOLLOWS_EDGES));
    assert!(harness.shows("1 edge of Extrude 1"));

    harness.select([Pickable::Edge {
        body: plate,
        edge: back,
    }]);
    let selection = harness.workspace.viewport.selection().clone();
    let transaction =
        sketch_pattern_tools::edges_change(&harness.model, &selection, along, &pattern).unwrap();
    harness.perform(Action::Apply(transaction));
    harness.settle();
    let PatternKind::Curve(curve) = &pattern_of(&harness, along).kind else {
        panic!("the pattern stays along the edges");
    };

    assert!(matches!(&curve.path, CurvePatternPath::Edges { edges, .. } if edges.len() == 2));
    assert!(harness.shows("2 edges of Extrude 1"));
}
