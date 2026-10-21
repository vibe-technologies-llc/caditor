use caditor_document::Transaction;
use egui::{Key, Modifiers, accesskit::Role};

use super::{CAMERA_SETTLE, Harness, run_from_palette, sample_document};
use crate::{app::Workspace, model::Action};

#[test]
fn model_properties_are_edited_in_a_dialog_as_one_undoable_change() {
    use caditor_document::ModelProperty;

    use crate::model_properties::{SAVE_LABEL, field_id};

    let mut harness = Harness::new();
    harness.context.enable_accesskit();

    run_from_palette(&mut harness, "model properties");
    harness.frame();
    let title_focused = harness.focused() == Some(field_id(ModelProperty::Title));
    let captioned = harness.captioned(Role::TextInput, "Part number");
    harness.type_text("Wall bracket");
    harness.type_into_field(field_id(ModelProperty::PartNumber), "BR-100");
    harness.type_into_field(field_id(ModelProperty::Notes), "Print it flat");
    harness.click(SAVE_LABEL);
    harness.frame();
    let closed = harness.workspace.model_properties.is_none();

    assert!(title_focused);
    assert!(captioned);
    assert!(closed);
    assert_eq!(harness.document().properties().title, "Wall bracket");
    assert_eq!(harness.document().properties().part_number, "BR-100");
    assert_eq!(harness.document().properties().notes, "Print it flat");
    assert_eq!(
        harness.model.undo_label(),
        Some(crate::model_properties::CHANGE_LABEL)
    );

    harness.perform(Action::Undo);
    assert!(harness.document().properties().is_empty());
}

fn turn_the_view(harness: &mut Harness) {
    harness.key(Key::ArrowLeft, Modifiers::NONE);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
}

#[test]
fn the_current_view_is_saved_from_the_palette_and_shown_again_from_a_search() {
    let mut harness = Harness::new();
    harness.settle();
    turn_the_view(&mut harness);
    let saved = harness.workspace.viewport.viewpoint();

    run_from_palette(&mut harness, "save the current view");
    harness.frame();
    let named = harness.document().saved_views().named.clone();
    let label = harness.model.undo_label().map(str::to_owned);

    turn_the_view(&mut harness);
    let moved = harness.workspace.viewport.viewpoint();
    run_from_palette(&mut harness, "view 1");
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let returned = harness.workspace.viewport.viewpoint();

    assert_eq!(named.len(), 1);
    assert_eq!(named[0].name, "View 1");
    assert_eq!(label.as_deref(), Some("Save view “View 1”"));
    assert_ne!(moved.orientation, saved.orientation);
    assert!(returned.orientation.angle_between(saved.orientation) < 1e-6);
    assert!((returned.distance - saved.distance).abs() < 1e-6);
    assert!(returned.target.distance(saved.target) < 1e-6);

    harness.perform(Action::Undo);
    assert!(harness.document().saved_views().is_empty());
}

#[test]
fn the_saved_views_dialog_saves_renames_updates_and_deletes_views() {
    use crate::saved_views::{SAVE_LABEL, name_field_id, rename_field_id};

    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.settle();

    run_from_palette(&mut harness, "saved views");
    harness.frame();
    let focused = harness.focused() == Some(name_field_id());
    harness.type_into_field(name_field_id(), "Hidden bore");
    harness.frame();
    let first = harness.document().saved_views().named.clone();
    harness.type_into_field(name_field_id(), "hidden bore");
    harness.frame();
    let refused = harness.shows_containing("There is already a view named");
    harness.type_into_field(name_field_id(), "As drawn");
    harness.click(SAVE_LABEL);
    harness.frame();
    let two = harness.document().saved_views().named.len();

    harness.click_button("Rename it");
    harness.frame();
    harness.type_into_field(rename_field_id(), "Bore");
    harness.frame();
    let renamed: Vec<String> = harness
        .document()
        .saved_views()
        .named
        .iter()
        .map(|named| named.name.clone())
        .collect();

    harness.click_button("Delete it");
    harness.frame();
    let left = harness.document().saved_views().named.len();

    assert!(focused);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name, "Hidden bore");
    assert!(refused);
    assert_eq!(two, 2);
    assert_eq!(renamed.len(), 2);
    assert!(renamed.contains(&"Bore".to_owned()));
    assert_eq!(left, 1);
}

