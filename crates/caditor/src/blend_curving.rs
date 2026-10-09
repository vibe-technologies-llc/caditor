use caditor_document::{FeatureId, Transaction};
use caditor_sketch::{
    BlendCurve, BlendEnd, BlendError, Continuity, Entity, EntityId, Faceting, Sketch,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    model::Model,
    modifying::{Hint, Outcome, Prompt},
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
};

pub const TRANSACTION: &str = "Draw blend curve";
pub const PROMPT: &str = "Click near the end of a line, arc or spline to blend from";
pub const SECOND_PROMPT: &str = "Click near the end of another curve to blend to";
const PICK_KEYS: &str = "Esc: back to Select";
const NOTHING_HIGHLIGHTED: &str =
    "Highlight the end of a line, arc or spline first, with Highlight the next item in the view";
const NO_ENDS: &str = "The sketch has no line, arc or spline whose end could be blended";

#[derive(Debug, Clone, Default)]
pub struct BlendCurving {
    continuity: Continuity,
    first: Option<BlendEnd>,
    hover: Option<BlendEnd>,
    highlight: Option<BlendEnd>,
    blend: Option<(BlendEnd, BlendEnd, Result<BlendCurve, BlendError>)>,
}

impl BlendCurving {
    pub fn starting(sketch: &Sketch, selected: &[EntityId], continuity: Continuity) -> Self {
        Self {
            continuity,
            first: selected
                .iter()
                .find_map(|point| only_end_at(sketch, *point)),
            ..Self::default()
        }
    }

    pub fn sync(&mut self, sketch: &Sketch, continuity: Continuity) {
        if continuity != self.continuity {
            self.continuity = continuity;
            self.blend = None;
        }
        self.first = self.first.filter(|end| is_end(sketch, *end));
        self.highlight = self.highlight.filter(|end| is_end(sketch, *end));
        self.refresh(sketch);
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        self.hover = pointer.and_then(|pointer| end_under(sketch, screen, pointer));
        self.refresh(sketch);
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.highlight.is_none() {
            self.blend = None;
        }
    }

    fn aim(&self) -> Option<BlendEnd> {
        self.highlight
            .or(self.hover)
            .filter(|end| Some(*end) != self.first)
    }

    fn refresh(&mut self, sketch: &Sketch) {
        let pair = self.first.zip(self.aim());
        let fresh = matches!(
            (&self.blend, pair),
            (Some((first, second, _)), Some((from, to))) if *first == from && *second == to
        );
        if fresh {
            return;
        }
        self.blend = pair.map(|(first, second)| {
            let found = sketch.blend_curve(first, second, self.continuity);
            (first, second, found)
        });
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some((_, _, Ok(curve))) = &self.blend {
            preview.curves.push(curve.faceted(faceting));
            preview.points = curve.control_points.clone();
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.first
            .into_iter()
            .chain(self.aim())
            .flat_map(|end| [end.curve, end.point])
            .collect()
    }

    pub fn label(&self, sketch: &Sketch) -> Option<String> {
        if let Some((first, second, found)) = &self.blend {
            return Some(match found {
                Ok(_) => format!(
                    "Blend from {} to {}, {}",
                    end_name(sketch, *first),
                    end_name(sketch, *second),
                    continuity_words(self.continuity)
                ),
                Err(error) => capitalized(&error.to_string()),
            });
        }
        self.aim()
            .filter(|_| self.first.is_none())
            .map(|end| capitalized(&format!("blend from {}", end_name(sketch, end))))
    }

    pub fn prompt(&self) -> Prompt {
        match self.first {
            Some(_) => Prompt {
                text: SECOND_PROMPT,
                hint: Hint::Targets,
            },
            None => Prompt {
                text: PROMPT,
                hint: if self.highlight.is_some() {
                    Hint::Targets
                } else {
                    Hint::Keys(PICK_KEYS)
                },
            },
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        match self.hover {
            Some(end) => self.pick(model, feature, end),
            None => Outcome::Nothing,
        }
    }

    pub fn activate(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        match self.highlight.take() {
            Some(end) => self.pick(model, feature, end),
            None => Outcome::Nothing,
        }
    }

    fn pick(&mut self, model: &Model, feature: FeatureId, end: BlendEnd) -> Outcome {
        match self.first {
            Some(first) if first == end => {
                self.first = None;
                self.blend = None;
                Outcome::Nothing
            }
            Some(first) => self.draw(model, feature, first, end).into(),
            None => {
                self.first = Some(end);
                self.blend = None;
                Outcome::Nothing
            }
        }
    }

    fn draw(
        &mut self,
        model: &Model,
        feature: FeatureId,
        first: BlendEnd,
        second: BlendEnd,
    ) -> Result<Transaction, String> {
        let continuity = self.continuity;
        let drawn = trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
            sketch
                .blend(first, second, continuity)
                .map(|_| ())
                .map_err(|error| trimming::refusal(Tool::BlendCurve, &error.to_string()))
        })?;
        self.first = None;
        self.highlight = None;
        self.blend = None;
        Ok(drawn)
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), &'static str> {
        if ends(sketch).is_empty() {
            Err(NO_ENDS)
        } else {
            Ok(())
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let ends: Vec<BlendEnd> = ends(sketch)
            .into_iter()
            .filter(|end| Some(*end) != self.first)
            .collect();
        let count = ends.len() as isize;
        if count == 0 {
            return;
        }
        let current = self
            .highlight
            .and_then(|aim| ends.iter().position(|end| *end == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| ends.get(next).copied());
        self.refresh(sketch);
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NOTHING_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || self.first.is_some()
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() {
            self.first = None;
        }
        self.blend = None;
    }
}

fn continuity_words(continuity: Continuity) -> &'static str {
    match continuity {
        Continuity::Tangent => "tangent to both (G1)",
        Continuity::Curvature => "matching their tangent and curvature (G2)",
    }
}

