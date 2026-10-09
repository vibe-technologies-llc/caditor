use caditor_document::{SelectionSet, SelectionSets, SetMember};
use caditor_kernel::{EdgeName, EdgeReference, FaceName, FaceOrigin, FaceReference, VertexName};

use super::*;

fn sets_of(base: FeatureId, other: FeatureId) -> SelectionSets {
    let face = FaceReference::new(
        FaceName::from_digest(0xbeef),
        Some(FaceOrigin::EndCap {
            feature: base.raw(),
        }),
        [FaceName::from_digest(2), FaceName::from_digest(3)],
    );
    let edge = EdgeReference::new(
        EdgeName::from_digest(0xabcd),
        [FaceName::from_digest(1), FaceName::from_digest(2)],
        [VertexName::from_digest(3), VertexName::from_digest(4)],
    )
    .with_origins([
        Some(FaceOrigin::StartCap {
            feature: base.raw(),
        }),
        None,
    ]);
    SelectionSets {
        sets: vec![
            SelectionSet {
                name: "Mating faces".to_owned(),
                members: vec![
                    SetMember::Face { body: base, face },
                    SetMember::Edge { body: base, edge },
                ],
            },
            SelectionSet {
                name: "Second body".to_owned(),
                members: vec![SetMember::Body(other)],
            },
        ],
    }
}

#[test]
fn selection_sets_survive_saving_the_journal_and_its_snapshot() {
    let (mut document, base, other) = solid_model();
    let plain = encode(&document).unwrap();
    let change = Transaction::single(
        "Selection sets",
        Edit::SetSelectionSets {
            sets: Box::new(sets_of(base, other)),
        },
    );
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let recovered = journal::decode_journal(
        &journal::encode_journal(
            &journal::JournalHead {
                file: None,
                on_disk: None,
                loaded_with_problems: false,
                folded: 0,
            },
            &document,
            &[],
        )
        .unwrap(),
    )
    .unwrap();

    assert!(!plain.contains("\"selection_sets\""));
    assert!(text.contains("\"selection_sets\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn selection_sets_that_cannot_be_read_are_repaired_and_reported() {
    let (document, base, _) = solid_model();
    let body = base.raw();
    let sets = format!(
        r#"{{"selection_sets":{{"sets":[{{"name":"Bolts","members":[{{"body":{body}}},{{"face":{{"body":{body},"face":{{"face":"zz","origin":null,"neighbours":[]}}}}}},{{"cog":1}}]}},{{"name":"bolts","members":[{{"body":{body}}}]}},{{"name":"  ","members":[{{"body":{body}}}]}},{{"name":"Empty","members":[{{"edge":"broken"}}]}},{{"name":"Broken"}}]}}}}"#
    );
    let text = format!("{}\n{sets}", encode(&document).unwrap());

    let loaded = decode_text(&text);

    let names: Vec<&str> = loaded
        .document
        .selection_sets()
        .sets
        .iter()
        .map(|set| set.name.as_str())
        .collect();
    assert_eq!(names, ["Bolts", "bolts 2", "Set 3"]);
    assert_eq!(
        loaded.document.selection_sets().sets[0].members,
        vec![SetMember::Body(base)]
    );
    assert!(issues_mention(
        &loaded,
        "2 faces, edges or bodies in the selection set “Bolts” could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "Two selection sets were called “bolts”"
    ));
    assert!(issues_mention(&loaded, "A selection set had no name"));
    assert!(issues_mention(
        &loaded,
        "The selection set “Empty” held nothing that could be read"
    ));
    assert!(issues_mention(&loaded, "A selection set could not be read"));
}
