use caditor_document::{FeatureId, Finish};
use caditor_render::SurfaceFinish;
use egui::{Event, Key, Modifiers, Pos2, accesskit::Role};

use super::{Harness, extruded_plate, open_combo, run_from_palette, scroll_in_view};
use crate::{
    body_appearance::{FINISH_CAPTION, NO_FINISH, surface},
    model::Action,
    preferences::PreferenceChange,
};

const FINISH_UNDO: &str = "Change the finish of Extrude 1";
const MOST_TABS: usize = 16;

fn open_card(harness: &mut Harness) -> (FeatureId, crate::selection::Pickable) {
    let (plate, top) = extruded_plate(harness);
    harness.select([top]);
    run_from_palette(harness, "body colour and material");
    harness.settle();
    harness.select([]);
    harness
        .events
        .push(Event::PointerMoved(Pos2::new(150.0, 400.0)));
    scroll_in_view(harness, egui::vec2(0.0, 400.0));
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    (plate, top)
}

fn finish_of(harness: &Harness, body: FeatureId) -> Option<Finish> {
    harness.document().feature(body).unwrap().appearance.finish
}

fn drawn_finishes(harness: &mut Harness) -> Vec<Option<SurfaceFinish>> {
    harness
        .built_with_meshes(1)
        .scene
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.faces)
        .map(|face| face.finish)
        .collect()
}

fn focused_label(harness: &Harness) -> Option<String> {
    let focused = harness.focused()?.accesskit_id();
    harness
        .accessible
        .iter()
        .find(|(id, _)| *id == focused)
        .and_then(|(_, node)| node.label().map(str::to_owned))
}

#[test]
fn a_finish_chosen_on_the_body_card_shades_every_face_but_a_selected_one_and_undoes_in_one_step() {
    let mut harness = Harness::new();
    let (plate, top) = open_card(&mut harness);

    assert!(harness.captioned(Role::ComboBox, FINISH_CAPTION));
    assert!(harness.shows(NO_FINISH));
    assert!(drawn_finishes(&mut harness).iter().all(Option::is_none));

    open_combo(&mut harness, FINISH_CAPTION);
    assert!(harness.accessible_named(Role::Button, Finish::PolishedMetal.name()));
    harness.click(Finish::Satin.name());
    harness.settle();

    assert_eq!(finish_of(&harness, plate), Some(Finish::Satin));
    assert_eq!(harness.model.undo_label(), Some(FINISH_UNDO));
    let drawn = drawn_finishes(&mut harness);
    assert_eq!(drawn.len(), 6);
    assert!(
        drawn
            .iter()
            .all(|finish| *finish == Some(surface(Finish::Satin))),
        "{drawn:?}"
    );
    harness.select([top]);
    harness.frame();
    let selected = drawn_finishes(&mut harness);
    assert_eq!(selected.iter().filter(|finish| finish.is_none()).count(), 1);
    harness.select([]);

    harness.perform(Action::Undo);
    harness.settle();

    assert_eq!(finish_of(&harness, plate), None);
    assert!(drawn_finishes(&mut harness).iter().all(Option::is_none));
}

#[test]
fn the_finish_list_is_reached_and_chosen_from_the_keyboard() {
    let mut harness = Harness::new();
    let (plate, _) = open_card(&mut harness);
    let combo = harness
        .accessible
        .iter()
        .find(|(_, node)| {
            node.role() == Role::ComboBox
                && node.labelled_by().iter().any(|label| {
                    harness
                        .accessible
                        .iter()
                        .any(|(id, caption)| id == label && caption.value() == Some(FINISH_CAPTION))
                })
        })
        .map(|(id, _)| *id)
        .expect("the finish list is captioned");

    harness.events.push(Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Focus,
            target_node: combo,
            target_tree: egui::accesskit::TreeId::ROOT,
            data: None,
        },
    ));
    harness.frame();
    harness.frame();
    harness.key(Key::Enter, Modifiers::NONE);
    harness.show_new_windows();
    harness.frame();
    let wanted = Finish::PolishedMetal.name();
    for _ in 0..MOST_TABS {
        if focused_label(&harness).as_deref() == Some(wanted) {
            break;
        }
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
    }
    assert_eq!(focused_label(&harness).as_deref(), Some(wanted));
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();

    assert_eq!(finish_of(&harness, plate), Some(Finish::PolishedMetal));
    assert_eq!(harness.model.undo_label(), Some(FINISH_UNDO));
}

#[test]
fn a_material_brings_its_finish_and_high_contrast_draws_the_body_without_one() {
    let mut harness = Harness::new();
    let (plate, _) = open_card(&mut harness);

    open_combo(&mut harness, crate::body_appearance::MATERIAL_CAPTION);
    harness.click("Steel");
    harness.settle();

    assert_eq!(finish_of(&harness, plate), Some(Finish::BrushedMetal));
    assert!(
        drawn_finishes(&mut harness)
            .iter()
            .all(|finish| *finish == Some(surface(Finish::BrushedMetal)))
    );

    harness.perform(Action::Preferences(
        crate::preferences::PreferencesCommand::Change(PreferenceChange::HighContrast(true)),
    ));
    harness.settle();

    assert!(drawn_finishes(&mut harness).iter().all(Option::is_none));
}
