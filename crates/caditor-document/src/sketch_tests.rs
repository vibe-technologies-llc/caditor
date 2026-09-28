use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch, SketchError};

use crate::{
    Document, Edit, EditError, Editor, FeatureId, FeatureKind, ParameterValues, Transaction,
};

struct Rectangle {
    document: Document,
    feature: FeatureId,
    width: ParameterId,
    corners: [EntityId; 4],
    sides: [EntityId; 4],
    constraints: Vec<ConstraintId>,
}

fn rectangle() -> Rectangle {
    let mut document = Document::default();
    let mut transaction = document.transaction("New sketch");
    let width = transaction.add_parameter("width", transaction.parse("30 mm").unwrap());
    let feature = transaction.add_feature("Plate", FeatureKind::from(Sketch::new(Plane::XY)));
    document.apply(transaction.finish()).unwrap();

    let mut transaction = document.transaction("Draw rectangle");
    let drawn = [
        Point2::new(0.5, -0.25),
        Point2::new(31.0, 0.5),
        Point2::new(29.0, 21.0),
        Point2::new(-1.0, 19.5),
    ];
    let corners =
        drawn.map(|position| transaction.add_sketch_entity(feature, Entity::Point(position)));
    let sides = [0, 1, 2, 3].map(|index| {
        transaction.add_sketch_entity(
            feature,
            Entity::Line {
                start: corners[index],
                end: corners[(index + 1) % 4],
            },
        )
    });
    let constraints = vec![
        transaction.add_sketch_constraint(feature, Constraint::Horizontal(sides[0])),
        transaction.add_sketch_constraint(feature, Constraint::Vertical(sides[1])),
        transaction.add_sketch_constraint(feature, Constraint::Horizontal(sides[2])),
        transaction.add_sketch_constraint(feature, Constraint::Vertical(sides[3])),
        transaction.add_sketch_constraint(
            feature,
            Constraint::Coincident(corners[0], EntityId::ORIGIN),
        ),
        transaction.add_sketch_constraint(
            feature,
            Constraint::Distance {
                from: corners[0],
                to: corners[1],
                value: Expression::Parameter(width),
            },
        ),
        transaction.add_sketch_constraint(
            feature,
            Constraint::Distance {
                from: corners[1],
                to: corners[2],
                value: transaction.parse("width - 10 mm").unwrap(),
            },
        ),
    ];
    document.apply(transaction.finish()).unwrap();
    Rectangle {
        document,
        feature,
        width,
        corners,
        sides,
        constraints,
    }
}

fn sketch_of(document: &Document, feature: FeatureId) -> &Sketch {
    document.feature(feature).unwrap().kind.sketch().unwrap()
}

fn same_content(a: &Sketch, b: &Sketch) -> bool {
    a.plane() == b.plane() && a.entities().eq(b.entities()) && a.constraints().eq(b.constraints())
}

fn solve(document: &Document, feature: FeatureId) -> Sketch {
    let values = ParameterValues::evaluate(document);
    sketch_of(document, feature)
        .solve(&|id| values.value(id), &|| false)
        .unwrap()
        .geometry
}

#[test]
fn a_line_and_its_new_endpoints_are_added_in_one_transaction() {
    let shape = rectangle();
    let sketch = sketch_of(&shape.document, shape.feature);

    assert_eq!(sketch.entities().len(), 8);
    assert_eq!(sketch.constraints().len(), 7);
    assert_eq!(
        sketch.entity(shape.sides[3]),
        Some(&Entity::Line {
            start: shape.corners[3],
            end: shape.corners[0],
        })
    );
    let mut ids: Vec<u64> = shape
        .corners
        .iter()
        .chain(&shape.sides)
        .map(|id| id.raw())
        .collect();
    ids.extend(shape.constraints.iter().map(|id| id.raw()));
    assert_eq!(ids, (0..15).collect::<Vec<_>>());
    assert_eq!(sketch.next_id(), 15);
}