#[test]
fn escape_in_a_saved_view_s_rename_row_keeps_the_name_and_the_dialog() {
    use crate::saved_views::{SAVE_LABEL, name_field_id};

    let mut harness = Harness::new();
    harness.context.enable_accesskit();
    harness.settle();
    run_from_palette(&mut harness, "saved views");
    harness.frame();
    harness.type_into_field(name_field_id(), "As drawn");
    harness.click(SAVE_LABEL);
    harness.frame();
    harness.click_button("Rename it");
    harness.frame();
    harness.type_text("Changed");
    assert!(harness.has_button("Keep the name (Esc)"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.saved_views.is_some());
    assert_eq!(harness.document().saved_views().named[0].name, "As drawn");
    assert!(!harness.has_button("Keep the name (Esc)"));
    assert!(harness.has_button("Rename it"));

    harness.key(Key::Escape, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert!(harness.workspace.saved_views.is_none());
}

#[test]
fn the_isometric_view_is_redefined_by_the_current_view_and_reset() {
    let mut harness = Harness::new();
    harness.settle();
    turn_the_view(&mut harness);
    let wanted = harness.workspace.viewport.viewpoint();

    run_from_palette(&mut harness, "make the current view the isometric");
    harness.frame();
    let home = harness.document().saved_views().home;

    turn_the_view(&mut harness);
    turn_the_view(&mut harness);
    let away = harness.workspace.viewport.viewpoint();
    harness.key(Key::Num0, Modifiers::ALT);
    harness.frame();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let back = harness.workspace.viewport.viewpoint();

    run_from_palette(&mut harness, "reset the isometric");
    harness.frame();
    let reset = harness.document().saved_views().home;

    assert!(home.is_some());
    assert!(away.orientation.angle_between(wanted.orientation) > 0.01);
    assert!(back.orientation.angle_between(wanted.orientation) < 1e-6);
    assert!((back.distance - wanted.distance).abs() < 1e-6);
    assert_eq!(reset, None);
}

#[test]
fn a_model_with_a_redefined_isometric_view_opens_looking_from_it() {
    use caditor_document::{Edit, SavedView, SavedViews};
    use caditor_geometry::{Point3, Rotation3, Vector3};

    let mut document = sample_document().unwrap();
    let home = SavedView {
        target: Point3::new(5.0, 5.0, 5.0),
        orientation: Rotation3::from_axis_angle(Vector3::X, 1.2),
        distance: 90.0,
    };
    document
        .apply(Transaction::single(
            "Isometric",
            Edit::SetSavedViews {
                views: Box::new(SavedViews {
                    named: Vec::new(),
                    home: Some(home),
                }),
            },
        ))
        .unwrap();

    let mut harness = Harness::starting(None, document, Workspace::new());
    harness.settle();
    harness.workspace.viewport.advance(CAMERA_SETTLE);
    harness.frame();
    let opened = harness.workspace.viewport.viewpoint();

    assert!(opened.orientation.angle_between(home.orientation) < 1e-6);
}

#[test]
fn cancelling_the_model_properties_changes_nothing() {
    use caditor_document::ModelProperty;

    use crate::model_properties::{CANCEL_LABEL, field_id};

    let mut harness = Harness::new();
    let revision = harness.model.revision();

    harness.perform(Action::Preferences(
        crate::preferences::PreferencesCommand::ShowModelProperties,
    ));
    harness.frame();
    harness.type_into_field(field_id(ModelProperty::Revision), "B");
    harness.click(CANCEL_LABEL);
    harness.frame();

    assert!(harness.workspace.model_properties.is_none());
    assert!(harness.document().properties().is_empty());
    assert_eq!(harness.model.revision(), revision);
}
