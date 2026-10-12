use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Sketch};
use egui::{
    Color32, Key, Modifiers, Shape,
    accesskit::{Node, Role},
};
use tempfile::TempDir;

use super::{
    Harness, Painted, SCREEN, assert_readable, combine_nearly_touching_blocks, edit_base_sketch,
    entities_of_kind, extruded_plate, feature_named, set_high_contrast,
};
use crate::{
    annotations, canvas,
    editing::EditingCommand,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, PreferencesTab},
    scene, scene_palette,
    units::LengthUnit,
};

fn frame_strokes(harness: &Harness, color: Color32) -> (usize, usize) {
    fn walk(shape: &Shape, color: Color32, counts: &mut (usize, usize)) {
        match shape {
            Shape::Rect(rect)
                if rect.stroke.color == color
                    && rect.stroke.width == annotations::FRAME_WIDTH
                    && rect.fill == Color32::TRANSPARENT =>
            {
                counts.0 += 1;
            }
            Shape::LineSegment { stroke, .. }
                if stroke.color == color && stroke.width == annotations::FRAME_WIDTH =>
            {
                counts.1 += 1;
            }
            Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, color, counts);
                }
            }
            _ => {}
        }
    }
    let mut counts = (0, 0);
    for clipped in harness.painted.iter().flat_map(|painted| &painted.shapes) {
        walk(&clipped.shape, color, &mut counts);
    }
    counts
}

#[test]
fn high_contrast_frames_redundant_constraints_dashed_and_conflicting_ones_solid() {
    let mut harness = Harness::new();
    harness.painted = Some(Painted {
        shapes: Vec::new(),
        pixels_per_point: 1.0,
    });
    let base = edit_base_sketch(&mut harness);
    let line = entities_of_kind(harness.sketch(base), "Line")[0];

    harness.add_stored_constraint(base, Constraint::Horizontal(line));
    harness.frame();
    assert!(harness.shows("1 redundant constraint"));
    assert_eq!(frame_strokes(&harness, canvas::DARK_CHROME.warning), (0, 0));

    set_high_contrast(&mut harness, true);
    let (solid, dashes) = frame_strokes(&harness, canvas::DARK_CHROME.warning);
    assert_eq!(solid, 0);
    assert!(dashes >= 8, "{dashes} dashes");
    assert_eq!(frame_strokes(&harness, canvas::DARK_CHROME.error), (0, 0));

    harness.add_stored_constraint(base, Constraint::Vertical(line));
    harness.frame();
    assert!(harness.shows("Conflicting constraints"));
    let (solid, dashes) = frame_strokes(&harness, canvas::DARK_CHROME.error);
    assert!(solid >= 2, "{solid} frames");
    assert_eq!(dashes, 0);

    set_high_contrast(&mut harness, false);
    assert_eq!(frame_strokes(&harness, canvas::DARK_CHROME.error), (0, 0));
}

#[test]
fn high_contrast_reaches_the_scene_and_draws_free_points_hollow() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let feature = harness.add_sketch(sketch);
    harness.edit(feature);
    let standard = harness.built();
    let hole = |built: &scene::BuiltScene| {
        built
            .scene
            .markers()
            .filter(|marker| {
                marker.pick.is_none() && marker.color == scene_palette::HIGH_CONTRAST.hole
            })
            .count()
    };
    let grid = |built: &scene::BuiltScene| built.scene.grid.as_ref().map(|grid| grid.color);

    assert_eq!(grid(&standard), Some(scene_palette::STANDARD.grid));
    assert_eq!(hole(&standard), 0);

    set_high_contrast(&mut harness, true);
    let high = harness.built();
    assert_eq!(grid(&high), Some(scene_palette::HIGH_CONTRAST.grid));
    assert_eq!(hole(&high), 2);

    set_high_contrast(&mut harness, false);
    assert_eq!(grid(&harness.built()), Some(scene_palette::STANDARD.grid));
}

