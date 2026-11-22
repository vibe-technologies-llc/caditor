use std::{collections::BTreeSet, f64::consts::PI};

use caditor_geometry::Point2;

use super::*;
use crate::test_support::{circle, rectangle};

const DISC: u64 = 1;
const FACE: u64 = 1 << 62;
const WINDOW: u64 = FACE + 4;
const BOLT: u64 = FACE + 8;
const FAR_WINDOW: u64 = FACE + 9;

fn face_cutters() -> Vec<ProfileCurve> {
    let mut cutters = rectangle(FACE, (-20.0, -20.0), (20.0, 20.0));

    cutters.extend(rectangle(WINDOW, (5.0, -5.0), (15.0, 5.0)));
    cutters
}

fn cut(curves: &[ProfileCurve], cutters: &[ProfileCurve]) -> CutProfile {
    let plain = Profile::new(curves).unwrap();
    CutProfile::new(&plain, curves, cutters).unwrap()
}

fn offered_areas(cut: &CutProfile) -> Vec<f64> {
    let mut areas: Vec<f64> = cut.offered().map(|(region, _)| region.area()).collect();

    areas.sort_by(f64::total_cmp);
    areas
}

fn offered_keys(cut: &CutProfile) -> BTreeSet<RegionKey> {
    cut.offered().map(|(region, _)| region.key()).collect()
}

fn holding(cut: &CutProfile, point: Point2) -> &Region {
    cut.offered()
        .map(|(region, _)| region)
        .find(|region| region.contains(point))
        .unwrap()
}

fn window_piece_area() -> f64 {
    let corner = 75.0_f64.sqrt();
    5.0 * corner - 50.0 + 50.0 * PI / 3.0
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-6 * (1.0 + expected.abs())
}

#[test]
fn a_circle_crossing_a_window_of_the_face_offers_the_piece_inside_the_window() {
    let curves = [circle(DISC, (0.0, 0.0), 10.0)];
    let plain = Profile::new(&curves).unwrap();

    let cut = cut(&curves, &face_cutters());
    let areas = offered_areas(&cut);
    let disc = plain.regions().first().unwrap().key();
    let window = holding(&cut, Point2::new(7.0, 0.0));

    assert!(cut.is_cut());
    assert_eq!(areas.len(), 2, "{areas:?}");
    assert!(close(areas[0], window_piece_area()), "{areas:?}");
    assert!(
        close(areas[1], 100.0 * PI - window_piece_area()),
        "{areas:?}"
    );
    assert!(cut.offered().all(|(_, holder)| holder == disc));
    assert!(cut.bounded_by_cutter(window));
    assert!(window.pieces().any(|piece| piece.entity() == WINDOW + 3));
    assert_eq!(cut.cutters().len(), 3);
}

#[test]
fn faces_outside_every_sketch_curve_are_not_offered() {
    let curves = [circle(DISC, (0.0, 0.0), 10.0)];
    let cutters = rectangle(WINDOW, (5.0, -5.0), (9.0, 5.0));

    let cut = cut(&curves, &cutters);
    let corner_of_the_window = Point2::new(8.95, 4.95);

    assert!(
        cut.profile()
            .regions()
            .iter()
            .any(|region| region.contains(corner_of_the_window))
    );
    assert!(
        cut.offered()
            .all(|(region, _)| !region.contains(corner_of_the_window))
    );
    assert_eq!(cut.offered().count(), 3);
}

#[test]
fn a_hole_of_the_face_inside_the_circle_is_offered_whole() {
    let curves = [circle(DISC, (0.0, 0.0), 10.0)];
    let mut cutters = face_cutters();

    cutters.push(circle(BOLT, (-4.0, 0.0), 1.0));

    let cut = cut(&curves, &cutters);
    let areas = offered_areas(&cut);
    let bolt = holding(&cut, Point2::new(-4.0, 0.0));

    assert_eq!(areas.len(), 3, "{areas:?}");
    assert!(close(bolt.area(), PI), "{}", bolt.area());
    assert!(close(areas.iter().sum::<f64>(), 100.0 * PI));
}

#[test]
fn pieces_keep_their_names_when_far_edges_come_and_go_and_the_circle_moves_a_little() {
    let curves = [circle(DISC, (0.0, 0.0), 10.0)];
    let moved = [circle(DISC, (0.001, -0.002), 10.0)];
    let mut more = face_cutters();

    more.extend(rectangle(FAR_WINDOW, (-18.0, -18.0), (-15.0, -15.0)));

    let first = cut(&curves, &face_cutters());
    let with_far_window = cut(&curves, &more);
    let after_moving = cut(&moved, &face_cutters());
    let reference = RegionReference::capture(holding(&first, Point2::new(7.0, 0.0)), None);

    assert_eq!(offered_keys(&first), offered_keys(&with_far_window));
    assert_eq!(offered_keys(&first), offered_keys(&after_moving));
    assert_eq!(with_far_window.cutters().len(), 3);
    assert_eq!(
        reference.resolve(after_moving.offered().map(|(region, _)| region)),
        Ok(RegionMatch::Same(reference.key()))
    );
}

#[test]
fn a_circle_clear_of_every_edge_is_not_cut_and_keeps_its_region() {
    let curves = [circle(DISC, (2.0, 2.0), 2.5)];
    let plain = Profile::new(&curves).unwrap();
    let mut cutters = face_cutters();

    cutters.push(circle(BOLT, (5.0, 5.0), 1.0));

    let cut = cut(&curves, &cutters);

    assert!(!cut.is_cut());
    assert_eq!(cut.cutters().len(), 1);
    assert_eq!(
        offered_keys(&cut),
        plain.regions().iter().map(Region::key).collect()
    );
}

#[test]
fn a_sketch_with_no_region_takes_no_cutter() {
    let curves = [crate::test_support::line(DISC, (0.0, 0.0), (5.0, 5.0))];

    let cut = cut(&curves, &face_cutters());

    assert!(cut.cutters().is_empty());
    assert_eq!(cut.offered().count(), 0);
}
