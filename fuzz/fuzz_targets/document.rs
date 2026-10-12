#![no_main]

use caditor_document::Editor;
use caditor_fuzz::document::{References, recompute, run};
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let mut editor = Editor::default();

    run(&mut input, &mut editor, &References::default(), |_, _| {});
    let first = recompute(editor.document());

    let captured = References::capture(&first);
    run(&mut input, &mut editor, &captured, |_, _| {});
    let _ = recompute(editor.document());
});
