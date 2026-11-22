#![no_main]

use caditor_document::Document;
use caditor_file::{MAX_PARAMETER_ROWS, fuzzing};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(rows) = fuzzing::parameters(bytes) else {
        return;
    };
    let mut document = Document::default();

    assert!(rows.len() <= MAX_PARAMETER_ROWS);

    for _ in 0..2 {
        let Ok(plan) = document.plan_parameter_import("Import", &rows, true) else {
            return;
        };

        if let Some(transaction) = plan.transaction {
            document.apply(transaction).unwrap();
        }
    }
});
