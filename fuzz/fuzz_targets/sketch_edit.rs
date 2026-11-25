#![no_main]

use caditor_fuzz::sketch_edit::edited_sketch;
use caditor_geometry::Plane;
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let _ = edited_sketch(&mut input, Plane::XY);
});
