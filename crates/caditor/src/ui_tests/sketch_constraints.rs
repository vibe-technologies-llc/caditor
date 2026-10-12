use std::sync::Arc;

use caditor_document::{Edit, Feature, FeatureKind, Transaction};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use egui::{Event, Key, Modifiers};

use super::{
    Harness, assert_failure_shown_once, constraint_of_kind, constraints_of_kind, edit_base_sketch,
    edit_free_sketch, entities_of_kind, entity_pickables, line_ends, only_constraint,
};
use crate::{
    annotations, canvas, editing::EditingCommand, model::Action, panels::Focus, selection::Pickable,
};

#[test]
fn clicking_a_constraint_in_the_tree_edits_its_sketch_and_selects_it() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    let (constraint, _) = harness
        .sketch(base)
        .constraints()
        .find(|(_, constraint)| constraint.dimension().is_some())
        .unwrap();
    let description = harness.sketch(base).describe_constraint(constraint);
    harness.edit(base);
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    let finished = harness.editing();
    let name = harness.document().feature(base).unwrap().name.clone();
    harness.click_button(&format!("Show details of {name}"));
    harness.settle();

    harness.click_leftmost(&description);
    harness.settle();

    assert_eq!(finished, None);
    assert_eq!(harness.editing(), Some(base));
    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchConstraint {
                feature: base,
                constraint,
            })
    );
}

#[test]
fn every_conflicting_part_of_a_sketch_is_named_in_the_error() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().clone();
    let mut sketch = base.kind.sketch().unwrap().clone();
    let line = sketch
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    sketch.add_constraint(Constraint::Vertical(line)).unwrap();
    let other = sketch.add_line(Point2::new(0.0, 90.0), Point2::new(5.0, 90.0));
    sketch
        .add_constraint(Constraint::Horizontal(other))
        .unwrap();
    sketch.add_constraint(Constraint::Vertical(other)).unwrap();
    let replacement = Feature::new(base.id(), base.name.clone(), FeatureKind::from(sketch));
    harness.model.perform(Action::Apply(Transaction::new(
        "Add conflicts",
        vec![
            Edit::RemoveFeature { id: base.id() },
            Edit::InsertFeature {
                index: 0,
                feature: Arc::new(replacement),
            },
        ],
    )));
    harness.settle();

    assert_failure_shown_once(&harness, "Base sketch");
    assert!(harness.shows_containing("The sketch has 2 separate problems."));
    assert!(harness.shows_containing("Vertical Line 2 conflicts with Horizontal Line 2."));
}

#[test]
fn a_constraint_conflict_is_named_and_leads_to_the_newest_constraint() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().clone();
    let mut sketch = base.kind.sketch().unwrap().clone();
    let line = sketch
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    let vertical = sketch.add_constraint(Constraint::Vertical(line)).unwrap();
    let replacement = Feature::new(base.id(), base.name.clone(), FeatureKind::from(sketch));
    harness.model.perform(Action::Apply(Transaction::new(
        "Add vertical",
        vec![
            Edit::RemoveFeature { id: base.id() },
            Edit::InsertFeature {
                index: 0,
                feature: Arc::new(replacement),
            },
        ],
    )));
    harness.settle();

    assert_failure_shown_once(&harness, "Base sketch");
    assert!(harness.shows("Vertical Line 2 conflicts with Horizontal Line 2."));
    harness.click("Go to Vertical Line 2");
    harness.let_animations_finish();
    assert!(harness.shows("Vertical Line 2"));
    assert!(!harness.workspace.panels.wants_focus(Focus::Constraint {
        feature: base.id(),
        constraint: vertical,
    }));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(harness.shows("Up to date"));
}

#[test]
fn a_constraint_that_conflicts_only_once_solved_is_refused_naming_what_it_conflicts_with() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let (start, end) = line_ends(&sketch, line);
    for (point, at) in [(start, Point2::ZERO), (end, Point2::new(30.0, 10.0))] {
        sketch
            .add_constraint(Constraint::Fix { point, at })
            .unwrap();
    }
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();
    let revision = harness.model.revision();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.click_button("Horizontal");
    harness.settle();

    assert_eq!(harness.model.revision(), revision);
    assert!(constraints_of_kind(harness.sketch(feature), "Horizontal").is_empty());
    let notice = harness.model.notice().expect("the refusal is a notice");
    assert!(
        notice.text.contains("was not added: it conflicts with"),
        "{}",
        notice.text
    );
    assert!(notice.text.contains("Fix"), "{}", notice.text);
}

