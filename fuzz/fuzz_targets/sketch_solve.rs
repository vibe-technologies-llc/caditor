#![no_main]

use caditor_fuzz::{
    WORK_BUDGET, interrupt_after,
    sketch::{drag, no_parameters, sketch},
};
use caditor_geometry::Plane;
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    let Ok(sketch) = sketch(&mut input, Plane::XY) else {
        return;
    };
    let interrupt = interrupt_after(WORK_BUDGET);
    let cancelled = || interrupt();
    if let Ok(solved) = sketch.solve(&no_parameters, &cancelled) {
        let _ = solved
            .geometry
            .solve_from(&no_parameters, &cancelled, &[], Some(&solved.memo));
    }
    if let Ok(Some(drag)) = drag(&mut input, &sketch) {
        let _ = sketch.solve_dragging(&no_parameters, &cancelled, &[drag]);
    }
});
