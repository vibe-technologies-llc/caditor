use caditor_document::{Edit, FeatureId, Transaction};
use egui::{Event, Id, Pos2};

use super::{
    Harness, extruded_plate, painted_faces, run_from_palette, scroll_in_view, set_high_contrast,
};
use crate::{model::Action, scene_palette, selection::Pickable};

#[test]
fn a_body_drawn_half_see_through_goes_to_the_translucent_pass() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    harness.select([]);

    harness.click_lowest("Solid");
    harness.frame();
    harness.click("50%");
    harness.settle();
    let built = harness.built();

    assert_eq!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .opacity,
        Some(50)
    );
    assert!(built.scene.meshes.is_empty());
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| (face.color.alpha - 0.5).abs() < 1e-6 && face.pick.is_some())
    );
}

#[test]
fn high_contrast_draws_a_dark_body_light_enough_for_its_edges_and_highlights() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face, .. } = top else {
        panic!("{top:?}");
    };
    let shown = crate::bodies::shown(harness.model.evaluation(), plate).unwrap();
    let reference = caditor_kernel::FaceReference::capture(
        &shown.solid,
        crate::bodies::find_face(shown, face).unwrap(),
    )
    .unwrap();
    let charcoal = caditor_document::Rgb::new(60, 62, 68);
    let black = caditor_document::Rgb::new(0, 0, 0);
    harness.model.perform(Action::Apply(Transaction::single(
        "Paint the plate dark",
        Edit::SetBodyAppearance {
            id: plate,
            appearance: caditor_document::BodyAppearance {
                colour: Some(charcoal),
                faces: vec![caditor_document::FaceColour {
                    face: reference,
                    colour: black,
                    opacity: None,
                    finish: None,
                }],
                ..Default::default()
            },
        },
    )));
    harness.settle();
    let colours = |harness: &mut Harness| {
        let built = harness.built();
        let top_pick = built.picks.id_of(top);
        let faces: Vec<_> = built
            .scene
            .meshes
            .iter()
            .flat_map(|instance| instance.faces.iter())
            .collect();
        let of_top = faces
            .iter()
            .find(|face| face.pick == top_pick)
            .map(|face| face.color);
        let other = faces
            .iter()
            .find(|face| face.pick.is_some() && face.pick != top_pick)
            .map(|face| face.color);
        (of_top, other)
    };
    let as_painted =
        |rgb: caditor_document::Rgb| caditor_render::Color::from_rgb8(rgb.red, rgb.green, rgb.blue);
    let standard = colours(&mut harness);

    set_high_contrast(&mut harness, true);
    let high = colours(&mut harness);
    let palette = scene_palette::Contrast::High.palette(harness.workspace.viewport.canvas());

    assert_eq!(
        standard,
        (Some(as_painted(black)), Some(as_painted(charcoal)))
    );
    assert_eq!(
        high,
        (
            Some(palette.body(as_painted(black))),
            Some(palette.body(as_painted(charcoal)))
        )
    );
    assert!(
        high.0
            .is_some_and(|colour| palette.marks_stand_out_on(colour))
    );
    assert!(
        high.1
            .is_some_and(|colour| palette.marks_stand_out_on(colour))
    );
}

#[test]
fn a_face_of_its_own_opacity_is_drawn_see_through_and_the_rest_of_its_body_solid() {
    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let Pickable::Face { face, .. } = top else {
        panic!("{top:?}");
    };
    let shown = crate::bodies::shown(harness.model.evaluation(), plate).unwrap();
    let reference = caditor_kernel::FaceReference::capture(
        &shown.solid,
        crate::bodies::find_face(shown, face).unwrap(),
    )
    .unwrap();
    harness.model.perform(Action::Apply(Transaction::single(
        "See through the top",
        Edit::SetBodyAppearance {
            id: plate,
            appearance: caditor_document::BodyAppearance {
                faces: vec![caditor_document::FaceColour {
                    face: reference,
                    colour: caditor_document::Rgb::new(200, 64, 52),
                    opacity: Some(50),
                    finish: None,
                }],
                ..Default::default()
            },
        },
    )));
    harness.settle();

    let built = harness.built_with_meshes(2);
    let alphas = |faces: &[caditor_render::FaceStyle]| {
        let mut alphas: Vec<f32> = faces.iter().map(|face| face.color.alpha).collect();
        alphas.sort_by(f32::total_cmp);
        alphas
    };

    assert_eq!(built.scene.meshes.len(), 1);
    assert_eq!(built.scene.translucent_meshes.len(), 1);
    assert_eq!(
        alphas(&built.scene.meshes[0].faces),
        [0.0, 1.0, 1.0, 1.0, 1.0, 1.0]
    );
    assert_eq!(
        alphas(&built.scene.translucent_meshes[0].faces),
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.5]
    );
    assert!(
        built.scene.translucent_meshes[0]
            .faces
            .iter()
            .all(|face| face.pick.is_some())
    );
}

#[test]
fn selected_faces_take_a_colour_of_their_own_from_the_body_card() {
    use crate::body_appearance::{CLEAR_FACE_COLOURS, face_swatch_name};

    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let red = caditor_render::Color::from_rgb8(200, 64, 52);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    assert!(harness.shows("Colour the selected face"));
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click_button(&face_swatch_name("Red"));
    harness.settle();

    assert_eq!(
        harness.model.undo_label(),
        Some("Change the face colours of Extrude 1")
    );
    let appearance = &harness.document().feature(plate).unwrap().appearance;
    assert_eq!(appearance.faces.len(), 1);
    assert_eq!(appearance.colour, None);
    harness.select([]);
    harness.frame();
    assert_eq!(painted_faces(&mut harness, red), 1);
    assert!(!harness.shows("Colour the selected face"));
    harness.click(CLEAR_FACE_COLOURS);
    harness.settle();
    assert!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .faces
            .is_empty()
    );
    assert_eq!(painted_faces(&mut harness, red), 0);
}