#[test]
fn high_contrast_dashes_the_edges_of_a_failed_body() {
    let mut harness = Harness::new();
    combine_nearly_touching_blocks(&mut harness);
    let dashed = |harness: &mut Harness| {
        harness
            .built()
            .scene
            .lines()
            .filter(|line| {
                line.pick.is_some() && matches!(line.stroke, caditor_render::Stroke::Dashed { .. })
            })
            .count()
    };

    let standard = dashed(&mut harness);
    set_high_contrast(&mut harness, true);
    let high = dashed(&mut harness);

    assert_eq!(standard, 0);
    assert!(high > 0);
}

#[test]
fn the_3d_view_tells_screen_readers_what_it_shows() {
    let mut harness = Harness::new();
    let (_, _) = extruded_plate(&mut harness);
    harness.context.enable_accesskit();
    harness.frame();
    let shown = harness.view_description().unwrap_or_default();

    let sketch = harness
        .document()
        .features()
        .find(|feature| feature.name == "Base sketch")
        .map(caditor_document::Feature::id)
        .unwrap();
    harness.edit(sketch);
    harness.frame();
    let editing = harness.view_description().unwrap_or_default();

    assert!(
        shown.starts_with(
            "1 body shown: Extrude 1 (40 mm by 40 mm by 10 mm, lowest corner at 0, 0, 0 mm)"
        ),
        "{shown}"
    );
    assert!(shown.contains("sketch"), "{shown}");
    assert!(shown.contains("(facing +z, at z = 0 mm)"), "{shown}");
    assert!(editing.starts_with("Editing Base sketch: "), "{editing}");
    assert!(editing.contains("constraint"), "{editing}");
    assert!(editing.contains("; its points span x from "), "{editing}");
}

#[test]
fn the_3d_view_is_described_again_in_a_changed_unit() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.context.enable_accesskit();
    harness.frame();
    let millimetres = harness.view_description().unwrap_or_default();

    harness.model.set_length_unit(LengthUnit::Centimetre);
    harness.frame();
    harness.frame();
    let centimetres = harness.view_description().unwrap_or_default();

    assert!(
        millimetres.contains("(40 mm by 40 mm by 10 mm"),
        "{millimetres}"
    );
    assert!(
        centimetres.contains("(4 cm by 4 cm by 1 cm"),
        "{centimetres}"
    );
}

#[test]
fn the_3d_view_lists_each_body_with_its_faces_each_datum_where_it_lies_and_each_sketch_curve() {
    let mut harness = Harness::new();
    extruded_plate(&mut harness);
    harness.select([]);
    harness.key(Key::P, Modifiers::ALT | Modifiers::SHIFT);
    harness.settle();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
    let labelled = |harness: &Harness, start: &str| {
        harness
            .accessible
            .iter()
            .find(|(_, node)| spoken(node).is_some_and(|label| label.starts_with(start)))
            .map(|(id, node)| (*id, node.clone()))
    };
    let shown = harness.view_description().unwrap_or_default();

    let (_, body) = labelled(&harness, "Body Extrude 1").expect("the body has a node");
    let faces: Vec<String> = descendants(&harness, &body)
        .into_iter()
        .filter(|label| label.contains("Extrude 1"))
        .collect();
    let (_, datum) = labelled(&harness, "Datum ").expect("the datum has a node");

    assert_eq!(
        spoken(&body),
        Some("Body Extrude 1, 40 mm by 40 mm by 10 mm, lowest corner at 0, 0, 0 mm, 6 faces")
    );
    assert_eq!(faces.len(), 6, "{faces:?}");
    assert!(
        faces.iter().any(|face| face.contains("end face")),
        "{faces:?}"
    );
    assert!(
        spoken(&datum).is_some_and(|label| label.ends_with(", at 0, 0, 0 mm")),
        "{:?}",
        spoken(&datum)
    );
    assert!(shown.contains("(at 0, 0, 0 mm)"), "{shown}");
    let (_, view) = labelled(&harness, "3D view").expect("the view has a node");
    let listed = descendants(&harness, &view);
    assert!(
        listed
            .iter()
            .any(|label| label.starts_with("Body Extrude 1")),
        "{listed:?}"
    );
    assert!(
        listed.iter().any(|label| label.starts_with("Sketch ")),
        "{listed:?}"
    );

    let sketch = harness
        .document()
        .features()
        .find(|feature| feature.name == "Base sketch")
        .map(caditor_document::Feature::id)
        .unwrap();
    harness.edit(sketch);
    harness.frame();
    harness.frame();
    let (_, line) = labelled(&harness, "Line ").expect("each curve has a node");

    assert!(
        spoken(&line).is_some_and(|label| label.contains(" long") && label.contains(" from ")),
        "{:?}",
        spoken(&line)
    );
    assert!(labelled(&harness, "Body Extrude 1").is_none());
}

