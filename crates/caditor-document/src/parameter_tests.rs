use caditor_expression::Expression;

use crate::{tests::sample, *};

fn names(document: &Document) -> Vec<&str> {
    document
        .parameters()
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect()
}

fn text(document: &Document, expression: &Expression) -> String {
    document.expression_text(expression)
}

fn extrude_distance(document: &Document, feature: FeatureId) -> String {
    let Some(FeatureKind::Solid(SolidFeature::Extrude(extrude))) =
        document.feature(feature).map(|feature| &feature.kind)
    else {
        panic!("expected an extrusion");
    };
    let ExtrudeExtent::OneSide {
        end: ExtrudeEnd::Distance(distance),
        ..
    } = &extrude.extent
    else {
        panic!("expected a distance");
    };
    text(document, distance)
}

#[test]
fn moving_a_parameter_reorders_the_list_and_undo_puts_it_back() {
    let (document, ids) = sample();
    let mut editor = Editor::new(document);

    editor
        .apply(Transaction::single(
            "Move gap up",
            Edit::MoveParameter {
                id: ids.gap,
                index: 0,
            },
        ))
        .unwrap();
    let moved = names(editor.document()).join(" ");
    let past_the_end = editor.document().check(&Transaction::single(
        "Move",
        Edit::MoveParameter {
            id: ids.gap,
            index: 3,
        },
    ));
    editor.undo().unwrap();

    assert_eq!(moved, "gap width height");
    assert_eq!(past_the_end, Err(EditError::OutOfRange(3)));
    assert_eq!(names(editor.document()), ["width", "height", "gap"]);
    assert_eq!(
        editor.document().parameter_named("gap").unwrap().id(),
        ids.gap
    );
}

#[test]
fn a_note_is_trimmed_bounded_and_undone() {
    let (document, ids) = sample();
    let mut editor = Editor::new(document);
    let note = |text: &str| {
        Transaction::single(
            "Note",
            Edit::SetParameterNote {
                id: ids.width,
                note: text.to_owned(),
            },
        )
    };

    editor
        .apply(note("  Outer width of the bracket \n"))
        .unwrap();
    let kept = editor.document().parameter(ids.width).unwrap().note.clone();
    let too_long = editor
        .document()
        .check(&note(&"x".repeat(MAX_PARAMETER_NOTE_CHARS + 1)));
    editor.undo().unwrap();

    assert_eq!(kept, "Outer width of the bracket");
    assert_eq!(
        too_long,
        Err(EditError::NoteTooLong(MAX_PARAMETER_NOTE_CHARS + 1))
    );
    assert!(
        editor
            .document()
            .parameter(ids.width)
            .unwrap()
            .note
            .is_empty()
    );
}

#[test]
fn deleting_a_used_parameter_writes_its_expression_into_every_use() {
    let (mut document, ids) = sample();
    let mut transaction = document.transaction("Add");
    let double = transaction.add_parameter("double", transaction.parse("width * 2").unwrap());
    let extrude = transaction.add_feature(
        "Extrude 1",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: ids.base,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::one_side(transaction.parse("width / 4").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let mut transaction = document.transaction("Use height");
    transaction.add_parameter("span", transaction.parse("width + height").unwrap());
    document.apply(transaction.finish()).unwrap();
    let mut editor = Editor::new(document);
    let before = editor.document().clone();

    let inline = editor.document().inline_parameter(ids.width).unwrap();
    editor.apply(inline).unwrap();
    let after = editor.document();
    let base_value = after
        .feature(ids.base)
        .and_then(|feature| feature.kind.sketch())
        .and_then(|sketch| sketch.constraint(ids.base_distance))
        .and_then(caditor_sketch::Constraint::dimension)
        .map(|value| text(after, value));

    assert!(after.parameter(ids.width).is_none());
    assert_eq!(
        text(after, &after.parameter(double).unwrap().expression),
        "40 mm * 2"
    );
    assert_eq!(
        text(after, &after.parameter_named("span").unwrap().expression),
        "40 mm + height"
    );
    assert_eq!(extrude_distance(after, extrude), "40 mm / 4");
    assert_eq!(base_value.as_deref(), Some("40 mm"));
    assert!(!after.used_parameters().contains(&ids.width));

    editor.undo().unwrap();
    assert_eq!(*editor.document(), before);
}

#[test]
fn inlining_keeps_links_to_the_parameters_the_expression_uses() {
    let (document, ids) = sample();
    let mut editor = Editor::new(document);
    editor
        .apply(Transaction::single(
            "Edit gap",
            Edit::SetParameterExpression {
                id: ids.gap,
                expression: editor.document().parse("height / 2").unwrap(),
            },
        ))
        .unwrap();
    editor
        .apply(Transaction::single(
            "Edit width",
            Edit::SetParameterExpression {
                id: ids.width,
                expression: editor.document().parse("gap * 8 - gap").unwrap(),
            },
        ))
        .unwrap();

    let inline = editor.document().inline_parameter(ids.gap).unwrap();
    editor.apply(inline).unwrap();
    let document = editor.document();

    assert_eq!(
        text(document, &document.parameter(ids.width).unwrap().expression),
        "height / 2 * 8 - height / 2"
    );
}