#[test]
fn undo_and_redo_of_every_sketch_edit_restore_the_exact_sketch() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut editor = Editor::new(shape.document.clone());
    let before = sketch_of(editor.document(), feature).clone();
    let add_point = |document: &Document| {
        let mut transaction = document.transaction("Add point");
        transaction.add_sketch_entity(feature, Entity::Point(Point2::new(5.0, 5.0)));
        transaction.finish()
    };
    let add_constraint = |document: &Document| {
        let mut transaction = document.transaction("Add constraint");
        transaction.add_sketch_constraint(
            feature,
            Constraint::Parallel(shape.sides[0], shape.sides[2]),
        );
        transaction.finish()
    };
    let move_point = |_: &Document| {
        Transaction::single(
            "Move point",
            Edit::SetSketchEntity {
                feature,
                id: shape.corners[2],
                entity: Entity::Point(Point2::new(40.0, 40.0)),
            },
        )
    };
    let remove_constraint = |_: &Document| {
        Transaction::single(
            "Remove constraint",
            Edit::RemoveSketchConstraint {
                feature,
                id: shape.constraints[0],
            },
        )
    };
    let remove_side = |document: &Document| {
        let mut transaction = document.transaction("Remove side");
        transaction.remove_sketch_items(feature, [shape.sides[2]], []);
        let transaction = transaction.finish();
        assert_eq!(transaction.edits().len(), 3);
        transaction
    };
    let steps: [&dyn Fn(&Document) -> Transaction; 5] = [
        &add_point,
        &add_constraint,
        &move_point,
        &remove_constraint,
        &remove_side,
    ];

    for step in steps {
        let start = editor.document().clone();
        editor.apply(step(editor.document())).unwrap();
        let applied = editor.document().clone();

        editor.undo().unwrap();
        let undone = sketch_of(editor.document(), feature);
        assert!(same_content(undone, sketch_of(&start, feature)));
        assert_eq!(undone.next_id(), sketch_of(&applied, feature).next_id());

        editor.redo().unwrap();
        assert_eq!(*editor.document(), applied);
    }

    while editor.undo().unwrap().is_some() {}
    assert!(same_content(sketch_of(editor.document(), feature), &before));
    assert_eq!(sketch_of(editor.document(), feature).next_id(), 17);
}

#[test]
fn removing_a_used_point_is_refused_and_names_its_users() {
    let mut shape = rectangle();
    let before = shape.document.clone();

    let error = shape
        .document
        .apply(Transaction::single(
            "Delete point",
            Edit::RemoveSketchEntity {
                feature: shape.feature,
                id: shape.corners[1],
            },
        ))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "In Plate, Point 1 is used by Line 4, Line 5, Distance between Point 0 and Point 1 and \
         Distance between Point 1 and Point 2. Remove those first."
    );
    assert_eq!(shape.document, before);

    let error = shape
        .document
        .apply(Transaction::single(
            "Delete origin",
            Edit::RemoveSketchEntity {
                feature: shape.feature,
                id: EntityId::ORIGIN,
            },
        ))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Plate: Origin is reference geometry, which cannot be removed or changed"
    );
}

#[test]
fn setting_an_entity_only_changes_its_value() {
    let shape = rectangle();
    let feature = shape.feature;
    let refused = |id: EntityId, entity: Entity| {
        shape
            .document
            .check(&Transaction::single(
                "Change",
                Edit::SetSketchEntity {
                    feature,
                    id,
                    entity,
                },
            ))
            .unwrap_err()
    };

    assert_eq!(
        refused(
            shape.sides[0],
            Entity::Line {
                start: shape.corners[0],
                end: shape.corners[2],
            }
        )
        .to_string(),
        "Plate: only the position or size of Line 4 can change, not its kind or the points it \
         uses"
    );
    assert!(matches!(
        refused(
            shape.corners[0],
            Entity::Circle {
                center: shape.corners[1],
                radius: 1.0
            }
        ),
        EditError::Sketch {
            error: SketchError::ChangesStructure { .. },
            ..
        }
    ));
    assert!(matches!(
        refused(shape.corners[0], Entity::Point(Point2::new(f64::NAN, 0.0))),
        EditError::Sketch {
            error: SketchError::NotFinite,
            ..
        }
    ));
    assert!(matches!(
        refused(EntityId::from_raw(99), Entity::Point(Point2::ZERO)),
        EditError::Sketch {
            error: SketchError::NoSuchEntity(_),
            ..
        }
    ));

    let mut document = shape.document.clone();
    let mut transaction = document.transaction("Add circle");
    let center = transaction.add_sketch_entity(feature, Entity::Point(Point2::new(50.0, 0.0)));
    let circle = transaction.add_sketch_entity(
        feature,
        Entity::Circle {
            center,
            radius: 3.0,
        },
    );
    document.apply(transaction.finish()).unwrap();
    let resize = |radius| {
        Transaction::single(
            "Resize",
            Edit::SetSketchEntity {
                feature,
                id: circle,
                entity: Entity::Circle { center, radius },
            },
        )
    };
    assert!(matches!(
        document.check(&resize(0.0)),
        Err(EditError::Sketch {
            error: SketchError::InvalidRadius,
            ..
        })
    ));
    document.apply(resize(7.5)).unwrap();
    assert_eq!(
        sketch_of(&document, feature).circle(circle),
        Some((Point2::new(50.0, 0.0), 7.5))
    );
}

