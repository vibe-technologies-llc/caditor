use caditor_document::Document;
use caditor_geometry::Point2;
use egui::{Event, Key, KeyboardShortcut, Modifiers, Pos2, Rect, accesskit::Role};
use tempfile::TempDir;

use super::{
    CAMERA_SETTLE, Harness, Wheel, add_block, add_peg, combine_nearly_touching_blocks, drag_screen,
    extruded_plate, write_drawing,
};
use crate::{
    analysis::Kind,
    app::Workspace,
    commands::Command,
    export::ExportCommand,
    files::FileCommand,
    interference_panel,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    samples::Sample,
    selection::Pickable,
    shortcut_editor, sketch_toolbar, window_frame,
};

const LARGEST: f32 = 2.0;
const SLACK: f32 = 0.5;
const LAYERS: usize = 4;

fn at_scale(scale: f32, dir: &TempDir) -> Harness {
    let mut workspace = Workspace::new();
    workspace.preferences.appearance.scale = scale;
    let mut harness = Harness::starting(Some(dir.path()), Document::default(), workspace);
    harness.settle();
    harness
}

fn with_sample(scale: f32, dir: &TempDir, sample: Sample) -> Harness {
    let mut harness = at_scale(scale, dir);
    let session = harness.model.session();
    harness.command(FileCommand::OpenSample(sample));
    harness.wait_until("the sample opens", |harness| {
        harness.model.session() != session
    });
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.let_animations_finish();
    harness
}

fn shown_within(harness: &Harness, area: Rect) -> Vec<(&str, Rect, Rect)> {
    harness
        .texts
        .iter()
        .zip(&harness.text_clips)
        .filter(|((_, rect), clip)| {
            area.expand(SLACK).contains_rect(**clip) && clip.intersect(*rect).is_positive()
        })
        .map(|((text, rect), clip)| (text.as_str(), *rect, *clip))
        .collect()
}

fn overlaps(shown: &[(&str, Rect, Rect)]) -> Vec<String> {
    shown
        .iter()
        .enumerate()
        .flat_map(|(index, (first, a, a_clip))| {
            shown[index + 1..]
                .iter()
                .filter(move |(_, b, b_clip)| {
                    a.intersect(*a_clip)
                        .shrink(SLACK)
                        .intersects(b.intersect(*b_clip).shrink(SLACK))
                })
                .map(move |(second, b, _)| format!("{first:?} {a:?} overlaps {second:?} {b:?}"))
        })
        .collect()
}

fn panel_problems(harness: &Harness, id: &str) -> Vec<String> {
    let panel = harness.panel_rect(id);
    let screen = harness.context.content_rect();
    let view = harness
        .workspace
        .viewport
        .rect()
        .expect("the 3D view is shown");
    let shown = shown_within(harness, panel);
    let outside = (!screen.expand(SLACK).contains_rect(panel))
        .then(|| format!("the {id} panel {panel:?} runs off the window {screen:?}"));
    let covered = (view.intersect(panel).width() > SLACK)
        .then(|| format!("the {id} panel {panel:?} lies over the 3D view {view:?}"));
    let cut = shown
        .iter()
        .filter(|(_, rect, clip)| {
            rect.min.x < clip.min.x - SLACK || rect.max.x > clip.max.x + SLACK
        })
        .map(|(text, rect, clip)| format!("{text:?} {rect:?} is cut off at the side by {clip:?}"));
    outside
        .into_iter()
        .chain(covered)
        .chain(cut)
        .chain(overlaps(&shown))
        .collect()
}