fn spoken(node: &Node) -> Option<&str> {
    node.label().or_else(|| node.value())
}

fn descendants(harness: &Harness, node: &Node) -> Vec<String> {
    node.children()
        .iter()
        .filter_map(|child| harness.accessible.iter().find(|(id, _)| id == child))
        .flat_map(|(_, child)| {
            spoken(child)
                .map(str::to_owned)
                .into_iter()
                .chain(descendants(harness, child))
        })
        .collect()
}

#[test]
fn the_interface_scales_from_the_keyboard_and_high_contrast_changes_the_colours() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    let normal_panel = harness.context.global_style().visuals.panel_fill;
    harness.key(Key::Plus, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    assert_eq!(harness.workspace.preferences.appearance.scale, 1.125);
    assert_eq!(harness.context.zoom_factor(), 1.125);
    harness.key(Key::Num0, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.frame();
    assert_eq!(harness.context.zoom_factor(), 1.0);
    for _ in 0..3 {
        harness.key(Key::Minus, Modifiers::COMMAND);
        harness.frame();
    }
    assert_eq!(harness.workspace.preferences.appearance.scale, 0.75);
    harness.key(Key::Minus, Modifiers::COMMAND);
    harness.frame();
    assert_eq!(
        harness.model.notice().map(|notice| notice.text.as_str()),
        Some("Make the interface smaller: The interface is at its smallest, 75%")
    );

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(2.0),
    )));
    harness.frame();
    harness.frame();
    let visible = SCREEN.size() / 2.0;
    for label in ["File", "Up to date", "Features"] {
        let rect = harness
            .texts
            .iter()
            .find(|(shown, _)| shown == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1;
        assert!(
            rect.max.x <= visible.x && rect.max.y <= visible.y,
            "{label} at {rect:?}"
        );
    }
    let axis = harness.button_rect("Axis");
    assert!(
        axis.max.x <= visible.x && axis.max.y <= visible.y,
        "Axis at {axis:?}"
    );

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(1.0),
    )));
    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.frame();
    harness.show_new_windows();
    harness.click("Appearance");
    harness.click("High contrast");
    assert!(harness.workspace.preferences.appearance.high_contrast);
    let panel = harness.context.global_style().visuals.panel_fill;
    assert_ne!(panel, normal_panel);
    assert!(panel == Color32::BLACK || panel == Color32::WHITE);
    harness.wait_until("the appearance is saved", |_| {
        caditor_file::Settings::load(&dir.path().join("config")).flag("appearance.high_contrast")
            == Some(true)
    });
}

#[test]
fn the_view_and_the_tool_prompt_are_named_for_screen_readers() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);
    harness.frame();
    harness.frame();

    assert!(
        harness
            .accessible
            .iter()
            .any(|(_, node)| node.label() == Some("3D view"))
    );
    let prompt = harness.accessible.iter().find(|(_, node)| {
        node.role() == Role::Label
            && node
                .value()
                .is_some_and(|value| value.starts_with("Click the start of the line"))
    });
    let (_, prompt) = prompt.expect("the prompt is on the accessibility tree");
    assert_eq!(prompt.live(), Some(egui::accesskit::Live::Polite));
    assert!(prompt.value().unwrap().contains("Esc: back to Select"));
}

