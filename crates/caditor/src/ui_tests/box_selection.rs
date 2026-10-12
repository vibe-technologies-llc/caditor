use caditor_geometry::{Plane, Point2, Point3, Vector3};
use egui::{Key, Modifiers, Pos2};

use super::{
    CAMERA_SETTLE, Harness, drag_path, drag_screen, extruded_plate, plate_on_screen,
    run_from_palette, selected_kinds,
};
use crate::{
    display_style::DisplayStyle,
    scene_palette,
    selection::{Pickable, SelectionFilter},
};

#[test]
fn a_lasso_drawn_around_the_model_takes_what_lies_inside_it() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = 12.0;
    let middle = low.lerp(high, 0.5);
    let ring: Vec<Pos2> = (0..=24)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / 24.0;
            let reach = (high - low) * 0.75 + egui::vec2(margin, margin);
            middle + egui::vec2(angle.cos() * reach.x, angle.sin() * reach.y)
        })
        .collect();
    let small: Vec<Pos2> = ring
        .iter()
        .map(|point| middle + (*point - middle) * 0.05)
        .collect();

    run_from_palette(&mut harness, "select with a lasso");
    harness.frame();
    let lasso = harness.workspace.viewport.lasso();
    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Vertices);
    drag_path(&mut harness, &ring);
    let around = selected_kinds(&harness);
    drag_path(&mut harness, &small);
    let inside = selected_kinds(&harness);

    assert!(lasso);
    assert_eq!(around, (0, 0, 7));
    assert_eq!(inside, (0, 0, 0));
}

#[test]
fn select_through_lets_a_box_take_the_edges_and_corners_hidden_behind_the_model() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Vertices);
    drag_screen(&mut harness, low - margin, high + margin);
    let seen = selected_kinds(&harness);

    run_from_palette(&mut harness, "select through to what is hidden");
    harness.frame();
    assert!(harness.workspace.viewport.select_through());
    let (low, high) = plate_on_screen(&harness);
    drag_screen(&mut harness, low - margin, high + margin);
    let through = selected_kinds(&harness);

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    drag_screen(&mut harness, low - margin, high + margin);
    let edges_through = selected_kinds(&harness);

    assert_eq!(seen, (0, 0, 7));
    assert_eq!(through, (0, 0, 8));
    assert_eq!(edges_through, (0, 12, 0));
}

#[test]
fn where_the_style_picks_no_faces_a_box_takes_every_edge_and_painting_draws_a_box() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);
    let mut caught = Vec::new();

    for style in [DisplayStyle::Wireframe, DisplayStyle::XRay] {
        harness.workspace.viewport.set_style(style);
        harness.frame();
        for filter in [
            SelectionFilter::Everything,
            SelectionFilter::Edges,
            SelectionFilter::Vertices,
        ] {
            harness.workspace.viewport.set_filter(filter);
            drag_screen(&mut harness, low - margin, high + margin);
            caught.push(selected_kinds(&harness));
        }
    }
    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Everything);
    run_from_palette(&mut harness, "select faces by painting");
    harness.frame();
    harness
        .workspace
        .viewport
        .set_style(DisplayStyle::Wireframe);
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    drag_screen(&mut harness, low - margin, high + margin);
    let painted_in_wireframe = selected_kinds(&harness);

    let through = [(0, 12, 0), (0, 12, 0), (0, 0, 8)];
    assert_eq!(caught, [through, through].concat());
    assert!(harness.workspace.viewport.paint());
    assert_eq!(painted_in_wireframe, (0, 12, 0));
}

#[test]
fn a_box_dragged_over_the_model_selects_what_it_holds_or_touches_by_the_filter() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let (low, high) = plate_on_screen(&harness);
    let margin = egui::vec2(12.0, 12.0);

    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (3, 0, 0));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Edges);
    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (0, 9, 0));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Vertices);
    drag_screen(&mut harness, low - margin, high + margin);
    assert_eq!(selected_kinds(&harness), (0, 0, 7));

    harness
        .workspace
        .viewport
        .set_filter(SelectionFilter::Everything);
    let middle = harness
        .workspace
        .viewport
        .screen_position(
            Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap(),
            Point2::new(20.0, 20.0),
        )
        .unwrap();
    drag_screen(
        &mut harness,
        middle + egui::vec2(5.0, 5.0),
        middle - egui::vec2(5.0, 5.0),
    );
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        vec![top]
    );
}