fn assert_fits_beside_the_view(harness: &Harness, ids: &[&str]) {
    let problems: Vec<String> = ids
        .iter()
        .flat_map(|id| panel_problems(harness, id))
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

fn text_rect(harness: &Harness, label: &str) -> Rect {
    harness
        .texts
        .iter()
        .find(|(shown, _)| shown == label)
        .unwrap_or_else(|| panic!("'{label}' is not on screen"))
        .1
}

fn assert_clear_of_the_window_controls(harness: &Harness, title: &str, footer: &[&str]) {
    let screen = harness.context.content_rect();
    let controls = window_frame::controls_bottom(&harness.context);

    assert!(controls > 0.0);
    assert!(
        text_rect(harness, title).min.y >= controls,
        "{title} under the controls"
    );
    for label in footer {
        assert!(harness.fully_shown(label), "{label} is not fully shown");
        assert!(text_rect(harness, label).max.y <= screen.max.y);
    }
}

fn overlapping_blocks(harness: &mut Harness) {
    extruded_plate(harness);
    add_peg(harness);
    add_block(
        harness,
        "Block",
        [Point2::new(-20.0, 0.0), Point2::new(0.0, 10.0)],
        "5 mm",
    );
    harness.select([]);
}

#[test]
fn a_right_hand_panel_reserves_the_width_it_paints_and_its_findings_wrap_at_the_largest_size() {
    let dir = TempDir::new().unwrap();
    let mut harness = at_scale(LARGEST, &dir);

    overlapping_blocks(&mut harness);
    harness.workspace.interference.toggle();
    harness.wait_until("every pair is checked", |harness| {
        harness.shows("1 pair overlaps and 1 pair touches.")
    });
    harness.let_animations_finish();
    let panel = harness.panel_rect("interference");
    harness.wheel_until_shown(panel.center(), interference_panel::SHOW_PLACE, Wheel::Down);

    assert_fits_beside_the_view(&harness, &["model", "interference"]);
}

#[test]
fn every_right_hand_panel_fits_beside_the_view_at_the_largest_size() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_sample(LARGEST, &dir, Sample::Bracket);
    let body = harness
        .document()
        .features()
        .find(|feature| feature.kind.solid().is_some())
        .map(|feature| feature.id())
        .unwrap();
    let corners: Vec<_> = harness
        .workspace
        .viewport
        .bodies()
        .get(body)
        .map(|mesh| mesh.vertices.iter().map(|vertex| vertex.key).collect())
        .unwrap_or_default();

    harness.workspace.measure.toggle();
    harness.select(
        [corners.first(), corners.last()]
            .into_iter()
            .flatten()
            .map(|vertex| Pickable::Vertex {
                body,
                vertex: *vertex,
            }),
    );
    harness.wait_until("the corners are measured", |harness| {
        harness
            .workspace
            .measure
            .measurements
            .readout()
            .is_some_and(|readout| readout.line.is_some())
    });
    harness.let_animations_finish();
    assert_fits_beside_the_view(&harness, &["model", "measure"]);
    harness.workspace.measure.toggle();
    harness.select([]);

    for kind in [Kind::Draft, Kind::Curvature] {
        harness.workspace.analysis.toggle(kind);
        harness.let_animations_finish();
        assert_fits_beside_the_view(&harness, &["model", "analysis"]);
        harness.workspace.analysis.toggle(kind);
        harness.frame();
    }

    harness.workspace.comb.toggle();
    harness.let_animations_finish();
    assert_fits_beside_the_view(&harness, &["model", "curvature-comb"]);
    harness.workspace.comb.toggle();

    harness.workspace.isocurves.toggle();
    harness.let_animations_finish();
    assert_fits_beside_the_view(&harness, &["model", "isocurves"]);
    harness.workspace.isocurves.toggle();

    harness.workspace.section.toggle(&harness.model, None);
    harness.let_animations_finish();
    assert_fits_beside_the_view(&harness, &["model", "section"]);
    harness.workspace.section.toggle(&harness.model, None);

    harness.key(Key::F1, Modifiers::NONE);
    harness.let_animations_finish();
    assert_fits_beside_the_view(&harness, &["model", "guide"]);
}

#[test]
fn going_to_a_failed_feature_at_the_largest_size_brings_its_remedy_into_view() {
    let dir = TempDir::new().unwrap();
    let mut harness = at_scale(LARGEST, &dir);

    combine_nearly_touching_blocks(&mut harness);
    harness.key(Key::F8, Modifiers::NONE);
    harness.frame();
    harness.let_animations_finish();
    let tree = harness.panel_rect("model");

    assert!(harness.fully_shown("Show where"));
    assert!(tree.contains_rect(text_rect(&harness, "Show where")));
    assert!(harness.fully_shown("Combine 1"));
}

