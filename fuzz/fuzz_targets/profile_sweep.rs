#![no_main]

use caditor_fuzz::{
    WORK_BUDGET, check_solid, interrupt_after,
    profile::{FEATURE, profile_curves, selection, sweep},
};
use caditor_kernel::{Profile, SamplingTolerance, interruptible};
use libfuzzer_sys::{arbitrary::Unstructured, fuzz_target};

const MOST_SHAPES: usize = 6;
const CHORD: f64 = 0.05;
const ANGLE: f64 = 0.3;

fuzz_target!(|bytes: &[u8]| {
    let mut input = Unstructured::new(bytes);
    interruptible(interrupt_after(WORK_BUDGET), || {
        let Ok(curves) = profile_curves(&mut input, MOST_SHAPES) else {
            return;
        };
        let Ok(profile) = Profile::new(&curves) else {
            return;
        };
        if let Some(tolerance) = SamplingTolerance::new(CHORD, ANGLE) {
            for region in profile.regions() {
                let _ = region.triangulate(&tolerance);
            }
        }
        let Ok(chosen) = selection(&mut input, &profile) else {
            return;
        };
        let Ok(regions) = profile.select(&chosen) else {
            return;
        };
        if let Some(solid) = sweep(&mut input, &regions, FEATURE) {
            check_solid(&solid);
        }
    });
});
