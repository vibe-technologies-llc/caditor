use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::{
    BodyOperation, CancelToken, Document, Evaluation, Extrude, ExtrudeExtent, FeatureId,
    FeatureKind, ModelEvaluator, Recompute, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::{ArcGeometry, BSpline, Constraint, Entity, Sketch};

use crate::import::{
    Drawing, DrawingCurve, ImportError, MAX_DRAWING_CURVES, SketchTarget, drawing_transaction,
    parse_dxf,
};

type Pairs = Vec<(i32, String)>;

fn pair(code: i32, value: impl ToString) -> (i32, String) {
    (code, value.to_string())
}

fn entity(kind: &str, layer: &str, rest: &[(i32, f64)]) -> Pairs {
    let mut pairs = vec![pair(0, kind), pair(8, layer)];
    pairs.extend(rest.iter().map(|(code, value)| pair(*code, value)));
    pairs
}

fn line(start: (f64, f64), end: (f64, f64)) -> Pairs {
    entity(
        "LINE",
        "0",
        &[
            (10, start.0),
            (20, start.1),
            (30, 0.0),
            (11, end.0),
            (21, end.1),
            (31, 0.0),
        ],
    )
}

fn section(name: &str, content: Vec<Pairs>) -> Pairs {
    let mut pairs = vec![pair(0, "SECTION"), pair(2, name)];
    pairs.extend(content.into_iter().flatten());
    pairs.push(pair(0, "ENDSEC"));
    pairs
}

fn header(units: Option<i64>) -> Pairs {
    let mut content = vec![pair(9, "$ACADVER"), pair(1, "AC1027")];
    if let Some(units) = units {
        content.extend([pair(9, "$INSUNITS"), pair(70, units)]);
    }
    section("HEADER", vec![content])
}

fn text(sections: Vec<Pairs>) -> Vec<u8> {
    let mut out = String::new();
    for (code, value) in sections.into_iter().flatten().chain([pair(0, "EOF")]) {
        out.push_str(&format!("{code:>3}\r\n{value}\r\n"));
    }
    out.into_bytes()
}

fn drawing(units: Option<i64>, entities: Vec<Pairs>) -> Drawing {
    parse_dxf(&text(vec![header(units), section("ENTITIES", entities)])).unwrap()
}

fn millimetre_drawing(entities: Vec<Pairs>) -> Drawing {
    drawing(Some(4), entities)
}

fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) < 1e-9
}

fn lines(drawing: &Drawing) -> Vec<(Point2, Point2)> {
    drawing
        .curves
        .iter()
        .filter_map(|curve| match curve {
            DrawingCurve::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect()
}

fn arcs(drawing: &Drawing) -> Vec<ArcGeometry> {
    drawing
        .curves
        .iter()
        .filter_map(|curve| match curve {
            DrawingCurve::Arc { center, start, end } => {
                Some(ArcGeometry::from_points(*center, *start, *end))
            }
            _ => None,
        })
        .collect()
}

fn slot() -> Pairs {
    entity(
        "LWPOLYLINE",
        "Outline",
        &[
            (90, 4.0),
            (70, 1.0),
            (10, 0.0),
            (20, 0.0),
            (10, 20.0),
            (20, 0.0),
            (42, 1.0),
            (10, 20.0),
            (20, 10.0),
            (10, 0.0),
            (20, 10.0),
            (42, 1.0),
        ],
    )
}

#[test]
fn a_closed_polyline_with_bulges_becomes_lines_and_arcs() {
    let drawing = millimetre_drawing(vec![slot()]);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
    assert_eq!(lines(&drawing).len(), 2);
    let arcs = arcs(&drawing);
    assert_eq!(arcs.len(), 2);
    let right = arcs
        .iter()
        .find(|arc| near(arc.center, Point2::new(20.0, 5.0)))
        .unwrap();
    assert!((right.radius - 5.0).abs() < 1e-12);
    assert!((right.sweep - PI).abs() < 1e-12);
    assert!((right.start_angle + FRAC_PI_2).abs() < 1e-12);
    let left = arcs
        .iter()
        .find(|arc| near(arc.center, Point2::new(0.0, 5.0)))
        .unwrap();
    assert!((left.start_angle - FRAC_PI_2).abs() < 1e-12);
}

fn evaluate(document: &Document) -> Evaluation {
    Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
}

fn sketch(document: &Document, feature: FeatureId) -> &Sketch {
    document.feature(feature).unwrap().kind.sketch().unwrap()
}

#[test]
fn an_imported_outline_is_joined_and_extrudes_into_a_solid() {
    let drawing = millimetre_drawing(vec![
        slot(),
        entity(
            "CIRCLE",
            "Holes",
            &[(10, 10.0), (20, 5.0), (30, 0.0), (40, 2.0)],
        ),
    ]);
    let mut document = Document::default();
    let import = drawing_transaction(
        &document,
        &drawing,
        SketchTarget::New {
            name: "slot".to_owned(),
            plane: Plane::XY,
        },
        "Import slot.dxf",
    );
    assert_eq!(import.curves, 5);
    assert_eq!(import.joints, 4);
    document.apply(import.transaction).unwrap();
    let imported = sketch(&document, import.sketch);
    let coincidences = imported
        .constraints()
        .filter(|(_, constraint)| matches!(constraint, Constraint::Coincident(..)))
        .count();
    assert_eq!(coincidences, 4);

    let mut transaction = document.transaction("Extrude");
    let body = transaction.add_feature(
        "Plate",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: import.sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::OneSide {
                distance: Expression::parse_stored("2 mm").unwrap(),
                reversed: false,
            },
            operation: BodyOperation::NewBody,
        })),
    );
    document.apply(transaction.finish()).unwrap();
    let evaluation = evaluate(&document);
    assert_eq!(evaluation.failed_count(), 0);
    let volume = evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume;
    let expected = 2.0 * (20.0 * 10.0 + PI * 25.0 - PI * 4.0);
    assert!((volume - expected).abs() < 0.05, "{volume} vs {expected}");
}

