use std::{
    process,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use caditor_geometry::Vector2;
use caditor_sketch::SketchClip;

use crate::{feature_tree::count, model::Model};

const PASTE_SHIFT_FRACTION: f64 = 0.25;
const PASTE_SHIFT_FLOOR: f64 = 1.0;

static PROCESS: OnceLock<String> = OnceLock::new();

pub fn source(model: &Model) -> String {
    let process = PROCESS.get_or_init(|| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        format!("{:x}.{started:x}", process::id())
    });
    format!("{process}.{}", model.session())
}

pub fn same_sketch_shift(clip: &SketchClip) -> Vector2 {
    Vector2::splat(clip.size().max(PASTE_SHIFT_FLOOR) * PASTE_SHIFT_FRACTION)
}

pub fn described(clip: &SketchClip) -> String {
    let parts: Vec<String> = [
        (clip.curve_count(), "curve", "curves"),
        (clip.lone_point_count(), "point", "points"),
        (clip.constraint_count(), "constraint", "constraints"),
    ]
    .into_iter()
    .filter(|(amount, _, _)| *amount > 0)
    .map(|(amount, singular, plural)| count(amount, singular, plural))
    .collect();
    match parts.as_slice() {
        [] => "nothing".to_owned(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
