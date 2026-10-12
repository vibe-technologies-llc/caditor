use caditor_document::FeatureId;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::{Event, Id, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, constraint_of_kind, constraints_of_kind, edit_base_sketch,
    edit_free_sketch, entities_of_kind, entity_pickables, line_ends, near, only_constraint,
    run_from_palette, sketch_entity, type_point,
};
use crate::{annotations, dimensioning, editing::Tool, selection::Pickable};

fn add_distance_to_a_line(harness: &mut Harness) -> (FeatureId, EntityId, Id) {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 40.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);

    harness.key(Key::D, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    let (constraint, _) = only_constraint(harness.sketch(feature));
    harness.frame();
    (feature, line, annotations::field_id(feature, constraint))
}

fn dimension_text(harness: &Harness, feature: FeatureId) -> String {
    let (_, constraint) = only_constraint(harness.sketch(feature));
    harness
        .document()
        .expression_text(constraint.dimension().unwrap())
}

#[test]
fn a_new_distance_opens_its_field_on_the_canvas_with_the_measured_value_selected() {
    let mut harness = Harness::new();
    let (feature, line, field) = add_distance_to_a_line(&mut harness);
    let (start, end) = line_ends(harness.sketch(feature), line);
    assert_eq!(
        only_constraint(harness.sketch(feature)).1,
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(50.0, Unit::Millimetre),
        }
    );
    assert_eq!(harness.focused(), Some(field));

    harness.events.push(Event::Text("width / 2".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    assert_eq!(dimension_text(&harness, feature), "width / 2");
    assert_eq!(harness.model.undo_label(), Some("Edit dimension in Plate"));
    harness.settle();
    assert!(harness.shows("width / 2 = 20 mm"));
    assert_ne!(harness.focused(), Some(field));
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.distance(end) - 20.0).abs() < 1e-6);
    assert_eq!(harness.editing(), Some(feature));
}

#[test]
fn an_invalid_dimension_keeps_its_text_and_escape_reverts_it() {
    let mut harness = Harness::new();
    let (feature, line, field) = add_distance_to_a_line(&mut harness);

    harness.events.push(Event::Text("5 deg".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(harness.shows("It gives an angle, but a length is needed"));
    assert!(harness.shows("5 deg"));
    assert_eq!(harness.focused(), Some(field));
    assert_eq!(dimension_text(&harness, feature), "50 mm");

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(!harness.shows("It gives an angle, but a length is needed"));
    assert!(!harness.shows("5 deg"));
    assert!(harness.shows("50 mm"));
    assert_eq!(dimension_text(&harness, feature), "50 mm");
    assert_eq!(harness.editing(), Some(feature));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchEntity {
                feature,
                entity: line
            })
    );
}

