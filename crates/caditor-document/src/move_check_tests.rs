use caditor_expression::{Expression, ParameterId, Unit};
use caditor_geometry::Plane;
use caditor_kernel::{FaceName, FaceOrigin, FaceReference};
use caditor_sketch::Sketch;

use crate::{tally, *};

fn millimetres(value: f64) -> Expression {
    Expression::Measure(value, Unit::Millimetre)
}

fn sketch() -> FeatureKind {
    FeatureKind::from(Sketch::new(Plane::XY))
}

fn extrude(sketch: FeatureId, operation: BodyOperation) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent: ExtrudeExtent::one_side(millimetres(4.0), false),
        operation,
        start: None,
        other_bodies: Vec::new(),
        taper: None,
        wall: None,
        direction: None,
    }))
}

fn moving(id: FeatureId, index: usize) -> Transaction {
    Transaction::single("Move", Edit::MoveFeature { id, index })
}

fn applied(document: &Document, transaction: Transaction) -> Result<(), EditError> {
    document.clone().apply(transaction).map(|_| ())
}

struct Stack {
    document: Document,
    block: FeatureId,
    spare: FeatureId,
    lug: FeatureId,
}

fn stack() -> Stack {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let first = transaction.add_feature("Profile A", sketch());
    let second = transaction.add_feature("Profile B", sketch());
    let block = transaction.add_feature("Block", extrude(first, BodyOperation::NewBody));
    let spare = transaction.add_feature("Spare", sketch());

    let lug = transaction.add_feature("Lug", extrude(second, BodyOperation::Add(block)));

    transaction.add_feature("Cut", extrude(first, BodyOperation::Remove(block)));
    document.apply(transaction.finish()).unwrap();
    Stack {
        document,
        block,
        spare,
        lug,
    }
}

#[test]
fn a_refused_move_names_the_first_user_above_and_the_first_dependency_below() {
    let stack = stack();
    let document = &stack.document;

    let block_to_end = document.check_moves(&moving(stack.block, 5));
    let lug_to_top = document.check_moves(&moving(stack.lug, 0));
    let block_past_the_end = document.check_moves(&moving(stack.block, usize::MAX));
    let spare_past_the_end = document.check_moves(&moving(stack.spare, usize::MAX));
    let unknown = document.check_moves(&moving(FeatureId::from_raw(999), 0));
    let spare_to_top = document.check_moves(&moving(stack.spare, 0));

    let applied_block_to_end = applied(document, moving(stack.block, 5));
    let applied_lug_to_top = applied(document, moving(stack.lug, 0));
    let applied_past_the_end = applied(document, moving(stack.block, usize::MAX));
    let applied_unknown = applied(document, moving(FeatureId::from_raw(999), 0));

    let mut after_spare = document.transaction("Move");

    after_spare.edit(Edit::MoveFeature {
        id: stack.spare,
        index: 0,
    });
    after_spare.edit(Edit::MoveFeature {
        id: stack.block,
        index: 1,
    });

    let after_spare = after_spare.finish();
    let judged_after_spare = document.check_moves(&after_spare);
    let applied_after_spare = applied(document, after_spare);

    assert_eq!(
        block_to_end,
        Err(EditError::BelowDependent {
            name: "Block".to_owned(),
            other: "Lug".to_owned()
        })
    );
    assert_eq!(
        lug_to_top,
        Err(EditError::AboveDependency {
            name: "Lug".to_owned(),
            other: "Profile B".to_owned()
        })
    );
    assert_eq!(block_past_the_end, block_to_end);
    assert_eq!(spare_past_the_end, Err(EditError::OutOfRange(usize::MAX)));
    assert_eq!(unknown, Err(EditError::MissingFeature));
    assert_eq!(spare_to_top, Ok(()));

    assert_eq!(applied_block_to_end, block_to_end);
    assert_eq!(applied_lug_to_top, lug_to_top);
    assert_eq!(applied_past_the_end, Err(EditError::OutOfRange(usize::MAX)));
    assert_eq!(applied_unknown, Err(EditError::MissingFeature));

    assert_eq!(
        judged_after_spare,
        Err(EditError::AboveDependency {
            name: "Block".to_owned(),
            other: "Profile A".to_owned()
        })
    );
    assert_eq!(applied_after_spare, judged_after_spare);
}

struct Readers {
    document: Document,
    measurement: FeatureId,
    reader: FeatureId,
    other_measurement: FeatureId,
}

fn origin_gap(parameter: ParameterId) -> FeatureKind {
    FeatureKind::from(Measurement {
        reading: Reading::Between {
            quantity: Between::Distance,
            first: MeasuredItem::Point(PointReference::Origin),
            second: MeasuredItem::Plane(PlaneReference::Principal(PrincipalPlane::Xy)),
        },
        parameter: Some(parameter),
    })
}

fn offset_point(parameter: ParameterId) -> FeatureKind {
    FeatureKind::Datum(Datum::Point(DatumPoint {
        base: PointReference::Origin,
        offset: [
            Expression::Parameter(parameter),
            millimetres(0.0),
            millimetres(0.0),
        ],
    }))
}

