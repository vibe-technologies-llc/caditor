use std::{collections::BTreeSet, fs};

use caditor_document::{
    DetailParent, DetailScale, DetailView, Drawing, DrawingId, DrawingScale, Drawings, Projection,
    ScaleRatio, SectionPlace, SectionPlane, SectionView, SheetOrientation, SheetSize, StandardView,
};
use caditor_geometry::Point2;

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

pub(super) fn sectioned_drawings_of(base: FeatureId) -> Drawings {
    let mut drawings = drawings_of(base);
    if let Some(workshop) = drawings.drawings.first_mut() {
        workshop.sections = vec![
            SectionView {
                letter: 'A',
                parent: StandardView::Front,
                plane: SectionPlane::Yz,
                place: SectionPlace::Centre,
                reversed: false,
            },
            SectionView {
                letter: 'C',
                parent: StandardView::Left,
                plane: SectionPlane::Xy,
                place: SectionPlace::offset(-2.5),
                reversed: true,
            },
        ];
    }
    drawings
}

pub(super) fn detailed_drawings_of(base: FeatureId) -> Drawings {
    let mut drawings = sectioned_drawings_of(base);
    if let Some(workshop) = drawings.drawings.first_mut() {
        workshop.details = vec![
            DetailView {
                letter: 'B',
                parent: DetailParent::Standard(StandardView::Front),
                centre: Point2::new(12.5, -3.25),
                radius: 4.0,
                scale: DetailScale::Enlarged,
            },
            DetailView {
                letter: 'D',
                parent: DetailParent::Section('A'),
                centre: Point2::new(0.0, 1.5),
                radius: 0.75,
                scale: DetailScale::Ratio(ScaleRatio::new(10, 1)),
            },
        ];
    }
    drawings
}

pub(super) fn with_detail_views() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Detail views",
        Edit::SetDrawings {
            drawings: Box::new(detailed_drawings_of(base)),
        },
    );
    (document, change)
}

pub(super) fn with_section_views() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Section views",
        Edit::SetDrawings {
            drawings: Box::new(sectioned_drawings_of(base)),
        },
    );
    (document, change)
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

#[test]
fn section_views_survive_saving_and_the_journal() {
    let (mut document, change) = with_section_views();
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());

    assert!(text.contains("\"sections\""), "{text}");
    assert!(text.contains("\"offset\":-2.5"), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn drawings_saved_before_section_views_load_with_none() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/model-1");
    let bytes = fs::read(folder.join("drawings.caditor")).unwrap();

    let loaded = decode(&bytes).unwrap();

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document.drawings().drawings.len(), 2);
    assert!(
        loaded
            .document
            .drawings()
            .drawings
            .iter()
            .all(|drawing| drawing.sections.is_empty())
    );
}

#[test]
fn section_views_that_cannot_be_read_are_repaired_and_reported() {
    let (document, _, _) = solid_model();
    let drawings = r#"{"drawings":{"drawings":[{"id":1,"name":"Shop","views":["front"],"sections":[{"letter":"A","parent":"front","plane":"xz","place":{"offset":3.0}},{"letter":"A","parent":"bottom","plane":"yz","place":"anywhere","reversed":true},{"letter":"Q","parent":"top","plane":"tilted","place":{"offset":1e300}},{"parent":"front"},7]}],"next_id":2}}"#;
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let sections = &loaded.document.drawings().drawings[0].sections;
    let letters: Vec<char> = sections.iter().map(|section| section.letter).collect();

    assert_eq!(letters, ['A', 'B', 'C']);
    assert_eq!(sections[0].plane, SectionPlane::Yz);
    assert_eq!(sections[0].place, SectionPlace::offset(3.0));
    assert_eq!(sections[1].parent, StandardView::Front);
    assert_eq!(sections[1].place, SectionPlace::Centre);
    assert!(sections[1].reversed);
    assert_eq!(sections[2].parent, StandardView::Top);
    assert_eq!(sections[2].plane, SectionPlane::Yz);
    assert_eq!(sections[2].place, SectionPlace::Centre);
    assert!(issues_mention(&loaded, "The plane of section A\u{2013}A"));
    assert!(issues_mention(&loaded, "so it is B\u{2013}B"));
    assert!(issues_mention(
        &loaded,
        "The view section B\u{2013}B is cut on"
    ));
    assert!(issues_mention(&loaded, "Where section B\u{2013}B cuts"));
    assert!(issues_mention(&loaded, "Where section C\u{2013}C cuts"));
    assert!(issues_mention(
        &loaded,
        "A section view of the drawing “Shop” could not be read"
    ));
}

