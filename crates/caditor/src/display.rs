use std::{cell::RefCell, collections::BTreeMap, ops::Deref, sync::Arc};

use caditor_document::{Evaluation, Feature, FeatureId, FeatureResult, FeatureState};
use caditor_geometry::Aabb;
use caditor_sketch::Sketch;

use crate::{bodies::BodyMeshing, drag_solver::SketchDragging};

#[derive(Default)]
pub struct Display {
    pub meshing: BodyMeshing,
    pub sketches: DisplayedSketches,
    pub dragging: SketchDragging,
}

#[derive(Debug, Clone)]
pub enum Displayed<'a> {
    Borrowed(&'a Sketch),
    Shared(Arc<Sketch>),
}

impl Deref for Displayed<'_> {
    type Target = Sketch;

    fn deref(&self) -> &Sketch {
        match self {
            Self::Borrowed(sketch) => sketch,
            Self::Shared(sketch) => sketch,
        }
    }
}

#[derive(Debug, Clone)]
enum Shown {
    Definition,
    Solved,
    Merged(Arc<Sketch>),
}

#[derive(Debug, Clone)]
struct Dragged {
    feature: FeatureId,
    sketch: Arc<Sketch>,
    held_until: Option<u64>,
}

#[derive(Debug, Default)]
pub struct DisplayedSketches {
    dragged: Option<Dragged>,
    shown: RefCell<BTreeMap<FeatureId, Shown>>,
    bounds: RefCell<BTreeMap<FeatureId, Option<Aabb>>>,
    #[cfg(test)]
    merges: std::cell::Cell<usize>,
}

impl DisplayedSketches {
    pub fn forget(&mut self) {
        self.shown.get_mut().clear();
        self.bounds.get_mut().clear();
    }

    pub fn show_dragged(&mut self, feature: FeatureId, sketch: Arc<Sketch>) {
        self.bounds.get_mut().remove(&feature);
        self.dragged = Some(Dragged {
            feature,
            sketch,
            held_until: None,
        });
    }

    pub fn hold_dragged_until(&mut self, revision: u64) {
        if let Some(dragged) = &mut self.dragged {
            dragged.held_until = Some(revision);
        }
    }

    pub fn stop_showing_dragged(&mut self) {
        if let Some(dragged) = self.dragged.take() {
            self.bounds.get_mut().remove(&dragged.feature);
        }
    }

    pub fn evaluated(&mut self, revision: u64, evaluation: &Evaluation) {
        let reached = self.dragged.as_ref().is_some_and(|dragged| {
            dragged.held_until.is_some_and(|held| revision >= held)
                && evaluation
                    .feature(dragged.feature)
                    .is_some_and(|status| status.state != FeatureState::Outdated)
        });
        if reached {
            self.stop_showing_dragged();
        }
    }

    pub fn get<'a>(
        &self,
        evaluation: &'a Evaluation,
        feature: &'a Feature,
    ) -> Option<Displayed<'a>> {
        let definition = feature.kind.sketch()?;
        if let Some(dragged) = self
            .dragged
            .as_ref()
            .filter(|dragged| dragged.feature == feature.id())
        {
            return Some(Displayed::Shared(Arc::clone(&dragged.sketch)));
        }
        let solved = last_good(evaluation, feature);
        let known = self
            .shown
            .try_borrow()
            .ok()
            .and_then(|shown| shown.get(&feature.id()).cloned());
        let shown = match known {
            Some(shown) => shown,
            None => {
                let shown = self.show(definition, solved);
                if let Ok(mut memo) = self.shown.try_borrow_mut() {
                    memo.insert(feature.id(), shown.clone());
                }
                shown
            }
        };
        Some(match (shown, solved) {
            (Shown::Solved, Some(solved)) => Displayed::Borrowed(solved),
            (Shown::Merged(merged), _) => Displayed::Shared(merged),
            (Shown::Solved | Shown::Definition, _) => Displayed::Borrowed(definition),
        })
    }

    pub fn bounds(
        &self,
        evaluation: &Evaluation,
        feature: &Feature,
        measure: impl FnOnce(&Sketch) -> Option<Aabb>,
    ) -> Option<Aabb> {
        let known = self
            .bounds
            .try_borrow()
            .ok()
            .and_then(|bounds| bounds.get(&feature.id()).copied());
        if let Some(known) = known {
            return known;
        }
        let measured = self
            .get(evaluation, feature)
            .and_then(|sketch| measure(&sketch));
        if let Ok(mut memo) = self.bounds.try_borrow_mut() {
            memo.insert(feature.id(), measured);
        }
        measured
    }

    fn show(&self, definition: &Sketch, solved: Option<&Sketch>) -> Shown {
        match solved.map(|solved| merge(definition, solved)) {
            None => Shown::Definition,
            Some(None) => Shown::Solved,
            Some(Some(merged)) => {
                #[cfg(test)]
                self.merges.set(self.merges.get() + 1);
                Shown::Merged(Arc::new(merged))
            }
        }
    }

    #[cfg(test)]
    pub fn merges(&self) -> usize {
        self.merges.get()
    }
}

