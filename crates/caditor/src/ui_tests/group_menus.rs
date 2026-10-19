use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::Sketch;
use egui::{
    Key, Modifiers, Pos2, Rect,
    accesskit::{Role, Toggled},
};

use super::{Harness, SCREEN, Wheel, assert_readable, edit_free_sketch, lowest_in};
use crate::{
    analysis::AnalysisCommand,
    commands::Command,
    editing::Tool,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    section::SectionCommand,
    shape_modes::{OffsetMode, RectangleMode, ShapeMode},
    sketch_toolbar,
    sketch_tools::ConstraintTool,
};

const GROUPS: [&str; 7] = [
    "History",
    "Sketch",
    "Solid",
    "Modify",
    "Pattern",
    "Reference",
    "Analyse",
];

const ANALYSE_COMMANDS: [Command; 13] = [
    Command::Measure,
    Command::Interference,
    Command::Analysis(AnalysisCommand::Draft),
    Command::Analysis(AnalysisCommand::Radius),
    Command::Analysis(AnalysisCommand::Reach),
    Command::Analysis(AnalysisCommand::Curvature),
    Command::Analysis(AnalysisCommand::Zebra),
    Command::Analysis(AnalysisCommand::Chrome),
    Command::Analysis(AnalysisCommand::Comb),
    Command::Analysis(AnalysisCommand::Isocurves),
    Command::Section(SectionCommand::Toggle),
    Command::Section(SectionCommand::Add),
    Command::ToggleCentresOfMass,
];

fn menu_name(group: &str) -> String {
    format!("{group} commands")
}

fn open(harness: &mut Harness, group: &str) {
    harness.click_button(&menu_name(group));
    harness.frame();
    harness.frame();
}

fn close(harness: &mut Harness) {
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();
}

fn scaled(harness: &mut Harness, scale: f32) {
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Scale(scale),
    )));
    harness.frame();
    harness.frame();
    harness.frame();
}

fn marked(harness: &Harness, command: Command) -> Option<bool> {
    let title = command.title();
    harness
        .accessible
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(title.as_str()))
        .map(|(_, node)| node.toggled() == Some(Toggled::True))
}

#[test]
fn every_ribbon_group_has_a_title_button_named_for_screen_readers() {
    let mut harness = Harness::new();
    harness.frame();
    harness.read_screen();

    for group in GROUPS {
        assert!(
            harness.accessible_named(Role::Button, &menu_name(group)),
            "{group}"
        );
    }
}

#[test]
fn the_analyse_title_lists_every_analysis_command_and_nothing_else() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Analyse");

    for command in ANALYSE_COMMANDS {
        assert!(harness.shows(&command.title()), "{command:?}");
    }
    assert!(!harness.shows(&Command::Thread.title()));
    assert!(!harness.shows(&Command::OffsetFace.title()));
}

#[test]
fn the_other_titles_list_the_commands_that_have_no_button_of_their_own() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Solid");
    for command in [
        Command::Thread,
        Command::NewBox,
        Command::NewCone,
        Command::NewWedge,
        Command::NewPrism,
    ] {
        assert!(harness.shows(&command.title()), "{command:?}");
    }
    close(&mut harness);

    open(&mut harness, "Modify");
    for command in [
        Command::OffsetFace,
        Command::SplitFace,
        Command::CopyBody,
        Command::MirrorFaces,
        Command::Split,
        Command::Mate,
    ] {
        assert!(harness.shows(&command.title()), "{command:?}");
    }
}

#[test]
fn choosing_a_command_from_a_title_runs_it_as_its_button_does() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Analyse");
    harness.click(&Command::Interference.title());
    harness.frame();
    harness.frame();

    assert!(harness.workspace.interference.open);
    assert!(!harness.shows(&Command::ToggleCentresOfMass.title()));

    open(&mut harness, "Analyse");
    harness.click(&Command::ToggleCentresOfMass.title());
    harness.frame();
    harness.frame();

    assert!(harness.workspace.viewport.aids().centres_of_mass);
}

#[test]
fn a_command_chosen_from_a_title_that_is_not_ready_is_refused_in_words() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Modify");
    harness.click(&Command::Fillet.title());
    harness.frame();
    harness.frame();

    assert!(harness.workspace.editing.solid().is_none());
    assert!(harness.model.notice().is_some());
}

#[test]
fn the_title_marks_the_panels_that_are_open() {
    let mut harness = Harness::new();
    harness.frame();
    harness.context.enable_accesskit();
    open(&mut harness, "Analyse");

    assert_eq!(marked(&harness, Command::Interference), Some(false));
    assert_eq!(
        marked(&harness, Command::Analysis(AnalysisCommand::Zebra)),
        Some(false)
    );

    harness.click(&Command::Interference.title());
    harness.frame();
    harness.frame();
    open(&mut harness, "Analyse");

    assert_eq!(marked(&harness, Command::Interference), Some(true));
    assert_eq!(
        marked(&harness, Command::Analysis(AnalysisCommand::Zebra)),
        Some(false)
    );
}