#[test]
fn importing_into_an_existing_sketch_keeps_its_contents() {
    let mut document = Document::default();
    let mut existing = Sketch::new(Plane::XY);
    existing.add_line(Point2::ZERO, Point2::new(5.0, 0.0));
    let mut transaction = document.transaction("Sketch");
    let feature = transaction.add_feature("Sketch 1", FeatureKind::from(existing));
    document.apply(transaction.finish()).unwrap();
    let before = sketch(&document, feature).entities().len();

    let drawing = millimetre_drawing(vec![line((5.0, 0.0), (5.0, 5.0))]);
    let import = drawing_transaction(
        &document,
        &drawing,
        SketchTarget::Existing(feature),
        "Import",
    );
    assert_eq!(import.sketch, feature);
    document.apply(import.transaction).unwrap();
    assert_eq!(sketch(&document, feature).entities().len(), before + 3);
}

#[test]
fn units_are_converted_to_millimetres_and_named() {
    let inches = drawing(Some(1), vec![line((0.0, 0.0), (1.0, 0.0))]);
    assert_eq!(lines(&inches), vec![(Point2::ZERO, Point2::new(25.4, 0.0))]);
    assert!(inches.notes[0].contains("inches"), "{:?}", inches.notes);

    let unknown = drawing(None, vec![line((0.0, 0.0), (1.0, 0.0))]);
    assert_eq!(lines(&unknown), vec![(Point2::ZERO, Point2::X)]);
    assert!(unknown.notes[0].contains("read as millimetres"));

    let unitless = drawing(Some(0), vec![line((0.0, 0.0), (1.0, 0.0))]);
    assert!(unitless.notes[0].contains("read as millimetres"));
}

fn block(name: &str, base: (f64, f64), content: Vec<Pairs>) -> Pairs {
    let mut pairs = vec![
        pair(0, "BLOCK"),
        pair(8, "0"),
        pair(2, name),
        pair(70, 0),
        pair(10, base.0),
        pair(20, base.1),
        pair(30, 0.0),
    ];
    pairs.extend(content.into_iter().flatten());
    pairs.extend([pair(0, "ENDBLK"), pair(8, "0")]);
    pairs
}

fn insert(name: &str, layer: &str, rest: &[(i32, f64)]) -> Pairs {
    let mut pairs = vec![pair(0, "INSERT"), pair(8, layer), pair(2, name)];
    pairs.extend(rest.iter().map(|(code, value)| pair(*code, value)));
    pairs
}