fn readers() -> Readers {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let gap = transaction.add_parameter("gap", millimetres(0.0));
    let half = transaction.add_parameter("half", transaction.parse("gap / 2").unwrap());
    let other = transaction.add_parameter("other", millimetres(0.0));

    transaction.add_feature("Spare", sketch());

    let measurement = transaction.add_feature("Measure gap", origin_gap(gap));
    let other_measurement = transaction.add_feature("Measure other", origin_gap(other));

    transaction.add_feature("Between", sketch());

    let reader = transaction.add_feature("Reader", offset_point(half));

    document.apply(transaction.finish()).unwrap();
    Readers {
        document,
        measurement,
        reader,
        other_measurement,
    }
}

#[test]
fn a_measurement_moved_past_a_feature_reading_it_through_a_parameter_is_refused_naming_both() {
    let readers = readers();
    let document = &readers.document;

    let below_its_reader = document.check_moves(&moving(readers.measurement, 4));
    let reader_to_top = document.check_moves(&moving(readers.reader, 0));
    let past_the_sketch = document.check_moves(&moving(readers.measurement, 3));
    let other_below_the_reader = document.check_moves(&moving(readers.other_measurement, 4));

    let applied_below_its_reader = applied(document, moving(readers.measurement, 4));
    let applied_reader_to_top = applied(document, moving(readers.reader, 0));

    assert_eq!(
        below_its_reader,
        Err(EditError::BelowDependent {
            name: "Measure gap".to_owned(),
            other: "Reader".to_owned()
        })
    );
    assert_eq!(
        reader_to_top,
        Err(EditError::AboveDependency {
            name: "Reader".to_owned(),
            other: "Measure gap".to_owned()
        })
    );
    assert_eq!(past_the_sketch, Ok(()));
    assert_eq!(other_below_the_reader, Ok(()));
    assert_eq!(applied_below_its_reader, below_its_reader);
    assert_eq!(applied_reader_to_top, reader_to_top);
}

