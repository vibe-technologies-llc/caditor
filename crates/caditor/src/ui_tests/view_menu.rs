use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_sketch::Sketch;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, accesskit::Role, vec2};

use super::{
    CAMERA_SETTLE, Harness, SCREEN, edit_free_sketch, entities_of_kind, extruded_plate, open_menus,
    run_from_palette, sketch_entity, viewport_centre,
};
use crate::{
    model::Action,
    preferences::{InputMode, PreferenceChange, PreferencesCommand},
    selection::Pickable,
    view_menu::{self, Place},
    visibility,
};

const BOTTOM: &str = "Extrude 1 › Extrude 1 start face";

struct Plate {
    extrude: FeatureId,
    top: Pickable,
    bottom: Pickable,
    spot: Pos2,
}

fn plate(harness: &mut Harness) -> Plate {
    harness.context.enable_accesskit();
    let (extrude, top) = extruded_plate(harness);
    harness.select([]);
    run_from_palette(harness, "fit view");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    harness.frame();
    let bottom = harness
        .built()
        .picks
        .pickables()
        .find(|pickable| {
            pickable.describe(harness.document(), harness.model.evaluation()) == BOTTOM
        })
        .expect("the bottom face is pickable");
    let face = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    let spot = harness
        .workspace
        .viewport
        .screen_position(face, Point2::new(20.0, 20.0))
        .expect("the top face is in view");
    Plate {
        extrude,
        top,
        bottom,
        spot,
    }
}

fn hover_top(harness: &mut Harness, plate: &Plate) {
    let face = Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::Z, Vector3::X).unwrap();
    harness.hover_pickable(face, Point2::new(20.0, 20.0), plate.top);
}