#[test]
fn horizontal_from_the_selection_levels_a_line_and_updates_the_freedom() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    assert!(harness.shows("4 degrees of freedom left"));

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.click_button("Horizontal");
    harness.settle();

    assert_eq!(harness.model.undo_label(), Some("Add Horizontal"));
    assert!(harness.shows("3 degrees of freedom left"));
    let (start, end) = harness.shown(feature).line_endpoints(line).unwrap();
    assert!((start.y - end.y).abs() < 1e-9, "{start} {end}");

    harness.key(Key::H, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert_eq!(harness.sketch(feature).constraints().len(), 1);

    harness.add_stored_constraint(feature, Constraint::Horizontal(line));
    assert!(harness.shows("1 redundant constraint"));
    assert!(harness.shows("Redundant: Horizontal Line 2 already does this. Delete one of them."));
}

#[test]
fn curvature_joins_two_splines_tangent_and_bending_alike_from_its_key() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let second = sketch.add_spline(&[
        Point2::new(20.0, 0.0),
        Point2::new(25.0, -2.0),
        Point2::new(30.0, -8.0),
    ]);
    let points = [first, second].map(|spline| match sketch.entity(spline) {
        Some(caditor_sketch::Entity::Spline {
            points: control_points,
            ..
        }) => control_points.clone(),
        other => panic!("expected a spline, found {other:?}"),
    });
    sketch
        .add_constraint(Constraint::Coincident(points[1][0], points[0][2]))
        .unwrap();
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();

    harness.select([first, second].map(|entity| Pickable::SketchEntity { feature, entity }));
    harness.frame();
    harness.key(Key::G, Modifiers::SHIFT);
    harness.frame();
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Tangent").len(),
        1
    );
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Curvature").len(),
        1
    );
    let shown = harness.shown(feature);
    let curvature = |spline: EntityId, parameter: f64| {
        let [tangent, bend] = shown.spline(spline).unwrap().derivatives(parameter);
        tangent.perp_dot(bend) / tangent.length().powi(3)
    };
    let (arriving, leaving) = (curvature(first, 1.0), curvature(second, 0.0));
    assert!((arriving - leaving).abs() < 1e-6, "{arriving} {leaving}");

    harness.key(Key::G, Modifiers::SHIFT);
    harness.frame();
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Curvature").len(),
        1
    );
}