#[test]
fn a_dialog_taller_than_the_window_keeps_its_title_and_footer_on_screen() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_sample(LARGEST, &dir, Sample::Plate);

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Bind(
            Command::Undo,
            KeyboardShortcut::new(Modifiers::NONE, Key::F),
        ),
    )));
    harness.perform(Action::Preferences(PreferencesCommand::ShowShortcuts));
    harness.click(shortcut_editor::CHANGED_ONLY);
    harness.click_button("Reset Fit view to the shortcut caditor starts with");
    harness.let_animations_finish();

    assert!(harness.shows_containing("F is used by Undo"));
    assert_clear_of_the_window_controls(
        &harness,
        "Keyboard shortcuts",
        &["Reset all shortcuts", "Close"],
    );
}

#[test]
fn the_export_dialog_keeps_its_buttons_on_screen_with_every_choice_at_the_largest_size() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_sample(LARGEST, &dir, Sample::Bracket);

    for name in ["M4", "M6"] {
        let (added, _) = harness.document().adding_configuration(Some(name));
        harness.perform(Action::Apply(added));
    }
    harness.settle();
    harness.command(FileCommand::Export(ExportCommand::Show));
    harness.command(FileCommand::Export(ExportCommand::SetEveryConfiguration(
        true,
    )));
    harness.let_animations_finish();

    assert!(harness.shows("Format"));
    assert_clear_of_the_window_controls(&harness, "Export", &["Export…", "Cancel"]);
}

#[test]
fn a_dialog_body_taller_than_the_window_scrolls_under_a_footer_that_stays_on_screen() {
    let dir = TempDir::new().unwrap();
    let mut harness = at_scale(LARGEST, &dir);
    let plan = dir.path().join("plan.dxf");
    let entities: String = (0..LAYERS)
        .map(|layer| {
            format!("0\nLINE\n8\nLayer {layer}\n10\n0\n20\n{layer}\n11\n40\n21\n{layer}\n")
        })
        .collect();
    write_drawing(&plan, Some(4), &entities);
    let title = "Import “plan.dxf”";
    let summary = "4 curves drawn, 40.000 mm wide and 3.000 mm high.";

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("plan.dxf");
    harness.let_animations_finish();

    assert!(!harness.fully_shown(summary));
    assert_clear_of_the_window_controls(&harness, title, &["Import", "Cancel"]);

    let units = text_rect(&harness, "Units").center();
    harness.wheel_until_shown(units, summary, Wheel::Down);

    assert!(harness.fully_shown(summary));
    assert_clear_of_the_window_controls(&harness, title, &["Import", "Cancel"]);
}

#[test]
fn the_ribbon_drops_its_labels_in_a_short_window_and_keeps_them_otherwise() {
    let dir = TempDir::new().unwrap();
    let mut large = with_sample(LARGEST, &dir, Sample::Bracket);
    let mut usual = with_sample(1.0, &dir, Sample::Bracket);

    usual.draw_on_new_sketch();
    large.draw_on_new_sketch();
    large.let_animations_finish();
    let view = large
        .workspace
        .viewport
        .rect()
        .expect("the 3D view is shown");
    let window = large.context.content_rect();

    assert!(usual.shows("Extrude"));
    assert!(!large.shows("Extrude"));
    large.button_rect("Extrude");
    assert!(
        view.height() > 0.3 * window.height(),
        "{view:?} in {window:?}"
    );
}

#[test]
fn the_sketch_bar_folds_into_two_rows_of_named_icons_at_the_largest_size() {
    let dir = TempDir::new().unwrap();
    let mut large = with_sample(LARGEST, &dir, Sample::Bracket);

    large.draw_on_new_sketch();
    large.let_animations_finish();
    let bar = super::sketch_bar(&large);
    let window = large.context.content_rect();
    let line = large.button_rect("Line");
    let parallel = large.button_rect("Parallel");

    assert!(
        bar.height() < 0.25 * window.height(),
        "{bar:?} in {window:?}"
    );
    assert!(!large.shows("Line"));
    assert!(large.shows(sketch_toolbar::FINISH_LABEL));
    assert!(bar.contains_rect(line) && bar.contains_rect(parallel));
    assert!(parallel.top() > line.bottom());
}

