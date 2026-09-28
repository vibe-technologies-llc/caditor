#![no_main]

use caditor_expression::{Expression, ParameterId};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let known = |name: &str| (name == "width").then(|| ParameterId::from_raw(1));
    let _ = Expression::parse(text, &known);
    let _ = Expression::parse_stored(text);
});
