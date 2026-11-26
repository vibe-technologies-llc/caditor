use std::{collections::BTreeSet, fs};

use caditor_document::{
    AnnotationId, CentreMark, CentreTarget, DetailParent, DetailScale, DetailView,
    DimensionDirection, DimensionMeasure, DimensionTolerance, Drawing, DrawingCallout,
    DrawingDimension, DrawingId, DrawingNote, DrawingScale, Drawings, DrawnView, EdgePoint,
    EdgeSpot, IsoFit, NoteLeader, Projection, ScaleRatio, SectionPlace, SectionPlane, SectionView,
    SheetOrientation, SheetSize, StandardView,
};
use caditor_geometry::Point2;
use caditor_kernel::{EdgeName, FaceName};

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

pub(super) fn placed_drawings_of(base: FeatureId) -> Drawings {
    let mut drawings = detailed_drawings_of(base);

    if let Some(workshop) = drawings.drawings.first_mut() {
        workshop.positions = [
            (
                DrawnView::Standard(StandardView::Front),
                Point2::new(-20.5, 12.0),
            ),
            (
                DrawnView::Standard(StandardView::Left),
                Point2::new(60.0, 0.0),
            ),
            (DrawnView::Section('C'), Point2::new(40.0, -75.25)),
            (DrawnView::Detail('B'), Point2::new(-80.0, -60.0)),
        ]
        .into_iter()
        .collect();
    }
    drawings
}

pub(super) fn annotated_drawings_of(base: FeatureId) -> Drawings {
    let mut drawings = placed_drawings_of(base);

    if let Some(workshop) = drawings.drawings.first_mut() {
        let edge = |digest| EdgeName::from_digest(digest);

        workshop.dimensions = vec![
            DrawingDimension {
                id: AnnotationId::from_raw(0),
                view: DrawnView::Standard(StandardView::Front),
                measure: DimensionMeasure::Linear {
                    from: EdgePoint {
                        edge: edge(0xa1),
                        spot: EdgeSpot::Start,
                    },
                    to: EdgePoint {
                        edge: edge(0xa2),
                        spot: EdgeSpot::Centre,
                    },
                    direction: DimensionDirection::Horizontal,
                },
                place: Point2::new(5.0, 22.5),
                decimals: 1,
                text_offset: 0.0,
                tolerance: None,
            },
            DrawingDimension {
                id: AnnotationId::from_raw(2),
                view: DrawnView::Section('A'),
                measure: DimensionMeasure::Parallel {
                    first: edge(0xb1),
                    second: edge(0xb2),
                },
                place: Point2::new(-8.0, 0.0),
                decimals: 2,
                text_offset: 4.5,
                tolerance: None,
            },
            DrawingDimension {
                id: AnnotationId::from_raw(3),
                view: DrawnView::Detail('B'),
                measure: DimensionMeasure::Diameter(edge(0xc1)),
                place: Point2::new(14.0, -1.0),
                decimals: 3,
                text_offset: 0.0,
                tolerance: None,
            },
            DrawingDimension {
                id: AnnotationId::from_raw(4),
                view: DrawnView::Standard(StandardView::Left),
                measure: DimensionMeasure::Radius(edge(0xc2)),
                place: Point2::new(3.0, 3.0),
                decimals: 0,
                text_offset: 0.0,
                tolerance: None,
            },
            DrawingDimension {
                id: AnnotationId::from_raw(5),
                view: DrawnView::Standard(StandardView::Front),
                measure: DimensionMeasure::Angle {
                    first: edge(0xd1),
                    second: edge(0xd2),
                },
                place: Point2::new(1.0, 1.0),
                decimals: 2,
                text_offset: -2.0,
                tolerance: None,
            },
        ];
        workshop.notes = vec![
            DrawingNote {
                id: AnnotationId::from_raw(1),
                text: "Break all sharp edges".to_owned(),
                at: Point2::new(-90.0, -40.0),
                leader: None,
            },
            DrawingNote {
                id: AnnotationId::from_raw(6),
                text: "Ground face".to_owned(),
                at: Point2::new(70.0, 50.0),
                leader: Some(NoteLeader {
                    view: DrawnView::Standard(StandardView::Front),
                    edge: edge(0xe1),
                }),
            },
        ];
        workshop.next_annotation = 9;
    }
    drawings
}

