#![no_main]

use caditor_fuzz::expression::check_text;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| check_text(text));