#[test]
fn escape_closes_a_title_and_enter_runs_the_command_focused_when_it_opened() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Analyse");
    assert!(harness.shows(&Command::Interference.title()));
    close(&mut harness);

    assert!(!harness.shows(&Command::Interference.title()));
    assert!(!harness.workspace.measure.open);

    open(&mut harness, "Analyse");
    assert!(harness.focused().is_some());
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.frame();

    assert!(harness.workspace.measure.open);
    assert!(!harness.shows(&Command::Interference.title()));
}

#[test]
fn a_click_outside_a_title_menu_closes_it_without_running_anything() {
    let mut harness = Harness::new();
    harness.frame();

    open(&mut harness, "Analyse");
    harness.click_screen(Pos2::new(SCREEN.center().x, SCREEN.bottom() - 60.0));
    harness.frame();
    harness.frame();

    assert!(!harness.shows(&Command::Interference.title()));
    assert!(!harness.workspace.measure.open);
    assert!(!harness.workspace.interference.open);
}

#[test]
fn the_title_menus_open_and_stay_on_screen_from_75_to_200_percent() {
    let probes = [
        ("History", Command::Undo, Command::UndoHistory),
        ("Sketch", Command::NewSketch, Command::LookAtSketch),
        ("Solid", Command::Extrude, Command::NewPrism),
        ("Modify", Command::Fillet, Command::MateAngle),
        ("Pattern", Command::LinearPattern, Command::PointPattern),
        ("Reference", Command::DatumPlane, Command::InsertImage),
        ("Analyse", Command::Measure, Command::ToggleCentresOfMass),
    ];
    for scale in [0.75, 1.0, 1.5, 2.0] {
        let mut harness = Harness::new();
        scaled(&mut harness, scale);
        let room = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / scale);

        for (group, first, last) in probes {
            open(&mut harness, group);

            let first_title = first.title();
            assert!(
                harness.fully_shown(&first_title),
                "{group} first at {scale}"
            );

            let over = harness.position_of(&first_title);
            let title = last.title();
            harness.wheel_until_shown(over, &title, Wheel::Down);
            assert!(
                room.contains(lowest_in(&harness, room, &title)),
                "{group} at {scale}"
            );
            assert_readable(&harness, &format!("{group} menu at {scale}"));
            close(&mut harness);
        }
    }
}

#[test]
fn at_75_percent_the_captions_stay_and_choosing_from_a_title_still_runs_the_command() {
    let mut harness = Harness::new();
    scaled(&mut harness, 0.75);

    for group in GROUPS {
        assert!(harness.shows(group), "{group}");
    }

    open(&mut harness, "Analyse");
    harness.click(&Command::Analysis(AnalysisCommand::Zebra).title());
    harness.frame();
    harness.frame();

    assert!(harness.workspace.analysis.open);
}

const SKETCH_GROUPS: [&str; 6] = [
    "History",
    "Select",
    "Draw",
    "Modify",
    "Constrain",
    "Dimension",
];

fn sketch_menu_name(group: &str) -> String {
    sketch_toolbar::group_menu_name(group)
}

fn open_sketch_menu(harness: &mut Harness, group: &str) {
    harness.click_button(&sketch_menu_name(group));
    harness.frame();
    harness.frame();
}

fn choose(harness: &mut Harness, command: Command) {
    harness.click(&command.title());
    harness.frame();
    harness.frame();
}

fn sketch_with_a_lone_point(harness: &mut Harness) -> FeatureId {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_point(Point2::new(10.0, 10.0));
    let feature = edit_free_sketch(harness, sketch);
    harness.settle();
    feature
}

#[test]
fn every_sketch_ribbon_group_has_a_title_button_named_apart_from_the_main_ribbon() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.read_screen();

    for group in SKETCH_GROUPS {
        assert!(
            harness.accessible_named(Role::Button, &sketch_menu_name(group)),
            "{group}"
        );
    }
    for group in GROUPS {
        assert!(
            harness.accessible_named(Role::Button, &menu_name(group)),
            "{group}"
        );
    }
}

#[test]
fn each_sketch_group_menu_runs_a_command_that_has_no_button_of_its_own() {
    let mut harness = Harness::new();
    let feature = sketch_with_a_lone_point(&mut harness);

    open_sketch_menu(&mut harness, "History");
    choose(&mut harness, Command::UndoHistory);
    assert!(harness.workspace.undo_history_open);
    harness.perform(Action::Preferences(PreferencesCommand::CloseUndoHistory));
    harness.frame();

    open_sketch_menu(&mut harness, "Select");
    choose(&mut harness, Command::SelectFree);
    assert!(!harness.workspace.viewport.selection().is_empty());

    open_sketch_menu(&mut harness, "Draw");
    choose(&mut harness, Command::SketchTool(Tool::EllipticalArc));
    assert_eq!(harness.tool(), Some(Tool::EllipticalArc));

    open_sketch_menu(&mut harness, "Modify");
    choose(&mut harness, Command::SketchTool(Tool::TangentCircle));
    assert_eq!(harness.tool(), Some(Tool::TangentCircle));

    open_sketch_menu(&mut harness, "Constrain");
    choose(&mut harness, Command::FindRelations);
    assert_eq!(harness.workspace.tidying.feature(), Some(feature));

    assert!(!harness.workspace.viewport.first_dimension_scales());
    open_sketch_menu(&mut harness, "Dimension");
    choose(&mut harness, Command::ToggleFirstDimensionScales);
    assert!(harness.workspace.viewport.first_dimension_scales());
    assert_eq!(harness.editing(), Some(feature));
}

