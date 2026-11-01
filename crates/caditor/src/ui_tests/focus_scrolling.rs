use std::sync::Arc;

use caditor_cloud::{
    Design, DesignId, Folder, Project, ProjectId, Workspace as CloudWorkspace, WorkspaceId,
    WorkspaceKind, fake::FakeProvider,
};
use caditor_document::{
    ConfiguredValue, Edit, NamedView, SavedView, SavedViews, SelectionSet, SelectionSets,
    SetMember, Transaction,
};
use caditor_geometry::{Point3, Rotation3};
use egui::{Key, Modifiers};
use tempfile::TempDir;

use super::{
    Harness, extruded_plate,
    home::{open_home, with_cloud},
    run_from_palette, sample_document, write_drawing,
};
use crate::{
    app::Workspace, cloud_dialog, files::FileCommand, history::HistoryCommand, model::Action,
};

const MOST_TABS: usize = 120;

fn shown_whole(harness: &Harness, wanted: &dyn Fn(&str) -> bool) -> bool {
    harness
        .texts
        .iter()
        .zip(&harness.text_clips)
        .any(|((shown, rect), clip)| wanted(shown) && clip.expand(0.5).contains_rect(*rect))
}

fn tab_until(harness: &mut Harness, wanted: &dyn Fn(&str) -> bool) -> usize {
    harness.frame();
    assert!(
        !shown_whole(harness, wanted),
        "the row is on screen before any Tab"
    );
    for tabs in 1..=MOST_TABS {
        harness.key(Key::Tab, Modifiers::NONE);
        harness.frame();
        harness.let_animations_finish();
        if shown_whole(harness, wanted) {
            return tabs;
        }
    }
    panic!("the row never came into view by Tab");
}

fn tab_until_shown(harness: &mut Harness, label: &str) -> usize {
    tab_until(harness, &|shown| shown == label)
}

#[test]
fn tab_scrolls_a_saved_view_below_the_list_into_view() {
    let named = (1..=14)
        .map(|number| NamedView {
            name: format!("View {number:02}"),
            view: SavedView {
                target: Point3::ZERO,
                orientation: Rotation3::IDENTITY,
                distance: 100.0,
            },
        })
        .collect();
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.perform(Action::Apply(Transaction::single(
        "Saved views",
        Edit::SetSavedViews {
            views: Box::new(SavedViews { named, home: None }),
        },
    )));
    harness.settle();

    run_from_palette(&mut harness, "saved views");
    harness.show_new_windows();

    tab_until_shown(&mut harness, "View 14");
}

#[test]
fn tab_scrolls_a_selection_set_below_the_list_into_view() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let (plate, _) = extruded_plate(&mut harness);
    let sets = (1..=14)
        .map(|number| SelectionSet {
            name: format!("Set {number:02}"),
            members: vec![SetMember::Body(plate)],
        })
        .collect();
    harness.perform(Action::Apply(Transaction::single(
        "Selection sets",
        Edit::SetSelectionSets {
            sets: Box::new(SelectionSets { sets }),
        },
    )));
    harness.settle();

    run_from_palette(&mut harness, "selection sets");
    harness.show_new_windows();

    tab_until_shown(&mut harness, "Set 14");
}

#[test]
fn tab_scrolls_a_layer_below_the_import_options_list_into_view() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::with_directories(Some(dir.path()));
    harness.context.enable_accesskit();
    let plan = dir.path().join("plan.dxf");
    let entities: String = (1..=14)
        .map(|number| {
            format!("0\nLINE\n8\nLayer {number:02}\n10\n0\n20\n{number}\n11\n40\n21\n{number}\n")
        })
        .collect();
    write_drawing(&plan, Some(4), &entities);

    harness.answer_dialog(Some(plan));
    harness.command(FileCommand::Import { into: None });
    harness.wait_for_import_options("plan.dxf");

    tab_until_shown(&mut harness, "Layer 14 (1 curve)");
}

