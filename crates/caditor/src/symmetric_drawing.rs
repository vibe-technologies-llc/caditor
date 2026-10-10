use caditor_document::{Edit, FeatureId, FeatureKind, Transaction};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::{drawing::Preview, model::Model};

pub const NO_LINE_SELECTED: &str =
    "Select the one line or axis to draw about first, then turn on Draw symmetrically";
pub const SYMMETRICALLY: &str = "symmetrically";
pub const TURNED_OFF: &str = "Drawing symmetrically off: shapes are drawn once.";

pub fn turned_on(line: &str) -> String {
    format!("Drawing symmetrically about {line}: each shape drawn also gets its mirror image.")
}

pub fn prompt(text: &str, line: &str) -> String {
    format!("{text}, mirrored about {line}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symmetry {
    pub feature: FeatureId,
    pub line: EntityId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    pub line: EntityId,
    pub label: String,
    anchor: Point2,
    direction: Vector2,
}

impl Mirror {
    pub fn of(sketch: &Sketch, line: EntityId) -> Option<Self> {
        let (anchor, direction) = if line == EntityId::HORIZONTAL_AXIS {
            (Point2::ZERO, Vector2::X)
        } else if line == EntityId::VERTICAL_AXIS {
            (Point2::ZERO, Vector2::Y)
        } else {
            let Some(Entity::Line { .. }) = sketch.entity(line) else {
                return None;
            };
            let (start, end) = sketch.line_endpoints(line)?;
            (start, (end - start).try_normalize()?)
        };
        Some(Self {
            line,
            label: sketch.entity_label(line),
            anchor,
            direction,
        })
    }

    fn reflect(&self, point: Point2) -> Point2 {
        let offset = point - self.anchor;
        self.anchor + self.direction * (offset.dot(self.direction) * 2.0) - offset
    }

    pub fn add_image(&self, preview: &mut Preview) {
        let curves: Vec<Vec<Point2>> = preview
            .curves
            .iter()
            .map(|curve| curve.iter().map(|point| self.reflect(*point)).collect())
            .collect();
        let points: Vec<Point2> = preview
            .points
            .iter()
            .map(|point| self.reflect(*point))
            .collect();
        preview.curves.extend(curves);
        preview.points.extend(points);
    }
}

pub fn is_line(sketch: &Sketch, item: EntityId) -> bool {
    item == EntityId::HORIZONTAL_AXIS
        || item == EntityId::VERTICAL_AXIS
        || matches!(sketch.entity(item), Some(Entity::Line { .. }))
}

pub fn chosen_line(sketch: &Sketch, selected: &[EntityId]) -> Result<EntityId, &'static str> {
    match selected {
        [only] if is_line(sketch, *only) => Ok(*only),
        _ => Err(NO_LINE_SELECTED),
    }
}

pub fn mirrored(
    model: &Model,
    feature: FeatureId,
    transaction: Transaction,
    about: EntityId,
) -> Transaction {
    let Some(edits) = image_edits(model, feature, &transaction, about) else {
        return transaction;
    };
    let (label, mut all) = transaction.into_parts();
    all.extend(edits);
    Transaction::new(format!("{label} {SYMMETRICALLY}"), all)
}

fn image_edits(
    model: &Model,
    feature: FeatureId,
    transaction: &Transaction,
    about: EntityId,
) -> Option<Vec<Edit>> {
    let added: Vec<EntityId> = transaction
        .edits()
        .iter()
        .filter_map(|edit| match edit {
            Edit::AddSketchEntity {
                feature: owner, id, ..
            } if *owner == feature => Some(*id),
            _ => None,
        })
        .collect();
    if added.is_empty() {
        return None;
    }
    let mut document = model.document().clone();
    if let Err(error) = document.apply(transaction.clone()) {
        log::debug!("the drawn shape cannot be mirrored: {error}");
        return None;
    }
    let FeatureKind::Sketch(definition) = &document.feature(feature)?.kind else {
        return None;
    };
    let drawn = &definition.sketch;
    let items: Vec<EntityId> = added
        .into_iter()
        .filter(|item| !drawn.is_projected(*item))
        .collect();
    let mut image = drawn.clone();
    if let Err(error) = image.mirror(&items, about) {
        log::debug!("the drawn shape is not mirrored: {error}");
        return None;
    }
    let entities = image
        .entities()
        .filter(|(id, _)| drawn.entity(*id).is_none())
        .map(|(id, entity)| Edit::AddSketchEntity {
            feature,
            id,
            entity: entity.clone(),
            construction: image.is_construction(id),
        });
    let constraints = image
        .constraints()
        .filter(|(id, _)| drawn.constraint(*id).is_none())
        .map(|(id, constraint)| Edit::AddSketchConstraint {
            feature,
            id,
            constraint: constraint.clone(),
            inactive: false,
            label: None,
        });
    let edits: Vec<Edit> = entities.chain(constraints).collect();
    (!edits.is_empty()).then_some(edits)
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;

    #[test]
    fn the_preview_gains_its_image_about_a_slanted_line_and_only_lines_are_chosen() {
        let mut sketch = Sketch::new(Plane::XY);
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
        let circle = sketch.add_circle(Point2::new(5.0, 0.0), 2.0);
        let mirror = Mirror::of(&sketch, slanted).unwrap();
        let mut preview = Preview {
            curves: vec![vec![Point2::new(4.0, 0.0), Point2::new(6.0, 1.0)]],
            points: vec![Point2::new(3.0, 0.0)],
            ..Preview::default()
        };

        mirror.add_image(&mut preview);

        assert_eq!(preview.curves.len(), 2);
        assert!(preview.curves[1][0].distance(Point2::new(0.0, 4.0)) < 1e-12);
        assert!(preview.curves[1][1].distance(Point2::new(1.0, 6.0)) < 1e-12);
        assert!(preview.points[1].distance(Point2::new(0.0, 3.0)) < 1e-12);
        assert!(Mirror::of(&sketch, circle).is_none());
        assert_eq!(
            chosen_line(&sketch, &[EntityId::VERTICAL_AXIS]),
            Ok(EntityId::VERTICAL_AXIS)
        );
        assert_eq!(chosen_line(&sketch, &[circle]), Err(NO_LINE_SELECTED));
        assert_eq!(
            chosen_line(&sketch, &[slanted, circle]),
            Err(NO_LINE_SELECTED)
        );
    }
}