#[test]
fn double_clicking_a_dimension_label_selects_it_and_opens_its_field() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let distance = constraint_of_kind(harness.sketch(base), "Distance");
    assert!(harness.shows("width = 40 mm"));

    harness.double_click("width = 40 mm");
    let pickable = Pickable::SketchConstraint {
        feature: base,
        constraint: distance,
    };
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![pickable]
    );
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(annotations::field_id(base, distance))
    );

    harness.events.push(Event::Text("width / 4".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();
    assert!(harness.shows("width / 4 = 10 mm"));
}

#[test]
fn a_horizontal_distance_between_two_points_is_a_dimension_edited_on_the_canvas() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let left = sketch.add_point(Point2::new(0.0, 0.0));
    let right = sketch.add_point(Point2::new(30.0, 12.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select(entity_pickables(feature, &[left, right]));

    harness.key(Key::X, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    let (constraint, added) = only_constraint(harness.sketch(feature));
    assert_eq!(
        added,
        Constraint::HorizontalDistance {
            from: left,
            to: right,
            value: Expression::Measure(30.0, Unit::Millimetre),
        }
    );
    assert_eq!(harness.model.undo_label(), Some("Add Horizontal distance"));
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(annotations::field_id(feature, constraint))
    );

    harness.events.push(Event::Text("width / 2".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert!(harness.shows("width / 2 = 20 mm"));
    let shown = harness.shown(feature);
    let (from, to) = (shown.point(left).unwrap(), shown.point(right).unwrap());
    assert!(((to.x - from.x) - 20.0).abs() < 1e-6, "{from} {to}");
    assert!(((to.y - from.y) - 12.0).abs() < 1e-6, "{from} {to}");
    assert!(harness.shows("Horizontal distance between Point 0 and Point 1"));
}

#[test]
fn the_smart_dimension_takes_a_line_clicked_after_it_and_enter_dimensions_its_length() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 40.0));
    let other = sketch.add_point(Point2::new(40.0, 0.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    let (start, end) = line_ends(harness.sketch(feature), line);
    let label = harness.sketch(feature).entity_label(line);
    harness.select([sketch_entity(feature, other)]);

    harness.use_tool(Key::D);
    assert_eq!(harness.tool(), Some(Tool::Dimension));
    assert!(harness.workspace.viewport.selection().is_empty());
    assert!(harness.shows(dimensioning::PICK_FIRST));

    harness.hover_pickable(
        Plane::XY,
        Point2::new(15.0, 20.0),
        sketch_entity(feature, line),
    );
    harness.frame();
    assert!(harness.shows(&format!(
        "Click to pick {label}, then press Enter for the length of {label} or click a second item"
    )));
    harness.click_pickable(
        Plane::XY,
        Point2::new(15.0, 20.0),
        sketch_entity(feature, line),
    );
    harness.frame();
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(sketch_entity(feature, line))
    );
    assert!(harness.shows(&format!(
        "Click a second item to dimension against it, or press Enter for the length of {label}"
    )));
    assert_eq!(harness.sketch(feature).constraints().count(), 0);

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let (constraint, added) = only_constraint(harness.sketch(feature));
    assert_eq!(
        added,
        Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(50.0, Unit::Millimetre),
        }
    );
    assert_eq!(harness.model.undo_label(), Some("Add Distance"));
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.tool(), Some(Tool::Dimension));
    harness.frame();
    let field = annotations::field_id(feature, constraint);
    assert_eq!(harness.focused(), Some(field));

    harness.type_into_field(field, "25");
    harness.settle();
    let (from, to) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((from.distance(to) - 25.0).abs() < 1e-6);
}

#[test]
fn the_smart_dimension_dimensions_two_lines_by_their_angle_and_a_circle_by_a_click_on_nothing() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let level = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0));
    let slanted = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(20.0, 25.0));
    let circle = sketch.add_circle(Point2::new(50.0, 25.0), 6.0);
    let spline = sketch.add_spline(&[Point2::new(0.0, 30.0), Point2::new(10.0, 35.0)]);
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::D);

    harness.click_pickable(
        Plane::XY,
        Point2::new(20.0, 0.0),
        sketch_entity(feature, level),
    );
    harness.frame();
    let level_label = harness.sketch(feature).entity_label(level);
    let slanted_label = harness.sketch(feature).entity_label(slanted);
    harness.hover_pickable(
        Plane::XY,
        Point2::new(10.0, 15.0),
        sketch_entity(feature, slanted),
    );
    harness.frame();
    assert!(harness.shows(&format!(
        "Click to dimension the angle between {level_label} and {slanted_label}"
    )));
    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 15.0),
        sketch_entity(feature, slanted),
    );
    harness.frame();
    harness.frame();
    let angles = constraints_of_kind(harness.sketch(feature), "Angle");
    let [Constraint::Angle { value, .. }] = &angles[..] else {
        panic!("one angle is added: {angles:?}");
    };
    assert_eq!(*value, Expression::Measure(45.0, Unit::Degree));
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Dimension));

    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 32.5),
        sketch_entity(feature, spline),
    );
    harness.frame();
    assert_eq!(harness.workspace.viewport.selection().iter().count(), 1);
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.tool(), Some(Tool::Dimension));

    harness.click_pickable(
        Plane::XY,
        Point2::new(56.0, 25.0),
        sketch_entity(feature, circle),
    );
    harness.frame();
    harness.click_at(Point2::new(30.0, 15.0));
    harness.frame();
    harness.frame();
    let diameters = constraints_of_kind(harness.sketch(feature), "Diameter");
    assert!(
        matches!(
            &diameters[..],
            [Constraint::Diameter { entity, value }]
                if *entity == circle && *value == Expression::Measure(12.0, Unit::Millimetre)
        ),
        "{diameters:?}"
    );
}

