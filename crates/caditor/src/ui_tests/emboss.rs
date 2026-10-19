use std::f64::consts::PI;

use caditor_document::{EmbossForm, FeatureId, RegionChoice};
use caditor_geometry::{Plane, Point2, Point3};
use caditor_sketch::{Entity, Sketch};
use egui::{Key, Modifiers};

use super::{
    CAMERA_SETTLE, Harness, delete_face::holed_plate, drag_value_arrow, length_value,
    pipes::undone_and_redone, rectangle, volume_about,
};
use crate::{move_manipulator::Handle, selection, selection::Pickable, value_gauges::Measured};

const PLATE: f64 = 40.0 * 40.0 * 10.0;
const BORE: f64 = PI * 25.0 * 10.0;
const LABEL: f64 = 10.0 * 6.0;

fn emboss_of(harness: &Harness, feature: FeatureId) -> &caditor_document::Emboss {
    harness
        .document()
        .feature(feature)
        .and_then(|feature| feature.kind.emboss())
        .unwrap()
}

fn label(harness: &mut Harness) -> (FeatureId, Pickable) {
    label_on(
        harness,
        Plane::XY,
        Point2::new(5.0, 5.0),
        Point2::new(15.0, 11.0),
    )
}

fn label_on(
    harness: &mut Harness,
    plane: Plane,
    min: Point2,
    max: Point2,
) -> (FeatureId, Pickable) {
    let mut sketch = Sketch::new(plane);
    rectangle(&mut sketch, min, max);
    let edge = sketch
        .entities()
        .find(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let feature = harness.add_sketch(sketch);
    (
        feature,
        Pickable::SketchEntity {
            feature,
            entity: edge,
        },
    )
}

#[test]
fn emboss_raises_a_sketch_from_a_face_and_recesses_it_when_asked() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (label, edge) = label(&mut harness);
    assert!(volume_about(&harness, plate, PLATE - BORE));

    harness.select([top, edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    let emboss = harness
        .workspace
        .editing
        .solid()
        .expect("the emboss is open");
    assert_eq!(harness.model.undo_label(), Some("Create Emboss 1"));
    assert_eq!(emboss_of(&harness, emboss).sketch, Some(label));
    assert_eq!(emboss_of(&harness, emboss).form, EmbossForm::Raised);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));
    assert!(harness.shows("Click faces to emboss, and again to leave one out"));
    assert!(harness.shows("Faces to emboss"));

    harness.click_button("Recess");
    harness.settle();
    assert_eq!(emboss_of(&harness, emboss).form, EmbossForm::Recessed);
    assert_eq!(harness.model.evaluation().failed_count(), 0);
    assert!(volume_about(&harness, plate, PLATE - BORE - LABEL * 0.5));

    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }
    assert_eq!(harness.workspace.editing.solid(), None);
}

#[test]
fn the_emboss_command_needs_a_face_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (_, edge) = label(&mut harness);

    harness.select([edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert_eq!(harness.workspace.editing.solid(), None);
    assert_ne!(harness.model.undo_label(), Some("Create Emboss 1"));

    harness.select([top, edge]);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));
    for _ in 0..2 {
        harness.key(Key::Escape, Modifiers::NONE);
        harness.frame();
        harness.frame();
    }

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();
    assert!(volume_about(&harness, plate, PLATE - BORE));
}

fn two_labels(harness: &mut Harness) -> (FeatureId, Pickable) {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(5.0, 5.0), Point2::new(15.0, 11.0));
    rectangle(&mut sketch, Point2::new(22.0, 5.0), Point2::new(32.0, 11.0));
    let edge = sketch
        .entities()
        .find(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let feature = harness.add_sketch(sketch);
    (
        feature,
        Pickable::SketchEntity {
            feature,
            entity: edge,
        },
    )
}

fn region_at(harness: &Harness, sketch: FeatureId, at: Point2) -> Pickable {
    let regions = selection::sketch_regions(harness.model.evaluation(), sketch).unwrap();
    let found = regions
        .iter()
        .find(|region| region.region.contains(at))
        .expect("a region lies there");
    Pickable::SketchRegion {
        feature: sketch,
        region: found.region.key(),
    }
}

fn opened_emboss(harness: &mut Harness, selected: impl IntoIterator<Item = Pickable>) -> FeatureId {
    harness.select(selected);
    harness.use_tool_with(Key::Y, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the emboss is open")
}

#[test]
fn dragging_the_emboss_arrow_changes_its_depth_in_one_undoable_change() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (_, edge) = label(&mut harness);
    let emboss = opened_emboss(&mut harness, [top, edge]);
    harness.select([]);
    let depth = |harness: &Harness| length_value(harness, &emboss_of(harness, emboss).depth);
    let before = depth(&harness);

    let step = drag_value_arrow(&mut harness, Measured::EmbossDepth, 3.0);
    let after = depth(&harness);

    assert_eq!(before, 0.5);
    assert_eq!(harness.model.undo_label(), Some("Edit Emboss 1"));
    assert!(
        (after - before - 3.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * after));
    undone_and_redone(&mut harness, "the depth drag", depth);
}

#[test]
fn a_recess_has_its_arrow_on_the_floor_and_dragging_it_deeper_cuts_deeper() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (_, edge) = label(&mut harness);
    let emboss = opened_emboss(&mut harness, [top, edge]);
    harness.click_button("Recess");
    harness.settle();
    harness.select([]);

    let step = drag_value_arrow(&mut harness, Measured::EmbossDepth, 2.0);
    let depth = length_value(&harness, &emboss_of(&harness, emboss).depth);

    assert!(
        (depth - 2.5).abs() <= 2.0 * step,
        "{depth} with steps of {step}"
    );
    assert!(volume_about(&harness, plate, PLATE - BORE - LABEL * depth));
}

