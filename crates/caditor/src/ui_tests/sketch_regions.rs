use caditor_document::{FeatureId, RegionChoice};
use caditor_geometry::{Plane, Point2};
use caditor_kernel::RegionKey;
use caditor_render::Layer;
use caditor_sketch::Sketch;
use egui::{Key, Modifiers};

use super::{Harness, edit_free_sketch, rectangle};
use crate::{scene_palette, selection::Pickable};

const OUTSIDE_THE_FIRST: Point2 = Point2::new(13.0, 13.0);
const OUTSIDE_THE_SECOND: Point2 = Point2::new(3.0, 3.0);
const OVERLAP: Point2 = Point2::new(8.0, 8.0);
const L_SHAPED_AREA: f64 = 84.0;
const OVERLAP_AREA: f64 = 16.0;
const DEPTH: f64 = 10.0;

fn crossing_squares(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    rectangle(&mut sketch, Point2::new(0.0, 0.0), Point2::new(10.0, 10.0));
    rectangle(&mut sketch, Point2::new(6.0, 6.0), Point2::new(16.0, 16.0));
    edit_free_sketch(harness, sketch)
}

fn region_holding(harness: &Harness, sketch: FeatureId, point: Point2) -> RegionKey {
    crate::selection::sketch_regions(harness.model.evaluation(), sketch)
        .expect("the sketch has regions")
        .iter()
        .find(|region| region.region.contains(point))
        .map(|region| region.region.key())
        .expect("a region holds the point")
}

fn sketch_region(harness: &Harness, sketch: FeatureId, point: Point2) -> Pickable {
    Pickable::SketchRegion {
        feature: sketch,
        region: region_holding(harness, sketch, point),
    }
}

fn sketch_regions_offered(harness: &mut Harness) -> usize {
    harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::SketchRegion { .. }))
        .count()
}

fn extrude_selection(harness: &mut Harness) -> FeatureId {
    harness.click("Extrude");
    harness.settle();
    harness
        .workspace
        .editing
        .solid()
        .expect("the extrusion is open")
}

fn chosen_keys(harness: &Harness, extrude: FeatureId) -> Vec<RegionKey> {
    let RegionChoice::Chosen(references) = harness.solid(extrude).regions() else {
        panic!("the extrusion sweeps every region");
    };
    references.iter().map(|reference| reference.key()).collect()
}

#[test]
fn the_area_one_square_leaves_outside_another_is_picked_in_the_sketch_and_extruded_alone() {
    let mut harness = Harness::new();
    let sketch = crossing_squares(&mut harness);
    let outside = sketch_region(&harness, sketch, OUTSIDE_THE_FIRST);

    let offered = sketch_regions_offered(&mut harness);
    harness.hover_pickable(Plane::XY, OUTSIDE_THE_FIRST, outside);
    let hovered = harness.built();
    let hovered_fill = hovered
        .scene
        .fills()
        .find(|fill| fill.pick == hovered.picks.id_of(outside))
        .expect("the region is drawn");
    harness.click_pickable(Plane::XY, OUTSIDE_THE_FIRST, outside);
    let selected: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();

    assert_eq!(offered, 3);
    assert_eq!(hovered_fill.layer, Layer::Front);
    assert_ne!(
        hovered_fill.color,
        scene_palette::STANDARD.closed_region,
        "hovering tints the area under the pointer"
    );
    assert_eq!(selected, [outside]);

    let extrude = extrude_selection(&mut harness);

    assert_eq!(
        chosen_keys(&harness, extrude),
        [region_holding(&harness, sketch, OUTSIDE_THE_FIRST)]
    );
    assert!(
        (harness.body_volume(extrude) - L_SHAPED_AREA * DEPTH).abs() < 1.0,
        "{}",
        harness.body_volume(extrude)
    );
}

#[test]
fn shift_adds_the_overlap_to_a_picked_area_and_both_extrude_as_one() {
    let mut harness = Harness::new();
    let sketch = crossing_squares(&mut harness);
    let outside = sketch_region(&harness, sketch, OUTSIDE_THE_FIRST);
    let overlap = sketch_region(&harness, sketch, OVERLAP);

    harness.click_pickable(Plane::XY, OUTSIDE_THE_FIRST, outside);
    harness.hold(Modifiers::SHIFT);
    harness.click_pickable(Plane::XY, OVERLAP, overlap);
    harness.hold(Modifiers::NONE);
    let selected = harness.workspace.viewport.selection().len();
    let extrude = extrude_selection(&mut harness);

    assert_eq!(selected, 2);
    assert_eq!(chosen_keys(&harness, extrude).len(), 2);
    assert!(
        (harness.body_volume(extrude) - (L_SHAPED_AREA + OVERLAP_AREA) * DEPTH).abs() < 1.0,
        "{}",
        harness.body_volume(extrude)
    );
}

#[test]
fn regions_are_picked_only_with_the_select_tool() {
    let mut harness = Harness::new();
    crossing_squares(&mut harness);

    let selecting = sketch_regions_offered(&mut harness);
    harness.use_tool(Key::L);
    let drawing = sketch_regions_offered(&mut harness);

    assert_eq!(selecting, 3);
    assert_eq!(drawing, 0);
}

#[test]
fn a_chosen_region_of_an_open_extrusion_is_picked_in_front_of_its_preview() {
    let mut harness = Harness::new();
    let sketch = crossing_squares(&mut harness);
    let outside = sketch_region(&harness, sketch, OUTSIDE_THE_FIRST);
    harness.click_pickable(Plane::XY, OUTSIDE_THE_FIRST, outside);
    let extrude = extrude_selection(&mut harness);
    let other = region_holding(&harness, sketch, OUTSIDE_THE_SECOND);
    let chosen = Pickable::Region {
        feature: extrude,
        region: region_holding(&harness, sketch, OUTSIDE_THE_FIRST),
    };

    let built = harness.built();
    let chosen_fill = built
        .scene
        .fills()
        .find(|fill| fill.pick == built.picks.id_of(chosen))
        .expect("the chosen region is drawn");
    assert_eq!(chosen_fill.layer, Layer::Front);

    harness.click_pickable(
        Plane::XY,
        OUTSIDE_THE_SECOND,
        Pickable::Region {
            feature: extrude,
            region: other,
        },
    );
    harness.settle();
    assert_eq!(chosen_keys(&harness, extrude).len(), 2);
    assert!(
        (harness.body_volume(extrude) - 2.0 * L_SHAPED_AREA * DEPTH).abs() < 1.0,
        "{}",
        harness.body_volume(extrude)
    );

    harness.click_pickable(Plane::XY, OUTSIDE_THE_FIRST, chosen);
    harness.settle();
    assert_eq!(chosen_keys(&harness, extrude), [other]);
}