#[test]
fn sketch_ids_are_never_reused_after_undoing_an_add() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut editor = Editor::new(shape.document);
    let add = |editor: &Editor| {
        let mut transaction = editor.document().transaction("Add point");
        let point = transaction.add_sketch_entity(feature, Entity::Point(Point2::ZERO));
        let constraint = transaction
            .add_sketch_constraint(feature, Constraint::Coincident(point, shape.sides[0]));
        (point, constraint, transaction.finish())
    };

    let (first_point, first_constraint, transaction) = add(&editor);
    editor.apply(transaction).unwrap();
    editor.undo().unwrap();
    let (second_point, second_constraint, transaction) = add(&editor);
    editor.apply(transaction).unwrap();

    assert_eq!((first_point.raw(), first_constraint.raw()), (15, 16));
    assert_eq!((second_point.raw(), second_constraint.raw()), (17, 18));
    let sketch = sketch_of(editor.document(), feature);
    assert!(sketch.entity(first_point).is_none());
    assert!(sketch.constraint(first_constraint).is_none());
}

#[test]
fn removing_a_rectangle_corner_takes_its_sides_and_their_constraints() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut document = shape.document.clone();
    let mut transaction = document.transaction("Delete corner");
    transaction.remove_sketch_items(feature, [shape.corners[1], EntityId::ORIGIN], []);
    let transaction = transaction.finish();

    let removed_constraints = [0, 1, 5, 6].map(|index| shape.constraints[index]);
    let expected: Vec<Edit> = removed_constraints
        .iter()
        .map(|id| Edit::RemoveSketchConstraint { feature, id: *id })
        .chain(
            [shape.sides[0], shape.sides[1], shape.corners[1]]
                .map(|id| Edit::RemoveSketchEntity { feature, id }),
        )
        .collect();
    assert_eq!(transaction.edits(), expected);

    let inverse = document.apply(transaction).unwrap();
    let sketch = sketch_of(&document, feature);
    assert_eq!(sketch.entities().len(), 5);
    assert_eq!(sketch.constraints().len(), 3);
    assert!(sketch.entity(shape.corners[0]).is_some());

    document.apply(inverse).unwrap();
    assert_eq!(document, shape.document);
}

#[test]
fn removing_a_line_keeps_its_endpoints_and_each_item_is_removed_once() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut transaction = shape.document.transaction("Delete");
    transaction.remove_sketch_items(
        feature,
        [shape.sides[0], shape.sides[0]],
        [shape.constraints[0], shape.constraints[4]],
    );
    let transaction = transaction.finish();

    assert_eq!(
        transaction.edits(),
        [
            Edit::RemoveSketchConstraint {
                feature,
                id: shape.constraints[0]
            },
            Edit::RemoveSketchConstraint {
                feature,
                id: shape.constraints[4]
            },
            Edit::RemoveSketchEntity {
                feature,
                id: shape.sides[0]
            },
        ]
    );
    let mut document = shape.document.clone();
    document.apply(transaction).unwrap();
    assert!(
        sketch_of(&document, feature)
            .entity(shape.corners[0])
            .is_some()
    );
}

