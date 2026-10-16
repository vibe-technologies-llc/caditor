use caditor_document::Feature;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, Sketch};
use egui::{Event, Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, constraints_of_kind, entities_of_kind, extruded_plate, feature_named,
    near, offer, run_from_palette, type_point,
};
use crate::{
    commands::Command,
    selection::{Pickable, PrincipalPlane},
    typed_point,
};

#[test]
fn space_while_drawing_starts_a_line_at_the_highlighted_point() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let corner = sketch.add_point(Point2::new(10.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.use_tool(Key::L);
    let point = Pickable::SketchEntity {
        feature,
        entity: corner,
    };

    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(point) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(point));

    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();
    type_point(&mut harness, "@20 mm, 0");

    let lines = entities_of_kind(harness.sketch(feature), "Line");
    assert_eq!(lines.len(), 1);
    assert!(
        constraints_of_kind(harness.sketch(feature), "Coincident")
            .iter()
            .any(|constraint| matches!(
                constraint,
                Constraint::Coincident(a, b) if *a == corner || *b == corner
            ))
    );
    assert!(!harness.workspace.viewport.selection().contains(point));
}

#[test]
fn space_while_drawing_starts_a_line_at_the_middle_of_the_highlighted_line() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.use_tool(Key::L);
    let line = Pickable::SketchEntity {
        feature,
        entity: base,
    };

    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(line) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(line));

    harness.key(Key::Space, Modifiers::NONE);
    harness.frame();
    harness.frame();
    type_point(&mut harness, "@0, 15 mm");

    let sketch = harness.sketch(feature);
    let drawn = *entities_of_kind(sketch, "Line")
        .iter()
        .find(|candidate| **candidate != base)
        .expect("a second line is drawn");
    let (start, _) = sketch.line_endpoints(drawn).unwrap();
    assert!(near(start, Point2::new(10.0, 0.0)), "{start}");
    assert!(
        constraints_of_kind(sketch, "Midpoint")
            .iter()
            .any(|constraint| matches!(
                constraint,
                Constraint::Midpoint { curve, .. } if *curve == base
            ))
    );
}

#[test]
fn the_highlight_keys_reach_a_dimension_and_enter_edits_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0));
    let Some(&Entity::Line { start, end }) = sketch.entity(line) else {
        panic!("a line");
    };
    let length = sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(20.0, Unit::Millimetre),
        })
        .unwrap();
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();
    let dimension = Pickable::SketchConstraint {
        feature,
        constraint: length,
    };

    for _ in 0..30 {
        if harness.workspace.viewport.keyboard_highlight() == Some(dimension) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(
        harness.workspace.viewport.keyboard_highlight(),
        Some(dimension)
    );
    assert!(harness.shows_containing("Distance between"));

    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.frame();
    harness.events.push(Event::Text("35".to_owned()));
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.settle();

    assert_eq!(
        harness
            .sketch(feature)
            .constraint(length)
            .and_then(Constraint::dimension),
        Some(&Expression::Number(35.0))
    );
}