#[test]
fn distance_dimensions_a_point_from_a_spline_square_to_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let point = sketch.add_point(Point2::new(10.0, 9.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();

    harness.select([spline, point].map(|entity| Pickable::SketchEntity { feature, entity }));
    harness.frame();
    harness.click_button("Distance");
    harness.settle();

    let distances = constraints_of_kind(harness.sketch(feature), "Distance");
    let [Constraint::Distance { from, to, value }] = &distances[..] else {
        panic!("one distance is added: {distances:?}");
    };
    assert_eq!((*from, *to), (point, spline));
    assert_eq!(*value, Expression::Measure(4.0, Unit::Millimetre));
}

#[test]
fn distance_dimensions_a_line_from_where_a_spline_bulges_and_holds_it_there() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let line = sketch.add_line(Point2::new(0.0, 8.0), Point2::new(20.0, 8.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();

    harness.select([spline, line].map(|entity| Pickable::SketchEntity { feature, entity }));
    harness.frame();
    harness.click_button("Distance");
    harness.settle();

    let distances = constraints_of_kind(harness.sketch(feature), "Distance");
    let [Constraint::Distance { from, to, value }] = &distances[..] else {
        panic!("one distance is added: {distances:?}");
    };
    assert_eq!((*from, *to), (spline, line));
    assert_eq!(*value, Expression::Measure(3.0, Unit::Millimetre));
    assert!(harness.shows("3 mm"));
}

#[test]
fn clicking_a_glyph_selects_its_constraint_and_delete_removes_it_in_one_step() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    let before = harness.document().clone();
    let horizontal = constraint_of_kind(harness.sketch(base), "Horizontal");
    let pickable = Pickable::SketchConstraint {
        feature: base,
        constraint: horizontal,
    };

    harness.click("H");
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![pickable]
    );
    assert!(harness.shows("Base sketch › Horizontal Line 2"));
    harness.events.push(Event::PointerGone);
    harness.frame();
    harness.frame();
    assert_eq!(harness.color_of("H"), canvas::DARK_CHROME.selected);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    let sketch = harness.sketch(base);
    assert!(sketch.constraint(horizontal).is_none());
    assert_eq!(sketch.constraints().len(), 1);
    assert_eq!(sketch.entities().len(), 3);
    assert_eq!(harness.model.undo_label(), Some("Delete Horizontal Line 2"));
    harness.frame();
    assert!(!harness.shows("H"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(*harness.document(), before);
}

#[test]
fn a_conflict_colours_the_dimensions_and_glyphs_involved() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    assert_ne!(harness.color_of("H"), canvas::DARK_CHROME.error);
    assert_ne!(harness.color_of("width = 40 mm"), canvas::DARK_CHROME.error);
    let line = entities_of_kind(harness.sketch(base), "Line")[0];
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);

    harness.key(Key::V, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    harness.add_stored_constraint(base, Constraint::Vertical(line));
    assert!(harness.shows("Conflicting constraints"));
    for mark in ["H", "V"] {
        assert_eq!(harness.color_of(mark), canvas::DARK_CHROME.error, "{mark}");
    }
    assert_ne!(harness.color_of("width = 40 mm"), canvas::DARK_CHROME.error);
}

#[test]
fn with_the_line_tool_active_clicking_on_a_glyph_draws_instead_of_selecting() {
    let mut harness = Harness::new();
    let base = edit_base_sketch(&mut harness);
    harness.use_tool(Key::L);
    let glyph = harness.position_of("H");

    harness.click_screen(glyph);
    harness.frame();
    harness.click_at(Point2::new(20.0, 10.0));
    let lines = entities_of_kind(harness.sketch(base), "Line");
    assert_eq!(lines.len(), 2);
    assert!(harness.workspace.viewport.selection().is_empty());
    assert_eq!(harness.model.undo_label(), Some("Draw line"));
}

#[test]
fn a_constraint_button_explains_what_to_select_when_it_cannot_apply() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let (start, _) = line_ends(harness.sketch(feature), line);
    harness.select([Pickable::SketchEntity {
        feature,
        entity: start,
    }]);

    harness.click_button("Parallel");
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
    harness.hover_button("Parallel");
    assert!(harness.shows("Make lines parallel. Select two or more lines (Shift+P)"));

    harness.key(Key::P, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    assert!(harness.shows("Parallel: Select two or more lines"));
    assert_eq!(harness.sketch(feature).constraints().len(), 0);
}

#[test]
fn fixing_a_line_locks_both_ends_where_they_are_shown() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(2.0, 3.0), Point2::new(25.0, 9.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    assert!(harness.shows("4 degrees of freedom left"));
    harness.select(entity_pickables(feature, &[line]));

    harness.key(Key::F, Modifiers::SHIFT);
    harness.frame();
    harness.settle();

    let (start, end) = line_ends(harness.sketch(feature), line);
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Fix"),
        vec![
            Constraint::Fix {
                point: start,
                at: Point2::new(2.0, 3.0),
            },
            Constraint::Fix {
                point: end,
                at: Point2::new(25.0, 9.0),
            },
        ]
    );
    assert_eq!(harness.model.undo_label(), Some("Add Fix"));
    assert!(harness.shows("Fully constrained"));
    assert!(harness.shows("Fix Point 0"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(harness.sketch(feature).constraints().next().is_none());
    assert!(harness.shows("4 degrees of freedom left"));
}

#[test]
fn equal_and_parallel_take_several_lines_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let lines = [
        sketch.add_line(Point2::new(0.0, 0.0), Point2::new(20.0, 0.0)),
        sketch.add_line(Point2::new(0.0, 10.0), Point2::new(15.0, 14.0)),
        sketch.add_line(Point2::new(0.0, 20.0), Point2::new(30.0, 26.0)),
    ];
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.select(entity_pickables(feature, &lines));

    harness.click_button("Parallel");
    harness.click_button("Equal");
    harness.settle();

    let [first, second, third] = lines;
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Parallel"),
        vec![
            Constraint::Parallel(first, second),
            Constraint::Parallel(first, third)
        ]
    );
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Equal"),
        vec![
            Constraint::Equal(first, second),
            Constraint::Equal(first, third)
        ]
    );
    let shown = harness.shown(feature);
    let direction = |line| shown.line_direction(line).unwrap();
    for line in [second, third] {
        assert!(direction(first).perp_dot(direction(line)).abs() < 1e-6);
        assert!((direction(first).length() - direction(line).length()).abs() < 1e-6);
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert!(constraints_of_kind(harness.sketch(feature), "Equal").is_empty());
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Parallel").len(),
        2
    );
}