#[test]
fn settling_brings_the_definition_to_the_solved_shape() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut document = shape.document.clone();
    let solved = solve(&document, feature);
    assert_ne!(*sketch_of(&document, feature), solved);

    let mut transaction = document.transaction("Settle");
    transaction.settle_sketch(feature, &solved);
    let transaction = transaction.finish();
    assert_eq!(transaction.edits().len(), 4);
    document.apply(transaction).unwrap();

    assert_eq!(*sketch_of(&document, feature), solved);
    assert_eq!(solve(&document, feature), solved);
    assert_eq!(
        sketch_of(&document, feature).point(shape.corners[2]),
        Some(Point2::new(30.0, 20.0))
    );

    let mut transaction = document.transaction("Settle again");
    transaction.settle_sketch(feature, &solved);
    assert!(transaction.finish().is_empty());
}

#[test]
fn settling_from_a_stale_result_touches_only_matching_usable_entities() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut document = shape.document.clone();
    let mut stale = Sketch::new(Plane::XY);
    stale.add_point(Point2::new(f64::NAN, 1.0));
    stale.add_point(Point2::new(3.0, 4.0));
    stale.add_point(Point2::new(29.0, 21.0));
    stale.add_point(Point2::new(-1.0, 19.5));
    stale.add_circle(Point2::ZERO, 2.0);
    stale.add_circle(Point2::ZERO, f64::NAN);
    stale.add_point(Point2::ZERO);

    let mut transaction = document.transaction("Settle");
    transaction.settle_sketch(feature, &stale);
    let transaction = transaction.finish();

    assert_eq!(
        transaction.edits(),
        [Edit::SetSketchEntity {
            feature,
            id: shape.corners[1],
            entity: Entity::Point(Point2::new(3.0, 4.0)),
        }]
    );
    document.apply(transaction).unwrap();
}

#[test]
fn constraints_are_checked_by_the_sketch_and_the_document() {
    let shape = rectangle();
    let feature = shape.feature;
    let add = |constraint| {
        let mut transaction = shape.document.transaction("Add constraint");
        transaction.add_sketch_constraint(feature, constraint);
        shape.document.check(&transaction.finish())
    };

    assert_eq!(
        add(Constraint::Radius {
            entity: shape.corners[0],
            value: Expression::Number(1.0),
        }),
        Err(EditError::Sketch {
            name: "Plate".to_owned(),
            error: SketchError::WrongKind {
                entity: shape.corners[0],
                found: "Point 0".to_owned(),
                needed: "a circle or an arc",
            },
        })
    );
    assert_eq!(
        add(Constraint::Distance {
            from: shape.corners[0],
            to: shape.corners[2],
            value: Expression::Parameter(ParameterId::from_raw(40)),
        }),
        Err(EditError::MissingParameter)
    );
    assert_eq!(
        shape.document.check(&Transaction::single(
            "Add constraint",
            Edit::AddSketchConstraint {
                feature,
                id: ConstraintId::from_raw(u64::MAX),
                constraint: Constraint::Horizontal(shape.sides[0]),
            },
        )),
        Err(EditError::Sketch {
            name: "Plate".to_owned(),
            error: SketchError::ReservedId(u64::MAX),
        })
    );
    assert_eq!(
        shape.document.check(&Transaction::single(
            "Add point",
            Edit::AddSketchEntity {
                feature,
                id: shape.sides[0],
                entity: Entity::Point(Point2::ZERO),
            },
        )),
        Err(EditError::Sketch {
            name: "Plate".to_owned(),
            error: SketchError::DuplicateId(shape.sides[0].raw()),
        })
    );
    assert_eq!(
        shape.document.check(&Transaction::single(
            "Remove constraint",
            Edit::RemoveSketchConstraint {
                feature: FeatureId::from_raw(9),
                id: shape.constraints[0],
            },
        )),
        Err(EditError::MissingFeature)
    );
}

#[test]
fn check_reports_sketch_errors_without_changing_the_document() {
    let shape = rectangle();
    let mut transaction = shape.document.transaction("Delete then misuse");
    transaction.remove_sketch_items(shape.feature, [shape.corners[1]], []);
    transaction.add_sketch_constraint(shape.feature, Constraint::Horizontal(shape.sides[0]));
    let transaction = transaction.finish();

    assert!(matches!(
        shape.document.check(&transaction),
        Err(EditError::Sketch {
            error: SketchError::MissingEntity(_),
            ..
        })
    ));
    assert_eq!(
        sketch_of(&shape.document, shape.feature).entities().len(),
        8
    );

    let mut document = shape.document.clone();
    assert!(document.apply(transaction).is_err());
    assert_eq!(document, shape.document);
    assert_eq!(shape.width, document.parameter_named("width").unwrap().id());
}

