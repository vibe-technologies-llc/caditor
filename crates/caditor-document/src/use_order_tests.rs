use std::sync::Arc;

use caditor_expression::Expression;
use caditor_geometry::Plane;
use caditor_sketch::Sketch;

use crate::*;

fn sketch(raw: u64, name: &str) -> Arc<Feature> {
    Arc::new(Feature::new(
        FeatureId::from_raw(raw),
        name.to_owned(),
        FeatureKind::from(Sketch::new(Plane::XY)),
    ))
}

fn extrusion(raw: u64, name: &str, sketch: u64) -> Arc<Feature> {
    Arc::new(Feature::new(
        FeatureId::from_raw(raw),
        name.to_owned(),
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: FeatureId::from_raw(sketch),
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(Expression::Number(5.0), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        })),
    ))
}

fn removal(raw: u64, name: &str, body: u64) -> Arc<Feature> {
    Arc::new(Feature::new(
        FeatureId::from_raw(raw),
        name.to_owned(),
        FeatureKind::Remove(Remove {
            body: FeatureId::from_raw(body),
        }),
    ))
}

fn names(features: &[Arc<Feature>]) -> Vec<&str> {
    features
        .iter()
        .map(|feature| feature.name.as_str())
        .collect()
}

fn tree(document: &Document) -> Vec<&str> {
    document
        .features()
        .map(|feature| feature.name.as_str())
        .collect()
}

fn appended(features: &[Arc<Feature>]) -> Document {
    let mut document = Document::default();
    let inserts = features
        .iter()
        .enumerate()
        .map(|(index, feature)| Edit::InsertFeature {
            index,
            feature: Arc::clone(feature),
        })
        .collect();
    document.apply(Transaction::new("Load", inserts)).unwrap();
    document
}

#[test]
fn an_insert_landing_below_a_feature_using_it_is_refused() {
    let mut document = appended(&[sketch(1, "Outline"), extrusion(2, "Base", 1)]);

    let deleted = document
        .apply(Transaction::single(
            "Delete",
            Edit::RemoveFeature {
                id: FeatureId::from_raw(1),
            },
        ))
        .unwrap();
    let below = document.apply(Transaction::single(
        "Insert",
        Edit::InsertFeature {
            index: 1,
            feature: sketch(1, "Outline"),
        },
    ));
    let restored = document.apply(deleted);

    assert_eq!(
        below,
        Err(EditError::BelowUser {
            name: "Outline".to_owned(),
            other: "Base".to_owned(),
        })
    );
    assert!(restored.is_ok());
    assert_eq!(tree(&document), ["Outline", "Base"]);
}

#[test]
fn an_insert_finds_users_after_kind_changes_and_removals() {
    let mut document = appended(&[
        sketch(1, "Outline"),
        sketch(2, "Profile"),
        extrusion(3, "Base", 1),
        extrusion(4, "Lug", 2),
    ]);

    let repointed = document.apply(Transaction::single(
        "Edit",
        Edit::SetFeatureKind {
            id: FeatureId::from_raw(3),
            kind: extrusion(3, "Base", 2).kind.clone(),
        },
    ));
    let lug_gone = document.apply(Transaction::single(
        "Delete",
        Edit::RemoveFeature {
            id: FeatureId::from_raw(4),
        },
    ));
    let outline_gone = document.apply(Transaction::single(
        "Delete",
        Edit::RemoveFeature {
            id: FeatureId::from_raw(1),
        },
    ));
    let outline_last = document.apply(Transaction::single(
        "Insert",
        Edit::InsertFeature {
            index: 2,
            feature: sketch(1, "Outline"),
        },
    ));
    let profile_gone = document.apply(Transaction::single(
        "Delete",
        Edit::RemoveFeature {
            id: FeatureId::from_raw(2),
        },
    ));
    let profile_last = document.apply(Transaction::single(
        "Insert",
        Edit::InsertFeature {
            index: 2,
            feature: sketch(2, "Profile"),
        },
    ));

    assert!(repointed.is_ok());
    assert!(lug_gone.is_ok());
    assert!(outline_gone.is_ok());
    assert!(outline_last.is_ok());
    assert!(profile_gone.is_ok());
    assert_eq!(
        profile_last,
        Err(EditError::BelowUser {
            name: "Profile".to_owned(),
            other: "Base".to_owned(),
        })
    );
}

#[test]
fn features_stored_in_order_keep_it_and_nothing_is_lowered() {
    let stored = vec![
        sketch(1, "Outline"),
        extrusion(2, "Base", 1),
        removal(3, "Remove", 2),
    ];

    let ordered = Document::default().in_use_order(stored.clone());

    assert_eq!(ordered.features, stored);
    assert!(ordered.lowered.is_empty());
}

#[test]
fn a_user_stored_above_what_it_uses_is_lowered_just_below_it_and_the_rest_keep_their_order() {
    let stored = vec![
        extrusion(1, "Base", 3),
        removal(2, "Remove", 1),
        sketch(5, "Other"),
        sketch(3, "Outline"),
        sketch(4, "Last"),
    ];

    let ordered = Document::default().in_use_order(stored);

    assert_eq!(
        names(&ordered.features),
        ["Other", "Outline", "Base", "Remove", "Last"]
    );
    assert_eq!(
        ordered.lowered,
        [Lowered {
            name: "Base".to_owned(),
            below: "Outline".to_owned(),
        }]
    );
}

#[test]
fn features_using_each_other_in_a_circle_come_once_nothing_else_is_ready_and_are_not_reported() {
    let stored = vec![
        extrusion(1, "First", 2),
        extrusion(2, "Second", 1),
        sketch(3, "Outline"),
    ];

    let ordered = Document::default().in_use_order(stored);

    assert_eq!(names(&ordered.features), ["Outline", "First", "Second"]);
    assert!(ordered.lowered.is_empty());
}

#[test]
fn a_model_put_in_use_order_undoes_deleting_changing_and_moving_the_user() {
    let ordered = Document::default()
        .in_use_order(vec![
            extrusion(1, "Base", 2),
            sketch(2, "Outline"),
            sketch(3, "Note"),
        ])
        .features;
    let mut document = appended(&ordered);
    let base = FeatureId::from_raw(1);

    let deleted = document
        .apply(Transaction::single(
            "Delete",
            Edit::RemoveFeature { id: base },
        ))
        .unwrap();
    let undo_delete = document.apply(deleted);
    let changed = document
        .apply(Transaction::single(
            "Edit",
            Edit::SetFeatureKind {
                id: base,
                kind: extrusion(1, "Base", 9).kind.clone(),
            },
        ))
        .unwrap();
    let undo_change = document.apply(changed);
    let moved = document
        .apply(Transaction::single(
            "Move",
            Edit::MoveFeature { id: base, index: 2 },
        ))
        .unwrap();
    let after_move = tree(&document)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let undo_move = document.apply(moved);

    assert_eq!(names(&ordered), ["Outline", "Base", "Note"]);
    assert!(undo_delete.is_ok());
    assert!(undo_change.is_ok());
    assert_eq!(after_move, ["Outline", "Note", "Base"]);
    assert!(undo_move.is_ok());
    assert_eq!(tree(&document), ["Outline", "Base", "Note"]);
}