#[test]
fn a_point_goes_onto_a_spline_and_a_line_touches_it() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let spline = sketch.add_spline(&[
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 10.0),
        Point2::new(20.0, 0.0),
    ]);
    let point = sketch.add_point(Point2::new(4.0, 9.0));
    let line = sketch.add_line(Point2::new(2.0, 7.0), Point2::new(18.0, 6.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.select(entity_pickables(feature, &[spline, point]));

    harness.click_button("Coincident");
    harness.settle();
    harness.select(entity_pickables(feature, &[spline, line]));
    harness.click_button("Tangent");
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Coincident"),
        vec![Constraint::Coincident(spline, point)]
    );
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Tangent"),
        vec![Constraint::Tangent(spline, line)]
    );
    let shown = harness.shown(feature);
    let curve = shown.spline(spline).unwrap();
    let on_curve = shown.point(point).unwrap();
    let nearest = (0..=2000)
        .map(|step| curve.point_at(f64::from(step) / 2000.0).distance(on_curve))
        .fold(f64::INFINITY, f64::min);
    assert!(nearest < 1e-2, "{on_curve} is {nearest} from the spline");
    let (start, end) = shown.line_endpoints(line).unwrap();
    let highest = (0..=2000)
        .map(|step| {
            let at = curve.point_at(f64::from(step) / 2000.0);
            (end - start).normalize().perp_dot(at - start)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(highest.abs() < 1e-3, "the line is {highest} from touching");
    assert!(harness.shows("Coincident Spline 3 and Point 4"));
    assert!(harness.shows("Tangent Spline 3 and Line 7"));
    assert!(!harness.shows("Conflicting constraints"));
}

#[test]
fn symmetric_mirrors_two_points_about_the_selected_line_from_the_palette() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(-8.0, 5.0));
    let second = sketch.add_point(Point2::new(9.0, 6.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let mut chosen = entity_pickables(feature, &[first, second]);
    chosen.push(Pickable::SketchEntity {
        feature,
        entity: EntityId::VERTICAL_AXIS,
    });
    harness.select(chosen);

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.frame();
    harness.type_text("Symmetric");
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.settle();

    assert_eq!(
        only_constraint(harness.sketch(feature)).1,
        Constraint::Symmetric {
            first,
            second,
            about: EntityId::VERTICAL_AXIS,
        }
    );
    let shown = harness.shown(feature);
    let (a, b) = (shown.point(first).unwrap(), shown.point(second).unwrap());
    assert!(
        (a.x + b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6,
        "{a} {b}"
    );
    assert!(harness.shows("Symmetric Point 0 and Point 1 about Vertical axis"));
}

#[test]
fn a_conflicting_constraint_is_reported_and_undo_clears_it() {
    let mut harness = Harness::new();
    let base = harness.document().features().next().unwrap().id();
    harness.edit(base);
    assert!(harness.shows("2 degrees of freedom left"));
    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();
    harness.select([Pickable::SketchEntity {
        feature: base,
        entity: line,
    }]);

    harness.key(Key::V, Modifiers::SHIFT);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    harness.add_stored_constraint(base, Constraint::Vertical(line));
    assert!(harness.shows("Conflicting constraints"));
    assert!(harness.shows("Vertical Line 2 conflicts with Horizontal Line 2."));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(!harness.shows("Conflicting constraints"));
    assert!(harness.shows("2 degrees of freedom left"));
    assert_eq!(harness.editing(), Some(base));
}

#[test]
fn deleting_a_point_removes_its_line_and_constraints_in_one_undoable_step() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 0.0));
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let lone = sketch.add_point(Point2::new(5.0, 5.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let before = harness.document().clone();
    let (start, end) = line_ends(harness.sketch(feature), line);
    harness.select([
        Pickable::SketchEntity {
            feature,
            entity: start,
        },
        Pickable::SketchEntity {
            feature,
            entity: EntityId::ORIGIN,
        },
    ]);

    harness.key(Key::Delete, Modifiers::NONE);
    harness.frame();
    let sketch = harness.sketch(feature);
    assert!(sketch.entity(start).is_none());
    assert!(sketch.entity(line).is_none());
    assert!(sketch.entity(end).is_some());
    assert!(sketch.entity(lone).is_some());
    assert_eq!(sketch.constraints().len(), 0);
    assert_eq!(harness.model.undo_label(), Some("Delete Point 0"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(*harness.document(), before);
}

#[test]
fn a_selected_dimension_is_disabled_into_a_reference_that_shows_the_measured_value() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(37.0, 0.0));
    let (start, end) = match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    };
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    let length = sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        })
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    let label = Pickable::SketchConstraint {
        feature,
        constraint: length,
    };
    harness.select([label]);
    harness.frame();
    assert!(harness.shows("37 mm"));

    harness.click_button("Disable");
    harness.settle();

    let sketch = harness.sketch(feature);
    assert!(!sketch.is_active(length));
    assert!(harness.shows("(37 mm)"));
    assert_eq!(
        sketch.describe_constraint(length),
        format!(
            "{} (disabled)",
            sketch.describe(sketch.constraint(length).unwrap())
        )
    );

    harness.click_button("Enable");
    harness.settle();
    assert!(harness.sketch(feature).is_active(length));
    assert!(!harness.shows("(37 mm)"));
}

