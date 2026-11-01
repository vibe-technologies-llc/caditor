use caditor_document::FeatureId;
use caditor_geometry::Point2;
use egui::{
    Color32, Key, Shape,
    accesskit::Role,
    epaint::{CircleShape, ClippedShape},
};

use super::{CAMERA_SETTLE, Harness, extruded_plate, pickable_described};
use crate::{
    appearance,
    editing::EditingCommand,
    feature_links::{MARK_RADIUS, RING_WIDTH},
    model::Action,
    panels::Focus,
    selection::Pickable,
};

#[derive(Debug, Default, PartialEq, Eq)]
struct Marks {
    dots: usize,
    rings: usize,
}

fn circles(shape: &Shape, found: &mut Vec<CircleShape>) {
    match shape {
        Shape::Circle(circle) => found.push(*circle),
        Shape::Vec(shapes) => {
            for shape in shapes {
                circles(shape, found);
            }
        }
        _ => {}
    }
}

fn marks(harness: &mut Harness) -> Marks {
    let shapes: Vec<ClippedShape> = harness.pass();
    let tokens = appearance::tokens_of(&harness.context);
    let mut found = Vec::new();
    for clipped in &shapes {
        circles(&clipped.shape, &mut found);
    }
    let dots = found
        .iter()
        .filter(|circle| circle.radius == MARK_RADIUS && circle.fill == tokens.text_muted)
        .count();
    let rings = found
        .iter()
        .filter(|circle| {
            circle.radius == MARK_RADIUS - RING_WIDTH / 2.0
                && circle.fill == Color32::TRANSPARENT
                && circle.stroke.width == RING_WIDTH
        })
        .count();
    Marks { dots, rings }
}

fn row_description(harness: &mut Harness, name: &str) -> Option<String> {
    harness.read_screen();
    harness
        .accessible
        .iter()
        .find(|(_, node)| node.role() == Role::TreeItem && node.label() == Some(name))
        .and_then(|(_, node)| node.description().map(str::to_owned))
}

fn sketch_of_plate(harness: &Harness, extrude: FeatureId) -> FeatureId {
    harness
        .document()
        .features()
        .map(|feature| feature.id())
        .take_while(|id| *id != extrude)
        .last()
        .expect("the plate's sketch comes before it")
}

#[test]
fn a_chosen_row_marks_in_the_margin_the_features_it_uses_and_those_using_it_by_shape() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = sketch_of_plate(&harness, extrude);
    let sketch_name = harness.document().feature(sketch).unwrap().name.clone();
    let extrude_name = harness.document().feature(extrude).unwrap().name.clone();
    harness.workspace.panels.selected = None;
    harness.frame();
    let unchosen = marks(&mut harness);

    harness.workspace.panels.choose_only(extrude);
    harness.frame();
    let from_extrude = marks(&mut harness);
    let extrude_words = row_description(&mut harness, &extrude_name);
    let computed = harness.workspace.panels.links.computed();
    harness.frame();
    harness.frame();
    let recomputed = harness.workspace.panels.links.computed() - computed;

    harness.workspace.panels.choose_only(sketch);
    harness.frame();
    let from_sketch = marks(&mut harness);
    let sketch_words = row_description(&mut harness, &sketch_name);

    assert_eq!(unchosen, Marks::default());
    assert_eq!(from_extrude, Marks { dots: 1, rings: 0 });
    assert_eq!(from_sketch, Marks { dots: 0, rings: 1 });
    assert_eq!(
        extrude_words,
        Some(format!(
            "Uses {sketch_name}. No feature uses {extrude_name}."
        ))
    );
    assert_eq!(
        sketch_words,
        Some(format!(
            "{sketch_name} uses no other feature. Used by {extrude_name}."
        ))
    );
    assert_eq!(recomputed, 0, "the links are kept while the model stays");
}

#[test]
fn a_hovered_row_shows_its_own_links_over_the_chosen_one_and_the_keyboard_choice_shows_them_too() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = sketch_of_plate(&harness, extrude);
    let sketch_name = harness.document().feature(sketch).unwrap().name.clone();
    harness.workspace.panels.selected = None;
    harness.frame();

    harness
        .workspace
        .panels
        .request_focus(Focus::Feature(extrude));
    harness.frame();
    harness.frame();
    let chosen = harness.workspace.panels.chosen();
    let from_keyboard = marks(&mut harness);
    harness.hover(&sketch_name);
    let hovered = marks(&mut harness);

    assert_eq!(chosen, vec![extrude]);
    assert_eq!(from_keyboard, Marks { dots: 1, rings: 0 });
    assert_eq!(hovered, Marks { dots: 0, rings: 1 });
}

#[test]
fn a_sketch_chosen_in_the_tree_lights_the_face_it_lies_on_and_no_other() {
    let mut harness = Harness::new();
    let (_, top) = extruded_plate(&mut harness);
    harness.select([top]);
    harness.click("New sketch");
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.use_tool(Key::R);
    harness.click_at(Point2::new(10.0, 10.0));
    harness.click_at(Point2::new(20.0, 30.0));
    harness.settle();
    let drawn_on = harness.editing();
    harness.perform(Action::Editing(EditingCommand::Finish));
    harness.settle();
    let sketch = harness
        .document()
        .features()
        .last()
        .map(|feature| feature.id())
        .unwrap();
    assert_eq!(drawn_on, Some(sketch));
    harness.select([]);
    harness.workspace.panels.selected = None;
    harness.frame();
    harness.frame();
    let bottom = pickable_described(&mut harness, "Extrude 1 › Extrude 1 start face");
    let colour = |harness: &mut Harness, face: Pickable| {
        let built = harness.built();
        let pick = built.picks.id_of(face);
        built
            .scene
            .meshes
            .iter()
            .flat_map(|instance| instance.faces.iter())
            .find(|drawn| drawn.pick == pick)
            .map(|drawn| drawn.color)
            .expect("the face is drawn")
    };
    let plain_top = colour(&mut harness, top);
    let plain_bottom = colour(&mut harness, bottom);

    harness.workspace.panels.choose_only(sketch);
    harness.frame();
    harness.frame();
    let lit_top = colour(&mut harness, top);
    let lit_bottom = colour(&mut harness, bottom);
    let selected = harness.workspace.viewport.selection().is_empty();

    harness.workspace.panels.selected = None;
    harness.frame();
    harness.frame();
    let cleared = colour(&mut harness, top);

    assert_ne!(lit_top, plain_top);
    assert_eq!(lit_bottom, plain_bottom);
    assert!(selected, "lighting a reference does not select it");
    assert_eq!(cleared, plain_top);
}

#[test]
fn a_sketch_on_a_principal_plane_lights_that_plane_while_chosen() {
    let mut harness = Harness::new();
    let (extrude, _) = extruded_plate(&mut harness);
    let sketch = sketch_of_plate(&harness, extrude);
    harness.workspace.panels.selected = None;
    harness.frame();
    harness.frame();
    let plain = harness.workspace.viewport.lit_in_scene();

    harness.workspace.panels.choose_only(sketch);
    harness.frame();
    harness.frame();
    let lit = harness.workspace.viewport.lit_in_scene();

    assert!(!plain.contains(&Pickable::Plane(caditor_document::PrincipalPlane::Xy)));
    assert!(lit.contains(&Pickable::Plane(caditor_document::PrincipalPlane::Xy)));
}