#[test]
fn the_smart_dimension_places_two_points_horizontally_and_an_arc_by_its_length() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(0.0, 0.0));
    let second = sketch.add_point(Point2::new(30.0, 20.0));
    let arc = sketch.add_arc(
        Point2::new(60.0, 0.0),
        Point2::new(70.0, 0.0),
        Point2::new(60.0, 10.0),
    );
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::D);

    harness.click_pickable(Plane::XY, Point2::ZERO, sketch_entity(feature, first));
    harness.frame();
    harness.click_pickable(
        Plane::XY,
        Point2::new(30.0, 20.0),
        sketch_entity(feature, second),
    );
    harness.frame();
    let waiting = harness.sketch(feature).constraints().count();
    harness.point_at(Point2::new(15.0, 35.0));
    let horizontal_prompt = harness.shows_containing("here the horizontal distance between");
    harness.click_at(Point2::new(15.0, 35.0));
    harness.frame();
    harness.frame();
    let horizontal = constraints_of_kind(harness.sketch(feature), "Horizontal distance");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.click_pickable(
        Plane::XY,
        Point2::new(67.07, 7.07),
        sketch_entity(feature, arc),
    );
    harness.frame();
    harness.click_at(Point2::new(72.0, 12.0));
    harness.frame();
    harness.frame();
    let lengths = constraints_of_kind(harness.sketch(feature), "Arc length");

    assert_eq!(waiting, 0);
    assert!(horizontal_prompt);
    let [Constraint::HorizontalDistance { value, .. }] = &horizontal[..] else {
        panic!("one horizontal distance is added: {horizontal:?}");
    };
    assert_eq!(*value, Expression::Measure(30.0, Unit::Millimetre));
    assert_eq!(lengths.len(), 1, "{lengths:?}");
}

#[test]
fn the_smart_dimension_works_from_the_keyboard_and_escape_lets_go_before_leaving() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(0.0, 0.0));
    let second = sketch.add_point(Point2::new(30.0, 40.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::D);
    let highlight = |harness: &mut Harness, entity: EntityId| {
        let wanted = sketch_entity(feature, entity);
        for _ in 0..20 {
            if harness.workspace.viewport.keyboard_highlight() == Some(wanted) {
                break;
            }
            harness.key(Key::N, Modifiers::NONE);
            harness.frame();
        }
        assert_eq!(
            harness.workspace.viewport.keyboard_highlight(),
            Some(wanted)
        );
        harness.key(Key::Space, Modifiers::NONE);
        harness.frame();
        harness.frame();
    };

    highlight(&mut harness, first);
    let first_label = harness.sketch(feature).entity_label(first);
    assert!(harness.shows(&format!(
        "Click a second point, a line or a circle for its distance from {first_label}"
    )));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.tool(), Some(Tool::Dimension));

    highlight(&mut harness, first);
    highlight(&mut harness, second);
    let placing = harness.shows_containing("Click above or below for the horizontal distance");
    let waiting = harness.sketch(feature).constraints().count();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    assert!(placing);
    assert_eq!(waiting, 0);
    let (_, added) = only_constraint(harness.sketch(feature));
    assert_eq!(
        added,
        Constraint::Distance {
            from: first,
            to: second,
            value: Expression::Measure(50.0, Unit::Millimetre),
        }
    );

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    run_from_palette(&mut harness, "smart dimension");
    harness.frame();
    assert_eq!(harness.tool(), Some(Tool::Dimension));
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
    }
    harness.frame();
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), None);
    assert_eq!(harness.tool(), Some(Tool::Select));
}

