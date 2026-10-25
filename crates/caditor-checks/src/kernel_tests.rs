use std::sync::Arc;

use arbitrary::Unstructured;
use caditor_kernel::{
    BooleanOperation, Interrupt, MeshQuality, SamplingTolerance, Solid, boolean, interruptible,
    mass_properties_tessellating,
};

use crate::{
    invariants::assert_sound,
    profile::{FEATURE, placement, swept_solid},
    seeded::for_each_case,
};

const SWEPT_CASES: u64 = 400;
const BOOLEAN_CASES: u64 = 400;
const MOST_SHAPES: usize = 4;
const SECOND_FEATURE: u64 = 2;
const FEWEST_SOLIDS: usize = 150;
const FEWEST_PAIRS: usize = 8;
const CHORD: f64 = 0.01;
const ANGLE: f64 = 0.1;
const RELATIVE_VOLUME_TOLERANCE: f64 = 1e-6;

fn never() -> Interrupt {
    Arc::new(|| false)
}

fn volume(solid: &Solid) -> Option<f64> {
    let tolerance = SamplingTolerance::new(CHORD, ANGLE)?;
    mass_properties_tessellating(solid, &tolerance)
        .ok()
        .map(|mass| mass.properties.volume)
}

fn combine(first: &Solid, second: &Solid, operation: BooleanOperation) -> Option<Solid> {
    interruptible(never(), || boolean(first, second, operation).ok())
}

fn assert_close(left: f64, right: f64, scale: f64, what: &str) {
    assert!(
        (left - right).abs() <= RELATIVE_VOLUME_TOLERANCE * scale.max(1.0),
        "{what}: {left} against {right}"
    );
}

fn pair(input: &mut Unstructured) -> Option<(Solid, Solid)> {
    let first = swept_solid(input, MOST_SHAPES, FEATURE)?;
    let second = swept_solid(input, MOST_SHAPES, SECOND_FEATURE)?;
    let moved = second.transformed(&placement(input).ok()?).ok()?;
    Some((first, moved))
}

#[test]
fn swept_solids_are_sound() {
    let mut solids = 0;
    for_each_case("swept solids", SWEPT_CASES, |_, input| {
        if let Some(solid) = swept_solid(input, MOST_SHAPES, FEATURE) {
            assert_sound(&solid, &MeshQuality::COARSE);
            solids += 1;
        }
    });
    assert!(
        solids >= FEWEST_SOLIDS,
        "only {solids} solids were swept, so the generators no longer reach enough"
    );
}

#[test]
fn booleans_that_succeed_are_sound_and_obey_the_volume_identities() {
    let mut pairs = 0;
    for_each_case("boolean volumes", BOOLEAN_CASES, |_, input| {
        let Some((first, second)) = pair(input) else {
            return;
        };
        let (Some(first_volume), Some(second_volume)) = (volume(&first), volume(&second)) else {
            return;
        };
        let scale = first_volume.abs() + second_volume.abs();

        let union = combine(&first, &second, BooleanOperation::Union);
        let reversed = combine(&second, &first, BooleanOperation::Union);
        let common = combine(&first, &second, BooleanOperation::Intersection);
        let rest = combine(&first, &second, BooleanOperation::Difference);

        for result in [&union, &reversed, &common, &rest].into_iter().flatten() {
            assert_sound(result, &MeshQuality::COARSE);
        }

        if let (Some(union), Some(common)) = (&union, &common)
            && let (Some(union), Some(common)) = (volume(union), volume(common))
        {
            assert_close(
                union + common,
                first_volume + second_volume,
                scale,
                "union plus intersection against the two volumes",
            );
            pairs += 1;
        }
        if let (Some(common), Some(rest)) = (&common, &rest)
            && let (Some(common), Some(rest)) = (volume(common), volume(rest))
        {
            assert_close(
                common + rest,
                first_volume,
                scale,
                "intersection plus difference against the first volume",
            );
        }
        if let (Some(union), Some(reversed)) = (&union, &reversed)
            && let (Some(union), Some(reversed)) = (volume(union), volume(reversed))
        {
            assert_close(union, reversed, scale, "union against its reverse");
        }
    });
    assert!(
        pairs >= FEWEST_PAIRS,
        "only {pairs} pairs combined, so the generators no longer reach enough"
    );
}
