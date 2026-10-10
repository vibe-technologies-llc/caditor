#![no_main]

use caditor_fuzz::{WORK_BUDGET, interrupt_after};
use caditor_kernel::interruptible;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    interruptible(interrupt_after(WORK_BUDGET), || {
        let _ = caditor_step::read_step(text);
    });
});