#[test]
fn sketch_items_can_be_added_to_a_feature_created_in_the_same_transaction() {
    let mut document = Document::default();
    let mut transaction = document.transaction("New sketch with a line");
    let feature = transaction.add_feature("Sketch", FeatureKind::from(Sketch::new(Plane::XY)));
    let start = transaction.add_sketch_entity(feature, Entity::Point(Point2::ZERO));
    let end = transaction.add_sketch_entity(feature, Entity::Point(Point2::X));
    let line = transaction.add_sketch_entity(feature, Entity::Line { start, end });
    document.apply(transaction.finish()).unwrap();

    assert_eq!(
        sketch_of(&document, feature).line_endpoints(line),
        Some((Point2::ZERO, Point2::X))
    );
}

fn next_sketch_id(document: &Document, feature: FeatureId) -> u64 {
    document
        .feature(feature)
        .and_then(|feature| feature.kind.sketch())
        .map(Sketch::next_id)
        .unwrap()
}

#[test]
fn adding_then_undoing_leaves_the_same_content_with_higher_counters() {
    let Rectangle {
        mut document,
        feature,
        ..
    } = rectangle();
    let before = document.clone();
    let mut transaction = document.transaction("Draw point");
    transaction.add_sketch_entity(feature, Entity::Point(Point2::new(3.0, 3.0)));
    let undo = document.apply(transaction.finish()).unwrap();
    document.apply(undo).unwrap();
    assert!(document.same_content(&before));
    assert_ne!(document, before);
    assert!(next_sketch_id(&document, feature) > next_sketch_id(&before, feature));
}

#[test]
fn restoring_an_earlier_version_never_lowers_a_sketch_counter() {
    let Rectangle {
        mut document,
        feature,
        ..
    } = rectangle();
    let version = document.clone();
    let mut transaction = document.transaction("Draw point");
    transaction.add_sketch_entity(feature, Entity::Point(Point2::new(3.0, 3.0)));
    document.apply(transaction.finish()).unwrap();
    let reached = next_sketch_id(&document, feature);

    let restore = document.transaction_to(&version, "Restore earlier version");
    document.apply(restore).unwrap();
    assert!(document.same_content(&version));
    assert_eq!(next_sketch_id(&document, feature), reached);
}

#[test]
fn recompute_reanalyses_only_the_parts_of_a_sketch_an_edit_touches() {
    let shape = rectangle();
    let feature = shape.feature;
    let mut document = shape.document.clone();
    let mut transaction = document.transaction("Add circle");
    let center = transaction.add_sketch_entity(feature, Entity::Point(Point2::new(60.0, 10.0)));
    let circle = transaction.add_sketch_entity(
        feature,
        Entity::Circle {
            center,
            radius: 4.0,
        },
    );
    let radius = transaction.add_sketch_constraint(
        feature,
        Constraint::Radius {
            entity: circle,
            value: transaction.parse("4 mm").unwrap(),
        },
    );
    document.apply(transaction.finish()).unwrap();

    let mut engine = crate::Recompute::default();
    let run = |engine: &mut crate::Recompute, document: &Document| {
        engine.run(
            document,
            &crate::ModelEvaluator,
            &crate::CancelToken::never(),
            &|_, _| {},
        )
    };
    let first = run(&mut engine, &document);
    let solved = first
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(crate::FeatureResult::sketch)
        .unwrap()
        .geometry
        .clone();
    let mut transaction = document.transaction("Settle");
    transaction.settle_sketch(feature, &solved);
    document.apply(transaction.finish()).unwrap();
    document
        .apply(Transaction::single(
            "Set radius",
            Edit::SetDimension {
                feature,
                constraint: radius,
                value: document.parse("6 mm").unwrap(),
            },
        ))
        .unwrap();

    let second = run(&mut engine, &document);
    let result = second
        .feature(feature)
        .and_then(|status| status.result.as_deref())
        .and_then(crate::FeatureResult::sketch)
        .unwrap();
    assert_eq!(result.memo().recalled(), 1);
    assert_eq!(result.geometry.circle(circle).unwrap().1, 6.0);
    assert_eq!(result.solution.degrees_of_freedom(), 2);
}
