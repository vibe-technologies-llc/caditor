#![no_main]

use caditor_fuzz::{
    WORK_BUDGET, check_solid, interrupt_after,
    profile::{placement, swept_solid},
};
use caditor_kernel::{BooleanOperation, boolean, interruptible};
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

const MOST_SHAPES: usize = 3;
const FIRST: u64 = 1;
const SECOND: u64 = 2;
const OPERATIONS: [BooleanOperation; 3] = [
    BooleanOperation::Union,
    BooleanOperation::Difference,
    BooleanOperation::Intersection,
];

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    interruptible(interrupt_after(WORK_BUDGET), || {
        let Some(first) = swept_solid(&mut input, MOST_SHAPES, FIRST) else {
            return;
        };
        let Some(second) = swept_solid(&mut input, MOST_SHAPES, SECOND) else {
            return;
        };
        let Ok(placement) = placement(&mut input) else {
            return;
        };
        let Ok(second) = second.transformed(&placement) else {
            return;
        };
        let Ok(operation) = input.choose(&OPERATIONS) else {
            return;
        };
        if let Ok(result) = boolean(&first, &second, *operation) {
            check_solid(&result);
        }
    });
});