fn layer(name: &str, color: i64, flags: i64) -> Pairs {
    vec![
        pair(0, "LAYER"),
        pair(2, name),
        pair(70, flags),
        pair(62, color),
    ]
}

fn tables(layers: Vec<Pairs>) -> Pairs {
    let mut content = vec![vec![pair(0, "TABLE"), pair(2, "LAYER")]];
    content.extend(layers);
    content.push(vec![pair(0, "ENDTAB")]);
    section("TABLES", content)
}

#[test]
fn inserted_blocks_are_placed_rotated_scaled_and_repeated() {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![
                block("Tick", (1.0, 0.0), vec![line((1.0, 0.0), (2.0, 0.0))]),
                block(
                    "Pair",
                    (0.0, 0.0),
                    vec![insert("tick", "0", &[(10, 0.0), (20, 0.0), (30, 0.0)])],
                ),
            ],
        ),
        section(
            "ENTITIES",
            vec![
                insert(
                    "Tick",
                    "0",
                    &[(10, 10.0), (20, 0.0), (30, 0.0), (41, 2.0), (50, 90.0)],
                ),
                insert(
                    "Pair",
                    "0",
                    &[(10, 0.0), (20, 20.0), (30, 0.0), (70, 3.0), (44, 5.0)],
                ),
            ],
        ),
    ]);
    let drawing = parse_dxf(&bytes).unwrap();
    let found = lines(&drawing);
    assert_eq!(found.len(), 4);
    assert!(near(found[0].0, Point2::new(10.0, 0.0)));
    assert!(near(found[0].1, Point2::new(10.0, 2.0)));
    for (index, (start, end)) in found[1..].iter().enumerate() {
        let x = 5.0 * index as f64;
        assert!(near(*start, Point2::new(x, 20.0)));
        assert!(near(*end, Point2::new(x + 1.0, 20.0)));
    }
}

#[test]
fn hidden_layers_paper_space_and_annotations_are_left_out_with_a_note() {
    let mut paper = line((0.0, 0.0), (9.0, 9.0));
    paper.push(pair(67, 1));
    let bytes = text(vec![
        header(Some(4)),
        tables(vec![
            layer("Off", -7, 0),
            layer("Frozen", 7, 1),
            layer("Shown", 7, 0),
        ]),
        section(
            "BLOCKS",
            vec![block(
                "Mark",
                (0.0, 0.0),
                vec![line((0.0, 0.0), (1.0, 0.0))],
            )],
        ),
        section(
            "ENTITIES",
            vec![
                entity("LINE", "Off", &[(10, 0.0), (20, 0.0), (11, 1.0), (21, 1.0)]),
                entity(
                    "LINE",
                    "frozen",
                    &[(10, 0.0), (20, 0.0), (11, 1.0), (21, 1.0)],
                ),
                entity(
                    "LINE",
                    "Shown",
                    &[(10, 0.0), (20, 0.0), (11, 3.0), (21, 0.0)],
                ),
                insert("Mark", "Off", &[(10, 0.0), (20, 0.0)]),
                insert("Missing", "Shown", &[(10, 0.0), (20, 0.0)]),
                paper,
                vec![pair(0, "TEXT"), pair(8, "Shown"), pair(1, "Note")],
                vec![pair(0, "TEXT"), pair(8, "Shown"), pair(1, "Other")],
                vec![pair(0, "DIMENSION"), pair(8, "Shown")],
                entity("LINE", "Shown", &[(10, 0.0), (20, 0.0)]),
            ],
        ),
    ]);
    let drawing = parse_dxf(&bytes).unwrap();
    assert_eq!(lines(&drawing), vec![(Point2::ZERO, Point2::new(3.0, 0.0))]);
    let notes = drawing.notes.join("\n");
    assert!(
        notes.contains("1 dimension and 2 texts were left out"),
        "{notes}"
    );
    assert!(
        notes.contains("3 objects on hidden or frozen layers"),
        "{notes}"
    );
    assert!(
        notes.contains("does not contain were left out: Missing"),
        "{notes}"
    );
    assert!(
        notes.contains("1 damaged object could not be read"),
        "{notes}"
    );
}

