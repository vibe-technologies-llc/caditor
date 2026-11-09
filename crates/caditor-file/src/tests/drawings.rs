use std::{collections::BTreeSet, fs};

use caditor_document::{
    Drawing, DrawingId, DrawingScale, Drawings, Projection, ScaleRatio, SheetOrientation,
    SheetSize, StandardView,
};

use super::*;

pub(super) fn drawings_of(base: FeatureId) -> Drawings {
    let mut workshop = Drawing::new(DrawingId::from_raw(0), "Workshop".to_owned());
    workshop.size = SheetSize::A4;
    workshop.orientation = SheetOrientation::Portrait;
    workshop.projection = Projection::ThirdAngle;
    workshop.scale = DrawingScale::Ratio(ScaleRatio::new(1, 2));
    workshop.views = [StandardView::Front, StandardView::Left]
        .into_iter()
        .collect::<BTreeSet<_>>();
    workshop.bodies = vec![base];
    workshop.hidden_lines = false;
    let overview = Drawing::new(DrawingId::from_raw(3), "Overview".to_owned());
    Drawings {
        drawings: vec![workshop, overview],
        next_id: 5,
    }
}

pub(super) fn with_drawings() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Drawings",
        Edit::SetDrawings {
            drawings: Box::new(drawings_of(base)),
        },
    );
    (document, change)
}

#[test]
fn drawings_survive_saving_the_journal_and_its_snapshot() {
    let (mut document, change) = with_drawings();
    let plain = encode(&document).unwrap();
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

    assert!(!plain.contains("\"drawings\""));
    assert!(text.contains("\"drawings\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.document.drawings().next_id, 5);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
    assert_eq!(recovered.issues, Vec::<String>::new());
    assert_eq!(recovered.base, document);
}

#[test]
fn models_saved_before_drawings_load_with_none() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/model-1");
    let older = ["document.caditor", "bodies.caditor", "faces.caditor"];

    for name in older {
        let bytes = fs::read(folder.join(name)).unwrap();
        let loaded = decode(&bytes).unwrap();

        assert_eq!(loaded.issues, Vec::<String>::new(), "{name}");
        assert!(loaded.document.drawings().is_empty(), "{name}");
    }
}

#[test]
fn drawings_that_cannot_be_read_are_repaired_and_reported() {
    let (document, base, _) = solid_model();
    let body = base.raw();
    let drawings = format!(
        r#"{{"drawings":{{"drawings":[{{"id":1,"name":"Shop","size":"a9","orientation":"sideways","projection":"fourth_angle","scale":{{"ratio":{{"sheet":0,"model":2}}}},"views":["front","bottom"],"bodies":[{body}]}},{{"id":1,"name":"Again"}},{{"id":2,"name":"shop","scale":"warped"}},{{"id":4,"name":"  "}},{{"name":"No number"}}],"next_id":2}}}}"#
    );
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let restored = loaded.document.drawings();
    let names: Vec<&str> = restored
        .drawings
        .iter()
        .map(|drawing| drawing.name.as_str())
        .collect();
    let shop = &restored.drawings[0];

    assert_eq!(names, ["Shop", "shop 2", "Drawing 3"]);
    assert_eq!(shop.size, SheetSize::A3);
    assert_eq!(shop.orientation, SheetOrientation::Landscape);
    assert_eq!(shop.projection, Projection::FirstAngle);
    assert_eq!(shop.scale, DrawingScale::Fit);
    assert_eq!(shop.views, BTreeSet::from([StandardView::Front]));
    assert_eq!(shop.bodies, vec![base]);
    assert!(shop.hidden_lines);
    assert_eq!(restored.next_id, 5);
    assert!(issues_mention(
        &loaded,
        "The sheet size of the drawing “Shop”"
    ));
    assert!(issues_mention(
        &loaded,
        "The projection of the drawing “Shop”"
    ));
    assert!(issues_mention(
        &loaded,
        "The scale 0:2 of the drawing “Shop”"
    ));
    assert!(issues_mention(&loaded, "1 views of the drawing “Shop”"));
    assert!(issues_mention(&loaded, "Two drawings had the same number"));
    assert!(issues_mention(&loaded, "Two drawings were called “shop”"));
    assert!(issues_mention(&loaded, "The scale of the drawing “shop 2”"));
    assert!(issues_mention(&loaded, "A drawing had no name"));
    assert!(issues_mention(&loaded, "A drawing could not be read"));
}
