#![no_main]

use caditor_expression::{Expression, ParameterId};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let known = |name: &str| (name == "width").then(|| ParameterId::from_raw(1));
    if let Ok(parsed) = Expression::parse(text, &known) {
        let stored = parsed.to_stored_text();
        assert_eq!(Expression::parse_stored(&stored), Ok(parsed), "{stored}");
    }
    if let Ok(parsed) = Expression::parse_stored(text) {
        let stored = parsed.to_stored_text();
        assert_eq!(Expression::parse_stored(&stored), Ok(parsed), "{stored}");
    }
});