#[test]
fn a_downward_arc_keeps_its_place_and_turns_the_right_way() {
    let drawing = millimetre_drawing(vec![entity(
        "ARC",
        "0",
        &[
            (10, 5.0),
            (20, 0.0),
            (30, 0.0),
            (40, 2.0),
            (50, 0.0),
            (51, 90.0),
            (210, 0.0),
            (220, 0.0),
            (230, -1.0),
        ],
    )]);
    let arcs = arcs(&drawing);
    assert_eq!(arcs.len(), 1);
    let arc = arcs[0];
    assert!(near(arc.center, Point2::new(-5.0, 0.0)));
    assert!((arc.sweep - FRAC_PI_2).abs() < 1e-12);
    assert!(near(arc.point_at(arc.start_angle), Point2::new(-5.0, 2.0)));
    assert!(near(arc.point_at(arc.end_angle()), Point2::new(-7.0, 0.0)));
}

#[test]
fn uniform_cubic_splines_are_kept_exactly_and_others_are_fitted() {
    let control = [(0.0, 0.0), (1.0, 2.0), (3.0, 2.0), (4.0, 0.0), (6.0, 1.0)];
    let mut exact = vec![pair(0, "SPLINE"), pair(8, "0"), pair(70, 8), pair(71, 3)];
    for knot in [0.0, 0.0, 0.0, 0.0, 5.0, 10.0, 10.0, 10.0, 10.0] {
        exact.push(pair(40, knot));
    }
    for (x, y) in control {
        exact.extend([pair(10, x), pair(20, y), pair(30, 0.0)]);
    }
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let mut rational = vec![pair(0, "SPLINE"), pair(8, "0"), pair(70, 12), pair(71, 2)];
    for knot in [0.0, 0.0, 0.0, 1.0, 1.0, 1.0] {
        rational.push(pair(40, knot));
    }
    for weight in [1.0, half, 1.0] {
        rational.push(pair(41, weight));
    }
    for (x, y) in [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        rational.extend([pair(10, x), pair(20, y)]);
    }
    let mut fit = vec![pair(0, "SPLINE"), pair(8, "0"), pair(71, 3)];
    for (x, y) in [(0.0, 20.0), (5.0, 25.0), (10.0, 20.0), (15.0, 25.0)] {
        fit.extend([pair(11, x), pair(21, y)]);
    }
    let bytes = text(vec![
        header(Some(4)),
        section("ENTITIES", vec![exact, rational, fit]),
    ]);
    let drawing = parse_dxf(&bytes).unwrap();
    let splines: Vec<&Vec<Point2>> = drawing
        .curves
        .iter()
        .filter_map(|curve| match curve {
            DrawingCurve::Spline { control_points } => Some(control_points),
            _ => None,
        })
        .collect();
    assert_eq!(splines.len(), 3);
    let expected: Vec<Point2> = control.iter().map(|(x, y)| Point2::new(*x, *y)).collect();
    assert_eq!(*splines[0], expected);

    let arc = BSpline::clamped(splines[1].clone()).unwrap();
    for step in 0..=100 {
        let point = arc.point_at(step as f64 / 100.0);
        assert!((point.length() - 10.0).abs() < 1e-4, "{point}");
    }
    let through = BSpline::clamped(splines[2].clone()).unwrap();
    assert!(near(through.point_at(0.0), Point2::new(0.0, 20.0)));
    assert!(near(through.point_at(1.0), Point2::new(15.0, 25.0)));

    let notes = drawing.notes.join("\n");
    assert!(
        notes.contains("1 spline was converted to sketch splines"),
        "{notes}"
    );
    assert!(
        notes.contains("1 spline given only by points on the curve was rebuilt"),
        "{notes}"
    );
}

#[test]
fn ellipses_become_splines_unless_they_are_circles() {
    let drawing = millimetre_drawing(vec![
        entity(
            "ELLIPSE",
            "0",
            &[(10, 0.0), (20, 0.0), (11, 20.0), (21, 0.0), (40, 0.5)],
        ),
        entity(
            "ELLIPSE",
            "0",
            &[
                (10, 50.0),
                (20, 0.0),
                (11, 0.0),
                (21, 5.0),
                (40, 1.0),
                (41, 0.0),
                (42, PI),
            ],
        ),
    ]);
    let spline = drawing
        .curves
        .iter()
        .find_map(|curve| match curve {
            DrawingCurve::Spline { control_points } => BSpline::clamped(control_points.clone()),
            _ => None,
        })
        .unwrap();
    for step in 0..=200 {
        let point = spline.point_at(step as f64 / 200.0);
        let on_ellipse = (point.x / 20.0).powi(2) + (point.y / 10.0).powi(2);
        assert!((on_ellipse - 1.0).abs() < 1e-4, "{point}");
    }
    let arcs = arcs(&drawing);
    assert_eq!(arcs.len(), 1);
    assert!(near(arcs[0].center, Point2::new(50.0, 0.0)));
    assert!((arcs[0].sweep - PI).abs() < 1e-9);
    assert!(near(
        arcs[0].point_at(arcs[0].start_angle),
        Point2::new(50.0, 5.0)
    ));
    assert!(drawing.notes.join(" ").contains("1 ellipse was converted"));
}