#[test]
fn the_arrow_of_an_emboss_wrapped_into_a_bore_stands_on_its_wall_and_points_at_the_axis() {
    let mut harness = Harness::new();
    let (_, bore, _) = holed_plate(&mut harness);
    let (_, edge) = label_on(
        &mut harness,
        Plane::XZ,
        Point2::new(15.0, 2.0),
        Point2::new(25.0, 8.0),
    );
    let emboss = opened_emboss(&mut harness, [bore, edge]);
    harness.select([]);
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let depth = |harness: &Harness| length_value(harness, &emboss_of(harness, emboss).depth);
    let handle = Handle::Value(Measured::EmbossDepth);
    let from_axis = |point: Point3| (point.x - 20.0).hypot(point.y - 20.0);
    let grip = |harness: &Harness, along: f64| {
        from_axis(
            harness
                .workspace
                .viewport
                .handle_grip(handle, along)
                .expect("the depth arrow is shown"),
        )
    };
    let foot = harness
        .workspace
        .viewport
        .handle_foot(handle)
        .expect("the depth arrow stands somewhere");
    let before = depth(&harness);

    assert!((from_axis(foot) - (5.0 - before)).abs() < 1e-6);
    assert!((grip(&harness, 1.0) - grip(&harness, 0.0) + 1.0).abs() < 1e-6);

    let step = drag_value_arrow(&mut harness, Measured::EmbossDepth, 1.0);
    let after = depth(&harness);

    assert_eq!(harness.model.undo_label(), Some("Edit Emboss 1"));
    assert!(
        (after - before - 1.0).abs() <= 2.0 * step,
        "{before} to {after} with steps of {step}"
    );
    undone_and_redone(&mut harness, "the depth drag", depth);
}

#[test]
fn choosing_regions_embosses_only_those_and_a_click_in_the_view_toggles_one() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (sketch, edge) = two_labels(&mut harness);
    let emboss = opened_emboss(&mut harness, [top, edge]);
    assert_eq!(emboss_of(&harness, emboss).regions, RegionChoice::All);
    assert!(volume_about(
        &harness,
        plate,
        PLATE - BORE + 2.0 * LABEL * 0.5
    ));

    harness.click_button("Chosen");
    harness.settle();
    let RegionChoice::Chosen(chosen) = &emboss_of(&harness, emboss).regions else {
        panic!("the regions were chosen");
    };
    assert_eq!(chosen.len(), 2);
    assert!(harness.shows("2 regions chosen"));
    assert!(volume_about(
        &harness,
        plate,
        PLATE - BORE + 2.0 * LABEL * 0.5
    ));

    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let Pickable::SketchRegion { region, .. } = region_at(&harness, sketch, Point2::new(27.0, 8.0))
    else {
        panic!("a region was found");
    };
    harness.click_pickable(
        Plane::XY,
        Point2::new(27.0, 8.0),
        Pickable::Region {
            feature: emboss,
            region,
        },
    );
    harness.settle();

    let RegionChoice::Chosen(chosen) = &emboss_of(&harness, emboss).regions else {
        panic!("the regions stay chosen");
    };
    assert_eq!(chosen.len(), 1);
    assert_eq!(
        harness.model.undo_label(),
        Some("Choose regions of Emboss 1")
    );
    assert!(harness.shows("1 region chosen"));
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.settle();
    assert!(volume_about(
        &harness,
        plate,
        PLATE - BORE + 2.0 * LABEL * 0.5
    ));

    harness.click_button("All closed");
    harness.settle();
    assert_eq!(emboss_of(&harness, emboss).regions, RegionChoice::All);
}

#[test]
fn regions_selected_before_embossing_are_the_only_ones_embossed() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (sketch, _) = two_labels(&mut harness);
    let right = region_at(&harness, sketch, Point2::new(27.0, 8.0));

    let emboss = opened_emboss(&mut harness, [top, right]);

    let RegionChoice::Chosen(chosen) = &emboss_of(&harness, emboss).regions else {
        panic!("the selected region was chosen");
    };
    assert_eq!(chosen.len(), 1);
    assert_eq!(emboss_of(&harness, emboss).sketch, Some(sketch));
    assert!(volume_about(&harness, plate, PLATE - BORE + LABEL * 0.5));
}

#[test]
fn clearing_the_chosen_regions_waits_for_regions_and_the_palette_selects_all_again() {
    let mut harness = Harness::new();
    let (plate, _, top) = holed_plate(&mut harness);
    let (sketch, _) = two_labels(&mut harness);
    let right = region_at(&harness, sketch, Point2::new(27.0, 8.0));
    let emboss = opened_emboss(&mut harness, [top, right]);

    harness.click_button("Clear");
    harness.settle();

    assert_eq!(
        emboss_of(&harness, emboss).regions,
        RegionChoice::Chosen(Vec::new())
    );
    let name = harness.document().feature(sketch).unwrap().name.clone();
    assert!(harness.shows(&format!("Choose at least one region of {name} to emboss.")));
    assert!(volume_about(&harness, plate, PLATE - BORE));

    harness.click_button("Select all");
    harness.settle();

    assert_eq!(emboss_of(&harness, emboss).regions, RegionChoice::All);
    assert!(volume_about(
        &harness,
        plate,
        PLATE - BORE + 2.0 * LABEL * 0.5
    ));
}
