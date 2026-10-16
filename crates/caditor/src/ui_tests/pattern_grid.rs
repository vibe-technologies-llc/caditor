use caditor_document::{FeatureId, Pattern};
use caditor_geometry::Point3;
use egui::{Id, Pos2};

use super::{CAMERA_SETTLE, Harness, extruded_plate, pattern_of, run_from_palette};
use crate::selection::Axis;

const SPACING: f64 = 48.0;
const PLATE_CENTRE: Point3 = Point3::new(20.0, 20.0, 5.0);
const COUNT: u32 = 3;

struct Grid {
    pattern: FeatureId,
    plate: FeatureId,
}

fn box_name(instance: [u32; 2]) -> String {
    caditor_document::capitalized(&caditor_document::instance_name(instance))
}

fn open_grid(harness: &mut Harness, (reversed_first, reversed_second): (bool, bool)) -> Grid {
    let (plate, _) = extruded_plate(harness);
    harness.select([]);
    harness.click("Linear pattern");
    harness.settle();
    let pattern = harness
        .workspace
        .editing
        .solid()
        .expect("the pattern is open");
    harness.type_into_field(Id::unique(("pattern-field", "count", pattern)), "3");
    harness.settle();
    harness.select([crate::selection::Pickable::Axis(Axis::Y)]);
    run_from_palette(harness, "pattern also along selected");
    harness.settle();
    harness.type_into_field(Id::unique(("pattern-field", "second-count", pattern)), "3");
    harness.settle();
    harness.type_into_field(
        Id::unique(("pattern-field", "second-spacing", pattern)),
        "48 mm",
    );
    harness.settle();
    if reversed_first {
        harness.click_lowest(crate::feature_fields::REVERSE_DIRECTION);
        harness.settle();
    }
    if reversed_second {
        harness.click_lowest("Reverse second direction");
        harness.settle();
    }
    run_from_palette(harness, "top view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    Grid { pattern, plate }
}

fn signs(pattern: &Pattern) -> (f64, f64) {
    let (first, second) = pattern.reversed_directions();
    let sign = |reversed: bool| if reversed { -1.0 } else { 1.0 };
    (sign(first), sign(second))
}

fn copy_centre(pattern: &Pattern, [column, row]: [u32; 2]) -> Point3 {
    let (along, across) = signs(pattern);
    PLATE_CENTRE
        + Point3::new(
            SPACING * f64::from(column) * along,
            SPACING * f64::from(row) * across,
            0.0,
        )
}

fn box_position(harness: &mut Harness, instance: [u32; 2]) -> Pos2 {
    harness.button_rect(&box_name(instance)).center()
}

fn copy_position(harness: &Harness, grid: &Grid, instance: [u32; 2]) -> Pos2 {
    harness
        .workspace
        .viewport
        .screen_of(copy_centre(pattern_of(harness, grid.pattern), instance))
        .expect("the copy is in view")
}

fn instances() -> Vec<[u32; 2]> {
    (0..COUNT)
        .flat_map(|row| (0..COUNT).map(move |column| [column, row]))
        .collect()
}

fn centroid_without(harness: &Harness, grid: &Grid, left_out: [u32; 2]) -> Point3 {
    let pattern = pattern_of(harness, grid.pattern);
    let kept: Vec<Point3> = instances()
        .into_iter()
        .filter(|instance| *instance != left_out)
        .map(|instance| copy_centre(pattern, instance))
        .collect();
    kept.iter().fold(Point3::ZERO, |sum, point| sum + *point) / kept.len() as f64
}

fn centroid(harness: &Harness, grid: &Grid) -> Point3 {
    harness
        .model
        .evaluation()
        .body_result(grid.plate)
        .and_then(|result| result.solid())
        .and_then(caditor_document::SolidResult::mesh)
        .expect("the body is meshed")
        .mass_properties()
        .centroid
}

fn assert_grid_matches_view(harness: &mut Harness, grid: &Grid) {
    for first in instances() {
        for second in instances() {
            if first == second {
                continue;
            }
            let (box_a, box_b) = (box_position(harness, first), box_position(harness, second));
            let (view_a, view_b) = (
                copy_position(harness, grid, first),
                copy_position(harness, grid, second),
            );
            let horizontal = |a: f32, b: f32| (a - b).abs() > 2.0;
            if horizontal(view_a.x, view_b.x) {
                assert_eq!(
                    box_a.x < box_b.x,
                    view_a.x < view_b.x,
                    "{first:?} and {second:?} run the same way across"
                );
            }
            if horizontal(view_a.y, view_b.y) {
                assert_eq!(
                    box_a.y < box_b.y,
                    view_a.y < view_b.y,
                    "{first:?} and {second:?} run the same way down"
                );
            }
        }
    }
}

fn top_middle(positions: &[([u32; 2], Pos2)]) -> [u32; 2] {
    let mut rows: Vec<&([u32; 2], Pos2)> = positions.iter().collect();
    rows.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
    let mut top: Vec<&([u32; 2], Pos2)> = rows.into_iter().take(COUNT as usize).collect();
    top.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    top.get(1).map(|(instance, _)| *instance).unwrap()
}

fn assert_clicking_the_top_middle_box_leaves_out_the_copy_there(reversed: (bool, bool)) {
    let mut harness = Harness::new();
    let grid = open_grid(&mut harness, reversed);
    assert_eq!(
        pattern_of(&harness, grid.pattern).reversed_directions(),
        reversed
    );

    let boxes: Vec<([u32; 2], Pos2)> = instances()
        .into_iter()
        .map(|instance| (instance, box_position(&mut harness, instance)))
        .collect();
    let copies: Vec<([u32; 2], Pos2)> = instances()
        .into_iter()
        .map(|instance| (instance, copy_position(&harness, &grid, instance)))
        .collect();
    let target = top_middle(&copies);
    let in_the_grid = top_middle(&boxes);

    assert_eq!(target, [1, if reversed.1 { 0 } else { 2 }]);
    assert_eq!(in_the_grid, target);

    harness.click_button(&box_name(in_the_grid));
    harness.settle();

    assert!(pattern_of(&harness, grid.pattern).is_skipped(target));
    assert_eq!(pattern_of(&harness, grid.pattern).skipped.len(), 1);
    let expected = centroid_without(&harness, &grid, target);
    assert!(centroid(&harness, &grid).distance(expected) < 0.01);
}

#[test]
fn the_instances_grid_lies_the_way_the_copies_do_in_the_top_view() {
    let mut harness = Harness::new();
    let grid = open_grid(&mut harness, (false, false));

    let origin = harness.workspace.viewport.screen_of(Point3::ZERO).unwrap();
    let along_x = harness
        .workspace
        .viewport
        .screen_of(Point3::new(10.0, 0.0, 0.0))
        .unwrap();
    let along_y = harness
        .workspace
        .viewport
        .screen_of(Point3::new(0.0, 10.0, 0.0))
        .unwrap();
    assert!(along_x.x > origin.x);
    assert!(along_y.y < origin.y);

    assert_grid_matches_view(&mut harness, &grid);
}

#[test]
fn the_instances_grid_follows_a_reversed_second_direction() {
    let mut harness = Harness::new();
    let grid = open_grid(&mut harness, (false, true));

    assert_grid_matches_view(&mut harness, &grid);
}

#[test]
fn the_instances_grid_follows_a_reversed_first_direction() {
    let mut harness = Harness::new();
    let grid = open_grid(&mut harness, (true, false));

    assert_grid_matches_view(&mut harness, &grid);
}

#[test]
fn unchecking_the_box_at_the_top_of_the_grid_leaves_out_the_copy_at_the_top_of_the_view() {
    assert_clicking_the_top_middle_box_leaves_out_the_copy_there((false, false));
}

#[test]
fn unchecking_a_box_with_the_second_direction_reversed_leaves_out_the_copy_there() {
    assert_clicking_the_top_middle_box_leaves_out_the_copy_there((false, true));
}

#[test]
fn unchecking_a_box_with_both_directions_reversed_leaves_out_the_copy_there() {
    assert_clicking_the_top_middle_box_leaves_out_the_copy_there((true, true));
}
