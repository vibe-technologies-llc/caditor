use caditor_document::FeatureKind;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, EntityId, Sketch};
use egui::accesskit::Role;

use super::{Harness, rectangle, run_from_palette};
use crate::{model::Action, panels::REVEAL_FRAMES, scene_palette};

#[test]
fn a_large_sketch_being_edited_shows_its_closed_regions_once_they_are_found() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));
    for index in 0..2_000 {
        sketch.add_point(Point2::new(
            60.0 + f64::from(index % 50),
            f64::from(index / 50),
        ));
    }
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    harness.settle();
    let found = |harness: &Harness| {
        harness
            .model
            .evaluation()
            .feature(feature)
            .and_then(|status| status.result.as_deref())
            .and_then(caditor_document::FeatureResult::sketch)
            .is_some_and(
                |result| matches!(result.regions(), Some(Ok(regions)) if !regions.is_empty()),
            )
    };

    harness.wait_until("the regions are found", found);
    harness.frame();
    let tinted = harness
        .built()
        .scene
        .fills()
        .any(|fill| fill.color == scene_palette::STANDARD.closed_region);

    assert!(tinted);
}

#[test]
fn feature_rows_far_out_of_view_are_not_laid_out_until_reached() {
    let mut harness = Harness::new();
    let mut transaction = harness.document().transaction("Add sketches");
    for index in 0..150 {
        transaction.add_feature(
            format!("Far sketch {index}"),
            FeatureKind::from(Sketch::new(Plane::XY)),
        );
    }
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    let laid_out = |harness: &Harness, name: &str| {
        harness.accessible_named(Role::Button, &format!("More actions for {name}"))
    };
    let near_laid_out = laid_out(&harness, "Far sketch 0");
    let far_laid_out = laid_out(&harness, "Far sketch 149");

    run_from_palette(&mut harness, "Far sketch 149");
    for _ in 0..REVEAL_FRAMES {
        harness.frame();
    }

    assert!(near_laid_out);
    assert!(!far_laid_out);
    assert!(laid_out(&harness, "Far sketch 149"));
    assert!(harness.shows("Far sketch 149"));
    assert!(!laid_out(&harness, "Far sketch 0"));
    assert_eq!(
        harness.workspace.panels.chosen(),
        vec![
            harness
                .document()
                .features()
                .find(|feature| feature.name == "Far sketch 149")
                .unwrap()
                .id()
        ]
    );
}

#[test]
fn constraint_rows_far_out_of_view_are_not_laid_out() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    for index in 0..200 {
        let y = f64::from(index);
        let line = sketch.add_line(Point2::new(0.0, y), Point2::new(10.0, y + 0.5));
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
    }
    let feature = harness.add_sketch(sketch);
    harness.context.enable_accesskit();
    harness.edit(feature);
    harness.frame();
    harness.frame();
    let delete_buttons = harness
        .accessible
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Button && node.label() == Some("Delete this constraint")
        })
        .count();

    assert!(harness.shows(crate::feature_tree::CONSTRAINTS_TITLE));
    assert!(delete_buttons > 0);
    assert!(delete_buttons < 100, "{delete_buttons}");
}

#[test]
fn redundant_constraint_rows_far_out_of_view_are_not_laid_out_once_measured() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    let mut lines = Vec::new();
    for index in 0..200 {
        let y = f64::from(index);
        let line = sketch.add_line(Point2::new(0.0, y), Point2::new(10.0, y + 0.5));
        sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
        lines.push(line);
    }
    let feature = harness.add_sketch(sketch);
    let mut transaction = harness.document().transaction("Restate every constraint");
    for line in lines {
        transaction.add_sketch_constraint(
            feature,
            Constraint::Parallel(EntityId::HORIZONTAL_AXIS, line),
        );
    }
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
    harness.context.enable_accesskit();
    harness.edit(feature);
    harness.frame();
    harness.frame();
    let delete_buttons = harness
        .accessible
        .iter()
        .filter(|(_, node)| {
            node.role() == Role::Button && node.label() == Some("Delete this constraint")
        })
        .count();

    let redundant = crate::sketch_status::up_to_date_solution(harness.model.evaluation(), feature)
        .map(|solution| solution.redundancies().len());

    assert_eq!(redundant, Some(200));
    assert!(delete_buttons > 0);
    assert!(delete_buttons < 100, "{delete_buttons}");
}

#[test]
fn the_redundant_pill_shows_at_the_top_of_the_card_however_many_constraints_repeat() {
    for repeated in [20_usize, 60, 200] {
        let mut harness = Harness::new();
        let mut sketch = Sketch::new(Plane::XY);
        let mut lines = Vec::new();
        for index in 0..repeated {
            let y = index as f64;
            let line = sketch.add_line(Point2::new(0.0, y), Point2::new(10.0, y + 0.5));
            sketch.add_constraint(Constraint::Horizontal(line)).unwrap();
            lines.push(line);
        }
        let feature = harness.add_sketch(sketch);
        let mut transaction = harness.document().transaction("Restate every constraint");
        for line in lines {
            transaction.add_sketch_constraint(
                feature,
                Constraint::Parallel(EntityId::HORIZONTAL_AXIS, line),
            );
        }
        harness.perform(Action::Apply(transaction.finish()));
        harness.settle();
        harness.edit(feature);
        harness.frame();
        harness.frame();
        let words = format!("{repeated} redundant constraints");
        let pill = harness
            .texts
            .iter()
            .find(|(shown, _)| *shown == words)
            .map(|(_, rect)| *rect);
        let heading = harness
            .texts
            .iter()
            .find(|(shown, _)| *shown == crate::feature_tree::CONSTRAINTS_TITLE)
            .map(|(_, rect)| *rect);

        assert!(pill.is_some(), "{repeated}: {words} is not shown");
        assert!(
            pill.zip(heading)
                .is_some_and(|(pill, heading)| pill.min.y < heading.min.y),
            "{repeated}: {pill:?} {heading:?}"
        );
    }
}