#[test]
fn the_sketch_menus_list_every_command_of_their_kind_the_buttonless_ones_included() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();

    let lists = [
        (
            "Draw",
            vec![
                Command::SketchTool(Tool::ThreePointArc),
                Command::SketchTool(Tool::Conic),
                Command::SketchTool(Tool::Gear),
                Command::SketchTool(Tool::Text),
                Command::ShapeMode(ShapeMode::Rectangle(RectangleMode::ThreePoints)),
                Command::ChainArc,
                Command::DrawSymmetrically,
            ],
        ),
        (
            "Modify",
            vec![
                Command::Construction,
                Command::SketchTool(Tool::Intersect),
                Command::SketchTool(Tool::BlendCurve),
                Command::ShapeMode(ShapeMode::Offset(OffsetMode::BothFlat)),
                Command::RotateGeometry,
                Command::PasteGeometry,
                Command::IntersectBody,
            ],
        ),
        (
            "Constrain",
            vec![
                Command::Constraint(ConstraintTool::Curvature),
                Command::CheckSketch,
            ],
        ),
        (
            "Dimension",
            vec![
                Command::SketchTool(Tool::Dimension),
                Command::DimensionFromDatum,
            ],
        ),
    ];
    for (group, commands) in lists {
        open_sketch_menu(&mut harness, group);
        let over = commands
            .first()
            .map(|first| harness.position_of(&first.title()))
            .expect("every list names a command");
        for command in commands {
            harness.wheel_until_shown(over, &command.title(), Wheel::Down);
        }
        close(&mut harness);
    }
    assert!(harness.editing().is_some());
}

#[test]
fn the_sketch_draw_menu_marks_the_active_tool() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.context.enable_accesskit();

    open_sketch_menu(&mut harness, "Draw");
    assert_eq!(
        marked(&harness, Command::SketchTool(Tool::Conic)),
        Some(false)
    );
    choose(&mut harness, Command::SketchTool(Tool::Conic));

    open_sketch_menu(&mut harness, "Draw");
    assert_eq!(
        marked(&harness, Command::SketchTool(Tool::Conic)),
        Some(true)
    );
    assert_eq!(
        marked(&harness, Command::SketchTool(Tool::EllipticalArc)),
        Some(false)
    );
    let way = |mode| Command::ShapeMode(ShapeMode::Rectangle(mode));
    assert_eq!(marked(&harness, way(RectangleMode::Corners)), Some(true));
    assert_eq!(
        marked(&harness, way(RectangleMode::ThreePoints)),
        Some(false)
    );
}

#[test]
fn escape_closes_a_sketch_menu_without_leaving_the_sketch_and_opening_focuses_an_entry() {
    let mut harness = Harness::new();
    let feature = harness.draw_on_new_sketch();
    let conic = Command::SketchTool(Tool::Conic).title();

    open_sketch_menu(&mut harness, "Draw");
    assert!(harness.shows(&conic));
    assert!(harness.focused().is_some());
    close(&mut harness);

    assert!(!harness.shows(&conic));
    assert_eq!(harness.editing(), Some(feature));
}

#[test]
fn the_sketch_menus_open_and_stay_on_screen_from_75_to_200_percent() {
    let probes = [
        ("History", Command::UndoHistory, Command::CancelSketch),
        ("Select", Command::SelectFree, Command::SelectRedundant),
        (
            "Draw",
            Command::SketchTool(Tool::ThreePointArc),
            Command::Section(SectionCommand::SliceSketch),
        ),
        (
            "Modify",
            Command::SketchTool(Tool::Trim),
            Command::IntersectBody,
        ),
        (
            "Constrain",
            Command::Constraint(ConstraintTool::Curvature),
            Command::CheckSketch,
        ),
        (
            "Dimension",
            Command::DimensionFromDatum,
            Command::ToggleFirstDimensionScales,
        ),
    ];
    for scale in [0.75, 1.0, 1.5, 2.0] {
        let mut harness = Harness::new();
        scaled(&mut harness, scale);
        harness.draw_on_new_sketch();
        let room = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / scale);

        for (group, first, last) in probes {
            open_sketch_menu(&mut harness, group);
            let over = harness.position_of(&first.title());
            let title = last.title();
            harness.wheel_until_shown(over, &title, Wheel::Down);

            assert!(harness.fully_shown(&title), "{group} at {scale}");
            assert!(
                room.contains(lowest_in(&harness, room, &title)),
                "{group} at {scale}"
            );
            assert_readable(&harness, &format!("sketch {group} menu at {scale}"));
            close(&mut harness);
        }
    }
}