#[test]
fn a_feature_holding_another_s_faces_moves_above_it_only_by_the_plain_edit() {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let outline = transaction.add_feature("Outline", sketch());
    let base = transaction.add_feature("Base", extrude(outline, BodyOperation::NewBody));
    let note = transaction.add_feature("Note", sketch());

    transaction.add_feature("Spare", sketch());

    let shell = transaction.add_feature(
        "Shell",
        FeatureKind::Shell(Shell {
            body: base,
            open: vec![FaceReference::new(
                FaceName::from_digest(1),
                Some(FaceOrigin::EndCap {
                    feature: note.raw(),
                }),
                [],
            )],
            thickness: millimetres(1.0),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );

    document.apply(transaction.finish()).unwrap();

    let above_the_note = document.check_moves(&moving(shell, 2));
    let note_below = document.check_moves(&moving(note, 4));
    let applied_above_the_note = applied(&document, moving(shell, 2));
    let applied_note_below = applied(&document, moving(note, 4));

    assert_eq!(
        above_the_note,
        Err(EditError::AboveDependency {
            name: "Shell".to_owned(),
            other: "Note".to_owned()
        })
    );
    assert_eq!(
        note_below,
        Err(EditError::BelowDependent {
            name: "Note".to_owned(),
            other: "Shell".to_owned()
        })
    );
    assert_eq!(applied_above_the_note, Ok(()));
    assert_eq!(applied_note_below, Ok(()));
}

struct Tools {
    document: Document,
    user: FeatureId,
    first_combine: FeatureId,
    second_combine: FeatureId,
    piece_user: FeatureId,
    piece_combine: FeatureId,
}

fn tools() -> Tools {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let outline = transaction.add_feature("Outline", sketch());
    let body = transaction.add_feature("Body", extrude(outline, BodyOperation::NewBody));
    let spare_tool =
        transaction.add_feature("Spare tool", extrude(outline, BodyOperation::NewBody));
    let tool = transaction.add_feature("Tool", extrude(outline, BodyOperation::NewBody));
    let chunk = transaction.add_feature("Chunk", extrude(outline, BodyOperation::NewBody));
    let piece = chunk.piece(PieceKey::from_raw(1)).unwrap();

    let user = transaction.add_feature("User", extrude(outline, BodyOperation::Add(tool)));
    let piece_user =
        transaction.add_feature("Piece user", extrude(outline, BodyOperation::Add(piece)));

    transaction.add_feature("Spare", sketch());

    let mut first = Combine::new(body, spare_tool, CombineOperation::Join);

    first.more_tools.push(tool);

    let first_combine = transaction.add_feature("Combine 1", FeatureKind::Combine(first));
    let second_combine = transaction.add_feature(
        "Combine 2",
        FeatureKind::Combine(Combine::new(body, tool, CombineOperation::Cut)),
    );
    let piece_combine = transaction.add_feature(
        "Combine 3",
        FeatureKind::Combine(Combine::new(body, piece, CombineOperation::Join)),
    );

    document.apply(transaction.finish()).unwrap();
    Tools {
        document,
        user,
        first_combine,
        second_combine,
        piece_user,
        piece_combine,
    }
}

#[test]
fn a_body_user_and_its_consumers_name_the_first_in_the_way_and_the_body() {
    let tools = tools();
    let document = &tools.document;

    let user_to_end = document.check_moves(&moving(tools.user, 10));
    let first_above_its_user = document.check_moves(&moving(tools.first_combine, 4));
    let second_above_its_user = document.check_moves(&moving(tools.second_combine, 5));
    let first_above_the_second = document.check_moves(&moving(tools.first_combine, 7));
    let piece_combine_above = document.check_moves(&moving(tools.piece_combine, 6));
    let piece_user_to_end = document.check_moves(&moving(tools.piece_user, 10));
    let applied_user_to_end = applied(document, moving(tools.user, 10));

    assert_eq!(
        user_to_end,
        Err(EditError::BelowConsumer {
            name: "User".to_owned(),
            other: "Combine 1".to_owned(),
            body: "Tool".to_owned(),
        })
    );
    assert_eq!(
        first_above_its_user,
        Err(EditError::AboveConsumedUse {
            name: "Combine 1".to_owned(),
            other: "User".to_owned(),
            body: "Tool".to_owned(),
        })
    );
    assert_eq!(
        second_above_its_user,
        Err(EditError::AboveConsumedUse {
            name: "Combine 2".to_owned(),
            other: "User".to_owned(),
            body: "Tool".to_owned(),
        })
    );
    assert_eq!(
        first_above_the_second,
        Err(EditError::AboveConsumedUse {
            name: "Combine 1".to_owned(),
            other: "Combine 2".to_owned(),
            body: "Tool".to_owned(),
        })
    );
    assert_eq!(
        piece_combine_above,
        Err(EditError::AboveConsumedUse {
            name: "Combine 3".to_owned(),
            other: "Piece user".to_owned(),
            body: String::new(),
        })
    );
    assert_eq!(
        piece_user_to_end,
        Err(EditError::BelowConsumer {
            name: "Piece user".to_owned(),
            other: "Combine 3".to_owned(),
            body: String::new(),
        })
    );
    assert_eq!(applied_user_to_end, Ok(()));
}

#[test]
fn moves_taking_the_model_past_the_records_a_file_opens_are_refused_by_their_check() {
    let mut document = Document::default();

    let mut fill = document.transaction("Fill");

    for index in 0..MAX_RECORDS - 2 {
        fill.add_parameter(format!("filler_{index}"), Expression::Number(1.0));
    }

    let first = fill.add_feature("First", sketch());

    fill.add_feature("Second", sketch());
    document.apply(fill.finish()).unwrap();

    let mut past = document.transaction("Move and add");

    past.edit(Edit::MoveFeature {
        id: first,
        index: 1,
    });
    past.add_feature("Third", sketch());

    let past = past.finish();

    assert_eq!(
        document.check_moves(&past),
        Err(EditError::TooManyRecords {
            records: MAX_RECORDS + 1
        })
    );
    assert_eq!(document.check_moves(&moving(first, 1)), Ok(()));
}

const FOLDERS: usize = 250;
const GROWTH: usize = 4;
const LINEAR_SLACK: usize = 2;

fn foldered(folders: usize) -> (Document, Vec<[FeatureId; 2]>) {
    let mut document = Document::default();

    let mut transaction = document.transaction("Build");

    let outline = transaction.add_feature("Outline", sketch());
    let base = transaction.add_feature("Base", extrude(outline, BodyOperation::NewBody));
    let mut members = Vec::new();

    for index in 0..folders {
        let profile = transaction.add_feature(format!("Profile {index}"), sketch());
        let boss = transaction.add_feature(
            format!("Boss {index}"),
            extrude(profile, BodyOperation::Add(base)),
        );

        let folder = [profile, boss];

        for id in folder {
            transaction.edit(Edit::SetFeatureGroup {
                id,
                group: Some(format!("Folder {index}")),
            });
        }
        members.push(folder);
    }

    document.apply(transaction.finish()).unwrap();
    (document, members)
}

fn drag_folders_to_the_end(folders: usize) -> usize {
    let (mut document, members) = foldered(folders);

    let ((), units) = tally::measure(|| {
        for folder in members.iter().take(folders / 2) {
            let end = document.tree_rows().len() - 1;
            let dragged = document
                .move_features(folder, end, "Move 2 features")
                .unwrap();

            document.apply(dragged).unwrap();
        }
    });

    let last = members.get(folders / 2 - 1).unwrap();

    assert_eq!(
        document.feature_index(last[1]),
        Some(document.features().len() - 1)
    );
    units
}

#[test]
fn dragging_folders_through_a_large_tree_checks_each_move_against_its_ties_alone() {
    let short = drag_folders_to_the_end(FOLDERS);
    let long = drag_folders_to_the_end(FOLDERS * GROWTH);

    assert!(short > 0, "the tally counted nothing");
    assert!(
        long <= short * GROWTH * LINEAR_SLACK,
        "{GROWTH} times the folders took {long} units of work against {short}"
    );
}