#[test]
fn old_style_polylines_and_three_dimensional_ones_are_read() {
    let mut pairs = vec![
        pair(0, "POLYLINE"),
        pair(8, "0"),
        pair(66, 1),
        pair(70, 1),
        pair(10, 0.0),
        pair(20, 0.0),
        pair(30, 0.0),
    ];
    for (x, y, bulge) in [(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0)] {
        pairs.extend([
            pair(0, "VERTEX"),
            pair(8, "0"),
            pair(10, x),
            pair(20, y),
            pair(30, 0.0),
            pair(42, bulge),
        ]);
    }
    pairs.extend([pair(0, "SEQEND"), pair(8, "0")]);
    let mut three = vec![pair(0, "POLYLINE"), pair(8, "0"), pair(70, 8)];
    for (x, y, z) in [(0.0, 20.0, 0.0), (5.0, 20.0, 3.0)] {
        three.extend([
            pair(0, "VERTEX"),
            pair(8, "0"),
            pair(10, x),
            pair(20, y),
            pair(30, z),
            pair(70, 32),
        ]);
    }
    three.extend([pair(0, "SEQEND")]);
    let drawing = millimetre_drawing(vec![pairs, three]);
    assert_eq!(lines(&drawing).len(), 4);
    assert!(drawing.notes.join(" ").contains("not flat"));
}

#[test]
fn a_binary_drawing_reads_like_a_text_one() {
    let mut bytes = b"AutoCAD Binary DXF\r\n\x1a\0".to_vec();
    let code = |bytes: &mut Vec<u8>, code: i16| bytes.extend(code.to_le_bytes());
    let string = |bytes: &mut Vec<u8>, text: &str| {
        bytes.extend(text.as_bytes());
        bytes.push(0);
    };
    code(&mut bytes, 0);
    string(&mut bytes, "SECTION");
    code(&mut bytes, 2);
    string(&mut bytes, "ENTITIES");
    code(&mut bytes, 0);
    string(&mut bytes, "LINE");
    code(&mut bytes, 8);
    string(&mut bytes, "0");
    for (group, value) in [
        (10, 1.0),
        (20, 2.0),
        (30, 0.0),
        (11, 4.0),
        (21, 6.0),
        (31, 0.0),
    ] {
        code(&mut bytes, group);
        bytes.extend(f64::to_le_bytes(value));
    }
    code(&mut bytes, 0);
    string(&mut bytes, "ENDSEC");
    code(&mut bytes, 0);
    string(&mut bytes, "EOF");
    let drawing = parse_dxf(&bytes).unwrap();
    assert_eq!(
        lines(&drawing),
        vec![(Point2::new(1.0, 2.0), Point2::new(4.0, 6.0))]
    );
}

#[test]
fn files_that_are_not_usable_drawings_are_refused_in_words() {
    assert_eq!(
        parse_dxf(b"\x89PNG\r\n\x1a\n\0\0"),
        Err(ImportError::NotDxf)
    );
    assert_eq!(
        parse_dxf(b"  0\nSECTION\n  2\nENTITIES\nnot a code\nLINE\n"),
        Err(ImportError::DamagedAt(5))
    );
    let only_text = text(vec![section(
        "ENTITIES",
        vec![vec![pair(0, "TEXT"), pair(8, "0"), pair(1, "Hello")]],
    )]);
    assert_eq!(parse_dxf(&only_text), Err(ImportError::Empty));
    let many: Vec<Pairs> = (0..=MAX_DRAWING_CURVES)
        .map(|index| line((index as f64, 0.0), (index as f64, 1.0)))
        .collect();
    let huge = text(vec![section("ENTITIES", many)]);
    assert_eq!(parse_dxf(&huge), Err(ImportError::TooLarge));
    let truncated = b"AutoCAD Binary DXF\r\n\x1a\0\0\0SECTION\0\x02\0ENT";
    assert_eq!(parse_dxf(truncated), Err(ImportError::Damaged));
}