pub(super) fn parameter_offset_document() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let offset = document.parse("depth + 2 mm").unwrap();
    let mut drawings = drawings_of(base);
    if let Some(workshop) = drawings.drawings.first_mut() {
        workshop.sections = vec![SectionView {
            letter: 'D',
            parent: StandardView::Top,
            plane: SectionPlane::Xz,
            place: SectionPlace::Offset(offset),
            reversed: false,
        }];
    }
    let change = Transaction::single(
        "Section views",
        Edit::SetDrawings {
            drawings: Box::new(drawings),
        },
    );
    (document, change)
}

#[test]
fn a_section_offset_naming_a_parameter_survives_saving_and_the_journal() {
    let (mut document, change) = parameter_offset_document();
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());

    assert!(text.contains("\"expression\":\"$0 + 2 mm\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn a_section_offset_naming_a_missing_parameter_is_repaired_and_reported() {
    let (document, _, _) = solid_model();
    let drawings = r#"{"drawings":{"drawings":[{"id":1,"name":"Shop","views":["front"],"sections":[{"letter":"A","parent":"front","plane":"xz","place":{"expression":"$7 + 2 mm"}},{"letter":"B","parent":"front","plane":"xz","place":{"expression":"(("}},{"letter":"C","parent":"front","plane":"xz","place":{"expression":"$0 * 2"}}]}],"next_id":2}}"#;
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let sections = &loaded.document.drawings().drawings[0].sections;

    assert_eq!(sections[0].place, SectionPlace::Centre);
    assert_eq!(sections[1].place, SectionPlace::Centre);
    assert!(matches!(sections[2].place, SectionPlace::Offset(_)));
    assert!(issues_mention(&loaded, "Where section A\u{2013}A cuts"));
    assert!(issues_mention(&loaded, "Where section B\u{2013}B cuts"));
}

#[test]
fn detail_views_survive_saving_and_the_journal() {
    let (mut document, change) = with_detail_views();
    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());

    assert!(text.contains("\"details\""), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn drawings_saved_before_detail_views_load_with_none() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/model-1");

    for name in ["drawings.caditor", "section-views.caditor"] {
        let bytes = fs::read(folder.join(name)).unwrap();
        let loaded = decode(&bytes).unwrap();

        assert_eq!(loaded.issues, Vec::<String>::new(), "{name}");
        assert!(
            loaded
                .document
                .drawings()
                .drawings
                .iter()
                .all(|drawing| drawing.details.is_empty()),
            "{name}"
        );
    }
}

#[test]
fn detail_views_that_cannot_be_read_are_repaired_and_reported() {
    let (document, _, _) = solid_model();
    let drawings = r#"{"drawings":{"drawings":[{"id":1,"name":"Shop","views":["front"],"sections":[{"letter":"A","parent":"front","plane":"yz"}],"details":[{"letter":"A","parent":{"standard":"front"},"centre":[1.0,2.0],"radius":3.0,"scale":"warped"},{"letter":"B","parent":{"section":"A"},"centre":[0.0,0.0],"radius":1.0,"scale":{"ratio":{"sheet":0,"model":1}}},{"letter":"C","parent":{"section":"Z"},"centre":[0.0,0.0],"radius":1.0},{"letter":"D","parent":{"standard":"underneath"},"centre":[0.0,0.0],"radius":1.0},{"letter":"E","parent":{"standard":"top"},"centre":[0.0,0.0],"radius":0.0},{"letter":"F","parent":"upwards","centre":[0.0,0.0],"radius":1.0},{"letter":"G","parent":{"standard":"top"},"centre":[1e300,0.0],"radius":1.0},{"letter":"","parent":{"standard":"right"},"centre":[0.0,0.0],"radius":2.0},7]}],"next_id":2}}"#;
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let details = &loaded.document.drawings().drawings[0].details;
    let letters: Vec<char> = details.iter().map(|detail| detail.letter).collect();

    assert_eq!(letters, ['B', 'C', 'D']);
    assert_eq!(
        details[0].parent,
        DetailParent::Standard(StandardView::Front)
    );
    assert_eq!(details[0].centre, Point2::new(1.0, 2.0));
    assert_eq!(details[0].scale, DetailScale::Enlarged);
    assert_eq!(details[1].parent, DetailParent::Section('A'));
    assert_eq!(details[1].scale, DetailScale::Enlarged);
    assert_eq!(
        details[2].parent,
        DetailParent::Standard(StandardView::Right)
    );
    assert_eq!(details[2].radius, 2.0);
    assert!(issues_mention(&loaded, "so it is B"));
    assert!(issues_mention(
        &loaded,
        "The scale of detail view B was not understood"
    ));
    assert!(issues_mention(
        &loaded,
        "The scale 0:1 of detail view C cannot be used"
    ));
    assert!(issues_mention(&loaded, "so it is D"));
    assert!(issues_mention(&loaded, "The view detail"));
    assert!(issues_mention(&loaded, "The circle of detail view"));
    assert!(issues_mention(
        &loaded,
        "A detail view of the drawing “Shop” could not be read"
    ));
}
