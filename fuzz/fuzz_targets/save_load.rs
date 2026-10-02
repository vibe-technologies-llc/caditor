#![no_main]

use caditor_document::{Document, Editor};
use caditor_file::fuzzing;
use caditor_fuzz::document::run;
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fn save(document: &Document, previous: Option<&[u8]>, seconds: u64) -> Vec<u8> {
    let Some(bytes) = fuzzing::save_over(document, previous, seconds) else {
        panic!("a document could not be saved");
    };
    let loaded = match caditor_file::decode(&bytes) {
        Ok(loaded) => loaded,
        Err(error) => panic!("a saved document could not be loaded: {error}"),
    };
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert!(
        loaded.document.same_content(document),
        "a saved document loaded differently"
    );
    bytes
}

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let mut editor = Editor::default();
    let mut saved = vec![editor.document().clone()];
    let mut file = save(editor.document(), None, 1);
    run(&mut input, &mut editor, |editor| {
        let seconds = saved.len() as u64 + 1;
        file = save(editor.document(), Some(&file), seconds);
        saved.push(editor.document().clone());
    });
    for version in fuzzing::history(&file).versions {
        if !version.available {
            continue;
        }
        let loaded = match fuzzing::load_version(&file, version.index) {
            Ok(loaded) => loaded,
            Err(error) => panic!("version {} could not be loaded: {error}", version.index),
        };
        assert!(
            saved
                .iter()
                .any(|document| loaded.document.same_content(document)),
            "version {} holds a state that was never saved",
            version.index
        );
    }
});