#[test]
fn a_self_inserting_block_stops_with_a_note() {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![block(
                "Loop",
                (0.0, 0.0),
                vec![
                    line((0.0, 0.0), (1.0, 0.0)),
                    insert("Loop", "0", &[(10, 1.0), (20, 0.0)]),
                ],
            )],
        ),
        section(
            "ENTITIES",
            vec![insert("Loop", "0", &[(10, 0.0), (20, 0.0)])],
        ),
    ]);
    let drawing = parse_dxf(&bytes).unwrap();
    assert_eq!(lines(&drawing).len(), 1);
    assert!(drawing.notes.join(" ").contains("inside themselves"));
}

#[test]
fn short_curves_and_closed_arcs_are_tidied_before_they_reach_the_sketch() {
    let drawing = Drawing {
        curves: vec![
            DrawingCurve::Line {
                start: Point2::ZERO,
                end: Point2::new(100.0, 0.0),
            },
            DrawingCurve::Line {
                start: Point2::ZERO,
                end: Point2::new(1e-9, 0.0),
            },
            DrawingCurve::Arc {
                center: Point2::new(50.0, 50.0),
                start: Point2::new(60.0, 50.0),
                end: Point2::new(60.0, 50.0 - 1e-9),
            },
        ],
        notes: Vec::new(),
    };
    let document = Document::default();
    let import = drawing_transaction(
        &document,
        &drawing,
        SketchTarget::New {
            name: "tidy".to_owned(),
            plane: Plane::XY,
        },
        "Import",
    );
    assert_eq!(import.curves, 2);
    let mut document = document;
    document.apply(import.transaction).unwrap();
    let imported = sketch(&document, import.sketch);
    let circles = imported
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Circle { .. }))
        .count();
    assert_eq!(circles, 1);
}

mod step {
    use std::time::SystemTime;

    use caditor_document::{
        Blend, BlendKind, CancelToken, Document, FeatureKind, ModelEvaluator, Recompute,
    };
    use caditor_expression::Expression;
    use caditor_geometry::{Plane, Point2, Point3};
    use caditor_kernel::{
        EdgeReference, LinearExtent, Profile, ProfileCurve, SamplingTolerance, Selection, Solid,
        extrude,
    };
    use caditor_step::{StepBody, write_step};

    use crate::{
        decode, encode,
        import::{ImportError, bodies_transaction, parse_step},
    };

