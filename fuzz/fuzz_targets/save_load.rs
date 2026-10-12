#![no_main]

use caditor_document::{Document, Editor};
use caditor_file::fuzzing::{self, Journal, replay_journal};
use caditor_fuzz::document::{References, run};
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fn save(document: &Document, previous: Option<&[u8]>, seconds: u64) -> Vec<u8> {
    let bytes = match fuzzing::save_over(document, previous, seconds) {
        Ok(bytes) => bytes,
        Err(error) => panic!("a document could not be saved: {error}"),
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

fn check_journal(journal: &Journal, editor: &Editor) {
    let Some(bytes) = journal.encoded() else {
        panic!("the recovery journal could not be written");
    };
    let Some(replayed) = replay_journal(&bytes) else {
        panic!("the recovery journal could not be read back");
    };
    assert!(replayed.issues.is_empty(), "{:?}", replayed.issues);
    assert_eq!(
        replayed.unreadable_entries, 0,
        "a journal entry could not be read"
    );
    assert!(
        !replayed.stopped_early,
        "replaying the journal stopped early"
    );
    assert_eq!(replayed.replayed, journal.logged());
    assert!(
        replayed.document.same_content(editor.document()),
        "replaying the journal reached another document"
    );
}

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let mut editor = Editor::default();
    let mut saved = vec![editor.document().clone()];
    let mut file = save(editor.document(), None, 1);
    let mut journal = Journal::new(editor.document().clone());

    run(
        &mut input,
        &mut editor,
        &References::default(),
        |editor, entry| {
            journal.log(entry);
            let seconds = saved.len() as u64 + 1;
            file = save(editor.document(), Some(&file), seconds);
            saved.push(editor.document().clone());
        },
    );

    check_journal(&journal, &editor);
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
