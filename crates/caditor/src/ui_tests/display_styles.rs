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

fn styles_of(harness: &mut Harness) -> Vec<caditor_render::FaceStyle> {
    let built = harness.built();
    built
        .scene
        .meshes
        .iter()
        .flat_map(|instance| instance.faces.iter().copied())
        .collect()
}

#[test]
fn the_rendered_style_lights_the_scene_and_finishes_every_face_with_the_material_s_preset() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    harness.select([]);
    harness.model.perform(crate::model::Action::Apply(
        caditor_document::Transaction::single(
            "Make the plate steel",
            caditor_document::Edit::SetBodyAppearance {
                id: plate,
                appearance: caditor_document::BodyAppearance {
                    material: Some("steel".to_owned()),
                    ..Default::default()
                },
            },
        ),
    ));
    harness.settle();
    let shaded = styles_of(&mut harness);
    let lit_before = harness.built().scene.lighting;

    choose_from_menu(&mut harness, "Rendered");
    harness.frame();
    let rendered = styles_of(&mut harness);
    let lighting = harness.built().scene.lighting;

    choose_from_menu(&mut harness, "Shaded with edges");
    harness.frame();
    let back = styles_of(&mut harness);

    let brushed = crate::body_appearance::surface(caditor_document::Finish::BrushedMetal);
    assert!(shaded.iter().all(|face| face.finish.is_none()));
    assert_eq!(lit_before, None);
    assert_eq!(
        harness.workspace.viewport.style(),
        DisplayStyle::ShadedWithEdges
    );
    assert!(!rendered.is_empty());
    assert!(rendered.iter().all(|face| face.finish == Some(brushed)));
    assert_eq!(
        lighting,
        Some(harness.workspace.preferences.lighting.render())
    );
    assert!(back.iter().all(|face| face.finish.is_none()));
}

#[test]
fn a_body_without_finish_or_known_material_renders_satin_and_a_selected_face_keeps_its_plain_look()
{
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([]);
    choose_from_menu(&mut harness, "Rendered");
    harness.frame();
    let satin = crate::body_appearance::surface(caditor_document::Finish::Satin);
    let rendered = styles_of(&mut harness);

    harness.select([top]);
    harness.frame();
    let selected = styles_of(&mut harness);

    assert!(rendered.iter().all(|face| face.finish == Some(satin)));
    assert_eq!(
        selected.iter().filter(|face| face.finish.is_none()).count(),
        1
    );
}

#[test]
fn the_lighting_panel_opens_from_the_palette_and_its_settings_reach_the_rendered_scene() {
    let mut harness = plate();
    choose_from_menu(&mut harness, "Rendered");

    run_from_palette(&mut harness, "light the rendered view");
    harness.frame();
    let open = harness.workspace.lighting.open;
    let low_sun = crate::lighting::Lighting {
        elevation: 10.0,
        shadows: false,
        ..crate::lighting::Lighting::default()
    };
    harness.perform(crate::model::Action::Preferences(
        crate::preferences::PreferencesCommand::Change(
            crate::preferences::PreferenceChange::Lighting(low_sun),
        ),
    ));
    harness.frame();
    let lit = harness.built().scene.lighting;

    run_from_palette(&mut harness, "reset the rendered view's lighting");
    harness.frame();
    let reset = harness.workspace.preferences.lighting;

    assert!(open);
    assert!(harness.shows(crate::lighting_panel::TITLE));
    assert_eq!(lit, Some(low_sun.render()));
    assert_eq!(reset, crate::lighting::Lighting::default());
}
