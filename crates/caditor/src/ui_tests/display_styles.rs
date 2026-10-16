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
    harness.click_lowest(title);
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
fn a_body_without_finish_or_known_material_renders_satin_and_a_selected_face_stays_rendered_in_its_highlight()
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
    assert!(selected.iter().all(|face| face.finish == Some(satin)));
    assert_ne!(selected, rendered);
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

#[test]
fn the_lighting_panel_offers_a_plain_backdrop_and_a_visible_floor_that_reach_the_rendered_scene() {
    let mut harness = plate();
    choose_from_menu(&mut harness, "Rendered");
    run_from_palette(&mut harness, "light the rendered view");
    harness.frame();
    let surroundings = |harness: &mut Harness| harness.built().scene.lighting.unwrap().surroundings;
    let at_first = surroundings(&mut harness);

    harness.click_button(&crate::lighting_panel::backdrop_swatch_name("Charcoal"));
    harness.frame();
    harness.frame();
    let charcoal = surroundings(&mut harness);
    let stored =
        crate::lighting::Lighting::from_settings(&harness.workspace.preferences.settings());

    harness.click(crate::lighting_panel::FLOOR);
    harness.frame();
    harness.frame();
    let floored = surroundings(&mut harness);

    harness.click_button(crate::lighting_panel::CUSTOM_BACKDROP);
    harness.frame();
    let hex =
        crate::colour_selector::hex_id(egui::Id::unique(crate::lighting_panel::BACKDROP_SELECTOR));
    let hiding = crate::body_appearance::DEFAULT_COLOUR.hex();
    harness.type_into_field(hex, &hiding);
    harness.frame();
    let refused = surroundings(&mut harness);
    let said = harness.shows_containing("shows bodies in");

    harness.click_button(crate::lighting_panel::SKY_SWATCH);
    harness.frame();
    harness.frame();
    let sky_again = surroundings(&mut harness);

    assert_eq!(at_first, caditor_render::Surroundings::default());
    assert_eq!(
        charcoal.backdrop,
        Some(caditor_render::Color::from_rgb8(0x2b, 0x2e, 0x34))
    );
    assert!(!charcoal.floor);
    assert_eq!(
        stored.backdrop_colour,
        Some(caditor_document::Rgb::new(0x2b, 0x2e, 0x34))
    );
    assert!(floored.floor);
    assert_eq!(floored.backdrop, charcoal.backdrop);
    assert_eq!(refused, floored);
    assert!(said);
    assert_eq!(sky_again.backdrop, None);
    assert!(sky_again.floor);
}

#[test]
fn the_display_picker_in_the_view_names_the_style_and_switches_it_from_its_menu() {
    let mut harness = plate();
    let shaded = crate::display_picker::name_of(DisplayStyle::ShadedWithEdges);
    let rendered = crate::display_picker::name_of(DisplayStyle::Rendered);
    let at_first = harness.button_rect(&shaded);

    harness.click_button(&shaded);
    harness.frame();
    harness.click("Rendered");
    harness.frame();
    harness.frame();
    let style = harness.workspace.viewport.style();
    let renamed = harness.button_rect(&rendered);

    harness.click_button(&rendered);
    harness.frame();
    harness.click(&crate::commands::Command::Lighting.title());
    harness.frame();
    harness.frame();

    assert!(at_first.width() > 0.0);
    assert_eq!(style, DisplayStyle::Rendered);
    assert!(renamed.width() > 0.0);
    assert!(harness.workspace.lighting.open);
}

fn radiance_file(folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 16 +X 32\n".to_vec();
    for _ in 0..32 * 16 {
        bytes.extend([128, 64, 32, 131]);
    }
    let path = folder.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_sky_picture_chosen_in_the_lighting_panel_is_read_in_the_background_and_reaches_the_rendered_scene()
 {
    let folder = tempfile::tempdir().unwrap();
    let picture = radiance_file(folder.path(), "studio.hdr");
    let mut harness = plate();
    choose_from_menu(&mut harness, "Rendered");
    run_from_palette(&mut harness, "light the rendered view");
    harness.frame();
    let plain = harness.built().scene.backdrop.is_none();

    harness.answer_dialog(Some(picture.clone()));
    run_from_palette(&mut harness, "choose a sky picture for the rendered view");
    harness.wait_until("the sky picture is ready", |harness| {
        harness.workspace.backdrop.environment().is_some()
    });
    harness.frame();
    let backdrop = harness.built().scene.backdrop;
    let stored = harness.workspace.preferences.backdrop.clone();
    let remembered = crate::backdrop::read(&harness.workspace.preferences.settings());

    run_from_palette(&mut harness, "use the plain sky in the rendered view");
    harness.frame();
    harness.frame();

    assert!(plain);
    assert!(harness.shows(crate::lighting_panel::SKY_PICTURE));
    let backdrop = backdrop.expect("the rendered scene carries the picture");
    assert_eq!(
        (backdrop.environment.width(), backdrop.environment.height()),
        (32, 16)
    );
    assert_eq!(stored.as_deref(), Some(picture.as_path()));
    assert_eq!(remembered.as_deref(), Some(picture.as_path()));
    assert!(harness.workspace.preferences.backdrop.is_none());
    assert!(harness.built().scene.backdrop.is_none());
    assert!(harness.workspace.backdrop.environment().is_none());
}

#[test]
fn a_sky_picture_that_cannot_be_read_says_so_and_the_plain_sky_stays() {
    let folder = tempfile::tempdir().unwrap();
    let notes = folder.path().join("notes.hdr");
    std::fs::write(&notes, b"not a picture").unwrap();
    let mut harness = plate();
    choose_from_menu(&mut harness, "Rendered");
    run_from_palette(&mut harness, "light the rendered view");

    harness.answer_dialog(Some(notes));
    run_from_palette(&mut harness, "choose a sky picture for the rendered view");
    harness.wait_until("the sky picture is refused", |harness| {
        matches!(
            harness.workspace.backdrop.status(),
            crate::backdrop::Status::Failed(..)
        )
    });
    harness.frame();
    harness.frame();

    let notice = harness.model.notice().expect("the refusal is announced");
    assert!(notice.text.contains("Radiance"), "{}", notice.text);
    assert!(notice.text.contains("notes.hdr"), "{}", notice.text);
    assert!(harness.workspace.backdrop.environment().is_none());
    assert!(harness.built().scene.backdrop.is_none());
    assert!(harness.workspace.preferences.backdrop.is_some());
}