fn right_click(harness: &mut Harness, position: Pos2) {
    for pressed in [true, false] {
        harness.events.push(Event::PointerButton {
            pos: position,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.frame();
    }
    harness.frame();
    harness.frame();
}

fn empty_spot(harness: &Harness) -> Pos2 {
    let rect = harness.workspace.viewport.rect().unwrap();
    rect.left_top() + vec2(rect.width() * 0.2, rect.height() * 0.5)
}

fn point_at(harness: &mut Harness, position: Pos2) {
    harness.events.push(Event::PointerMoved(position));
    harness.frame();
    harness.frame();
}

fn selected(harness: &Harness) -> Vec<Pickable> {
    let mut items: Vec<Pickable> = harness.workspace.viewport.selection().iter().collect();
    items.sort();
    items
}

fn menu(harness: &Harness) -> Option<Place> {
    harness.workspace.viewport.context_menu()
}

fn press(harness: &mut Harness, key: Key, modifiers: Modifiers) {
    harness.key(key, modifiers);
    harness.frame();
    harness.frame();
}

fn entry(harness: &Harness, name: &str) -> Option<Pos2> {
    let menus = open_menus(harness);
    harness
        .accessible
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
        .filter_map(|(_, node)| node.bounds())
        .map(|bounds| {
            Rect::from_min_max(
                Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
            )
        })
        .find(|rect| menus.iter().any(|menu| menu.contains_rect(*rect)))
        .map(|rect| rect.center())
}

fn has_entry(harness: &Harness, name: &str) -> bool {
    entry(harness, name).is_some()
}

fn choose(harness: &mut Harness, name: &str) {
    let position = entry(harness, name).unwrap_or_else(|| panic!("no menu entry named '{name}'"));
    harness.click_screen(position);
    harness.show_new_windows();
}

#[test]
fn a_right_click_on_an_unselected_face_selects_it_alone_and_its_menu_hides_the_body() {
    let mut harness = Harness::new();
    let plate = plate(&mut harness);
    harness.select([plate.bottom]);

    hover_top(&mut harness, &plate);
    right_click(&mut harness, plate.spot);
    let opened = menu(&harness);
    let picked = selected(&harness);
    let edit = has_entry(&harness, "Edit Extrude 1");
    let unreadable = harness.unreadable_nodes();
    choose(&mut harness, view_menu::HIDE);

    assert_eq!(
        opened,
        Some(Place::Model {
            on_item: true,
            feature_open: false,
            selected: true,
        })
    );
    assert_eq!(picked, vec![plate.top]);
    assert!(edit);
    assert!(unreadable.is_empty(), "{unreadable:?}");
    assert!(!visibility::is_shown(harness.document(), plate.extrude));
    assert_eq!(menu(&harness), None);
}

#[test]
fn a_right_click_keeps_a_selection_it_lands_on_or_beside_and_escape_closes_the_menu_only() {
    let mut harness = Harness::new();
    let plate = plate(&mut harness);
    let mut both = vec![plate.top, plate.bottom];
    both.sort();
    harness.select([plate.top, plate.bottom]);

    hover_top(&mut harness, &plate);
    right_click(&mut harness, plate.spot);
    let on_selected = (menu(&harness), selected(&harness));
    press(&mut harness, Key::Escape, Modifiers::NONE);
    let after_escape = (menu(&harness), selected(&harness));

    let empty = empty_spot(&harness);
    point_at(&mut harness, empty);
    right_click(&mut harness, empty);
    let in_space = menu(&harness);
    let space_entries = [
        view_menu::FIT_SELECTION,
        view_menu::STANDARD_VIEWS,
        view_menu::SELECTION_FILTER,
        "Paste features",
    ]
    .map(|name| has_entry(&harness, name));
    harness.press(empty - vec2(40.0, 0.0));
    harness.frame();

    assert!(on_selected.0.is_some());
    assert_eq!(on_selected.1, both);
    assert_eq!(after_escape, (None, both.clone()));
    assert_eq!(
        in_space,
        Some(Place::Model {
            on_item: false,
            feature_open: false,
            selected: true,
        })
    );
    assert_eq!(space_entries, [true; 4]);
    assert_eq!(menu(&harness), None);
    assert_eq!(selected(&harness), both);
}

#[test]
fn a_right_drag_orbits_in_every_input_mode_without_opening_the_menu_and_a_click_opens_it() {
    let mut harness = Harness::new();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();

    for mode in InputMode::ALL {
        harness.perform(Action::Preferences(PreferencesCommand::Change(
            PreferenceChange::InputMode(mode),
        )));
        harness.frame();
        let centre = viewport_centre(&harness);
        let before = harness.workspace.viewport.destination();
        drag(&mut harness, centre, centre + vec2(80.0, 30.0));
        let after = harness.workspace.viewport.destination();
        let after_drag = menu(&harness);
        right_click(&mut harness, centre);
        let after_click = menu(&harness);
        press(&mut harness, Key::Escape, Modifiers::NONE);

        assert!(
            before.orientation.angle_between(after.orientation) > 1e-3,
            "{mode:?} did not orbit"
        );
        assert_eq!(after_drag, None, "{mode:?} opened the menu after a drag");
        assert!(after_click.is_some(), "{mode:?} did not open the menu");
    }
}

fn drag(harness: &mut Harness, from: Pos2, to: Pos2) {
    harness.events.push(Event::PointerMoved(from));
    harness.frame();
    harness.events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    for step in 1..=4 {
        let position = from + (to - from) * (step as f32 / 4.0);
        harness.events.push(Event::PointerMoved(position));
        harness.frame();
    }
    harness.events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.frame();
    harness.frame();
}

#[test]
fn a_change_to_the_model_closes_the_menu() {
    let mut harness = Harness::new();
    let plate = plate(&mut harness);

    hover_top(&mut harness, &plate);
    right_click(&mut harness, plate.spot);
    let opened = menu(&harness).is_some();
    harness.key(Key::H, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(opened);
    assert!(!visibility::is_shown(harness.document(), plate.extrude));
    assert_eq!(menu(&harness), None);
}

#[test]
fn the_menu_key_opens_it_at_the_selection_with_its_first_entry_focused_for_the_arrows() {
    let mut harness = Harness::new();
    let plate = plate(&mut harness);
    harness.select([plate.bottom]);
    let rect = harness.workspace.viewport.rect().unwrap();
    let centre_of_bottom = harness
        .workspace
        .viewport
        .screen_position(Plane::XY, Point2::new(20.0, 20.0))
        .unwrap();

    press(&mut harness, Key::F10, Modifiers::SHIFT);
    harness.frame();
    let opened = menu(&harness);
    let anchor = harness.workspace.viewport.context_menu_anchor();
    let first = harness.focused();
    press(&mut harness, Key::ArrowDown, Modifiers::NONE);
    let second = harness.focused();
    press(&mut harness, Key::Enter, Modifiers::NONE);
    harness.frame();

    assert_eq!(
        opened,
        Some(Place::Model {
            on_item: true,
            feature_open: false,
            selected: true,
        })
    );
    assert!(anchor.is_some_and(|anchor| anchor.distance(centre_of_bottom) < 1.0));
    assert!(centre_of_bottom.distance(rect.center()) > 10.0);
    assert!(first.is_some());
    assert!(second.is_some() && second != first);
    assert!(!visibility::is_shown(harness.document(), plate.extrude));
    assert_eq!(menu(&harness), None);
}

#[test]
fn with_nothing_selected_the_menu_key_opens_it_at_the_centre_of_the_view() {
    let mut harness = Harness::new();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let rect = harness.workspace.viewport.rect().unwrap();

    run_from_palette(&mut harness, "show the context menu");
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.workspace.viewport.context_menu_anchor(),
        Some(rect.center())
    );
}

#[test]
fn in_a_sketch_the_menu_constrains_the_curve_clicked_and_mid_shape_it_finishes_the_shape() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 4.0));
    let feature = edit_free_sketch(&mut harness, sketch);

    let spot = harness.hover_pickable(
        Plane::XY,
        Point2::new(15.0, 2.0),
        sketch_entity(feature, line),
    );
    right_click(&mut harness, spot);
    let opened = menu(&harness);
    let picked = selected(&harness);
    if !has_entry(&harness, "Horizontal") {
        choose(&mut harness, view_menu::CONSTRAIN);
    }
    choose(&mut harness, "Horizontal");
    harness.settle();
    let horizontal = harness
        .sketch(feature)
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == "Horizontal")
        .count();

    harness.use_tool(Key::L);
    harness.click_at(Point2::new(5.0, 12.0));
    let pending = harness.on_screen(Point2::new(20.0, 16.0));
    point_at(&mut harness, pending);
    right_click(&mut harness, pending);
    let mid_shape = menu(&harness);
    let take_back = has_entry(&harness, "Take back the last point");
    choose(&mut harness, "Finish the shape");

    assert_eq!(opened, Some(Place::Sketch { on_item: true }));
    assert_eq!(picked, vec![sketch_entity(feature, line)]);
    assert_eq!(horizontal, 1);
    assert_eq!(mid_shape, Some(Place::Shape));
    assert!(take_back);
    assert!(!harness.workspace.viewport.is_drawing());
    assert_eq!(entities_of_kind(harness.sketch(feature), "Line").len(), 1);
}

#[test]
fn the_menu_stays_on_screen_near_the_corner_at_the_largest_interface_size() {
    let mut harness = Harness::new();
    let plate = plate(&mut harness);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    harness.select([plate.top]);
    let visible = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / 2.0);
    let rect = harness.workspace.viewport.rect().unwrap();
    let corner = rect.right_bottom() - vec2(30.0, 30.0);

    point_at(&mut harness, corner);
    right_click(&mut harness, corner);
    let opened = menu(&harness).is_some();
    let menus = open_menus(&harness);

    assert!(opened);
    assert!(!menus.is_empty());
    for shown in menus {
        assert!(visible.expand(0.5).contains_rect(shown), "{shown:?}");
    }
}
