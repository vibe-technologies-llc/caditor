use caditor_expression::{Expression, ParameterId, Quantity};
use caditor_geometry::Plane;

use crate::{
    solid_tests::{extrude, rectangle},
    tests::sample,
    *,
};

fn block() -> (Document, FeatureId) {
    let mut document = Document::default();
    let mut transaction = document.transaction("Block");
    let sketch = transaction.add_feature(
        "Sketch 1",
        FeatureKind::from(rectangle(Plane::XY, (0.0, 0.0), (10.0, 10.0))),
    );
    let extrusion = transaction.add_feature(
        "Extrude 1",
        extrude(sketch, "10 mm", false, BodyOperation::NewBody),
    );
    document.apply(transaction.finish()).unwrap();
    (document, extrusion)
}

fn distance_owner(feature: FeatureId) -> ParameterOwner {
    ParameterOwner::Feature {
        feature,
        value: "Distance".to_owned(),
    }
}

fn distance(document: &Document, feature: FeatureId) -> Expression {
    let Some(FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        extent:
            ExtrudeExtent::OneSide {
                end: ExtrudeEnd::Distance(distance),
                ..
            },
        ..
    }))) = document.feature(feature).map(|feature| &feature.kind)
    else {
        panic!("expected a one-sided extrusion");
    };
    distance.clone()
}

fn with_distance(document: &Document, feature: FeatureId, value: Expression) -> Transaction {
    let Some(FeatureKind::Solid(SolidFeature::Extrude(mut changed))) = document
        .feature(feature)
        .map(|feature| feature.kind.clone())
    else {
        panic!("expected an extrusion");
    };
    changed.extent = ExtrudeExtent::one_side(value, false);
    Transaction::single(
        "Distance",
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Solid(SolidFeature::Extrude(changed)),
        },
    )
}

fn named(document: &Document, feature: FeatureId, name: &str, text: &str) -> Transaction {
    let value = document.parse(text).unwrap();
    let marker = Expression::Negate(Box::new(Expression::Negate(Box::new(value.clone()))));
    let holding = with_distance(document, feature, marker.clone());
    let mut transaction = document.transaction(format!("Name {name}"));
    let id = transaction.add_owned_parameter(name, value, distance_owner(feature));
    let (holding, count) = holding.substituting(&marker, &Expression::Parameter(id));
    assert_eq!(count, 1);
    for edit in holding.edits() {
        transaction.edit(edit.clone());
    }
    transaction.finish()
}

fn id_of(document: &Document, name: &str) -> ParameterId {
    document.parameter_named(name).unwrap().id()
}

#[test]
fn a_named_value_is_a_parameter_its_feature_holds_and_other_expressions_use() {
    let (document, extrusion) = block();
    let mut editor = Editor::new(document);

    editor
        .apply(named(editor.document(), extrusion, "depth", "12 mm"))
        .unwrap();
    let depth = id_of(editor.document(), "depth");
    let mut transaction = editor.document().transaction("Double");
    let double = transaction.parse("depth * 2").unwrap();
    transaction.add_parameter("double", double);
    editor.apply(transaction.finish()).unwrap();
    let document = editor.document();
    let values = ParameterValues::evaluate(document);
    let held = document.owned_parameter(&distance_owner(extrusion), &distance(document, extrusion));

    assert_eq!(distance(document, extrusion), Expression::Parameter(depth));
    assert_eq!(held.map(Parameter::id), Some(depth));
    assert_eq!(
        document.owner_text(&distance_owner(extrusion)).as_deref(),
        Some("Extrude 1 · Distance")
    );
    assert_eq!(
        values.value(id_of(document, "double")),
        Ok(Quantity::length(24.0))
    );
    assert!(
        !document
            .parameter_named("double")
            .unwrap()
            .is_model_parameter()
    );

    editor
        .apply(Transaction::single(
            "Rename",
            Edit::RenameParameter {
                id: depth,
                name: "deep".to_owned(),
            },
        ))
        .unwrap();
    let document = editor.document();
    let double = document.parameter_named("double").unwrap();

    assert_eq!(document.expression_text(&double.expression), "deep * 2");
    assert_eq!(
        document.expression_text(&distance(document, extrusion)),
        "deep"
    );

    let cycle = document.check(&Transaction::single(
        "Loop",
        Edit::SetParameterExpression {
            id: depth,
            expression: document.parse("double / 2").unwrap(),
        },
    ));

    assert!(matches!(cycle, Err(EditError::Cycle { .. })));

    editor.undo().unwrap();
    editor.undo().unwrap();
    editor.undo().unwrap();

    assert!(editor.document().parameters().is_empty());
    assert_eq!(
        distance(editor.document(), extrusion),
        Expression::parse_stored("10 mm").unwrap()
    );
}