#[test]
fn readouts_in_a_short_sketch_view_give_way_to_the_prompt() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_sample(LARGEST, &dir, Sample::Bracket);

    harness.draw_on_new_sketch();
    harness.use_tool(Key::R);
    let view = harness
        .workspace
        .viewport
        .rect()
        .expect("the 3D view is shown");
    let left_of_the_cube = view.lerp_inside(egui::vec2(0.3, 0.5));
    harness
        .events
        .push(egui::Event::PointerMoved(left_of_the_cube));
    harness.let_animations_finish();
    let view = harness
        .workspace
        .viewport
        .rect()
        .expect("the 3D view is shown");
    let canvas: Vec<(&str, Rect, Rect)> = shown_within(&harness, view)
        .into_iter()
        .filter(|(text, _, _)| !text.trim().is_empty())
        .collect();
    let problems = overlaps(&canvas);

    assert!(harness.tool().is_some());
    assert!(problems.is_empty(), "{problems:#?}");
}

const SCALES: [f32; 4] = [0.75, 1.0, 1.5, LARGEST];
const NAME_INSET: f32 = 6.0;
const ELLIPSIS: &str = "…";
const LONG_NAME: &str = "front_suspension_bracket_thickness_after_the_second_revision";

fn add_parameter_named(harness: &mut Harness, name: &str) {
    let document = harness.document();
    let mut transaction = document.transaction(format!("Add {name}"));
    transaction.add_parameter(
        name,
        caditor_expression::Expression::parse("5 mm", &|_| None).unwrap(),
    );
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();
}

fn name_field(harness: &Harness, name: &str) -> Rect {
    let bounds = harness
        .accessible
        .iter()
        .find(|(_, node)| node.role() == Role::TextInput && node.value() == Some(name))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("the field of {name} is not on screen"));
    Rect::from_min_max(
        egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
        egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
    )
}

#[test]
fn parameter_names_of_normal_length_show_whole_from_75_to_200_percent() {
    for scale in SCALES {
        let dir = TempDir::new().unwrap();
        let mut harness = with_sample(scale, &dir, Sample::Bracket);
        let wide = harness.panel_rect("model");
        drag_screen(
            &mut harness,
            Pos2::new(wide.max.x - 1.0, 300.0),
            Pos2::new(60.0, 300.0),
        );
        harness.let_animations_finish();
        let panel = harness.panel_rect("model");
        harness.wheel_until_shown(panel.center(), "hole", Wheel::Down);

        assert!(panel.width() < wide.width());
        harness.read_screen();
        for name in ["leg", "width", "thickness", "hole"] {
            let field = name_field(&harness, name);
            let text = text_rect(&harness, name);
            assert!(
                text.max.x <= field.max.x - NAME_INSET,
                "{name} {text:?} runs out of its field {field:?} at {scale}"
            );
        }
        assert!(!harness.shows(ELLIPSIS), "a name is shortened at {scale}");
    }
}

#[test]
fn a_parameter_name_longer_than_its_column_ends_in_an_ellipsis_and_shows_whole_on_hover() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_sample(LARGEST, &dir, Sample::Bracket);
    add_parameter_named(&mut harness, LONG_NAME);
    let panel = harness.panel_rect("model");
    harness.events.push(Event::PointerMoved(panel.center()));
    harness.frame();
    for _ in 0..super::WHEEL_TRIES {
        if harness.fully_shown(ELLIPSIS) && harness.shows(LONG_NAME) {
            break;
        }
        harness.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, Wheel::Down.delta()),
            phase: egui::TouchPhase::Move,
            modifiers: harness.held,
        });
        harness.let_animations_finish();
    }

    assert!(harness.fully_shown(ELLIPSIS));
    assert_eq!(harness.count_shown(LONG_NAME), 1);

    let field = text_rect(&harness, LONG_NAME);
    harness.events.push(Event::PointerMoved(egui::pos2(
        field.min.x + 4.0,
        field.center().y,
    )));
    for _ in 0..super::TOOLTIP_FRAMES {
        harness.frame();
    }

    assert_eq!(harness.count_shown(LONG_NAME), 2);
}
