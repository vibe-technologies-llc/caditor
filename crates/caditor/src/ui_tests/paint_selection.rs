use caditor_geometry::{Plane, Point2, Point3, Vector3};
use egui::Pos2;

use super::{
    CAMERA_SETTLE, Harness, drag_path, extruded_plate, plate_on_screen, run_from_palette,
    selected_kinds,
};
use crate::selection::{Pickable, SelectionFilter};

fn on_top(harness: &Harness, points: &[(f64, f64)]) -> Vec<Pos2> {
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    points
        .iter()
        .map(|(x, y)| {
            harness
                .workspace
                .viewport
                .screen_position(top, Point2::new(*x, *y))
                .unwrap()
        })
        .collect()
}

fn ring_inside(harness: &Harness, share: f32) -> Vec<Pos2> {
    let (low, high) = plate_on_screen(harness);
    let middle = low.lerp(high, 0.5);
    let reach = (high - low) * 0.5 * share;
    (0..=48)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 48.0;
            middle + egui::vec2(angle.cos() * reach.x, angle.sin() * reach.y)
        })
        .collect()
}

#[test]
fn painting_over_faces_selects_each_face_the_brush_passes_over() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();

    run_from_palette(&mut harness, "select faces by painting");
    harness.frame();
    let painting = harness.workspace.viewport.paint();
    let stroke = on_top(&harness, &[(12.0, 12.0), (20.0, 20.0), (28.0, 28.0)]);
    drag_path(&mut harness, &stroke);
    let on_the_top: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();

    let ring = ring_inside(&harness, 0.8);
    drag_path(&mut harness, &ring);
    let around = selected_kinds(&harness);
    let top_kept = harness.workspace.viewport.selection().contains(top);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Bodies);
    drag_path(&mut harness, &stroke);
    let whole = selected_kinds(&harness);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    drag_path(&mut harness, &stroke);
    let boxed = selected_kinds(&harness);

    assert!(painting);
    assert_eq!(on_the_top, vec![top]);
    assert_eq!(around, (3, 0, 0));
    assert!(top_kept);
    assert_eq!(whole, (6, 0, 0));
    assert_eq!(boxed, (0, 0, 0));
}
