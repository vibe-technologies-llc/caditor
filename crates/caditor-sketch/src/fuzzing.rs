use std::collections::BTreeMap;

use crate::{id::EntityId, sketch::Sketch};

pub fn kept_uses(sketch: &Sketch) -> &BTreeMap<EntityId, usize> {
    sketch.kept_uses()
}

pub fn uses_from_scratch(sketch: &Sketch) -> BTreeMap<EntityId, usize> {
    sketch.counted_from_scratch()
}