#[test]
fn tab_scrolls_a_configured_value_beyond_the_table_into_view() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    let mut transaction = harness.document().transaction("Add sizes");
    for number in 1..=24 {
        let expression = transaction.parse("10 mm").unwrap();
        transaction.add_parameter(format!("size_{number:02}"), expression);
    }
    harness.perform(Action::Apply(transaction.finish()));
    harness.settle();

    run_from_palette(&mut harness, "configurations");
    harness.show_new_windows();
    harness.click_button(crate::configurations::ADD_LABEL);
    harness.frame();
    for number in 1..=24 {
        let parameter = harness.parameter(&format!("size_{number:02}"));
        let configure = harness
            .document()
            .configuring(ConfiguredValue::Parameter(parameter))
            .unwrap();
        harness.perform(Action::Apply(configure));
        harness.frame();
    }

    tab_until_shown(&mut harness, "size_24");
}

#[test]
fn tab_scrolls_a_version_below_the_history_list_into_view() {
    let dir = TempDir::new().unwrap();
    let plate = sample_document().unwrap();
    let mut harness = Harness::starting(Some(dir.path()), plate, Workspace::new());
    harness.context.enable_accesskit();
    harness.answer_dialog(Some(dir.path().join("plate.caditor")));
    harness.command(FileCommand::SaveAs);
    harness.wait_until("the model is saved", |harness| {
        harness.model.path().is_some() && !harness.model.is_saving()
    });
    for number in 1..=12 {
        let mut transaction = harness.document().transaction(format!("Step {number:02}"));
        let expression = transaction.parse("1 mm").unwrap();
        transaction.add_parameter(format!("extra_{number:02}"), expression);
        harness.perform(Action::Apply(transaction.finish()));
        harness.settle();
        harness.command(FileCommand::Save);
        harness.wait_until("the change is saved", |harness| !harness.model.is_dirty());
    }

    harness.context.set_zoom_factor(2.0);
    harness.command(FileCommand::History(HistoryCommand::Show));
    harness.wait_until("the versions are listed", |harness| {
        harness.shows_containing("Saved just now") && harness.shows("Restore")
    });
    harness.show_new_windows();

    tab_until(&mut harness, &|shown| shown.ends_with("after “Step 02”"));
}

fn crowded_provider() -> Arc<FakeProvider> {
    let hub = WorkspaceId::new("hub");
    let designs: Vec<Design> = (1..=14)
        .map(|number| Design {
            id: DesignId::new(format!("d{number}")),
            project: ProjectId::new("p1"),
            name: format!("Design {number:02}"),
            modified: None,
        })
        .collect();
    let provider = (1..=14).fold(
        FakeProvider::new().workspace(CloudWorkspace {
            id: hub.clone(),
            name: "Studio".to_owned(),
            kind: WorkspaceKind::Team,
        }),
        |provider, number| {
            let root = Folder {
                designs: if number == 1 {
                    designs.clone()
                } else {
                    Vec::new()
                },
                ..Folder::default()
            };
            provider.project(
                Project {
                    id: ProjectId::new(format!("p{number}")),
                    workspace: hub.clone(),
                    name: format!("Project {number:02}"),
                },
                root,
            )
        },
    );
    Arc::new(provider)
}

fn signed_in_to_the_cloud_dialog(dir: &TempDir, provider: &Arc<FakeProvider>) -> Harness {
    let mut harness = with_cloud(dir.path(), provider);
    run_from_palette(&mut harness, "Cloud projects");
    harness.frame();
    harness.frame();
    harness.click(cloud_dialog::SIGN_IN);
    harness.wait_until("the projects are listed", |harness| {
        harness.shows("Project 01")
    });
    harness
}

#[test]
fn tab_scrolls_a_project_below_the_cloud_dialog_s_list_into_view() {
    let dir = TempDir::new().unwrap();
    let provider = crowded_provider();
    let mut harness = signed_in_to_the_cloud_dialog(&dir, &provider);

    tab_until_shown(&mut harness, "Project 14");
}

#[test]
fn tab_scrolls_a_sample_below_the_fold_of_home_into_view() {
    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.context.set_zoom_factor(2.0);
    open_home(&mut harness);

    tab_until_shown(&mut harness, "Angle bracket");
}

#[test]
fn tab_scrolls_a_design_below_the_cloud_dialog_s_list_into_view() {
    let dir = TempDir::new().unwrap();
    let provider = crowded_provider();
    let mut harness = signed_in_to_the_cloud_dialog(&dir, &provider);
    harness.click("Project 01");
    harness.wait_until("the designs are listed", |harness| {
        harness.shows("Design 01")
    });

    tab_until_shown(&mut harness, "Design 14");
}
