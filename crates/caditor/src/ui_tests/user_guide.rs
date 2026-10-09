use egui::{Key, Modifiers, Pos2};

use super::Harness;
use crate::{defender, guide::Page, guide_panel, model::Action, preferences::PreferencesCommand};

const LINK_INSET: f32 = 3.0;

fn click_link(harness: &mut Harness, text: &str) {
    let rect = harness
        .texts
        .iter()
        .find(|(shown, _)| shown == text)
        .unwrap_or_else(|| panic!("no link '{text}'"))
        .1;
    harness.click_screen(Pos2::new(rect.right() - LINK_INSET, rect.center().y));
    harness.frame();
}

#[test]
fn f1_opens_the_page_of_the_tool_in_use_and_its_links_and_back_lead_on() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();
    harness.use_tool(Key::L);

    harness.key(Key::F1, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.guide.open);
    assert_eq!(harness.workspace.guide.page(), Some(Page::Line));
    assert!(harness.shows("Sketch tools"));

    click_link(&mut harness, "arcs");

    assert_eq!(harness.workspace.guide.page(), Some(Page::Arcs));

    harness.click_button(guide_panel::BACK);
    harness.frame();

    assert_eq!(harness.workspace.guide.page(), Some(Page::Line));

    harness.key(Key::F1, Modifiers::NONE);
    harness.frame();

    assert!(!harness.workspace.guide.open);
}

#[test]
fn panels_link_to_their_page_and_the_guide_searches_every_page() {
    let mut harness = Harness::new();
    harness.draw_on_new_sketch();

    harness.click("Help on sketches");
    harness.frame();

    assert_eq!(harness.workspace.guide.page(), Some(Page::Sketches));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.key(Key::I, Modifiers::NONE);
    harness.frame();
    harness.frame();
    harness.click_button("Open “Measure” in the user guide");
    harness.frame();

    assert_eq!(harness.workspace.guide.page(), Some(Page::Measure));

    harness.click(guide_panel::SEARCH_HINT);
    harness.type_text("functions");

    assert!(harness.shows("Expressions"));

    click_link(&mut harness, "Expressions");

    assert_eq!(harness.workspace.guide.page(), Some(Page::Expressions));
}

#[test]
fn the_defender_reminder_shows_when_asked_and_leads_to_its_page() {
    let mut harness = Harness::new();
    harness.workspace.preferences.onboarding.defender_reminded = false;

    harness.perform(Action::Preferences(
        PreferencesCommand::ShowDefenderReminder,
    ));
    harness.frame();

    assert!(harness.shows(defender::TITLE));

    harness.click(defender::READ_MORE);
    harness.frame();

    assert!(!harness.shows(defender::TITLE));
    assert_eq!(harness.workspace.guide.page(), Some(Page::WindowsDefender));
    assert!(harness.workspace.preferences.onboarding.defender_reminded);
}
