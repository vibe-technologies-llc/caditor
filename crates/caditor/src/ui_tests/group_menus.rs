use egui::{
    Key, Modifiers, Pos2, Rect,
    accesskit::{Role, Toggled},
};

use super::{Harness, SCREEN, assert_readable, lowest_in};
use crate::{
    analysis::AnalysisCommand,
    commands::Command,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand},
    section::SectionCommand,
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
        ("History", Command::UndoHistory),
        ("Sketch", Command::EditSketch),
        ("Solid", Command::Thread),
        ("Modify", Command::MirrorFaces),
        ("Pattern", Command::PointPattern),
        ("Reference", Command::CoordinateSystem),
        ("Analyse", Command::Interference),
    ];
    for scale in [0.75, 1.0, 1.5, 2.0] {
        let mut harness = Harness::new();
        scaled(&mut harness, scale);
        let room = Rect::from_min_size(Pos2::ZERO, SCREEN.size() / scale);

        for (group, probe) in probes {
            open(&mut harness, group);

            let title = probe.title();
            assert!(harness.fully_shown(&title), "{group} at {scale}");
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
