use caditor_document::{Document, FeatureKind, Import};
use caditor_kernel::Solid;

use super::*;
use crate::step_cache::tests::block_text;

fn imported(texts: &[&str]) -> Document {
    let mut document = Document::default();
    let mut transaction = document.transaction("Import");
    for (index, text) in texts.iter().enumerate() {
        transaction.add_feature(
            format!("Block {index}"),
            FeatureKind::Import(Import::new("block.step", Solid::default(), *text)),
        );
    }
    document.apply(transaction.finish()).unwrap();
    document
}

fn wrapped(kind: FeatureKindRecord) -> FeatureKindRecord {
    let scaled = FeatureKindRecord::ScaledImport(Box::new(ScaledImportRecord {
        feature: kind,
        scale: "2".to_owned(),
    }));
    let framed = FeatureKindRecord::ImportInFrame(Box::new(MoveInFrameRecord {
        feature: scaled,
        frame: 1,
    }));
    FeatureKindRecord::SeparatePieces(Box::new(SeparatePiecesRecord { feature: framed }))
}

#[test]
fn every_distinct_import_text_is_read_ahead_once_however_its_record_is_wrapped() {
    let texts = [block_text(3.0), block_text(4.0), block_text(5.0)];
    let document = imported(&[&texts[0], &texts[1], &texts[0], &texts[2]]);
    let mut records: Vec<FeatureRecord> = feature_records(&document).collect();

    if let Some(last) = records.last_mut() {
        last.kind = wrapped(last.kind.clone());
    }
    let ahead = ImportTexts::reading_ahead(&records);
    let mut issues = Vec::new();
    let restored: Vec<Feature> = {
        let mut ahead = ImportTexts::reading_ahead(&records);
        records
            .iter()
            .map(|record| restore_feature_sharing(record, &mut ahead, &mut issues))
            .collect()
    };

    assert_eq!(ahead.read_ahead.len(), 3);
    assert!(
        ahead
            .read_ahead
            .values()
            .all(|read| read.as_ref().is_ok_and(Option::is_some))
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert!(restored.iter().all(|feature| {
        feature
            .kind
            .import()
            .is_some_and(|import| import.solid.faces().count() == 6)
    }));
}

#[test]
fn texts_read_ahead_and_texts_an_import_stores_are_marked_checked() {
    let loaded = block_text(6.0);
    let document = imported(&[&loaded]);
    let records: Vec<FeatureRecord> = feature_records(&document).collect();
    let solid = crate::step_cache::first_solid(&block_text(7.0))
        .unwrap()
        .unwrap();

    let unread = crate::checked_texts::is_checked(&blake3::hash(loaded.as_bytes()));
    ImportTexts::reading_ahead(&records);
    let stored = crate::import::canonical("Peg", &solid).unwrap();

    assert!(!unread);
    assert!(crate::checked_texts::is_checked(&blake3::hash(
        loaded.as_bytes()
    )));
    assert!(
        stored
            .iter()
            .all(|(_, step)| crate::checked_texts::is_checked(&blake3::hash(step.as_bytes())))
    );
}