#[test]
fn a_display_style_hides_the_faces_or_the_edges_but_keeps_what_is_left_pickable() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.frame();
    let edge_alphas = |harness: &mut Harness| -> Vec<f32> {
        let built = harness.built();
        let edges: Vec<_> = built
            .picks
            .pickables()
            .filter(|pickable| matches!(pickable, Pickable::Edge { .. }))
            .filter_map(|pickable| built.picks.id_of(pickable))
            .collect();
        built
            .scene
            .batches
            .iter()
            .flat_map(|batch| batch.lines.iter())
            .filter(|line| line.pick.is_some_and(|pick| edges.contains(&pick)))
            .map(|line| line.color.alpha)
            .collect()
    };
    let silhouettes = |harness: &mut Harness| -> Vec<caditor_render::Color> {
        harness
            .built()
            .scene
            .silhouettes
            .iter()
            .map(|silhouette| silhouette.color)
            .collect()
    };
    let faces_pickable = |harness: &mut Harness| {
        harness
            .built()
            .picks
            .pickables()
            .any(|pickable| matches!(pickable, Pickable::Face { .. }))
    };
    assert_eq!(
        harness.workspace.viewport.style(),
        DisplayStyle::ShadedWithEdges
    );
    assert_eq!(harness.built().scene.meshes.len(), 1);
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));
    assert_eq!(
        silhouettes(&mut harness),
        vec![scene_palette::STANDARD.body_edge]
    );

    run_from_palette(&mut harness, "shaded without edges");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Shaded);
    assert_eq!(harness.built().scene.meshes.len(), 1);
    let hidden = edge_alphas(&mut harness);
    assert!(!hidden.is_empty() && hidden.iter().all(|alpha| *alpha == 0.0));
    assert!(silhouettes(&mut harness).is_empty());
    assert!(faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "wireframe");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Wireframe);
    assert_eq!(
        silhouettes(&mut harness),
        vec![scene_palette::STANDARD.body_edge]
    );
    assert!(harness.built().scene.meshes.is_empty());
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));
    assert!(!faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::XRay);
    assert_eq!(
        silhouettes(&mut harness),
        vec![scene_palette::STANDARD.body_edge]
    );
    let built = harness.built();
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| face.pick.is_none() && face.color.alpha < 1.0)
    );
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha > 0.0));
    assert!(!faces_pickable(&mut harness));

    harness.key(Key::Escape, Modifiers::NONE);
    run_from_palette(&mut harness, "hidden lines removed");
    harness.frame();
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::HiddenLine);
    assert_eq!(
        silhouettes(&mut harness),
        vec![scene_palette::STANDARD.body_edge]
    );
    let built = harness.built();
    assert!(built.scene.meshes.is_empty());
    assert!(built.scene.translucent_meshes.is_empty());
    assert_eq!(built.scene.flat_meshes.len(), 1);
    let first = built.scene.flat_meshes[0].faces[0].color;
    assert!(
        built.scene.flat_meshes[0]
            .faces
            .iter()
            .all(|face| face.color == first && face.pick.is_some())
    );
    assert!(first.red > 0.9 && first.green > 0.9 && first.blue > 0.9);
    assert!(edge_alphas(&mut harness).iter().all(|alpha| *alpha == 1.0));
    assert!(faces_pickable(&mut harness));
}

#[test]
fn shaded_with_hidden_edges_dashed_draws_each_edge_dashed_behind_the_faces_without_a_copy() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    let hidden_lines = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .lines()
            .filter(|line| line.stroke.dashes_where_hidden())
            .cloned()
            .collect::<Vec<_>>()
    };
    let line_count = |harness: &mut Harness| harness.built().scene.lines().count();
    let silhouettes_dashed_where_hidden = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .silhouettes
            .iter()
            .map(|silhouette| silhouette.dashed_where_hidden)
            .collect::<Vec<_>>()
    };
    let before = hidden_lines(&mut harness).len();
    let lines_before = line_count(&mut harness);
    let silhouettes_before = silhouettes_dashed_where_hidden(&mut harness);

    run_from_palette(&mut harness, "shaded with hidden edges dashed");
    harness.frame();
    let style = harness.workspace.viewport.style();
    let lines = hidden_lines(&mut harness);
    let lines_shown = line_count(&mut harness);
    let silhouettes = silhouettes_dashed_where_hidden(&mut harness);
    let faces = harness.built().scene.meshes.len();
    let edges_pickable = harness
        .built()
        .picks
        .pickables()
        .filter(|pickable| matches!(pickable, Pickable::Edge { .. }))
        .count();

    run_from_palette(&mut harness, "shaded with edges");
    harness.frame();
    let after = hidden_lines(&mut harness).len();
    let silhouettes_after = silhouettes_dashed_where_hidden(&mut harness);

    assert_eq!(style, DisplayStyle::ShadedWithHiddenEdges);
    assert_eq!(before, 0);
    assert_eq!(silhouettes_before, vec![false]);
    assert_eq!(silhouettes, vec![true]);
    assert_eq!(silhouettes_after, vec![false]);
    assert!(!lines.is_empty());
    assert_eq!(lines_shown, lines_before);
    assert!(lines.iter().all(|line| line.pick.is_some()
        && line.layer == caditor_render::Layer::Model
        && !line.stroke.is_dashed_where_seen()));
    assert_eq!(faces, 1);
    assert!(edges_pickable > 0);
    assert_eq!(after, 0);
}