fn end_name(sketch: &Sketch, end: BlendEnd) -> String {
    let which = match sketch.ends_of_curve(end.curve) {
        Some([start, _]) if start == end.point => "start",
        _ => "end",
    };
    format!("the {which} of {}", sketch.entity_label(end.curve))
}

fn is_blendable(sketch: &Sketch, curve: EntityId) -> bool {
    !curve.is_reference()
        && matches!(
            sketch.entity(curve),
            Some(Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. })
        )
}

fn is_end(sketch: &Sketch, end: BlendEnd) -> bool {
    is_blendable(sketch, end.curve)
        && sketch
            .ends_of_curve(end.curve)
            .is_some_and(|ends| ends.contains(&end.point))
}

fn ends(sketch: &Sketch) -> Vec<BlendEnd> {
    sketch
        .entities()
        .map(|(curve, _)| curve)
        .filter(|curve| is_blendable(sketch, *curve))
        .filter_map(|curve| Some((curve, sketch.ends_of_curve(curve)?)))
        .flat_map(|(curve, points)| points.map(|point| BlendEnd { curve, point }))
        .collect()
}

fn only_end_at(sketch: &Sketch, point: EntityId) -> Option<BlendEnd> {
    let mut found = ends(sketch).into_iter().filter(|end| end.point == point);
    match (found.next(), found.next()) {
        (Some(end), None) => Some(end),
        _ => None,
    }
}

fn end_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<BlendEnd> {
    let at = pointer.sketch;
    let (_, curve) = sketch
        .entities()
        .map(|(curve, _)| curve)
        .filter(|curve| is_blendable(sketch, *curve))
        .filter_map(|curve| {
            let on = sketch.closest_on_curve(curve, at)?;
            let offset = screen.to_screen(on)?.distance(pointer.screen);
            (offset <= snap::CURVE_TOLERANCE).then_some((offset, curve))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let [start, end] = sketch.ends_of_curve(curve)?;
    let distance = |point: EntityId| {
        sketch
            .point(point)
            .map_or(f64::INFINITY, |p| p.distance(at))
    };
    let point = if distance(start) <= distance(end) {
        start
    } else {
        end
    };
    Some(BlendEnd { curve, point })
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2, Vector2};

    use super::*;

    struct Flat;

    impl Screen for Flat {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(Vector2::new(point.x, point.y))
        }
    }

    fn pointer(x: f64, y: f64) -> Option<Pointer> {
        Some(Pointer {
            screen: Vector2::new(x, y),
            sketch: Point2::new(x, y),
        })
    }

    #[test]
    fn the_end_nearest_the_pointer_is_aimed_at_and_a_second_end_previews_the_blend() {
        let mut sketch = Sketch::new(Plane::XY);
        let left = sketch.add_line(Point2::new(-100.0, 0.0), Point2::new(-20.0, 0.0));
        sketch.add_line(Point2::new(20.0, 40.0), Point2::new(20.0, 120.0));
        let mut curving = BlendCurving::default();

        curving.hover(&sketch, &Flat, pointer(-30.0, 1.0));
        let [_, left_end] = sketch.ends_of_curve(left).unwrap();
        assert_eq!(
            curving.hover,
            Some(BlendEnd {
                curve: left,
                point: left_end
            })
        );
        assert_eq!(
            curving.label(&sketch).as_deref(),
            Some("Blend from the end of Line 2")
        );

        curving.first = curving.hover;
        curving.hover(&sketch, &Flat, pointer(21.0, 50.0));
        let preview = curving.preview(Faceting::within(0.1));

        assert_eq!(preview.points.len(), 4);
        assert!(!preview.curves.is_empty());
        assert_eq!(
            curving.label(&sketch).as_deref(),
            Some("Blend from the end of Line 2 to the start of Line 5, tangent to both (G1)")
        );
        assert_eq!(curving.highlighted_entities().len(), 4);

        curving.sync(&sketch, Continuity::Curvature);
        assert_eq!(curving.preview(Faceting::within(0.1)).points.len(), 6);
    }

    #[test]
    fn stepping_goes_through_every_curve_end_but_the_one_chosen() {
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        sketch.add_circle(Point2::new(30.0, 0.0), 5.0);
        sketch.add_arc(
            Point2::new(0.0, 30.0),
            Point2::new(10.0, 30.0),
            Point2::new(0.0, 40.0),
        );
        let mut curving = BlendCurving::default();

        assert!(curving.steppable(&sketch).is_ok());
        curving.step(&sketch, 1);
        let first = curving.highlight;
        curving.first = first;
        curving.highlight = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            curving.step(&sketch, 1);
            seen.push(curving.highlight.unwrap());
        }

        assert_eq!(ends(&sketch).len(), 4);
        assert!(!seen.contains(&first.unwrap()));
        assert_eq!(seen.first(), seen.last());
        assert!(
            BlendCurving::default()
                .steppable(&Sketch::new(Plane::XY))
                .is_err()
        );
    }
}