#[test]
fn a_part_can_be_modelled_from_the_keyboard_alone() {
    let mut harness = Harness::new();
    run_from_palette(&mut harness, "new sketch");
    assert!(harness.workspace.editing.is_choosing_plane());

    let xy = Pickable::Plane(PrincipalPlane::Xy);
    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight() == Some(xy) {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(harness.workspace.viewport.keyboard_highlight(), Some(xy));
    assert!(harness.shows("XY plane"));
    harness.key(Key::Space, Modifiers::NONE);
    harness.show_new_windows();
    harness.settle();
    let sketch = harness
        .editing()
        .expect("a sketch on the XY plane is being edited");

    harness.use_tool(Key::R);
    type_point(&mut harness, "5, 5");
    assert!(harness.workspace.viewport.is_drawing());
    assert!(!harness.shows(typed_point::FIELD_LABEL));
    type_point(&mut harness, "5, nowhere");
    assert!(harness.shows(typed_point::FIELD_LABEL));
    assert!(
        harness
            .texts
            .iter()
            .any(|(shown, _)| shown.starts_with("y: "))
    );
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(!harness.shows(typed_point::FIELD_LABEL));
    assert!(harness.workspace.viewport.is_drawing());
    type_point(&mut harness, "@2000 m, 5");
    assert!(harness.shows("Keep the point within 1000 m of the sketch's origin"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    type_point(&mut harness, "@20 mm, 0");
    assert!(harness.shows("A rectangle needs its corners apart in both directions"));
    assert!(entities_of_kind(harness.sketch(sketch), "Line").is_empty());
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert!(harness.workspace.viewport.is_drawing());
    type_point(&mut harness, "@20 mm, width / 4");
    assert_eq!(entities_of_kind(harness.sketch(sketch), "Line").len(), 4);

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    assert_eq!(harness.editing(), None);

    run_from_palette(&mut harness, "extrude");
    harness.settle();
    let extrude = harness
        .document()
        .features()
        .find(|feature| feature.name == "Extrude 1")
        .map(Feature::id)
        .expect("the extrusion was created");
    assert!((harness.body_volume(extrude) - 2000.0).abs() < 1.0);

    harness.key(Key::Num2, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let forward = harness.workspace.viewport.viewpoint().forward();
    assert!(
        forward.dot(caditor_geometry::Vector3::NEG_Z) > 0.999,
        "{forward:?}"
    );
    harness.key(Key::ArrowUp, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let turned = harness.workspace.viewport.viewpoint().forward();
    assert!(turned.dot(forward) < 0.9999);
}

#[test]
fn holding_a_key_repeats_only_commands_that_should_repeat() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    for _ in 0..20 {
        if harness.workspace.viewport.keyboard_highlight().is_some() {
            break;
        }
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
    }
    let target = harness.workspace.viewport.keyboard_highlight().unwrap();
    for repeat in [false, true, true, true] {
        harness.events.push(Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        });
    }
    harness.frame();
    assert!(harness.workspace.viewport.selection().contains(target));
}

#[test]
fn command_shortcuts_work_while_a_button_has_focus() {
    let mut harness = Harness::new();
    harness.edit_width("45 mm");
    let button = egui::Id::unique("focused button");
    harness
        .context
        .memory_mut(|memory| memory.request_focus(button));
    harness.frame();
    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(harness.expression_text("width"), "40 mm");
}

#[test]
fn stepping_the_highlight_visits_a_datum_plane_once() {
    let mut harness = Harness::new();
    harness.click("Plane");
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
    let mut visited = Vec::new();
    for _ in 0..40 {
        harness.key(Key::N, Modifiers::NONE);
        harness.frame();
        let Some(highlight) = harness.workspace.viewport.keyboard_highlight() else {
            continue;
        };
        if visited.first() == Some(&highlight) {
            break;
        }
        visited.push(highlight);
    }
    let datums = visited
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Datum(_)))
        .count();
    assert_eq!(datums, 1, "{visited:?}");
    let distinct: std::collections::BTreeSet<_> = visited.iter().collect();
    assert_eq!(distinct.len(), visited.len());
}

#[test]
fn features_are_opened_and_finished_from_the_keyboard() {
    let mut harness = Harness::new();
    let side = feature_named(&harness, "Side sketch");
    assert!(offer(&harness, Command::EditFeature).availability.is_err());

    harness.click("Side sketch");
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.editing(), Some(side));
    run_from_palette(&mut harness, "finish sketch");
    assert_eq!(harness.editing(), None);

    let (extrude, _) = extruded_plate(&mut harness);
    assert!(offer(&harness, Command::CloseFeature).availability.is_err());
    harness.click("Extrude 1");
    assert_eq!(
        offer(&harness, Command::EditFeature).title(),
        "Edit feature: Extrude 1"
    );
    harness.key(Key::E, Modifiers::NONE);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), Some(extrude));
    harness.key(Key::E, Modifiers::NONE);
    harness.frame();
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Edit feature: Extrude 1 is already being edited")
    );

    run_from_palette(&mut harness, "finish editing feature");
    assert_eq!(harness.workspace.editing.solid(), None);
}