fn glass_top(harness: &mut Harness) -> (FeatureId, Pickable) {
    let (plate, top) = extruded_plate(harness);
    let Pickable::Face { face, .. } = top else {
        panic!("{top:?}");
    };
    let shown = crate::bodies::shown(harness.model.evaluation(), plate).unwrap();
    let reference = caditor_kernel::FaceReference::capture(
        &shown.solid,
        crate::bodies::find_face(shown, face).unwrap(),
    )
    .unwrap();
    harness.model.perform(Action::Apply(Transaction::single(
        "Glass top",
        Edit::SetBodyAppearance {
            id: plate,
            appearance: caditor_document::BodyAppearance {
                faces: vec![caditor_document::FaceColour {
                    face: reference,
                    colour: caditor_document::Rgb::new(200, 64, 52),
                    opacity: Some(50),
                    finish: Some(caditor_document::Finish::Glossy),
                }],
                ..Default::default()
            },
        },
    )));
    harness.settle();
    (plate, top)
}

#[test]
fn clearing_face_colours_keeps_the_opacity_and_finish_a_face_came_with() {
    use crate::body_appearance::{CLEAR_FACE_COLOURS, DEFAULT_COLOUR};

    let mut harness = Harness::new();
    let (plate, top) = glass_top(&mut harness);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    harness.select([]);
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click(CLEAR_FACE_COLOURS);
    harness.settle();

    let faces = harness
        .document()
        .feature(plate)
        .unwrap()
        .appearance
        .faces
        .clone();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].colour, DEFAULT_COLOUR);
    assert_eq!(faces[0].opacity, Some(50));
    assert_eq!(faces[0].finish, Some(caditor_document::Finish::Glossy));
    assert!(!harness.shows(CLEAR_FACE_COLOURS));
}

#[test]
fn the_body_colour_swatch_of_selected_faces_keeps_the_opacity_and_finish_they_came_with() {
    use crate::body_appearance::{BODY_COLOUR_NAME, DEFAULT_COLOUR, face_swatch_name};

    let mut harness = Harness::new();
    let (plate, top) = glass_top(&mut harness);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click_button(&face_swatch_name(BODY_COLOUR_NAME));
    harness.settle();

    let faces = harness
        .document()
        .feature(plate)
        .unwrap()
        .appearance
        .faces
        .clone();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].colour, DEFAULT_COLOUR);
    assert_eq!(faces[0].opacity, Some(50));
    assert_eq!(faces[0].finish, Some(caditor_document::Finish::Glossy));
}

#[test]
fn a_body_takes_a_colour_and_a_material_whose_density_gives_its_mass() {
    use crate::body_appearance::{DENSITY_CAPTION, NO_MATERIAL};

    let mut harness = Harness::new();
    let (plate, top) = extruded_plate(&mut harness);
    let steel_blue = caditor_render::Color::from_rgb8(70, 130, 180);

    harness.select([top]);
    run_from_palette(&mut harness, "body colour and material");
    harness.settle();
    let opened = harness
        .workspace
        .panels
        .painting
        .map(|painting| painting.body);
    let listed = harness.shows(DENSITY_CAPTION) && harness.shows(NO_MATERIAL);
    harness.select([]);
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(&mut harness, egui::vec2(0.0, 400.0));
    harness.click_button("Steel blue");
    harness.settle();
    let colour_label = harness.model.undo_label().map(str::to_owned);
    let painted = painted_faces(&mut harness, steel_blue);

    assert_eq!(opened, Some(plate));
    assert!(listed);
    assert_eq!(
        colour_label.as_deref(),
        Some("Change the colour of Extrude 1")
    );
    assert_eq!(painted, 6);

    harness.type_into_field(Id::unique(("body-density", plate)), "-1");
    harness.settle();
    let refused_label = harness.model.undo_label().map(str::to_owned);
    let refusal_shown = harness.shows("The density must be above zero, and -1 is not");
    harness.type_into_field(Id::unique(("body-density", plate)), "7.85");
    harness.type_into_field(Id::unique(("body-material-name", plate)), "Steel");
    harness.settle();
    harness.click(crate::toolbar::MEASURE_LABEL);
    harness.wait_until("the mass is shown", |harness| harness.shows("125.60 g"));

    assert_eq!(refused_label, colour_label);
    assert!(refusal_shown);
    assert_eq!(
        harness.model.undo_label(),
        Some("Change the material of Extrude 1")
    );
    assert!(harness.shows("Steel"));
    let appearance = &harness.document().feature(plate).unwrap().appearance;
    assert_eq!(appearance.material.as_deref(), Some("Steel"));

    harness.type_into_field(Id::unique(("body-colour", plate)), "#f80");
    harness.settle();
    let typed = harness.document().feature(plate).unwrap().appearance.colour;
    assert_eq!(typed, Some(caditor_document::Rgb::new(255, 136, 0)));

    harness.click_button("Close the colour and material of Extrude 1");
    let closed = harness.workspace.panels.painting.is_none();
    for _ in 0..4 {
        harness.perform(Action::Undo);
    }
    harness.settle();
    let unpainted = painted_faces(&mut harness, steel_blue);

    assert!(closed);
    assert!(
        harness
            .document()
            .feature(plate)
            .unwrap()
            .appearance
            .is_default()
    );
    assert_eq!(unpainted, 0);
}
