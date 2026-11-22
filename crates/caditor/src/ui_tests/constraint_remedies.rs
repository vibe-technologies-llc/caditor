use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, ConstraintId, Entity, Sketch};
use egui::{Key, Modifiers};

use super::{Harness, constraints_of_kind, line_ends};
use crate::{
    constraint_remedies::{RESOLVE, Remedy},
    model::NoticeAction,
    selection::Pickable,
};

fn ids_of_kind(sketch: &Sketch, kind: &str) -> Vec<ConstraintId> {
    sketch
        .constraints()
        .filter(|(_, constraint)| constraint.kind_name() == kind)
        .map(|(id, _)| id)
        .collect()
}

fn fixed_slanted_line(harness: &mut Harness) -> (FeatureId, Vec<ConstraintId>) {
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));
    let (start, end) = line_ends(&sketch, line);

    let fixes = [(start, Point2::ZERO), (end, Point2::new(30.0, 10.0))]
        .into_iter()
        .map(|(point, at)| {
            sketch
                .add_constraint(Constraint::Fix { point, at })
                .unwrap()
        })
        .collect();
    let feature = harness.add_sketch(sketch);

    harness.edit(feature);
    harness.settle();
    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.click_button("Horizontal");
    harness.settle();
    (feature, fixes)
}

fn offered_remedies(harness: &Harness, feature: FeatureId) -> Vec<ConstraintId> {
    let notice = harness.model.notice().expect("the refusal is a notice");
    let Some(NoticeAction::Remedies(remedies)) = &notice.action else {
        panic!("the refusal offers no remedies: {notice:?}");
    };
    remedies.present(harness.sketch(feature)).collect()
}

fn take(harness: &mut Harness, feature: FeatureId, constraint: ConstraintId, remedy: Remedy) {
    let name = harness.sketch(feature).describe_constraint(constraint);

    harness.click_button(RESOLVE);
    harness.click_button(&remedy.label(&name));
    harness.settle();
}

#[test]
fn a_refused_constraint_offers_to_select_or_delete_each_constraint_it_conflicts_with() {
    let mut harness = Harness::new();

    let (feature, fixes) = fixed_slanted_line(&mut harness);
    let offered = offered_remedies(&harness, feature);
    let fix = *offered.first().unwrap();
    let revision = harness.model.revision();

    assert!(!offered.is_empty());
    assert!(offered.iter().all(|constraint| fixes.contains(constraint)));
    assert!(constraints_of_kind(harness.sketch(feature), "Horizontal").is_empty());

    take(&mut harness, feature, fix, Remedy::Select);

    assert_eq!(harness.model.revision(), revision);
    assert_eq!(harness.editing(), Some(feature));
    assert_eq!(
        harness
            .workspace
            .viewport
            .selection()
            .iter()
            .collect::<Vec<_>>(),
        [Pickable::SketchConstraint {
            feature,
            constraint: fix,
        }]
    );

    take(&mut harness, feature, fix, Remedy::Delete);

    assert!(harness.sketch(feature).constraint(fix).is_none());
    assert_eq!(harness.sketch(feature).constraints().len(), fixes.len() - 1);
    assert!(harness.model.notice().is_none());

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert!(harness.sketch(feature).constraint(fix).is_some());
    assert_eq!(harness.sketch(feature).constraints().len(), fixes.len());
}

#[test]
fn a_refused_constraint_can_turn_off_what_it_conflicts_with_and_then_be_added() {
    let mut harness = Harness::new();

    let (feature, _) = fixed_slanted_line(&mut harness);

    let fix = *offered_remedies(&harness, feature).first().unwrap();

    take(&mut harness, feature, fix, Remedy::Disable);

    assert!(!harness.sketch(feature).is_active(fix));
    assert!(harness.model.notice().is_none());

    let line = harness
        .sketch(feature)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();

    harness.select([Pickable::SketchEntity {
        feature,
        entity: line,
    }]);
    harness.click_button("Horizontal");
    harness.settle();

    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
}

#[test]
fn deleting_a_conflicting_constraint_from_the_sketch_error_lets_the_sketch_solve() {
    let mut harness = Harness::new();

    let base = harness.document().features().next().unwrap().id();

    harness.edit(base);

    let line = harness
        .sketch(base)
        .entities()
        .find_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id))
        .unwrap();

    harness.add_stored_constraint(base, Constraint::Vertical(line));

    assert!(harness.shows("Conflicting constraints"));

    let vertical = *ids_of_kind(harness.sketch(base), "Vertical")
        .first()
        .unwrap();

    take(&mut harness, base, vertical, Remedy::Select);

    assert!(
        harness
            .workspace
            .viewport
            .selection()
            .contains(Pickable::SketchConstraint {
                feature: base,
                constraint: vertical,
            })
    );

    take(&mut harness, base, vertical, Remedy::Delete);

    assert!(harness.sketch(base).constraint(vertical).is_none());
    assert!(!harness.shows("Conflicting constraints"));
    assert!(harness.shows("2 degrees of freedom left"));

    harness.key(Key::Z, Modifiers::COMMAND);
    harness.frame();
    harness.settle();

    assert!(harness.shows("Conflicting constraints"));
}

#[test]
fn a_redundant_constraint_offers_to_delete_the_one_it_repeats() {
    let mut harness = Harness::new();
    let mut sketch = Sketch::new(Plane::XY);

    let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(30.0, 10.0));

    sketch.add_constraint(Constraint::Horizontal(line)).unwrap();

    let feature = harness.add_sketch(sketch);

    harness.edit(feature);
    harness.add_stored_constraint(feature, Constraint::Horizontal(line));

    assert!(harness.shows("1 redundant constraint"));

    let repeated = ids_of_kind(harness.sketch(feature), "Horizontal");
    let first = *repeated.first().unwrap();

    take(&mut harness, feature, first, Remedy::Delete);

    assert!(harness.sketch(feature).constraint(first).is_none());
    assert_eq!(
        constraints_of_kind(harness.sketch(feature), "Horizontal").len(),
        1
    );
    assert!(!harness.shows("1 redundant constraint"));
}