#[test]
fn icon_buttons_are_named_and_captions_label_their_fields_for_screen_readers() {
    let mut harness = Harness::new();
    harness.read_screen();
    assert_readable(&harness, "The empty window");
    assert!(harness.accessible_named(Role::Button, "More actions for Side sketch"));
    assert!(harness.accessible_named(Role::Button, "Show details of Base sketch"));
    assert!(harness.accessible_named(Role::Button, "Edit Base sketch"));
    assert!(harness.accessible_named(Role::Button, "Add parameter"));
    assert!(harness.accessible_named(Role::Button, "Length unit: millimetres"));

    harness.click("File");
    harness.read_screen();
    assert_readable(&harness, "The File menu");
    assert!(harness.accessible_named(Role::Button, "Open sample"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    let (extrude, _) = extruded_plate(&mut harness);
    harness.perform(Action::Editing(EditingCommand::OpenSolid(extrude)));
    harness.settle();
    harness.read_screen();
    assert_readable(&harness, "An open extrusion");
    assert!(harness.captioned(Role::TextInput, "Distance"));
    assert!(harness.captioned(Role::ComboBox, "Extent"));
    assert!(harness.captioned(Role::ComboBox, "Result"));

    harness.select([]);
    harness.click("Plane");
    harness.settle();
    harness.read_screen();
    assert_readable(&harness, "An open datum plane");
    assert!(harness.captioned(Role::TextInput, "Offset"));
    assert!(harness.accessible_named(Role::Button, crate::feature_fields::CHOOSE_IN_VIEW));

    harness.key(Key::Comma, Modifiers::COMMAND);
    harness.show_new_windows();
    harness.read_screen();
    assert_readable(&harness, "Preferences");
    for tab in PreferencesTab::ALL {
        assert!(harness.accessible_named(Role::Tab, tab.label()), "{tab:?}");
    }
    harness.click("Appearance");
    harness.read_screen();
    assert_readable(&harness, "The Appearance preferences");
    assert!(harness.accessible_named(Role::Button, "Make the interface smaller"));
    assert!(harness.accessible_named(Role::Button, "Make the interface larger"));
    harness.click("Navigation");
    harness.read_screen();
    assert!(harness.captioned(Role::Slider, "Orbit speed"));
    harness.click("Graphics");
    harness.read_screen();
    assert_readable(&harness, "The Graphics preferences");
    assert!(harness.captioned(Role::CheckBox, "Vsync"));
    assert!(harness.accessible_named(Role::Button, crate::graphics::COPY_DETAILS));

    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.show_new_windows();
    harness.read_screen();
    assert_readable(&harness, "The shortcut editor");
    assert!(harness.accessible_named(Role::Button, "Record a new shortcut for Undo"));
    assert!(harness.accessible_named(Role::Button, "Record a new shortcut for Redo"));
    assert!(harness.accessible_named(Role::Button, "Remove Ctrl+Z from Undo"));
    assert!(harness.accessible_named(
        Role::Button,
        "Reset Undo to the shortcut caditor starts with"
    ));
    assert!(!harness.accessible_named(Role::Button, "Add…"));
    assert!(!harness.accessible_named(Role::Button, "Reset"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    harness.key(Key::P, Modifiers::COMMAND | Modifiers::SHIFT);
    harness.show_new_windows();
    harness.read_screen();
    assert_readable(&harness, "The command palette");
    assert!(harness.accessible_named(Role::Button, "Fit view"));
    harness.key(Key::Escape, Modifiers::NONE);
    harness.show_new_windows();

    let base = feature_named(&harness, "Base sketch");
    harness.edit(base);
    harness.read_screen();
    assert_readable(&harness, "An edited sketch");
    assert!(harness.accessible_named(Role::Button, "Horizontal"));
}