#[test]
fn a_dimension_of_geometry_already_determined_is_added_as_a_reference() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(37.0, 0.0));
    let (start, end) = match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("expected a line, found {other:?}"),
    };
    sketch
        .add_constraint(Constraint::Coincident(start, EntityId::ORIGIN))
        .unwrap();
    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        })
        .unwrap();
    let feature = edit_free_sketch(&mut harness, sketch);
    harness.settle();
    harness.select(entity_pickables(feature, &[start, end]));

    harness.key(Key::X, Modifiers::SHIFT);
    harness.frame();
    harness.frame();
    assert!(
        harness
            .model
            .notice()
            .is_some_and(|notice| notice.text.contains("already fully determined"))
    );
    harness.settle();

    let sketch = harness.sketch(feature);
    let (added, constraint) = sketch
        .constraints()
        .find(|(_, constraint)| matches!(constraint, Constraint::HorizontalDistance { .. }))
        .map(|(id, constraint)| (id, constraint.clone()))
        .unwrap();
    assert_eq!(
        constraint,
        Constraint::HorizontalDistance {
            from: start,
            to: end,
            value: Expression::Measure(37.0, Unit::Millimetre),
        }
    );
    assert!(!sketch.is_active(added));
    assert!(
        harness
            .model
            .settled_solution(feature)
            .is_some_and(|solution| solution.redundancies().is_empty())
    );
    assert_eq!(
        harness.model.undo_label(),
        Some("Add reference Horizontal distance")
    );
    assert_ne!(
        harness.focused(),
        Some(annotations::field_id(feature, added))
    );
}

#[test]
fn a_crowded_entity_shows_a_few_glyphs_and_counts_the_rest() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let base = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(60.0, 0.0));
    for index in 1..=7 {
        let y = f64::from(index) * 12.0;
        let other = sketch.add_line(Point2::new(0.0, y), Point2::new(60.0, y));
        sketch
            .add_constraint(Constraint::Equal(base, other))
            .unwrap();
    }
    edit_free_sketch(&mut harness, sketch);
    harness.settle();
    harness.frame();

    assert!(harness.shows("+4"));
    assert!(!harness.shows("+7"));

    harness.key(Key::G, Modifiers::ALT);
    harness.frame();
    harness.frame();
    assert!(!harness.workspace.viewport.glyphs_shown());
    assert!(!harness.shows("+4"));
    harness.key(Key::G, Modifiers::ALT);
    harness.frame();
    harness.frame();
    assert!(harness.shows("+4"));
}

#[test]
fn a_point_held_past_the_end_of_its_line_is_counted_and_explained() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    let past = sketch.add_point(Point2::new(15.0, 0.0));
    sketch
        .add_constraint(Constraint::Coincident(past, line))
        .unwrap();
    let feature = harness.add_sketch(sketch);
    harness.context.enable_accesskit();

    harness.edit(feature);
    harness.frame();

    assert!(harness.shows("1 point beyond its curve"));
    assert!(
        harness
            .view_description()
            .is_some_and(|description| description.contains("1 point beyond its curve"))
    );
}
