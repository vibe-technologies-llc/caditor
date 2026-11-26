use std::collections::BTreeMap;

use crate::{id::EntityId, sketch::Sketch, solve::tally};

pub fn kept_uses(sketch: &Sketch) -> &BTreeMap<EntityId, usize> {
    sketch.kept_uses()
}

pub fn uses_from_scratch(sketch: &Sketch) -> BTreeMap<EntityId, usize> {
    sketch.counted_from_scratch()
}

pub fn measure_work<T>(work: impl FnOnce() -> T) -> (T, usize) {
    tally::measure(work)
}
