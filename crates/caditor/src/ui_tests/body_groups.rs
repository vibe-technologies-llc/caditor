use egui::{Key, Modifiers};

use super::{Harness, add_peg, extruded_plate, hidden, run_from_palette};
use crate::{icons, model::Action};

fn folder(harness: &Harness, body: caditor_document::FeatureId) -> Vec<String> {
    harness.document().body_folder(body)
}

fn named(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn group_and_name(harness: &mut Harness, name: &str) {
    run_from_palette(harness, "group bodies");
    harness.settle();
    harness.frame();
    harness.replace_text(name);
    harness.key(Key::Enter, Modifiers::NONE);
    harness.settle();
}

#[test]
fn bodies_group_nest_hide_together_and_ungroup_from_the_palette() {
    let mut harness = Harness::new();
    let (plate, _) = extruded_plate(&mut harness);
    let peg = add_peg(&mut harness);
    harness.settle();

    harness.workspace.panels.choose_all(&[plate, peg]);
    harness.frame();
    group_and_name(&mut harness, "Fixtures");
    let grouped = (folder(&harness, plate), folder(&harness, peg));
    let renamed_label = harness.model.undo_label().map(str::to_owned);
    let listed = harness.shows("Fixtures");

    harness.workspace.panels.choose_only(peg);
    harness.frame();
    group_and_name(&mut harness, "Pins");
    let nested = folder(&harness, peg);
    let nested_listed = harness.shows("Pins");

    harness.click_beside(icons::SHOW, "Fixtures");
    harness.settle();
    let both_hidden = hidden(&harness, plate) && hidden(&harness, peg);
    let hide_label = harness.model.undo_label().map(str::to_owned);
    harness.perform(Action::Undo);
    harness.settle();

    harness.click("Fixtures");
    harness.frame();
    run_from_palette(&mut harness, "ungroup bodies");
    harness.settle();
    let ungrouped = (folder(&harness, plate), folder(&harness, peg));

    harness.workspace.panels.choose_only(peg);
    harness.workspace.panels.body_group = None;
    harness.frame();
    run_from_palette(&mut harness, "move out of the group");
    harness.settle();

    assert_eq!(grouped, (named(&["Fixtures"]), named(&["Fixtures"])));
    assert_eq!(renamed_label.as_deref(), Some("Rename Group 1"));
    assert!(listed);
    assert_eq!(nested, named(&["Fixtures", "Pins"]));
    assert!(nested_listed);
    assert!(both_hidden);
    assert_eq!(hide_label.as_deref(), Some("Hide Fixtures"));
    assert_eq!(ungrouped, (named(&[]), named(&["Pins"])));
    assert!(folder(&harness, peg).is_empty());
    assert!(!hidden(&harness, plate) && !hidden(&harness, peg));
}