fn last_good<'a>(evaluation: &'a Evaluation, feature: &Feature) -> Option<&'a Sketch> {
    evaluation
        .feature(feature.id())
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

fn merge(definition: &Sketch, solved: &Sketch) -> Option<Sketch> {
    let same_entities =
        definition.entities().len() == solved.entities().len()
            && definition.entities().zip(solved.entities()).all(
                |((id, defined), (other, settled))| id == other && defined.same_structure(settled),
            );
    let same_construction = definition.construction().eq(solved.construction());
    if same_entities && same_construction && definition.plane() == solved.plane() {
        return None;
    }
    let mut merged = definition.clone();
    merged.set_plane(solved.plane());
    for (id, settled) in solved.entities() {
        let fits = definition
            .entity(id)
            .is_some_and(|defined| defined.same_structure(settled));
        if fits && let Err(error) = merged.replace_entity(id, settled.clone()) {
            log::debug!("showing the drawn position of entity {id}: {error}");
        }
    }
    Some(merged)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use caditor_document::{CancelToken, Document, Edit, FeatureKind, ModelEvaluator, Recompute};
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::{Entity, EntityId};

    use super::*;

    fn sketched() -> (Document, FeatureId, Evaluation) {
        let mut document = Document::default();
        let mut sketch = Sketch::new(Plane::XY);
        sketch.add_line(Point2::ZERO, Point2::new(40.0, 0.0));
        let mut transaction = document.transaction("Add sketch");
        let feature = transaction.add_feature("Sketch 1", FeatureKind::from(sketch));
        document.apply(transaction.finish()).unwrap();
        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        (document, feature, evaluation)
    }

    fn solved(evaluation: &Evaluation, feature: FeatureId) -> &Sketch {
        &evaluation
            .feature(feature)
            .unwrap()
            .result
            .as_deref()
            .unwrap()
            .sketch()
            .unwrap()
            .geometry
    }

    #[test]
    fn a_solved_sketch_is_shown_as_it_was_solved_without_a_copy() {
        let (document, feature, evaluation) = sketched();
        let sketches = DisplayedSketches::default();
        let owner = document.feature(feature).unwrap();

        let shown = sketches.get(&evaluation, owner).unwrap();

        assert!(
            matches!(shown, Displayed::Borrowed(sketch) if std::ptr::eq(sketch, solved(&evaluation, feature)))
        );
        assert_eq!(sketches.merges(), 0);
    }

    #[test]
    fn a_sketch_changed_since_it_was_solved_is_merged_once_until_forgotten() {
        let (mut document, feature, evaluation) = sketched();
        document
            .apply(caditor_document::Transaction::single(
                "Add a point",
                Edit::AddSketchEntity {
                    feature,
                    id: EntityId::from_raw(100),
                    entity: Entity::Point(Point2::new(5.0, 5.0)),
                    construction: false,
                },
            ))
            .unwrap();
        let mut sketches = DisplayedSketches::default();
        let owner = document.feature(feature).unwrap();

        let first = sketches.get(&evaluation, owner).unwrap();
        let second = sketches.get(&evaluation, owner).unwrap();
        let (Displayed::Shared(first), Displayed::Shared(second)) = (first, second) else {
            panic!("the changed sketch is merged");
        };

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(sketches.merges(), 1);
        assert_eq!(
            first.entities().len(),
            solved(&evaluation, feature).entities().len() + 1
        );

        sketches.forget();
        sketches.get(&evaluation, owner).unwrap();
        assert_eq!(sketches.merges(), 2);
    }

    #[test]
    fn a_curve_made_construction_since_the_solve_shows_as_construction() {
        let (mut document, feature, evaluation) = sketched();
        let line = solved(&evaluation, feature)
            .entities()
            .find(|(_, entity)| matches!(entity, Entity::Line { .. }))
            .map(|(id, _)| id)
            .unwrap();
        document
            .apply(caditor_document::Transaction::single(
                "Make construction",
                Edit::SetSketchConstruction {
                    feature,
                    id: line,
                    construction: true,
                },
            ))
            .unwrap();
        let sketches = DisplayedSketches::default();
        let owner = document.feature(feature).unwrap();

        let shown = sketches.get(&evaluation, owner).unwrap();

        assert!(shown.is_construction(line));
        assert_eq!(sketches.merges(), 1);
    }

    #[test]
    fn a_sketch_is_measured_once_until_forgotten() {
        let (document, feature, evaluation) = sketched();
        let mut sketches = DisplayedSketches::default();
        let owner = document.feature(feature).unwrap();
        let measured = Cell::new(0);
        let measure = |sketch: &Sketch| {
            measured.set(measured.get() + 1);
            Some(Aabb::from_point(sketch.plane().origin()))
        };

        let first = sketches.bounds(&evaluation, owner, measure);
        let again = sketches.bounds(&evaluation, owner, measure);
        assert_eq!(first, again);
        assert_eq!(measured.get(), 1);

        sketches.forget();
        sketches.bounds(&evaluation, owner, measure);
        assert_eq!(measured.get(), 2);
    }
}
