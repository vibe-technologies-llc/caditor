#![no_main]

use caditor_file::TextOutlines;
use libfuzzer_sys::fuzz_target;

const INTER: &[u8] = include_bytes!("../../crates/caditor/assets/fonts/InterVariable.ttf");
const INTER_ITALIC: &[u8] =
    include_bytes!("../../crates/caditor/assets/fonts/InterVariable-Italic.ttf");

fuzz_target!(|bytes: &[u8]| {
    let _ = caditor_file::parse_svg(
        bytes,
        TextOutlines::InFont {
            upright: INTER,
            italic: Some(INTER_ITALIC),
        },
    );
});
