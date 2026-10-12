use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Key, Modifiers};

use super::{
    DRAWN, Harness, close, constraints_of_kind, edit_free_sketch, entities_of_kind,
    entity_pickables, fillet_arc, mirror_fixture, near, radii_of_circles, rectangle,
    run_from_palette, type_point,
};
use crate::{editing::Tool, filleting, mirroring, offsetting, selection::Pickable, sketch_tools};

#[test]
fn offset_takes_the_clicked_chain_and_runs_where_the_pointer_is_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let bottom = entities_of_kind(&sketch, "Line")[0];
    let bottom_label = sketch.entity_label(bottom);
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::W);
    assert_eq!(harness.tool(), Some(Tool::Offset));
    assert!(harness.shows(offsetting::CHOOSE_PROMPT));
    harness.point_at(Point2::new(20.0, 0.0));
    assert!(harness.shows(&format!("Take the chain of {bottom_label} to offset")));
    harness.click_at(Point2::new(20.0, 0.0));
    assert_eq!(
        sketch_tools::selected_entities(harness.workspace.viewport.selection(), feature).len(),
        4
    );
    assert!(harness.shows(offsetting::PROMPT));

    harness.point_at(Point2::new(20.0, -5.0));
    assert!(harness.shows("Offset 4 curves by 5 mm"));
    harness.click_at(Point2::new(20.0, -5.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 8);
    assert_eq!(constraints_of_kind(sketch, "Distance").len(), 4);
    let corners: Vec<Point2> = entities_of_kind(sketch, "Line")
        .into_iter()
        .filter_map(|line| sketch.line_endpoints(line))
        .map(|(start, _)| start)
        .collect();
    for expected in [
        Point2::new(-5.0, -5.0),
        Point2::new(45.0, -5.0),
        Point2::new(45.0, 25.0),
        Point2::new(-5.0, 25.0),
    ] {
        assert!(
            corners.iter().any(|corner| near(*corner, expected)),
            "{expected} in {corners:?}"
        );
    }
    assert_eq!(harness.model.undo_label(), Some(offsetting::TRANSACTION));
    assert_eq!(harness.tool(), Some(Tool::Offset));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn offset_by_a_typed_distance_needs_no_pointer_and_a_negative_one_goes_the_other_way() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::ZERO, 10.0);
    let circle_label = sketch.entity_label(circle);
    let spline = sketch.add_spline(&[Point2::new(30.0, 0.0), Point2::new(40.0, 10.0)]);
    let spline_label = sketch.entity_label(spline);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[circle]));
    run_from_palette(&mut harness, "offset sketch curves");
    assert_eq!(harness.tool(), Some(Tool::Offset));
    type_point(&mut harness, "3");
    assert!(close(
        &radii_of_circles(harness.sketch(feature)),
        &[10.0, 13.0]
    ));
    assert_eq!(harness.model.undo_label(), Some(offsetting::TRANSACTION));

    type_point(&mut harness, "-2");
    assert!(close(
        &radii_of_circles(harness.sketch(feature)),
        &[8.0, 10.0, 13.0]
    ));

    type_point(&mut harness, "-12");
    assert!(harness.shows(&format!(
        "Offset: offsetting {circle_label} this far would shrink it to nothing."
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(radii_of_circles(harness.sketch(feature)).len(), 3);

    harness.select(entity_pickables(feature, &[spline]));
    harness.frame();
    assert!(harness.shows(&format!(
        "{spline_label} cannot be offset; only lines, arcs and circles can"
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn mirror_copies_the_selection_about_the_clicked_line_and_keeps_it_symmetric() {
    let mut harness = Harness::new();
    let (sketch, mirror, line, circle) = mirror_fixture();
    let mirror_label = sketch.entity_label(mirror);
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.use_tool(Key::Y);
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    assert!(harness.shows(mirroring::SELECT_FIRST));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert_eq!(
        harness.model.notice().unwrap().text,
        "Mirror: select what to mirror first: click it or drag a box around it."
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));

    harness.select(entity_pickables(feature, &[line, circle]));
    harness.use_tool(Key::Y);
    assert!(harness.shows(mirroring::PROMPT));
    harness.point_at(Point2::new(0.0, 20.0));
    assert!(harness.shows(&format!("Mirror 2 items about {mirror_label}")));
    harness.click_at(Point2::new(0.0, 20.0));

    let sketch = harness.sketch(feature);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 3);
    assert_eq!(entities_of_kind(sketch, "Circle").len(), 2);
    let copy = entities_of_kind(sketch, "Line")
        .into_iter()
        .find(|id| ![mirror, line].contains(id))
        .unwrap();
    let (start, end) = sketch.line_endpoints(copy).unwrap();
    assert!(near(start, Point2::new(-5.0, 0.0)) && near(end, Point2::new(-15.0, 10.0)));
    assert_eq!(constraints_of_kind(sketch, "Symmetric").len(), 3);
    assert_eq!(constraints_of_kind(sketch, "Equal").len(), 1);
    assert_eq!(harness.model.undo_label(), Some(mirroring::TRANSACTION));

    harness.settle();
    assert!(
        harness
            .model
            .evaluation()
            .feature(feature)
            .is_some_and(|status| status.state == caditor_document::FeatureState::UpToDate)
    );
}

#[test]
fn mirror_steps_through_lines_and_axes_from_the_keyboard() {
    let mut harness = Harness::new();
    let (sketch, _, line, circle) = mirror_fixture();
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select(entity_pickables(feature, &[line, circle]));
    run_from_palette(&mut harness, "mirror sketch geometry");
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Mirror 2 items about Horizontal axis"));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows("Mirror 2 items about Vertical axis"));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();

    let sketch = harness.sketch(feature);
    let centres: Vec<Point2> = entities_of_kind(sketch, "Circle")
        .into_iter()
        .filter_map(|circle| sketch.circle(circle))
        .map(|(centre, _)| centre)
        .collect();
    assert!(
        centres
            .iter()
            .any(|centre| near(*centre, Point2::new(-10.0, -10.0)))
    );
    assert!(
        constraints_of_kind(sketch, "Symmetric")
            .iter()
            .all(|constraint| matches!(
                constraint,
                Constraint::Symmetric { about, .. } if *about == EntityId::VERTICAL_AXIS
            ))
    );

    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Mirror));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_sketch_fillet_rounds_the_clicked_corner_with_a_typed_radius_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lines = entities_of_kind(&sketch, "Line");
    let [first, second] = [lines[0], lines[1]].map(|line| sketch.entity_label(line));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool(Key::B);
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    assert!(harness.shows(filleting::CORNER_PROMPT));
    harness.point_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(&format!("Round the corner of {first} and {second}")));
    harness.click_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));

    type_point(&mut harness, "30");
    assert!(harness.shows(&format!(
        "Sketch fillet: the radius is too large for {second}."
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(fillet_arc(harness.sketch(feature)).is_none());

    type_point(&mut harness, "5");
    let sketch = harness.sketch(feature);
    let arc = fillet_arc(sketch).unwrap();
    assert!(near(arc.center, Point2::new(35.0, 5.0)));
    assert!((arc.radius - 5.0).abs() < DRAWN);
    assert_eq!(constraints_of_kind(sketch, "Tangent").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Radius").len(), 1);
    assert_eq!(harness.model.undo_label(), Some(filleting::TRANSACTION));
    assert!(harness.shows(filleting::CORNER_PROMPT));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_sketch_chamfer_cuts_the_clicked_corner_with_a_typed_distance_kept_as_dimensions() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lines = entities_of_kind(&sketch, "Line");
    let [first, second] = [lines[0], lines[1]].map(|line| sketch.entity_label(line));
    let feature = edit_free_sketch(&mut harness, sketch);
    let before = harness.sketch(feature).clone();

    harness.use_tool_with(Key::B, Modifiers::SHIFT);
    assert_eq!(harness.tool(), Some(Tool::Chamfer));
    assert!(harness.shows(filleting::CHAMFER_CORNER_PROMPT));
    harness.point_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(&format!("Cut the corner of {first} and {second}")));
    harness.click_at(Point2::new(40.0, 0.0));
    assert!(harness.shows(filleting::DISTANCE_PROMPT));

    type_point(&mut harness, "25");
    assert!(harness.shows(&format!(
        "Sketch chamfer: the chamfer distance is too large for {second}."
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();

    type_point(&mut harness, "5");
    let sketch = harness.sketch(feature);
    let lines = entities_of_kind(sketch, "Line");
    assert_eq!(lines.len(), 5);
    assert!(
        lines
            .iter()
            .filter_map(|line| sketch.line_endpoints(*line))
            .any(|(start, end)| near(start, Point2::new(35.0, 0.0))
                && near(end, Point2::new(40.0, 5.0)))
    );
    assert_eq!(constraints_of_kind(sketch, "Distance").len(), 2);
    assert_eq!(
        harness.model.undo_label(),
        Some(filleting::CHAMFER_TRANSACTION)
    );

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(harness.sketch(feature).same_content(&before));
}

#[test]
fn a_line_is_split_at_the_selected_point_on_it_from_the_palette() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
    let point = sketch.add_point(Point2::new(15.0, 0.0));
    sketch
        .add_constraint(Constraint::Coincident(point, line))
        .unwrap();
    let off = sketch.add_point(Point2::new(15.0, 5.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.select([Pickable::SketchEntity {
        feature,
        entity: off,
    }]);
    run_from_palette(&mut harness, "split the selected curve");
    harness.settle();
    let refused = entities_of_kind(harness.sketch(feature), "Line").len() == 1
        && harness.shows_containing(crate::sketch_tools::NOTHING_TO_SPLIT);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.select([Pickable::SketchEntity {
        feature,
        entity: point,
    }]);
    run_from_palette(&mut harness, "split the selected curve");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(refused);
    assert_eq!(entities_of_kind(sketch, "Line").len(), 2);
    assert_eq!(constraints_of_kind(sketch, "Collinear").len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some(crate::sketch_tools::SPLIT_TITLE)
    );
}

#[test]
fn a_sketch_fillet_takes_its_radius_from_the_pointer_and_escape_backs_out_a_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    harness.click_button("Sketch fillet");
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    harness.click_at(Point2::new(0.0, 20.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(filleting::CORNER_PROMPT));
    assert_eq!(harness.tool(), Some(Tool::Fillet));

    harness.click_at(Point2::new(40.0, 0.0));
    harness.point_at(Point2::new(38.0, 2.0));
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    harness.click_at(Point2::new(38.0, 2.0));
    let arc = fillet_arc(harness.sketch(feature)).unwrap();
    let middle = arc.point_at(arc.start_angle + arc.sweep / 2.0);
    assert!(middle.distance(Point2::new(38.0, 2.0)) < 0.05, "{middle}");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_sketch_fillet_is_chosen_typed_and_placed_from_the_keyboard() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::ZERO, Point2::new(40.0, 20.0));
    let lines = entities_of_kind(&sketch, "Line");
    let [first, second] = [lines[0], lines[1]].map(|line| sketch.entity_label(line));
    let feature = edit_free_sketch(&mut harness, sketch);

    run_from_palette(&mut harness, "fillet a sketch corner");
    assert_eq!(harness.tool(), Some(Tool::Fillet));
    harness.key(Key::N, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(&format!("Round the corner of {first} and {second}")));
    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    assert!(harness.shows(filleting::RADIUS_PROMPT));
    type_point(&mut harness, "4");

    let arc = fillet_arc(harness.sketch(feature)).unwrap();
    assert!((arc.radius - 4.0).abs() < DRAWN);
    assert_eq!(harness.model.undo_label(), Some(filleting::TRANSACTION));
}

#[test]
fn the_modify_tools_name_their_keys_and_what_they_do() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    for (name, keys) in [("Offset", "W"), ("Mirror", "Y"), ("Sketch fillet", "B")] {
        harness.hover_button(name);
        assert!(
            harness
                .texts
                .iter()
                .any(|(text, _)| text.ends_with(&format!("({keys})"))),
            "{name}"
        );
    }
}
