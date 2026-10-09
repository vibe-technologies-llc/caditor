use caditor_geometry::Vector2;
use caditor_render::View;

use crate::{
    scene::BuiltScene,
    selection::{Pickable, Selection, SelectionFilter},
};

pub const BRUSH_STEP_POINTS: f64 = 4.0;
pub const MAX_BRUSH_SAMPLES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Painting {
    pub before: Selection,
    last: Vector2,
    started: bool,
}

impl Painting {
    pub fn new(before: Selection, at: Vector2) -> Self {
        Self {
            before,
            last: at,
            started: false,
        }
    }

    pub fn stroke_to(&mut self, cursor: Vector2, pixels_per_point: f64) -> Vec<Vector2> {
        let samples = if self.started {
            brush_samples(self.last, cursor, BRUSH_STEP_POINTS * pixels_per_point)
        } else {
            let mut first = vec![self.last];
            first.extend(brush_samples(
                self.last,
                cursor,
                BRUSH_STEP_POINTS * pixels_per_point,
            ));
            first
        };
        self.started = true;
        self.last = cursor;
        samples
    }
}

pub fn takes_faces(filter: SelectionFilter) -> bool {
    matches!(
        filter,
        SelectionFilter::Everything | SelectionFilter::Faces | SelectionFilter::Bodies
    )
}

pub fn brush_samples(from: Vector2, to: Vector2, step: f64) -> Vec<Vector2> {
    let length = (to - from).length();
    if !length.is_finite() || length == 0.0 || step.is_nan() || step <= 0.0 {
        return Vec::new();
    }
    let count = ((length / step).ceil() as usize).clamp(1, MAX_BRUSH_SAMPLES);
    (1..=count)
        .map(|index| from.lerp(to, index as f64 / count as f64))
        .collect()
}

pub fn faces_under(
    built: &BuiltScene,
    view: &View,
    at: Vector2,
    pixels_per_point: f32,
    through: bool,
) -> Vec<Pickable> {
    let hits = built.scene.hits_through(view, at, pixels_per_point);
    let faces = built.picks.listed(&hits, SelectionFilter::Faces);
    match through {
        true => faces,
        false => faces.into_iter().take(1).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stroke_is_sampled_every_few_points_and_never_more_than_the_limit() {
        let from = Vector2::new(0.0, 0.0);

        let short = brush_samples(from, Vector2::new(10.0, 0.0), 4.0);
        let long = brush_samples(from, Vector2::new(1e6, 0.0), 4.0);
        let still = brush_samples(from, from, 4.0);

        assert_eq!(short.len(), 3);
        assert_eq!(short.last(), Some(&Vector2::new(10.0, 0.0)));
        assert_eq!(long.len(), MAX_BRUSH_SAMPLES);
        assert!(still.is_empty());
    }

    #[test]
    fn the_first_stroke_starts_where_the_press_was() {
        let mut painting = Painting::new(Selection::default(), Vector2::new(5.0, 5.0));

        let first = painting.stroke_to(Vector2::new(5.0, 5.0), 1.0);
        let next = painting.stroke_to(Vector2::new(13.0, 5.0), 2.0);

        assert_eq!(first, vec![Vector2::new(5.0, 5.0)]);
        assert_eq!(next, vec![Vector2::new(13.0, 5.0)]);
    }
}
