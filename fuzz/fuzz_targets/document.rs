#![no_main]

use caditor_document::Editor;
use caditor_fuzz::document::{recompute, run};
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let mut editor = Editor::default();
    run(&mut input, &mut editor, |_| {});
    let _ = recompute(editor.document());
});