pub(super) fn marked_drawings_of(base: FeatureId) -> Drawings {
    let mut drawings = annotated_drawings_of(base);

    if let Some(workshop) = drawings.drawings.first_mut() {
        let tolerances = [
            DimensionTolerance::Symmetric(0.1),
            DimensionTolerance::Deviations {
                upper: 0.02,
                lower: -0.01,
            },
            DimensionTolerance::Fit {
                fit: IsoFit::from_code("H7").unwrap(),
                deviations: true,
            },
            DimensionTolerance::Limits {
                upper: 0.5,
                lower: 0.0,
            },
        ];

        for (dimension, tolerance) in workshop.dimensions.iter_mut().zip(tolerances) {
            dimension.tolerance = Some(tolerance);
        }
        workshop.centres = vec![
            CentreMark {
                id: AnnotationId::from_raw(7),
                view: DrawnView::Standard(StandardView::Front),
                target: CentreTarget::Circle(EdgeName::from_digest(0xf1)),
            },
            CentreMark {
                id: AnnotationId::from_raw(8),
                view: DrawnView::Detail('B'),
                target: CentreTarget::Cylinder(FaceName::from_digest(0xf2)),
            },
        ];
        workshop.callouts = vec![DrawingCallout {
            id: AnnotationId::from_raw(9),
            view: DrawnView::Standard(StandardView::Left),
            feature: base,
            edge: EdgeName::from_digest(0xf3),
            at: Point2::new(-60.0, 30.5),
        }];
        workshop.next_annotation = 12;
    }
    drawings
}

pub(super) fn with_marks() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Centre marks, callouts and tolerances",
        Edit::SetDrawings {
            drawings: Box::new(marked_drawings_of(base)),
        },
    );
    (document, change)
}

pub(super) fn with_annotations() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Dimensions and notes",
        Edit::SetDrawings {
            drawings: Box::new(annotated_drawings_of(base)),
        },
    );
    (document, change)
}