#[test]
fn deleting_the_feature_takes_its_unused_named_values_and_keeps_used_ones_as_parameters() {
    let (document, extrusion) = block();
    let mut editor = Editor::new(document);
    editor
        .apply(named(editor.document(), extrusion, "depth", "12 mm"))
        .unwrap();

    editor
        .apply(editor.document().deletion(&[extrusion], "Delete"))
        .unwrap();

    assert!(editor.document().parameters().is_empty());

    editor.undo().unwrap();
    let mut transaction = editor.document().transaction("Use");
    let used = transaction.parse("depth + 1 mm").unwrap();
    transaction.add_parameter("taller", used);
    editor.apply(transaction.finish()).unwrap();
    editor
        .apply(editor.document().deletion(&[extrusion], "Delete"))
        .unwrap();
    let depth = editor.document().parameter_named("depth").unwrap();

    assert_eq!(depth.owner, None);
    assert_eq!(
        editor.document().expression_text(&depth.expression),
        "12 mm"
    );

    editor.undo().unwrap();
    let document = editor.document();

    assert_eq!(
        document.parameter_named("depth").unwrap().owner,
        Some(distance_owner(extrusion))
    );
    assert_eq!(
        distance(document, extrusion),
        Expression::Parameter(id_of(document, "depth"))
    );
}

#[test]
fn removing_a_named_dimension_takes_its_parameter_with_it() {
    let (document, ids) = sample();
    let owner = ParameterOwner::Dimension {
        sketch: ids.base,
        constraint: ids.base_distance,
    };
    let mut transaction = document.transaction("Name");
    let length = transaction.add_owned_parameter(
        "length",
        Expression::parse_stored("30 mm").unwrap(),
        owner.clone(),
    );
    transaction.edit(Edit::SetDimension {
        feature: ids.base,
        constraint: ids.base_distance,
        value: Expression::Parameter(length),
    });
    let naming = transaction.finish();
    let mut editor = Editor::new(document);
    editor.apply(naming).unwrap();

    assert_eq!(
        editor.document().owner_text(&owner).as_deref(),
        Some("Base sketch · Distance")
    );

    let mut removal = editor.document().transaction("Remove");
    removal.remove_sketch_items(ids.base, [], [ids.base_distance]);
    editor.apply(removal.finish()).unwrap();

    assert!(editor.document().parameter(length).is_none());

    editor.undo().unwrap();

    assert_eq!(
        editor.document().parameter(length).unwrap().owner,
        Some(owner)
    );
}

#[test]
fn a_value_description_is_kept_on_one_line_and_bounded() {
    let (document, extrusion) = block();
    let owner = |value: &str| ParameterOwner::Feature {
        feature: extrusion,
        value: value.to_owned(),
    };
    let mut transaction = document.transaction("Name");
    let id = transaction.add_owned_parameter(
        "depth",
        Expression::parse_stored("1 mm").unwrap(),
        owner(" Second\ndistance "),
    );
    let naming = transaction.finish();
    let mut document = document;
    document.apply(naming).unwrap();

    assert_eq!(
        document.parameter(id).unwrap().owner,
        Some(owner("Second distance"))
    );

    let long = "x".repeat(MAX_VALUE_LABEL_CHARS + 1);
    let refused = document.check(&Transaction::single(
        "Owner",
        Edit::SetParameterOwner {
            id,
            owner: Some(owner(&long)),
        },
    ));

    assert_eq!(
        refused,
        Err(EditError::ValueLabelTooLong(MAX_VALUE_LABEL_CHARS + 1))
    );
}
