#![no_main]

use caditor_document::Document;
use caditor_file::fuzzing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(file) = fuzzing::configurations(bytes) else {
        return;
    };
    let mut document = Document::default();

    for row in &file.rows {
        assert_eq!(row.cells.len(), file.columns.len());
    }

    let mut transaction = document.transaction("Parameters");

    transaction.add_parameter("width", transaction.parse("40 mm").unwrap());
    transaction.add_parameter("turn", transaction.parse("-15 deg").unwrap());
    document.apply(transaction.finish()).unwrap();

    for _ in 0..2 {
        let Ok(plan) = document.plan_configuration_import("Import", &file, true) else {
            return;
        };

        if let Some(transaction) = plan.transaction {
            document.apply(transaction).unwrap();
        }
    }
});
