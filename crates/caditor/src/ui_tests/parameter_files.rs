use std::fs;

use tempfile::TempDir;

use super::{Harness, run_from_palette};
use crate::files::{FileCommand, parameters::ParametersCommand};

fn notice_says(harness: &Harness, text: &str) -> bool {
    harness
        .model
        .notice()
        .is_some_and(|notice| notice.text.contains(text))
}

#[test]
fn parameters_export_to_csv_and_import_back_after_a_preview_as_one_change() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();

    harness.answer_dialog(Some(dir.path().join("dimensions")));
    run_from_palette(&mut harness, "Export parameters");
    harness.wait_until("the parameters are exported", |harness| {
        notice_says(harness, "Exported 2 parameters")
    });
    let written = fs::read_to_string(dir.path().join("dimensions.csv")).unwrap();
    assert_eq!(
        written,
        "caditor-parameters,2\r\nname,expression,value,note\r\nwidth,40 mm,40 mm,\r\nheight,width / 2,20 mm,\r\n"
    );

    let edited = dir.path().join("edited.csv");
    fs::write(
        &edited,
        "name,expression,note\nwidth,60 mm,\ndepth,height + 5 mm,how deep\n2bad,1 mm,\nloop,loop,\n",
    )
    .unwrap();
    harness.answer_dialog(Some(edited));
    run_from_palette(&mut harness, "Import parameters");
    harness.wait_until("the preview opens", |harness| {
        harness.shows("Import parameters from “edited.csv”")
    });
    assert!(harness.shows("1 parameter is already in the model with another value."));
    assert!(
        harness.shows("2 rows cannot be imported and will be left out; the reason is beside each.")
    );
    assert!(harness.shows("60 mm (was 40 mm)"));
    assert!(harness.document().parameter_named("depth").is_none());

    harness.command(FileCommand::Parameters(ParametersCommand::Apply));
    harness.settle();

    let document = harness.document();
    assert_eq!(
        document.parameter_named("width").unwrap().expression,
        document.parse("60 mm").unwrap()
    );
    assert_eq!(document.parameter_named("depth").unwrap().note, "how deep");
    assert!(document.parameter_named("loop").is_none());
    assert_eq!(
        harness.model.undo_label(),
        Some("Import parameters from edited.csv")
    );
    assert!(notice_says(
        &harness,
        "added 1 parameter, changed 1, left out 2"
    ));
    assert!(!harness.shows("Import parameters from “edited.csv”"));
}

#[test]
fn a_file_without_the_columns_is_refused_in_words() {
    let dir = TempDir::new().unwrap();
    let mut harness = Harness::new();
    let file = dir.path().join("list.csv");
    fs::write(&file, "a,b\n1,2\n").unwrap();

    harness.answer_dialog(Some(file));
    harness.command(FileCommand::ImportParameters);
    harness.wait_until("the file is refused", |harness| {
        notice_says(
            harness,
            "Could not import parameters from “list.csv”: its first row does not name a “name” \
             and an “expression” column.",
        )
    });
}