#[test]
fn a_diameter_is_labelled_with_its_sign_and_sets_the_circle() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let circle = sketch.add_circle(Point2::new(10.0, 10.0), 5.0);
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.select(entity_pickables(feature, &[circle]));

    harness.click_button("Diameter");
    harness.frame();
    let (constraint, _) = only_constraint(harness.sketch(feature));
    harness.frame();
    harness.type_into_field(annotations::field_id(feature, constraint), "14");
    harness.settle();

    assert!(harness.shows("Ø 14"));
    assert!(harness.shows("Diameter of Circle 1"));
    let (_, radius) = harness.shown(feature).circle(circle).unwrap();
    assert!((radius - 7.0).abs() < 1e-9, "{radius}");
}

#[test]
fn the_smart_dimension_placed_across_a_construction_axis_holds_the_diameter_as_a_lathe_drawing_shows_it()
 {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let axis = sketch.add_line(Point2::new(0.0, -20.0), Point2::new(0.0, 20.0));
    sketch.set_construction(axis, true).unwrap();
    let (start, end) = line_ends(&sketch, axis);
    for (point, at) in [
        (start, Point2::new(0.0, -20.0)),
        (end, Point2::new(0.0, 20.0)),
    ] {
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }
    let rim = sketch.add_point(Point2::new(10.0, 5.0));
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.use_tool(Key::D);

    harness.click_pickable(
        Plane::XY,
        Point2::new(10.0, 5.0),
        sketch_entity(feature, rim),
    );
    harness.frame();
    harness.click_pickable(
        Plane::XY,
        Point2::new(0.0, 15.0),
        sketch_entity(feature, axis),
    );
    harness.frame();
    let waiting = harness.sketch(feature).constraints().count();
    harness.point_at(Point2::new(-8.0, 10.0));
    let prompted = harness.shows_containing("here the diameter of");
    harness.click_at(Point2::new(-8.0, 10.0));
    harness.frame();
    harness.frame();
    let added = constraints_of_kind(harness.sketch(feature), "Diameter across");
    let (constraint, _) = harness
        .sketch(feature)
        .constraints()
        .last()
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    harness.type_into_field(annotations::field_id(feature, constraint), "30");
    harness.settle();

    assert_eq!(waiting, 2);
    assert!(prompted);
    assert!(
        matches!(added[..], [Constraint::AxisDiameter { point, axis: across, .. }] if point == rim && across == axis),
        "{added:?}"
    );
    assert!(harness.shows("Ø 30"));
    let moved = harness.shown(feature).point(rim).unwrap();
    assert!((moved.x - 15.0).abs() < 1e-9, "{moved}");
}

#[test]
fn typed_values_are_kept_as_dimensions_unless_turned_off() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    type_point(&mut harness, "10, 0");
    type_point(&mut harness, "@30 < 45");
    type_point(&mut harness, "@-5, 8");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let kept: Vec<String> = harness
        .sketch(feature)
        .constraints()
        .filter(|(_, constraint)| constraint.dimension().is_some())
        .map(|(id, _)| harness.sketch(feature).describe_constraint(id))
        .collect();

    run_from_palette(&mut harness, "Keep typed values as dimensions");
    harness.frame();
    let before = harness.sketch(feature).constraints().len();
    type_point(&mut harness, "50, 50");
    type_point(&mut harness, "@10, 0");
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    let dimensions = harness
        .sketch(feature)
        .constraints()
        .filter(|(_, constraint)| constraint.dimension().is_some())
        .count();

    assert_eq!(
        kept,
        [
            "Horizontal distance between Origin and Point 0",
            "Distance between Point 0 and Point 1",
            "Angle between Horizontal axis and Line 2",
            "Horizontal distance between Point 6 and Point 8",
            "Vertical distance between Point 6 and Point 8",
        ]
    );
    assert!(harness.sketch(feature).constraints().len() >= before);
    assert_eq!(dimensions, 5);
    let lines = entities_of_kind(harness.sketch(feature), "Line");
    let ends: Vec<Point2> = lines
        .iter()
        .take(2)
        .map(|line| harness.sketch(feature).line_endpoints(*line).unwrap().1)
        .collect();
    let corner = Point2::new(10.0, 0.0) + Vector2::from_angle(45f64.to_radians()) * 30.0;
    assert!(near(ends[0], corner), "{}", ends[0]);
    assert!(
        near(ends[1], corner + Vector2::new(-5.0, 8.0)),
        "{}",
        ends[1]
    );
}
