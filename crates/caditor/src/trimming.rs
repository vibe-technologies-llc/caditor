use caditor_document::{FeatureId, Transaction};
use caditor_geometry::Point2;
use caditor_sketch::{
    Entity, EntityId, ExtendError, Extension, Faceting, Piece, Sketch, TrimError,
};

use crate::{
    drawing::Preview,
    editing::{ActiveSketch, Tool},
    feature_tree::count,
    model::Model,
    sketch_tools,
    snap::{self, Pointer, Screen},
};

const CARRIER_TOLERANCE: f64 = 1e-7;
const NOTHING_TO_TRIM: &str = "The sketch has no line, circle or arc to trim";
const NOTHING_TO_EXTEND: &str = "The sketch has no line or arc to extend";
pub const TRIM_PROMPT: &str = "Click a piece of a curve to trim it away, or drag across pieces";
pub const EXTEND_PROMPT: &str = "Click near the end of a line or arc to extend it";

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Trim(Result<Piece, TrimError>),
    Extend(Result<Extension, ExtendError>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Aim {
    pub curve: EntityId,
    pub near: Point2,
    pub outcome: Outcome,
}

impl Aim {
    fn of(sketch: &Sketch, tool: Tool, curve: EntityId, near: Point2) -> Self {
        let outcome = match tool {
            Tool::Extend => Outcome::Extend(sketch.extension(curve, near)),
            _ => Outcome::Trim(sketch.trim_piece(curve, near)),
        };
        Self {
            curve,
            near,
            outcome,
        }
    }

    fn is_usable(&self) -> bool {
        match &self.outcome {
            Outcome::Trim(piece) => piece.is_ok(),
            Outcome::Extend(extension) => extension.is_ok(),
        }
    }

    pub fn label(&self, sketch: &Sketch) -> String {
        let curve = sketch.entity_label(self.curve);
        match &self.outcome {
            Outcome::Trim(Ok(piece)) if piece.is_whole() => {
                format!("Delete {curve}, which nothing crosses")
            }
            Outcome::Trim(Ok(piece)) => {
                let cutters: Vec<String> = piece
                    .cutters()
                    .into_iter()
                    .map(|cutter| sketch.entity_label(cutter))
                    .collect();
                format!("Trim {curve} back to {}", cutters.join(" and "))
            }
            Outcome::Extend(Ok(extension)) => format!(
                "Extend {curve} to {}",
                sketch.entity_label(extension.target)
            ),
            Outcome::Trim(Err(error)) => capitalized(&error.to_string()),
            Outcome::Extend(Err(error)) => capitalized(&error.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TrimPath {
    last: Option<Point2>,
    crossed: Vec<Piece>,
}

#[derive(Debug, Clone, Default)]
pub struct Trimming {
    context: Option<(FeatureId, Tool)>,
    hover: Option<Aim>,
    highlight: Option<Aim>,
    path: Option<TrimPath>,
}

impl Trimming {
    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    pub fn has_highlight(&self) -> bool {
        self.highlight.is_some()
    }

    pub fn sync(&mut self, active: Option<ActiveSketch>, sketch: Option<&Sketch>) {
        let context = active
            .filter(|active| active.tool.trims())
            .map(|active| (active.feature, active.tool));
        if context != self.context {
            *self = Self {
                context,
                ..Self::default()
            };
        }
        let (Some((_, tool)), Some(sketch)) = (self.context, sketch) else {
            return;
        };
        self.highlight = self
            .highlight
            .take()
            .filter(|aim| sketch.entity(aim.curve).is_some())
            .map(|aim| Aim::of(sketch, tool, aim.curve, aim.near))
            .filter(Aim::is_usable);
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        let Some((_, tool)) = self.context else {
            self.hover = None;
            return;
        };
        if let (Some(path), Some(pointer)) = (&mut self.path, pointer) {
            path.follow(sketch, pointer.sketch);
        }
        self.hover = pointer
            .and_then(|pointer| curve_under(sketch, screen, pointer))
            .map(|(curve, near)| Aim::of(sketch, tool, curve, near));
    }

    pub fn leave(&mut self) {
        self.hover = None;
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn aim(&self) -> Option<&Aim> {
        self.highlight.as_ref().or(self.hover.as_ref())
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), &'static str> {
        let Some((_, tool)) = self.context else {
            return Err(NOTHING_TO_TRIM);
        };
        let offered = sketch.entities().any(|(_, entity)| match entity {
            Entity::Line { .. } | Entity::Arc { .. } => true,
            Entity::Circle { .. } | Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => {
                tool == Tool::Trim
            }
            Entity::Point(_) | Entity::Spline { .. } => false,
        });
        match (offered, tool) {
            (true, _) => Ok(()),
            (false, Tool::Extend) => Err(NOTHING_TO_EXTEND),
            (false, _) => Err(NOTHING_TO_TRIM),
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let Some((_, tool)) = self.context else {
            return;
        };
        let targets = targets(sketch, tool);
        let count = targets.len();
        if count == 0 {
            return;
        }
        let current = self.aim().and_then(|aim| {
            targets.iter().position(|target| {
                target.curve == aim.curve && same_outcome(&target.outcome, &aim.outcome)
            })
        });
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count as isize) as usize,
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = targets.into_iter().nth(next);
    }

    pub fn click(&mut self, model: &Model) -> Result<Option<Transaction>, String> {
        let aim = self.hover.clone();
        self.act(model, aim)
    }

    pub fn activate(&mut self, model: &Model) -> Result<Option<Transaction>, String> {
        let aim = self.aim().cloned();
        self.highlight = None;
        self.act(model, aim)
    }

    pub fn begin_path(&mut self, from: Point2) {
        if matches!(self.context, Some((_, Tool::Trim))) {
            self.path = Some(TrimPath {
                last: Some(from),
                crossed: Vec::new(),
            });
        }
    }

    pub fn cancel_path(&mut self) {
        self.path = None;
    }

    pub fn finish_path(&mut self, model: &Model) -> Result<Option<Transaction>, String> {
        let (Some(path), Some((feature, _))) = (self.path.take(), self.context) else {
            return Ok(None);
        };
        let pieces = path.crossed;
        let label = match pieces.as_slice() {
            [] => return Ok(None),
            [piece] => {
                let sketch = working(model, feature).ok_or_else(sketch_gone)?;
                format!("Trim {}", sketch.entity_label(piece.curve))
            }
            pieces => format!("Trim {}", count(pieces.len(), "piece", "pieces")),
        };
        reshaped(model, feature, label, |sketch| {
            for piece in &pieces {
                let middle = piece.middle();
                if let Some(curve) = carrier(sketch, piece.curve, middle) {
                    sketch
                        .trim(curve, middle)
                        .map_err(|error| refusal(Tool::Trim, &error.to_string()))?;
                }
            }
            Ok(())
        })
        .map(Some)
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        if let Some(path) = &self.path {
            return path.crossed.iter().map(|piece| piece.curve).collect();
        }
        match self.aim().map(|aim| &aim.outcome) {
            Some(Outcome::Trim(Ok(piece))) => piece.cutters(),
            Some(Outcome::Extend(Ok(extension))) => vec![extension.target],
            _ => Vec::new(),
        }
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some(path) = &self.path {
            preview.removed = path
                .crossed
                .iter()
                .map(|piece| piece.faceted(faceting))
                .collect();
            return preview;
        }
        match self.aim().map(|aim| &aim.outcome) {
            Some(Outcome::Trim(Ok(piece))) => {
                preview.removed.push(piece.faceted(faceting));
                preview.points = [piece.start, piece.end]
                    .into_iter()
                    .flatten()
                    .map(|cut| cut.position)
                    .collect();
            }
            Some(Outcome::Extend(Ok(extension))) => {
                preview.curves.push(extension.faceted(faceting));
                preview.snap = Some(extension.to);
            }
            _ => {}
        }
        preview
    }

    pub fn label(&self, sketch: &Sketch) -> Option<String> {
        if let Some(path) = &self.path {
            return (!path.crossed.is_empty())
                .then(|| format!("Trim {}", count(path.crossed.len(), "piece", "pieces")));
        }
        self.aim().map(|aim| aim.label(sketch))
    }

    pub fn prompt(&self) -> Option<&'static str> {
        match self.context? {
            (_, Tool::Trim) => Some(TRIM_PROMPT),
            (_, Tool::Extend) => Some(EXTEND_PROMPT),
            _ => None,
        }
    }

    fn act(&mut self, model: &Model, aim: Option<Aim>) -> Result<Option<Transaction>, String> {
        let (Some((feature, tool)), Some(aim)) = (self.context, aim) else {
            return Ok(None);
        };
        let sketch = working(model, feature).ok_or_else(sketch_gone)?;
        let curve = sketch.entity_label(aim.curve);
        let transaction = match tool {
            Tool::Extend => reshaped(model, feature, format!("Extend {curve}"), |sketch| {
                sketch
                    .extend(aim.curve, aim.near)
                    .map(|_| ())
                    .map_err(|error| refusal(tool, &error.to_string()))
            }),
            _ => reshaped(model, feature, format!("Trim {curve}"), |sketch| {
                sketch
                    .trim(aim.curve, aim.near)
                    .map(|_| ())
                    .map_err(|error| refusal(tool, &error.to_string()))
            }),
        }?;
        self.hover = None;
        Ok(Some(transaction))
    }
}

impl TrimPath {
    fn follow(&mut self, sketch: &Sketch, to: Point2) {
        let Some(from) = self.last.replace(to) else {
            return;
        };
        if from == to {
            return;
        }
        let trimmable: Vec<EntityId> = sketch
            .entities()
            .filter(|(_, entity)| is_trimmable(entity))
            .map(|(id, _)| id)
            .collect();
        let mut crossings: Vec<(f64, EntityId, Point2)> = trimmable
            .into_iter()
            .flat_map(|curve| {
                sketch
                    .segment_crossings(curve, from, to)
                    .into_iter()
                    .map(move |point| (from.distance(point), curve, point))
            })
            .collect();
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, curve, point) in crossings {
            if let Ok(piece) = sketch.trim_piece(curve, point)
                && !self.crossed.contains(&piece)
            {
                self.crossed.push(piece);
            }
        }
    }
}

fn same_outcome(a: &Outcome, b: &Outcome) -> bool {
    match (a, b) {
        (Outcome::Trim(Ok(a)), Outcome::Trim(Ok(b))) => a == b,
        (Outcome::Extend(Ok(a)), Outcome::Extend(Ok(b))) => a.end == b.end,
        _ => false,
    }
}

fn is_trimmable(entity: &Entity) -> bool {
    matches!(
        entity,
        Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Ellipse { .. }
            | Entity::EllipticalArc { .. }
    )
}

fn targets(sketch: &Sketch, tool: Tool) -> Vec<Aim> {
    let curves = sketch.entities().filter(|(_, entity)| is_trimmable(entity));
    match tool {
        Tool::Extend => curves
            .filter(|(_, entity)| {
                matches!(
                    entity,
                    Entity::Line { .. } | Entity::Circle { .. } | Entity::Arc { .. }
                )
            })
            .flat_map(|(curve, entity)| {
                entity
                    .points()
                    .into_iter()
                    .skip(usize::from(matches!(entity, Entity::Arc { .. })))
                    .filter_map(move |end| Some((curve, sketch.point(end)?)))
            })
            .map(|(curve, near)| Aim::of(sketch, tool, curve, near))
            .filter(Aim::is_usable)
            .collect(),
        _ => curves
            .flat_map(|(curve, _)| sketch.trim_pieces(curve).unwrap_or_default())
            .map(|piece| Aim {
                curve: piece.curve,
                near: piece.middle(),
                outcome: Outcome::Trim(Ok(piece)),
            })
            .collect(),
    }
}

pub fn curve_under(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
) -> Option<(EntityId, Point2)> {
    sketch
        .entities()
        .filter(|(_, entity)| !matches!(entity, Entity::Point(_)))
        .filter_map(|(curve, _)| {
            let on = sketch.closest_on_curve(curve, pointer.sketch)?;
            let offset = screen.to_screen(on)?.distance(pointer.screen);
            (offset <= snap::CURVE_TOLERANCE).then_some((offset, curve, on))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, curve, on)| (curve, on))
}

fn carrier(sketch: &Sketch, preferred: EntityId, point: Point2) -> Option<EntityId> {
    let tolerance = CARRIER_TOLERANCE * point.x.abs().max(point.y.abs()).max(1.0);
    let carries = |curve: EntityId| {
        sketch
            .closest_on_curve(curve, point)
            .is_some_and(|on| on.distance(point) <= tolerance)
    };
    if carries(preferred) {
        return Some(preferred);
    }
    sketch
        .entities()
        .filter(|(_, entity)| is_trimmable(entity))
        .map(|(curve, _)| curve)
        .find(|curve| carries(*curve))
}

pub fn working(model: &Model, feature: FeatureId) -> Option<Sketch> {
    let owner = model.document().feature(feature)?;
    let definition = owner.kind.sketch()?;
    let shown = model.displayed_sketch(owner)?;
    let mut working = definition.clone();
    for (id, entity) in shown.entities() {
        let same = definition
            .entity(id)
            .is_some_and(|defined| defined != entity && defined.same_structure(entity));
        if same {
            let _ = working.replace_entity(id, entity.clone());
        }
    }
    Some(working)
}

pub fn reshaped(
    model: &Model,
    feature: FeatureId,
    label: String,
    change: impl FnOnce(&mut Sketch) -> Result<(), String>,
) -> Result<Transaction, String> {
    let before = working(model, feature).ok_or_else(sketch_gone)?;
    let mut after = before.clone();
    change(&mut after)?;
    let mut transaction = sketch_tools::settled_transaction(model, feature, label);
    transaction.reshape_sketch(feature, &before, &after);
    Ok(transaction.finish())
}

pub fn refusal(tool: Tool, reason: &str) -> String {
    format!("{}: {reason}.", tool.label())
}

pub fn sketch_gone() -> String {
    "The sketch being edited no longer exists.".to_owned()
}

pub fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;

    fn hovering(sketch: &Sketch, tool: Tool, curve: EntityId, near: Point2) -> Trimming {
        Trimming {
            hover: Some(Aim::of(sketch, tool, curve, near)),
            ..Trimming::default()
        }
    }

    #[test]
    fn a_trim_against_an_axis_previews_the_piece_removed_and_the_cut_marked() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(-30.0, 20.0), Point2::new(30.0, 20.0));
        let trimming = hovering(&sketch, Tool::Trim, line, Point2::new(-15.0, 20.0));

        let preview = trimming.preview(Faceting::within(0.01));
        let aim = trimming.aim().unwrap();

        assert_eq!(
            aim.label(&sketch),
            format!("Trim {} back to Vertical axis", sketch.entity_label(line))
        );
        assert_eq!(
            preview.removed,
            vec![vec![Point2::new(-30.0, 20.0), Point2::new(0.0, 20.0)]]
        );
        assert_eq!(preview.points, vec![Point2::new(0.0, 20.0)]);
        assert_eq!(
            trimming.highlighted_entities(),
            vec![EntityId::VERTICAL_AXIS]
        );
    }

    #[test]
    fn a_trim_across_a_collinear_overlap_previews_the_overlapped_piece() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(0.0, 20.0), Point2::new(60.0, 20.0));
        let other = sketch.add_line(Point2::new(20.0, 20.0), Point2::new(40.0, 20.0));
        let trimming = hovering(&sketch, Tool::Trim, line, Point2::new(30.0, 20.0));

        let preview = trimming.preview(Faceting::within(0.01));
        let aim = trimming.aim().unwrap();

        assert_eq!(
            aim.label(&sketch),
            format!(
                "Trim {} back to {}",
                sketch.entity_label(line),
                sketch.entity_label(other)
            )
        );
        assert_eq!(
            preview.removed,
            vec![vec![Point2::new(20.0, 20.0), Point2::new(40.0, 20.0)]]
        );
        assert_eq!(
            preview.points,
            vec![Point2::new(20.0, 20.0), Point2::new(40.0, 20.0)]
        );
        assert_eq!(trimming.highlighted_entities(), vec![other]);
    }

    #[test]
    fn an_extension_to_an_axis_is_previewed_and_named() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(10.0, 5.0), Point2::new(20.0, 5.0));
        let trimming = hovering(&sketch, Tool::Extend, line, Point2::new(11.0, 5.0));

        let preview = trimming.preview(Faceting::within(0.01));
        let aim = trimming.aim().unwrap();

        assert_eq!(
            aim.label(&sketch),
            format!("Extend {} to Vertical axis", sketch.entity_label(line))
        );
        assert_eq!(preview.snap, Some(Point2::new(0.0, 5.0)));
        assert_eq!(
            trimming.highlighted_entities(),
            vec![EntityId::VERTICAL_AXIS]
        );
    }
}
