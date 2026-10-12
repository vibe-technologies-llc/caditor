use super::{CAMERA_SETTLE, Harness, extruded_plate, run_from_palette};
use crate::display_style::DisplayStyle;

fn plate() -> Harness {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    run_from_palette(&mut harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    harness
}

fn choose_from_menu(harness: &mut Harness, title: &str) {
    harness.click("View");
    harness.hover("Display style");
    harness.click(title);
    harness.frame();
}

fn drawn_see_through(harness: &mut Harness) -> bool {
    let built = harness.built();
    assert_eq!(
        built.scene.meshes.len() + built.scene.translucent_meshes.len(),
        1
    );
    !built.scene.translucent_meshes.is_empty()
}

#[test]
fn x_ray_turns_on_and_off_from_the_palette() {
    let mut harness = plate();
    let at_first = drawn_see_through(&mut harness);

    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    let on = drawn_see_through(&mut harness);
    let style_on = harness.workspace.viewport.style();

    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    let off = drawn_see_through(&mut harness);
    let style_off = harness.workspace.viewport.style();

    assert!(!at_first);
    assert!(on);
    assert_eq!(style_on, DisplayStyle::XRay);
    assert!(!off);
    assert_eq!(style_off, DisplayStyle::ShadedWithEdges);
}

#[test]
fn x_ray_turns_on_and_off_from_the_view_menu() {
    let mut harness = plate();

    choose_from_menu(&mut harness, "X-ray");
    let on = drawn_see_through(&mut harness);

    choose_from_menu(&mut harness, "X-ray");
    let off = drawn_see_through(&mut harness);

    assert!(on);
    assert!(!off);
    assert_eq!(
        harness.workspace.viewport.style(),
        DisplayStyle::ShadedWithEdges
    );
}

#[test]
fn turning_x_ray_off_returns_to_the_style_it_replaced() {
    let mut harness = plate();

    run_from_palette(&mut harness, "shaded without edges");
    harness.frame();
    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    let on = drawn_see_through(&mut harness);
    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    let off = drawn_see_through(&mut harness);

    assert!(on);
    assert!(!off);
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Shaded);
}

#[test]
fn another_style_chosen_from_either_route_ends_x_ray() {
    let mut harness = plate();

    run_from_palette(&mut harness, "x-ray");
    harness.frame();
    run_from_palette(&mut harness, "shaded with edges");
    harness.frame();
    let from_palette = drawn_see_through(&mut harness);

    choose_from_menu(&mut harness, "X-ray");
    choose_from_menu(&mut harness, "Shaded without edges");
    let from_menu = drawn_see_through(&mut harness);

    assert!(!from_palette);
    assert!(!from_menu);
    assert_eq!(harness.workspace.viewport.style(), DisplayStyle::Shaded);
}
