#![no_main]

use caditor_file::MeshFormat;
use caditor_fuzz::{WORK_BUDGET, interrupt_after};
use caditor_kernel::interruptible;
use libfuzzer_sys::fuzz_target;

const FORMATS: [MeshFormat; 3] = [MeshFormat::Stl, MeshFormat::Obj, MeshFormat::ThreeMf];

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, file)) = bytes.split_first() else {
        return;
    };
    let Some(&format) = FORMATS.get(usize::from(tag) % FORMATS.len()) else {
        return;
    };
    interruptible(interrupt_after(WORK_BUDGET), || {
        let _ = caditor_file::parse_mesh(file, format, "fuzzed", "fuzzed");
    });
});
