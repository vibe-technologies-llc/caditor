use caditor_geometry::{Point3, Rotation3, Vector3};

use crate::{tests::sample, *};

fn set(views: SavedViews) -> Transaction {
    Transaction::single(
        "Saved views",
        Edit::SetSavedViews {
            views: Box::new(views),
        },
    )
}

fn view(distance: f64) -> SavedView {
    SavedView {
        target: Point3::new(10.0, 20.0, 5.0),
        orientation: Rotation3::from_axis_angle(Vector3::Z, 0.5),
        distance,
    }
}

fn named(name: &str, distance: f64) -> NamedView {
    NamedView {
        name: name.to_owned(),
        view: view(distance),
    }
}

fn bore() -> SavedViews {
    SavedViews {
        named: vec![named("Hidden bore", 120.0), named("As drawn", 300.0)],
        home: Some(view(250.0)),
    }
}

#[test]
fn a_new_model_has_no_saved_views() {
    let (document, _) = sample();

    assert!(document.saved_views().is_empty());
    assert_eq!(document.saved_views(), &SavedViews::default());
}

#[test]
fn saving_views_is_undoable_content() {
    let (document, _) = sample();
    let mut editor = Editor::new(document.clone());

    let changed = editor.apply(set(bore())).unwrap();
    let with_views = editor.document().clone();
    editor.undo().unwrap();

    assert!(changed);
    assert_eq!(with_views.saved_views(), &bore());
    assert!(!with_views.same_content(&document));
    assert!(editor.document().same_content(&document));
    assert_eq!(editor.redo_label(), Some("Saved views"));
}

#[test]
fn saving_the_same_views_again_is_no_change() {
    let (document, _) = sample();
    let mut editor = Editor::new(document);
    editor.apply(set(bore())).unwrap();
    let revision = editor.revision();

    let changed = editor.apply(set(bore())).unwrap();

    assert!(!changed);
    assert_eq!(editor.revision(), revision);
}

#[test]
fn view_names_are_trimmed_to_one_line_and_orientations_normalised() {
    let (mut document, _) = sample();
    let mut views = SavedViews {
        named: vec![named("  Hidden\r\n bore \n", 120.0)],
        home: None,
    };
    views.named[0].view.orientation *= 3.0;

    document.apply(set(views)).unwrap();

    let kept = &document.saved_views().named[0];
    assert_eq!(kept.name, "Hidden bore");
    assert!((kept.view.orientation.length() - 1.0).abs() < 1e-12);
}

#[test]
fn a_view_without_a_name_or_with_a_taken_one_is_refused() {
    let (mut document, _) = sample();
    let before = document.clone();
    let blank = SavedViews {
        named: vec![named("  \n ", 100.0)],
        home: None,
    };
    let twice = SavedViews {
        named: vec![named("Bore", 100.0), named("bore", 200.0)],
        home: None,
    };

    let empty = document.apply(set(blank));
    let taken = document.apply(set(twice));

    assert_eq!(empty, Err(EditError::ViewNameEmpty));
    assert_eq!(taken, Err(EditError::ViewNameTaken("bore".to_owned())));
    assert!(
        taken
            .unwrap_err()
            .to_string()
            .contains("Choose another name")
    );
    assert_eq!(document, before);
}

#[test]
fn a_name_too_long_and_too_many_views_are_refused() {
    let (mut document, _) = sample();
    let long = SavedViews {
        named: vec![named(&"v".repeat(MAX_VIEW_NAME_CHARS + 1), 100.0)],
        home: None,
    };
    let many = SavedViews {
        named: (0..=MAX_SAVED_VIEWS)
            .map(|number| named(&format!("View {number}"), 100.0))
            .collect(),
        home: None,
    };

    let too_long = document.apply(set(long));
    let too_many = document.apply(set(many));

    assert_eq!(
        too_long,
        Err(EditError::ViewNameTooLong {
            length: MAX_VIEW_NAME_CHARS + 1
        })
    );
    assert_eq!(too_many, Err(EditError::TooManyViews));
}

#[test]
fn a_view_the_camera_cannot_show_is_refused_naming_it() {
    let (mut document, _) = sample();
    let mut broken = named("Far", 100.0);
    broken.view.distance = f64::INFINITY;
    let mut flat = view(100.0);
    flat.distance = 0.0;

    let named_refused = document.apply(set(SavedViews {
        named: vec![broken],
        home: None,
    }));
    let home_refused = document.apply(set(SavedViews {
        named: Vec::new(),
        home: Some(flat),
    }));

    assert_eq!(
        named_refused,
        Err(EditError::ViewNotUsable("Far".to_owned()))
    );
    assert_eq!(
        home_refused,
        Err(EditError::ViewNotUsable(HOME_VIEW_NAME.to_owned()))
    );
}

#[test]
fn restoring_a_version_brings_its_saved_views_back() {
    let (earlier, _) = sample();
    let mut later = earlier.clone();
    later.apply(set(bore())).unwrap();

    let restore = later.transaction_to(&earlier, "Restore");
    let mut restored = later.clone();
    restored.apply(restore).unwrap();
    let forward = earlier.transaction_to(&later, "Restore");
    let mut forwarded = earlier.clone();
    forwarded.apply(forward).unwrap();

    assert!(restored.same_content(&earlier));
    assert!(forwarded.same_content(&later));
}

#[test]
fn a_views_change_is_touched_and_counted() {
    let transaction = set(bore());

    let touched = transaction.touched();

    assert!(touched.views);
    assert!(touched.features.is_empty());
    assert!(transaction.approximate_size() > bore().heap_size());
}

#[test]
fn the_next_unused_name_skips_the_ones_taken() {
    let views = SavedViews {
        named: vec![named("View 2", 100.0), named("view 3", 100.0)],
        home: None,
    };

    assert_eq!(views.unused_name(), "View 4");
    assert_eq!(SavedViews::default().unused_name(), "View 1");
    assert_eq!(views.position("VIEW 3"), Some(1));
}