pub(super) fn with_placed_views() -> (Document, Transaction) {
    let (document, base, _) = solid_model();
    let change = Transaction::single(
        "Placed views",
        Edit::SetDrawings {
            drawings: Box::new(placed_drawings_of(base)),
        },
    );
    (document, change)
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

#[test]
fn view_positions_survive_saving_and_the_journal() {
    let (mut document, change) = with_placed_views();

    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());

    assert!(text.contains("\"positions\""), "{text}");
    assert!(
        text.contains(r#"{"at":[40.0,-75.25],"view":{"section":"C"}}"#),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(loaded.document.drawings().drawings[0].positions.len(), 4);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn drawings_saved_before_views_were_placed_by_hand_load_laid_out_automatically() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/model-1");

    for name in [
        "drawings.caditor",
        "section-views.caditor",
        "detail-views.caditor",
        "section-parameter-offsets.caditor",
    ] {
        let bytes = fs::read(folder.join(name)).unwrap();
        let loaded = decode(&bytes).unwrap();
        let drawings = &loaded.document.drawings().drawings;

        assert_eq!(loaded.issues, Vec::<String>::new(), "{name}");
        assert!(!drawings.is_empty(), "{name}");
        assert!(
            drawings.iter().all(|drawing| drawing.positions.is_empty()),
            "{name}"
        );
    }

    let journal = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/journal-1/detail-views.journal"),
    )
    .unwrap();
    let contents = journal::decode_journal(&journal).unwrap();
    let replayed = journal::replay(contents.base, contents.entries);

    assert_eq!(contents.issues, Vec::<String>::new());
    assert!(
        replayed
            .editor
            .document()
            .drawings()
            .drawings
            .iter()
            .all(|drawing| drawing.positions.is_empty())
    );
}

#[test]
fn view_positions_that_cannot_be_used_are_left_out_and_reported() {
    let (document, _, _) = solid_model();
    let drawings = r#"{"drawings":{"drawings":[{"id":1,"name":"Shop","views":["front","top"],"sections":[{"letter":"A","parent":"front","plane":"yz"}],"positions":[{"view":{"standard":"front"},"at":[10.0,-5.0]},{"view":{"section":"A"},"at":[30.0,40.0]},{"view":{"standard":"front"},"at":[1.0,1.0]},{"view":{"standard":"right"},"at":[0.0,0.0]},{"view":{"detail":"B"},"at":[0.0,0.0]},{"view":{"standard":"top"},"at":[1e300,0.0]},{"view":{"sideways":"A"},"at":[0.0,0.0]},{"view":{"section":"AA"},"at":[0.0,0.0]},7]},{"id":2,"name":"Bench","views":["front"],"positions":[{"view":{"standard":"left"},"at":[0.0,0.0]}]}],"next_id":3}}"#;
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let shop = &loaded.document.drawings().drawings[0];

    assert_eq!(
        shop.positions
            .iter()
            .map(|(view, at)| (*view, *at))
            .collect::<Vec<_>>(),
        [
            (
                DrawnView::Standard(StandardView::Front),
                Point2::new(10.0, -5.0)
            ),
            (DrawnView::Section('A'), Point2::new(30.0, 40.0)),
        ]
    );
    assert!(loaded.document.drawings().drawings[1].positions.is_empty());
    assert!(issues_mention(
        &loaded,
        "Where 7 views of the drawing “Shop” were placed could not be used"
    ));
    assert!(issues_mention(
        &loaded,
        "Where a view of the drawing “Bench” was placed could not be used"
    ));
}

#[test]
fn dimensions_and_notes_survive_saving_and_the_journal() {
    let (mut document, change) = with_annotations();

    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let workshop = &loaded.document.drawings().drawings[0];

    assert!(text.contains("\"dimensions\""), "{text}");
    assert!(
        text.contains(r#""measure":{"diameter":"000000000000000000000000000000c1"}"#),
        "{text}"
    );
    assert!(
        text.contains(
            r#""leader":{"edge":"000000000000000000000000000000e1","view":{"standard":"front"}}"#
        ),
        "{text}"
    );
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(workshop.dimensions.len(), 5);
    assert_eq!(workshop.notes.len(), 2);
    assert_eq!(workshop.next_annotation, 9);
    assert!(loaded.document.drawings().drawings[1].dimensions.is_empty());
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn a_note_of_several_lines_keeps_its_newlines_through_saving_and_the_journal() {
    let (mut document, base, _) = solid_model();
    let mut drawings = annotated_drawings_of(base);

    if let Some(note) = drawings
        .drawings
        .first_mut()
        .and_then(|drawing| drawing.notes.first_mut())
    {
        note.text = "Break all\nsharp edges\n\nPaint grey".to_owned();
    }

    let change = Transaction::single(
        "Several lines",
        Edit::SetDrawings {
            drawings: Box::new(drawings),
        },
    );

    document.apply(change.clone()).unwrap();

    let loaded = decode_text(&encode(&document).unwrap());
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(
        loaded.document.drawings().drawings[0].notes[0].text,
        "Break all\nsharp edges\n\nPaint grey"
    );
    assert_eq!(format::restore_transaction(journaled), Some(change));
}

#[test]
fn drawings_saved_before_dimensions_load_with_none() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/model-1");

    for name in [
        "drawings.caditor",
        "section-views.caditor",
        "detail-views.caditor",
        "placed-views.caditor",
        "section-parameter-offsets.caditor",
    ] {
        let bytes = fs::read(folder.join(name)).unwrap();
        let loaded = decode(&bytes).unwrap();
        let drawings = &loaded.document.drawings().drawings;

        assert_eq!(loaded.issues, Vec::<String>::new(), "{name}");
        assert!(!drawings.is_empty(), "{name}");
        assert!(
            drawings
                .iter()
                .all(|drawing| drawing.dimensions.is_empty() && drawing.notes.is_empty()),
            "{name}"
        );
    }

    let journal = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/journal-1/placed-views.journal"),
    )
    .unwrap();
    let contents = journal::decode_journal(&journal).unwrap();
    let replayed = journal::replay(contents.base, contents.entries);

    assert_eq!(contents.issues, Vec::<String>::new());
    assert!(
        replayed
            .editor
            .document()
            .drawings()
            .drawings
            .iter()
            .all(|drawing| drawing.dimensions.is_empty() && drawing.next_annotation == 0)
    );
}

#[test]
fn dimensions_and_notes_that_cannot_be_read_are_left_out_and_reported() {
    let (document, _, _) = solid_model();
    let drawings = r#"{"drawings":{"drawings":[{"id":1,"name":"Shop","views":["front"],"dimensions":[{"id":4,"view":{"standard":"front"},"measure":{"radius":"ab"},"place":[1.0,2.0],"decimals":9},{"id":4,"view":{"standard":"front"},"measure":{"radius":"cd"},"place":[0.0,0.0]},{"id":5,"view":{"sideways":"A"},"measure":{"radius":"cd"},"place":[0.0,0.0]},{"id":6,"view":{"standard":"top"},"measure":{"bend":"cd"},"place":[0.0,0.0]},{"id":7,"view":{"standard":"top"},"measure":{"radius":"not hex"},"place":[0.0,0.0]},{"id":8,"view":{"standard":"top"},"measure":{"angle":{"first":"1","second":"2"}},"place":[1e300,0.0]},{"id":9,"view":{"standard":"right"},"measure":{"linear":{"from":{"edge":"1","spot":"start"},"to":{"edge":"2","spot":"end"},"direction":"askew"}},"place":[3.0,4.0]},12],"notes":[{"id":10,"text":"  Keep\n dry ","at":[0.0,0.0],"leader":{"view":{"standard":"front"},"edge":"zz"}},{"id":11,"text":"   ","at":[0.0,0.0]},{"id":12,"text":"Far","at":[5000.0,0.0]}]}],"next_id":2}}"#;
    let text = format!("{}\n{drawings}", encode(&document).unwrap());

    let loaded = decode_text(&text);
    let shop = &loaded.document.drawings().drawings[0];

    assert_eq!(
        shop.dimensions
            .iter()
            .map(|dimension| dimension.id.raw())
            .collect::<Vec<_>>(),
        [4, 9]
    );
    assert_eq!(shop.dimensions[0].decimals, 4);
    assert_eq!(
        shop.dimensions[1].measure,
        DimensionMeasure::Linear {
            from: EdgePoint {
                edge: EdgeName::from_digest(1),
                spot: EdgeSpot::Start,
            },
            to: EdgePoint {
                edge: EdgeName::from_digest(2),
                spot: EdgeSpot::End,
            },
            direction: DimensionDirection::Aligned,
        }
    );
    assert_eq!(shop.notes.len(), 1);
    assert_eq!(shop.notes[0].text, "Keep\ndry");
    assert_eq!(shop.notes[0].leader, None);
    assert_eq!(shop.next_annotation, 11);
    assert!(issues_mention(
        &loaded,
        "6 dimensions of the drawing “Shop” could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "2 notes of the drawing “Shop” could not be read"
    ));
    assert!(issues_mention(&loaded, "asked for more than 4 decimals"));
    assert!(issues_mention(&loaded, "stands without it"));
}

#[test]
fn centre_marks_callouts_and_tolerances_survive_saving_and_the_journal() {
    let (mut document, change) = with_marks();

    let undo = document.apply(change.clone()).unwrap();

    let text = encode(&document).unwrap();
    let loaded = decode_text(&text);
    let journaled: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&change)).unwrap());
    let undone: format::TransactionRecord =
        through_binary(&serde_json::to_string(&format::transaction_record(&undo)).unwrap());
    let workshop = &loaded.document.drawings().drawings[0];

    assert!(
        text.contains(r#""tolerance":{"fit":{"deviations":true,"fit":"H7"}}"#),
        "{text}"
    );
    assert!(
        text.contains(r#""target":{"cylinder":"000000000000000000000000000000f2"}"#),
        "{text}"
    );
    assert!(text.contains(r#""callouts":[{"at":[-60.0,30.5]"#), "{text}");
    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document, document);
    assert_eq!(
        workshop
            .dimensions
            .iter()
            .filter(|dimension| dimension.tolerance.is_some())
            .count(),
        4
    );
    assert_eq!(workshop.centres.len(), 2);
    assert_eq!(workshop.callouts.len(), 1);
    assert_eq!(workshop.next_annotation, 12);
    assert_eq!(format::restore_transaction(journaled), Some(change));
    assert_eq!(format::restore_transaction(undone), Some(undo));
}

#[test]
fn drawings_saved_before_centre_marks_callouts_and_tolerances_load_without_them() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let bytes = fs::read(corpus.join("model-1/dimensions-and-notes.caditor")).unwrap();
    let loaded = decode(&bytes).unwrap();
    let journal = fs::read(corpus.join("journal-1/dimensions-and-notes.journal")).unwrap();
    let contents = journal::decode_journal(&journal).unwrap();
    let replayed = journal::replay(contents.base, contents.entries);

    let unmarked = |drawings: &Drawings| {
        drawings.drawings.iter().all(|drawing| {
            drawing.centres.is_empty()
                && drawing.callouts.is_empty()
                && drawing
                    .dimensions
                    .iter()
                    .all(|dimension| dimension.tolerance.is_none())
        })
    };

    assert_eq!(loaded.issues, Vec::<String>::new());
    assert_eq!(loaded.document.drawings().drawings[0].dimensions.len(), 5);
    assert!(unmarked(loaded.document.drawings()));
    assert_eq!(contents.issues, Vec::<String>::new());
    assert_eq!(
        replayed.editor.document().drawings().drawings[0]
            .dimensions
            .len(),
        5
    );
    assert!(unmarked(replayed.editor.document().drawings()));
}

#[test]
fn centre_marks_callouts_and_tolerances_that_cannot_be_read_are_left_out_and_reported() {
    let (document, base, _) = solid_model();
    let body = base.raw();
    let drawings = format!(
        r#"{{"drawings":{{"drawings":[{{"id":1,"name":"Shop","views":["front"],"dimensions":[{{"id":1,"view":{{"standard":"front"}},"measure":{{"radius":"ab"}},"place":[1.0,2.0],"tolerance":{{"fit":{{"fit":"Q9"}}}}}},{{"id":2,"view":{{"standard":"front"}},"measure":{{"angle":{{"first":"1","second":"2"}}}},"place":[1.0,2.0],"tolerance":{{"fit":{{"fit":"H7"}}}}}},{{"id":3,"view":{{"standard":"front"}},"measure":{{"radius":"ab"}},"place":[1.0,2.0],"tolerance":{{"deviations":{{"upper":-1.0,"lower":1.0}}}}}},{{"id":4,"view":{{"standard":"front"}},"measure":{{"radius":"ab"}},"place":[1.0,2.0],"tolerance":{{"symmetric":0.05}}}}],"centres":[{{"id":5,"view":{{"standard":"front"}},"target":{{"circle":"f1"}}}},{{"id":5,"view":{{"standard":"front"}},"target":{{"circle":"f2"}}}},{{"id":6,"view":{{"standard":"front"}},"target":{{"ring":"f2"}}}},7],"callouts":[{{"id":8,"view":{{"standard":"front"}},"feature":{body},"edge":"f3","at":[1.0,1.0]}},{{"id":9,"view":{{"standard":"front"}},"feature":{body},"edge":"f3","at":[9000.0,1.0]}},{{"id":10,"view":{{"standard":"front"}},"feature":18446744073709551615,"edge":"f3","at":[1.0,1.0]}}]}}],"next_id":2}}}}"#
    );
    let text = format!(
        "{}
{drawings}",
        encode(&document).unwrap()
    );

    let loaded = decode_text(&text);
    let shop = &loaded.document.drawings().drawings[0];
    let tolerances: Vec<Option<DimensionTolerance>> = shop
        .dimensions
        .iter()
        .map(|dimension| dimension.tolerance)
        .collect();

    assert_eq!(
        tolerances,
        [None, None, None, Some(DimensionTolerance::Symmetric(0.05))]
    );
    assert_eq!(shop.centres.len(), 1);
    assert_eq!(shop.callouts.len(), 1);
    assert_eq!(shop.callouts[0].feature, base);
    assert_eq!(shop.next_annotation, 9);
    assert!(issues_mention(
        &loaded,
        "The tolerance of a dimension of the drawing “Shop” could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "3 centre marks and centrelines of the drawing “Shop” could not be read"
    ));
    assert!(issues_mention(
        &loaded,
        "2 callouts of the drawing “Shop” could not be read"
    ));
}