    fn block() -> Solid {
        let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 8.0), (0.0, 8.0)];
        let curves: Vec<ProfileCurve> = (0..4)
            .map(|index| {
                let (a, b) = (corners[index], corners[(index + 1) % 4]);
                ProfileCurve::line(
                    index as u64 + 1,
                    Point2::new(a.0, a.1),
                    Point2::new(b.0, b.1),
                )
            })
            .collect();
        let regions = Profile::new(&curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap();
        extrude(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(4.0).unwrap(),
            1,
        )
        .unwrap()
    }

    fn volume(solid: &Solid) -> f64 {
        solid
            .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
            .unwrap()
            .mass_properties()
            .volume
    }

    fn step_text() -> String {
        let solid = block();
        write_step(
            &[StepBody {
                name: "Block",
                solid: &solid,
            }],
            "block",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
    }

    #[test]
    fn an_imported_body_can_be_filleted_saved_and_loaded() {
        let import = parse_step(&step_text(), "block.step").unwrap();
        assert!(import.notes.is_empty(), "{:?}", import.notes);
        assert_eq!(import.bodies.len(), 1);
        let mut document = Document::default();
        let transaction = bodies_transaction(&document, &import.bodies, "Import block.step");
        document.apply(transaction).unwrap();
        let body = document.features().next().unwrap().id();
        assert_eq!(document.feature(body).unwrap().name, "Block");

        let mut engine = Recompute::default();
        let evaluation = engine.run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        assert_eq!(evaluation.failed_count(), 0);
        let solid = evaluation.body(body).unwrap();
        assert!((volume(solid) - 320.0).abs() < 1e-6);
        let top_edge = solid
            .edges()
            .find(|(_, edge)| {
                let middle = edge.curve().point(edge.interval().middle());
                middle.distance(Point3::new(5.0, 0.0, 4.0)) < 1e-9
            })
            .map(|(id, _)| id)
            .unwrap();
        let mut transaction = document.transaction("Fillet");
        let fillet = transaction.add_feature(
            "Fillet 1",
            FeatureKind::Blend(Blend {
                kind: BlendKind::Fillet,
                body,
                edges: vec![EdgeReference::capture(solid, top_edge).unwrap()],
                size: Expression::parse_stored("1 mm").unwrap(),
            }),
        );
        document.apply(transaction.finish()).unwrap();
        let evaluation = engine.run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        assert_eq!(evaluation.failed_count(), 0);
        let rounded = volume(evaluation.body(body).unwrap());
        let spandrel = (1.0 - std::f64::consts::PI / 4.0) * 10.0;
        assert!((rounded - (320.0 - spandrel)).abs() < 1e-2, "{rounded}");

        let loaded = decode(&encode(&document).unwrap()).unwrap();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(loaded.document, document);
        let mut fresh = Recompute::default();
        let evaluation = fresh.run(
            &loaded.document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        assert_eq!(evaluation.failed_count(), 0);
        assert!((volume(evaluation.body(body).unwrap()) - rounded).abs() < 1e-9);
        assert!(evaluation.feature(fillet).is_some());
    }

    #[test]
    fn several_bodies_get_distinct_names_and_other_files_are_refused() {
        let solid = block();
        let text = write_step(
            &[
                StepBody {
                    name: "Part",
                    solid: &solid,
                },
                StepBody {
                    name: "Part",
                    solid: &solid,
                },
            ],
            "parts",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let import = parse_step(&text, "parts.stp").unwrap();
        let mut document = Document::default();
        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let names: Vec<&str> = document
            .features()
            .map(|feature| feature.name.as_str())
            .collect();
        assert_eq!(names, ["Part", "Part 2"]);
        assert_eq!(
            parse_step("0\nSECTION\n", "x.step"),
            Err(ImportError::NotStep)
        );
        assert!(matches!(
            parse_step(
                "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n",
                "x.step"
            ),
            Err(ImportError::Model(_))
        ));
    }
}

#[test]
fn blocks_that_fan_out_into_millions_of_objects_are_refused_quickly() {
    let text_entity = vec![pair(0, "TEXT"), pair(8, "0"), pair(1, "note")];
    let mut blocks = vec![block("B0", (0.0, 0.0), vec![text_entity])];
    for level in 1..16 {
        let inner = format!("B{}", level - 1);
        let copies = (0..4)
            .map(|copy| insert(&inner, "0", &[(10, f64::from(copy)), (20, 0.0)]))
            .collect();
        blocks.push(block(&format!("B{level}"), (0.0, 0.0), copies));
    }
    let bytes = text(vec![
        header(Some(4)),
        section("BLOCKS", blocks),
        section(
            "ENTITIES",
            vec![insert("B15", "0", &[(10, 0.0), (20, 0.0)])],
        ),
    ]);
    let started = std::time::Instant::now();
    assert_eq!(parse_dxf(&bytes), Err(ImportError::TooManyObjects));
    assert!(started.elapsed().as_secs() < 10);
}

#[test]
fn splines_of_a_degree_above_the_kernel_limit_are_left_out() {
    let degree = 12;
    let count = degree + 1;
    let mut spline = vec![
        pair(0, "SPLINE"),
        pair(8, "0"),
        pair(70, 8),
        pair(71, degree),
    ];
    for index in 0..count + degree + 1 {
        spline.push(pair(40, if index <= degree { 0.0 } else { 1.0 }));
    }
    for index in 0..count {
        spline.extend([pair(10, index as f64), pair(20, 0.0), pair(30, 0.0)]);
    }
    let bytes = text(vec![
        header(Some(4)),
        section("ENTITIES", vec![spline, line((0.0, 0.0), (1.0, 0.0))]),
    ]);
    let drawing = parse_dxf(&bytes).unwrap();
    assert_eq!(drawing.curves.len(), 1);
    assert!(
        drawing.notes.join(" ").contains("could not be read"),
        "{:?}",
        drawing.notes
    );
}
