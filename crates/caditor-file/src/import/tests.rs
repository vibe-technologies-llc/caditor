use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_document::{
    BodyOperation, CancelToken, Document, Evaluation, Extrude, ExtrudeExtent, FeatureId,
    FeatureKind, ModelEvaluator, Recompute, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_kernel::{MAX_SPLINE_DEGREE, SamplingTolerance};
use caditor_sketch::{ArcGeometry, BSpline, Constraint, Entity, Sketch};

use crate::import::{
    Drawing, DrawingCurve, DrawingOptions, DrawingUnit, ImportError, MAX_DRAWING_CURVES,
    MAX_READ_CURVES, MAX_SCALE, MIN_SCALE, SketchTarget, drawing_transaction, parse_dxf,
};

pub(super) type Pairs = Vec<(i32, String)>;

pub(super) fn pair(code: i32, value: impl ToString) -> (i32, String) {
    (code, value.to_string())
}

pub(super) fn entity(kind: &str, layer: &str, rest: &[(i32, f64)]) -> Pairs {
    let mut pairs = vec![pair(0, kind), pair(8, layer)];
    pairs.extend(rest.iter().map(|(code, value)| pair(*code, value)));
    pairs
}

pub(super) fn line(start: (f64, f64), end: (f64, f64)) -> Pairs {
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

pub(super) fn section(name: &str, content: Vec<Pairs>) -> Pairs {
    let mut pairs = vec![pair(0, "SECTION"), pair(2, name)];
    pairs.extend(content.into_iter().flatten());
    pairs.push(pair(0, "ENDSEC"));
    pairs
}

pub(super) fn header(units: Option<i64>) -> Pairs {
    let mut content = vec![pair(9, "$ACADVER"), pair(1, "AC1027")];
    if let Some(units) = units {
        content.extend([pair(9, "$INSUNITS"), pair(70, units)]);
    }
    section("HEADER", vec![content])
}

pub(super) fn text(sections: Vec<Pairs>) -> Vec<u8> {
    let mut out = String::new();
    for (code, value) in sections.into_iter().flatten().chain([pair(0, "EOF")]) {
        out.push_str(&format!("{code:>3}\r\n{value}\r\n"));
    }
    out.into_bytes()
}

pub(super) fn drawing(units: Option<i64>, entities: Vec<Pairs>) -> Drawing {
    parse_dxf(&text(vec![header(units), section("ENTITIES", entities)])).unwrap()
}

pub(super) fn millimetre_drawing(entities: Vec<Pairs>) -> Drawing {
    drawing(Some(4), entities)
}

pub(super) fn near(a: Point2, b: Point2) -> bool {
    a.distance(b) < 1e-9
}

pub(super) fn lines(drawing: &Drawing) -> Vec<(Point2, Point2)> {
    drawing
        .curves
        .iter()
        .filter_map(|curve| match curve {
            DrawingCurve::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect()
}

pub(super) fn arcs(drawing: &Drawing) -> Vec<ArcGeometry> {
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

#[test]
fn a_closed_polyline_of_two_bulged_vertices_is_a_whole_circle() {
    let circle = entity(
        "LWPOLYLINE",
        "Outline",
        &[
            (90, 2.0),
            (70, 1.0),
            (10, 0.0),
            (20, 0.0),
            (42, 1.0),
            (10, 10.0),
            (20, 0.0),
            (42, 1.0),
        ],
    );
    let lens = entity(
        "LWPOLYLINE",
        "Outline",
        &[
            (90, 2.0),
            (70, 1.0),
            (10, 0.0),
            (20, 20.0),
            (42, 0.5),
            (10, 10.0),
            (20, 20.0),
        ],
    );
    let flat = entity(
        "LWPOLYLINE",
        "Outline",
        &[
            (90, 2.0),
            (70, 1.0),
            (10, 0.0),
            (20, 40.0),
            (10, 10.0),
            (20, 40.0),
        ],
    );

    let drawing = millimetre_drawing(vec![circle, lens, flat]);
    let halves: Vec<ArcGeometry> = arcs(&drawing)
        .into_iter()
        .filter(|arc| near(arc.center, Point2::new(5.0, 0.0)))
        .collect();

    assert_eq!(halves.len(), 2);
    assert!(halves.iter().all(|arc| (arc.sweep - PI).abs() < 1e-12));
    assert_eq!(arcs(&drawing).len(), 3);
    assert_eq!(lines(&drawing).len(), 2);
}

pub(super) fn evaluate(document: &Document) -> Evaluation {
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
            extent: ExtrudeExtent::one_side(Expression::parse_stored("2 mm").unwrap(), false),
            operation: BodyOperation::NewBody,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
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

    let survey = drawing(Some(21), vec![line((0.0, 0.0), (3_937.0, 0.0))]);
    let (_, end) = lines(&survey)[0];
    assert!((end.x - 1_200_000.0).abs() < 1e-6, "{end}");
    assert!(
        survey.notes[0].contains("US survey feet"),
        "{:?}",
        survey.notes
    );

    let future = drawing(Some(99), vec![line((0.0, 0.0), (1.0, 0.0))]);
    assert_eq!(lines(&future), vec![(Point2::ZERO, Point2::X)]);
    assert_eq!(
        future.notes[0],
        "The drawing names a unit caditor does not know (code 99), so its numbers were read as \
         millimetres."
    );

    for units in [None, Some(0)] {
        let mut content = vec![pair(9, "$MEASUREMENT"), pair(70, 0)];
        if let Some(units) = units {
            content.extend([pair(9, "$INSUNITS"), pair(70, units)]);
        }
        let imperial = parse_dxf(&text(vec![
            section("HEADER", vec![content]),
            section("ENTITIES", vec![line((0.0, 0.0), (1.0, 0.0))]),
        ]))
        .unwrap();
        assert_eq!(
            lines(&imperial),
            vec![(Point2::ZERO, Point2::new(25.4, 0.0))]
        );
        assert!(imperial.notes[0].contains("inches"), "{:?}", imperial.notes);
    }
    let metric_named = parse_dxf(&text(vec![
        section(
            "HEADER",
            vec![vec![
                pair(9, "$MEASUREMENT"),
                pair(70, 0),
                pair(9, "$INSUNITS"),
                pair(70, 4),
            ]],
        ),
        section("ENTITIES", vec![line((0.0, 0.0), (1.0, 0.0))]),
    ]))
    .unwrap();
    assert_eq!(lines(&metric_named), vec![(Point2::ZERO, Point2::X)]);
}

pub(super) fn block(name: &str, base: (f64, f64), content: Vec<Pairs>) -> Pairs {
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

pub(super) fn insert(name: &str, layer: &str, rest: &[(i32, f64)]) -> Pairs {
    let mut pairs = vec![pair(0, "INSERT"), pair(8, layer), pair(2, name)];
    pairs.extend(rest.iter().map(|(code, value)| pair(*code, value)));
    pairs
}

pub(super) fn layer(name: &str, color: i64, flags: i64) -> Pairs {
    vec![
        pair(0, "LAYER"),
        pair(2, name),
        pair(70, flags),
        pair(62, color),
    ]
}

pub(super) fn tables(layers: Vec<Pairs>) -> Pairs {
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
        notes.contains("3 objects on hidden, frozen or non-plotting layers"),
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
fn ellipses_stay_ellipses_unless_they_are_circles() {
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
    let ellipse = drawing
        .curves
        .iter()
        .find(|curve| matches!(curve, DrawingCurve::Ellipse { .. }))
        .unwrap();
    assert_eq!(
        ellipse,
        &DrawingCurve::Ellipse {
            center: Point2::ZERO,
            major: Vector2::new(20.0, 0.0),
            minor_radius: 10.0,
            ends: None,
        }
    );
    let arcs = arcs(&drawing);
    assert_eq!(arcs.len(), 1);
    assert!(near(arcs[0].center, Point2::new(50.0, 0.0)));
    assert!((arcs[0].sweep - PI).abs() < 1e-9);
    assert!(near(
        arcs[0].point_at(arcs[0].start_angle),
        Point2::new(50.0, 5.0)
    ));
    assert!(!drawing.notes.join(" ").contains("converted"));
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
fn a_drawing_with_carriage_return_line_endings_is_read() {
    let crlf = text(vec![
        header(Some(4)),
        section("ENTITIES", vec![line((1.0, 2.0), (4.0, 6.0))]),
    ]);
    let classic = String::from_utf8(crlf).unwrap().replace("\r\n", "\r");

    let drawing = parse_dxf(classic.as_bytes()).unwrap();

    assert_eq!(
        lines(&drawing),
        vec![(Point2::new(1.0, 2.0), Point2::new(4.0, 6.0))]
    );
}

#[test]
fn dimension_definition_points_on_the_defpoints_layer_are_left_out() {
    let drawing = millimetre_drawing(vec![
        entity(
            "LINE",
            "Defpoints",
            &[(10, 0.0), (20, 0.0), (11, 5.0), (21, 0.0)],
        ),
        entity("LINE", "0", &[(10, 0.0), (20, 0.0), (11, 0.0), (21, 7.0)]),
    ]);

    assert_eq!(lines(&drawing), vec![(Point2::ZERO, Point2::new(0.0, 7.0))]);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("1 object on hidden, frozen or non-plotting layers"))
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
    assert_eq!(
        parse_dxf(&only_text),
        Err(ImportError::Empty {
            left_out: vec![
                "1 text was left out, because sketches hold only points, lines, arcs, circles \
                 and splines."
                    .to_owned()
            ]
        })
    );
    let many: Vec<Pairs> = (0..=MAX_DRAWING_CURVES)
        .map(|index| line((index as f64, 0.0), (index as f64, 1.0)))
        .collect();
    let huge = text(vec![section("ENTITIES", many)]);
    let read = parse_dxf(&huge).unwrap();
    let kept = read.arranged(&DrawingOptions::default());
    assert_eq!(read.curves.len(), MAX_DRAWING_CURVES + 1);
    assert_eq!(kept.curves.len(), MAX_DRAWING_CURVES);
    assert!(
        kept.notes.contains(&format!(
            "Only the first {MAX_DRAWING_CURVES} curves were imported, because a sketch \
                 holds at most that many; 1 more was left out. Leave out layers or split the \
                 drawing to import the rest."
        )),
        "{:?}",
        kept.notes
    );
    let truncated = b"AutoCAD Binary DXF\r\n\x1a\0\0\0SECTION\0\x02\0ENT";
    assert_eq!(parse_dxf(truncated), Err(ImportError::Damaged));
}

#[test]
fn damage_part_way_through_keeps_what_comes_before_it_and_says_so() {
    let intact = String::from_utf8(text(vec![
        header(Some(4)),
        section(
            "ENTITIES",
            vec![
                line((0.0, 0.0), (5.0, 0.0)),
                line((5.0, 0.0), (5.0, 5.0)),
                line((5.0, 5.0), (0.0, 5.0)),
                line((0.0, 5.0), (0.0, 0.0)),
            ],
        ),
    ]))
    .unwrap();
    let last = intact.rfind("  0\r\nLINE").unwrap();
    let damaged = format!("{}zz{}", &intact[..last], &intact[last + 2..]);
    let line_of_damage = damaged[..last].matches("\r\n").count() + 1;

    let drawing = parse_dxf(damaged.as_bytes()).unwrap();

    assert_eq!(
        lines(&drawing),
        vec![
            (Point2::new(0.0, 0.0), Point2::new(5.0, 0.0)),
            (Point2::new(5.0, 0.0), Point2::new(5.0, 5.0)),
        ]
    );
    assert!(
        drawing.notes.contains(&format!(
            "The drawing is damaged near line {line_of_damage}, so only what comes before it was \
             imported."
        )),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn a_binary_drawing_cut_short_keeps_its_complete_entities() {
    let mut bytes = b"AutoCAD Binary DXF\r\n\x1a\0".to_vec();
    let record = |bytes: &mut Vec<u8>, code: u8, value: &[u8]| {
        bytes.push(code);
        bytes.extend_from_slice(value);
    };
    record(&mut bytes, 0, b"SECTION\0");
    record(&mut bytes, 2, b"ENTITIES\0");
    record(&mut bytes, 0, b"LINE\0");
    record(&mut bytes, 8, b"0\0");
    for (code, value) in [(10, 0.0f64), (20, 0.0), (11, 3.0), (21, 4.0)] {
        record(&mut bytes, code, &value.to_le_bytes());
    }
    record(&mut bytes, 0, b"LINE\0");
    bytes.push(10);
    bytes.extend_from_slice(&[0, 0, 0]);

    let drawing = parse_dxf(&bytes).unwrap();

    assert_eq!(
        lines(&drawing),
        vec![(Point2::new(0.0, 0.0), Point2::new(3.0, 4.0))]
    );
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("The drawing is damaged,"))
    );
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
        ..Drawing::default()
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

fn linetype(name: &str, elements: i64) -> Pairs {
    vec![
        pair(0, "LTYPE"),
        pair(2, name),
        pair(70, 0),
        pair(73, elements),
    ]
}

fn styled_tables(linetypes: Vec<Pairs>, layers: Vec<(&str, &str)>) -> Pairs {
    let mut content = vec![vec![pair(0, "TABLE"), pair(2, "LTYPE")]];
    content.extend(linetypes);
    content.push(vec![pair(0, "ENDTAB")]);
    content.push(vec![pair(0, "TABLE"), pair(2, "LAYER")]);
    for (name, linetype) in layers {
        let mut record = layer(name, 7, 0);
        record.push(pair(6, linetype));
        content.push(record);
    }
    content.push(vec![pair(0, "ENDTAB")]);
    section("TABLES", content)
}

fn with_linetype(mut entity: Pairs, linetype: &str) -> Pairs {
    entity.push(pair(6, linetype));
    entity
}

fn on_layer(mut entity: Pairs, layer: &str) -> Pairs {
    for (code, value) in &mut entity {
        if *code == 8 {
            *value = layer.to_owned();
        }
    }
    entity
}

fn styled_drawing(entities: Vec<Pairs>, blocks: Vec<Pairs>) -> Drawing {
    parse_dxf(&text(vec![
        header(Some(4)),
        styled_tables(
            vec![
                linetype("Continuous", 0),
                linetype("Hidden", 2),
                linetype("Center", 4),
            ],
            vec![
                ("0", "Continuous"),
                ("Axes", "Center"),
                ("Walls", "Continuous"),
            ],
        ),
        section("BLOCKS", blocks),
        section("ENTITIES", entities),
    ]))
    .unwrap()
}

#[test]
fn curves_drawn_in_a_dashed_linetype_are_imported_as_construction_geometry() {
    let bar = |y: f64| line((0.0, y), (9.0, y));
    let drawing = styled_drawing(
        vec![
            bar(0.0),
            with_linetype(bar(1.0), "HIDDEN"),
            with_linetype(bar(2.0), "continuous"),
            on_layer(bar(3.0), "Axes"),
            with_linetype(on_layer(bar(4.0), "Axes"), "ByLayer"),
            with_linetype(on_layer(bar(5.0), "Axes"), "Continuous"),
            on_layer(bar(6.0), "Walls"),
            with_linetype(bar(7.0), "Missing"),
        ],
        Vec::new(),
    );

    let ys: Vec<f64> = lines(&drawing).iter().map(|(start, _)| start.y).collect();
    let dashed: Vec<f64> = drawing
        .construction
        .iter()
        .map(|index| lines(&drawing)[*index].0.y)
        .collect();

    assert_eq!(ys, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    assert_eq!(dashed, [1.0, 3.0, 4.0]);
    assert!(
        drawing.notes.contains(
            &"3 curves with a dashed or centre linetype were imported as construction geometry, \
              which forms no regions."
                .to_owned()
        ),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn a_block_drawn_by_block_takes_the_dashing_of_the_insert_that_places_it() {
    let tick = |name: &str, linetype: &str| {
        block(
            name,
            (0.0, 0.0),
            vec![with_linetype(line((0.0, 0.0), (1.0, 0.0)), linetype)],
        )
    };
    let at = |name: &str, y: f64, linetype: &str| {
        with_linetype(
            insert(name, "0", &[(10, 0.0), (20, y), (30, 0.0)]),
            linetype,
        )
    };
    let drawing = styled_drawing(
        vec![
            at("Inherits", 0.0, "Hidden"),
            at("Inherits", 1.0, "Continuous"),
            at("Own", 2.0, "Hidden"),
        ],
        vec![tick("Inherits", "ByBlock"), tick("Own", "Continuous")],
    );

    let dashed: Vec<f64> = drawing
        .construction
        .iter()
        .map(|index| lines(&drawing)[*index].0.y)
        .collect();

    assert_eq!(dashed, [0.0]);
}

#[test]
fn construction_curves_stay_construction_in_the_sketch_and_their_points_do_not() {
    let drawing = styled_drawing(
        vec![
            line((0.0, 0.0), (9.0, 0.0)),
            with_linetype(line((0.0, 5.0), (9.0, 5.0)), "Center"),
        ],
        Vec::new(),
    );
    let document = Document::default();

    let import = drawing_transaction(
        &document,
        &drawing,
        SketchTarget::New {
            name: "plan".to_owned(),
            plane: Plane::XY,
        },
        "Import",
    );
    let mut document = document;
    document.apply(import.transaction).unwrap();
    let imported = sketch(&document, import.sketch);

    let construction: Vec<&Entity> = imported
        .entities()
        .filter(|(id, _)| imported.is_construction(*id))
        .map(|(_, entity)| entity)
        .collect();
    assert!(matches!(construction.as_slice(), [Entity::Line { .. }]));
    assert_eq!(imported.entities().count(), 6);
}

mod step {
    use std::time::SystemTime;

    use caditor_document::{
        Blend, BlendKind, CancelToken, Document, FeatureKind, ModelEvaluator, Recompute, Rgb,
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
        import::{ImportError, bodies_transaction, parse_step, read_step_file},
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
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
            }],
            "block",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
    }

    #[test]
    fn a_coloured_body_imports_with_its_colour_and_a_plain_one_without() {
        let solid = block();
        let text = write_step(
            &[
                StepBody {
                    name: "Red",
                    solid: &solid,
                    colour: Some([200, 30, 40]),
                    opacity: None,
                    layer: None,
                    threads: &[],
                },
                StepBody {
                    name: "Plain",
                    solid: &solid,
                    colour: None,
                    opacity: None,
                    layer: None,
                    threads: &[],
                },
            ],
            "pair",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let import = parse_step(&text, "pair.step").unwrap();
        let mut document = Document::default();

        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let colour = |name: &str| {
            document
                .features()
                .find(|feature| feature.name == name)
                .unwrap()
                .appearance
                .colour
        };

        assert_eq!(colour("Red"), Some(Rgb::new(200, 30, 40)));
        assert_eq!(colour("Plain"), None);
    }

    #[test]
    fn a_see_through_body_imports_at_the_nearest_opacity_step_and_a_nearly_solid_one_solid() {
        let solid = block();
        let body = |name, opacity| StepBody {
            name,
            solid: &solid,
            colour: Some([200, 30, 40]),
            opacity,
            layer: None,
            threads: &[],
        };
        let text = write_step(
            &[
                body("Acrylic", Some(30)),
                body("Smoked", Some(70)),
                body("Almost", Some(92)),
                body("Plain", None),
            ],
            "panels",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let import = parse_step(&text, "panels.step").unwrap();
        let mut document = Document::default();

        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let appearance = |name: &str| {
            document
                .features()
                .find(|feature| feature.name == name)
                .unwrap()
                .appearance
                .clone()
        };

        assert_eq!(appearance("Acrylic").opacity, Some(25));
        assert_eq!(appearance("Smoked").opacity, Some(75));
        assert_eq!(appearance("Almost").opacity, None);
        assert_eq!(appearance("Almost").colour, Some(Rgb::new(200, 30, 40)));
        assert_eq!(appearance("Plain").opacity, None);
    }

    #[test]
    fn faces_of_two_colours_and_a_see_through_one_import_onto_their_faces_and_are_saved() {
        let mut text = step_text();
        let faces: Vec<String> = text
            .lines()
            .filter_map(|line| line.split_once("=ADVANCED_FACE("))
            .map(|(id, _)| id.to_owned())
            .collect();
        let mut styles = String::from(
            "#900001=COLOUR_RGB('',0.,0.,1.);\n\
             #900002=FILL_AREA_STYLE_COLOUR('',#900001);\n\
             #900003=FILL_AREA_STYLE('',(#900002));\n\
             #900004=SURFACE_STYLE_FILL_AREA(#900003);\n\
             #900005=SURFACE_SIDE_STYLE('',(#900004));\n\
             #900006=SURFACE_STYLE_USAGE(.BOTH.,#900005);\n\
             #900007=PRESENTATION_STYLE_ASSIGNMENT((#900006));\n\
             #900011=COLOUR_RGB('',1.,0.,0.);\n\
             #900012=SURFACE_STYLE_TRANSPARENT(0.5);\n\
             #900013=SURFACE_STYLE_RENDERING_WITH_PROPERTIES(.NORMAL_SHADING.,#900011,(#900012));\n\
             #900015=SURFACE_SIDE_STYLE('',(#900013));\n\
             #900016=SURFACE_STYLE_USAGE(.BOTH.,#900015);\n\
             #900017=PRESENTATION_STYLE_ASSIGNMENT((#900016));\n",
        );
        for (index, face) in faces.iter().enumerate() {
            let style = if index % 2 == 0 { "#900017" } else { "#900007" };
            styles.push_str(&format!(
                "#{}=STYLED_ITEM('',({style}),{face});\n",
                910_000 + index
            ));
        }
        let end = text.rfind("ENDSEC;").unwrap();
        text.insert_str(end, &styles);
        let import = parse_step(&text, "block.step").unwrap();
        let mut document = Document::default();
        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let id = document.features().next().unwrap().id();
        let evaluation = evaluated(&document);
        let solid = evaluation.body(id).unwrap();
        let appearance = &document.feature(id).unwrap().appearance;
        let colours = appearance.face_colours(solid);
        let opacities = appearance.face_opacities(solid);
        let loaded = decode(&encode(&document).unwrap()).unwrap();

        assert_eq!(faces.len(), 6);
        assert_eq!((appearance.colour, appearance.opacity), (None, None));
        for (index, (face, _)) in solid.faces().enumerate() {
            let expected = if index % 2 == 0 {
                (Some(Rgb::new(255, 0, 0)), Some(50))
            } else {
                (Some(Rgb::new(0, 0, 255)), None)
            };
            assert_eq!(
                (colours.get(&face).copied(), opacities.get(&face).copied()),
                expected,
                "face {index}"
            );
        }
        assert_eq!(loaded.issues, Vec::<String>::new());
        assert_eq!(loaded.document.feature(id).unwrap().appearance, *appearance);
    }

    #[test]
    fn bodies_on_one_layer_import_together_into_a_folder_named_after_it() {
        let solid = block();
        let body = |name, layer| StepBody {
            name,
            solid: &solid,
            colour: None,
            opacity: None,
            layer,
            threads: &[],
        };
        let text = write_step(
            &[
                body("Bolt", Some("Hardware")),
                body("Plate", None),
                body("Nut", Some("Hardware")),
            ],
            "parts",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let import = parse_step(&text, "parts.step").unwrap();
        let mut document = Document::default();

        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let tree: Vec<(&str, Option<&str>)> = document
            .features()
            .map(|feature| (feature.name.as_str(), feature.group.as_deref()))
            .collect();

        assert_eq!(
            tree,
            [
                ("Bolt", Some("Hardware")),
                ("Nut", Some("Hardware")),
                ("Plate", None)
            ]
        );
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

    const ASSEMBLY: &str = include_str!("../../../caditor-step/src/read/samples/assembly.step");
    const SECOND_PIN: &str = "#9001 = AXIS2_PLACEMENT_3D('',#9002,#21,#22);\n\
         #9002 = CARTESIAN_POINT('',(30.,50.,0.));\n\
         #9003 = CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#9004,#9006);\n\
         #9004 = ( REPRESENTATION_RELATIONSHIP('','',#228,#10) \
         REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#9005) \
         SHAPE_REPRESENTATION_RELATIONSHIP() );\n\
         #9005 = ITEM_DEFINED_TRANSFORMATION('','',#11,#9001);\n\
         #9006 = PRODUCT_DEFINITION_SHAPE('Placement','Placement of an item',#9007);\n\
         #9007 = NEXT_ASSEMBLY_USAGE_OCCURRENCE('4','Pin','',#5,#223,$);\n\
         ENDSEC;\nEND-ISO-10303-21;";

    fn evaluated(document: &Document) -> caditor_document::Evaluation {
        Recompute::default().run(document, &ModelEvaluator, &CancelToken::never(), &|_, _| {})
    }

    fn spans(
        evaluation: &caditor_document::Evaluation,
        body: caditor_document::FeatureId,
        low: [f64; 3],
        high: [f64; 3],
    ) -> bool {
        let bounds = evaluation.body(body).unwrap().bounding_box().unwrap();
        bounds.min().distance(Point3::from_array(low)) < 1e-6
            && bounds.max().distance(Point3::from_array(high)) < 1e-6
    }

    #[test]
    fn an_assembly_stores_each_part_once_and_places_its_copies() {
        let text = ASSEMBLY.replace("ENDSEC;\nEND-ISO-10303-21;", SECOND_PIN);
        let import = parse_step(&text, "assembly.step").unwrap();
        let names: Vec<&str> = import
            .bodies
            .iter()
            .map(|body| body.name.as_str())
            .collect();
        let [block, pin, second] = &import.bodies[..] else {
            panic!("{names:?}");
        };
        let mut document = Document::default();
        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let ids: Vec<_> = document.features().map(|feature| feature.id()).collect();
        let evaluation = evaluated(&document);
        let encoded = encode(&document).unwrap();
        let loaded = decode(&encoded).unwrap();
        let reloaded = evaluated(&loaded.document);

        assert!(import.notes.is_empty(), "{:?}", import.notes);
        assert_eq!(names, ["Block", "Pin", "Pin 2"]);
        assert!(std::sync::Arc::ptr_eq(
            &pin.import.step,
            &second.import.step
        ));
        assert!(std::sync::Arc::ptr_eq(
            &pin.import.solid,
            &second.import.solid
        ));
        assert!(!block.import.placement.is_at_origin());
        assert!(!second.import.placement.is_at_origin());
        assert_eq!(evaluation.failed_count(), 0);
        assert!(spans(
            &evaluation,
            ids[0],
            [80.0, 0.0, 0.0],
            [100.0, 10.0, 5.0]
        ));
        assert!(spans(
            &evaluation,
            ids[1],
            [-2.0, 20.0, -2.0],
            [2.0, 50.0, 2.0]
        ));
        assert!(spans(
            &evaluation,
            ids[2],
            [28.0, 20.0, -2.0],
            [32.0, 50.0, 2.0]
        ));
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(loaded.document, document);
        assert_eq!(reloaded.failed_count(), 0);
        assert!(spans(
            &reloaded,
            ids[2],
            [28.0, 20.0, -2.0],
            [32.0, 50.0, 2.0]
        ));
        let held: Vec<_> = loaded
            .document
            .features()
            .filter_map(|feature| feature.kind.import())
            .collect();
        assert!(std::sync::Arc::ptr_eq(&held[1].step, &held[2].step));
        assert!(std::sync::Arc::ptr_eq(&held[1].solid, &held[2].solid));
    }

    #[test]
    fn a_copy_mirrored_by_its_assembly_is_stored_as_placed() {
        let text = ASSEMBLY
            .replace(
                "#48 = ITEM_DEFINED_TRANSFORMATION('','',#11,#15);",
                "#48 = CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',#18,#9100,#16,1.,#17);",
            )
            .replace(
                "ENDSEC;\nEND-ISO-10303-21;",
                "#9100=DIRECTION('',(1.,0.,0.));\nENDSEC;\nEND-ISO-10303-21;",
            );
        let import = parse_step(&text, "assembly.step").unwrap();
        let mut document = Document::default();
        document
            .apply(bodies_transaction(&document, &import.bodies, "Import"))
            .unwrap();
        let block = document.features().next().unwrap().id();
        let evaluation = evaluated(&document);

        assert!(import.notes.is_empty(), "{:?}", import.notes);
        assert!(import.bodies[0].import.placement.is_at_origin());
        assert_eq!(evaluation.failed_count(), 0);
        assert!(spans(
            &evaluation,
            block,
            [100.0, 0.0, 0.0],
            [120.0, 10.0, 5.0]
        ));
    }

    #[test]
    fn a_surface_model_of_several_closed_shells_is_imported_as_a_body_per_shell() {
        let square = |first: u64, x: f64| -> Vec<ProfileCurve> {
            let corners = [(x, 0.0), (x + 2.0, 0.0), (x + 2.0, 2.0), (x, 2.0)];
            (0..4)
                .map(|index| {
                    let (a, b) = (corners[index], corners[(index + 1) % 4]);
                    ProfileCurve::line(
                        first + index as u64,
                        Point2::new(a.0, a.1),
                        Point2::new(b.0, b.1),
                    )
                })
                .collect()
        };
        let curves: Vec<ProfileCurve> = square(1, 0.0).into_iter().chain(square(5, 5.0)).collect();
        let regions = Profile::new(&curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap();
        let pair = extrude(
            &Plane::XY,
            &regions,
            LinearExtent::one_side(1.0).unwrap(),
            1,
        )
        .unwrap();
        let written = write_step(
            &[StepBody {
                name: "Pair",
                solid: &pair,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
            }],
            "pair",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let breps: Vec<(String, String)> = written
            .lines()
            .filter_map(|line| {
                let (id, rest) = line.split_once("=MANIFOLD_SOLID_BREP('Pair',")?;
                Some((id.to_owned(), rest.trim_end_matches(");").to_owned()))
            })
            .collect();
        let [(first, first_shell), (second, second_shell)] = breps.as_slice() else {
            panic!("expected two breps in {written}");
        };
        let text = written
            .replace(
                &format!("{first}=MANIFOLD_SOLID_BREP('Pair',{first_shell});"),
                &format!(
                    "{first}=SHELL_BASED_SURFACE_MODEL('Pair',({first_shell},{second_shell}));"
                ),
            )
            .replace(
                &format!("{second}=MANIFOLD_SOLID_BREP('Pair',{second_shell});"),
                &format!("{second}=CARTESIAN_POINT('',(0.,0.,0.));"),
            );

        let import = parse_step(&text, "pair.step").unwrap();

        assert!(import.notes.is_empty(), "{:?}", import.notes);
        assert_eq!(import.bodies.len(), 2);
        for body in &import.bodies {
            assert!((volume(&body.import.solid) - 4.0).abs() < 1e-6);
            assert_eq!(body.import.solid.shells().count(), 1);
        }
    }

    fn block_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("block.step");
        std::fs::write(&path, written_block()).unwrap();
        path
    }

    #[test]
    fn a_cancelled_step_read_stops_with_the_import_stopped() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = block_file(&dir);
        let cancelled = CancelToken::new(|| true);

        assert_eq!(
            read_step_file(&path, &cancelled).map(|_| ()),
            Err(ImportError::Cancelled)
        );
        assert_eq!(
            read_step_file(&path, &CancelToken::never())
                .unwrap()
                .bodies
                .len(),
            1
        );
    }

    #[test]
    fn a_step_read_notices_a_cancel_raised_while_it_is_running() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = block_file(&dir);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let polls = std::sync::Arc::new(polls);
        let seen = std::sync::Arc::clone(&polls);
        let cancel =
            CancelToken::new(move || seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2);

        assert_eq!(
            read_step_file(&path, &cancel).map(|_| ()),
            Err(ImportError::Cancelled)
        );
        assert!(polls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    }

    #[test]
    fn a_latin_1_file_is_read_with_its_names_and_says_so() {
        let solid = block();
        let text = write_step(
            &[StepBody {
                name: "Part",
                solid: &solid,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
            }],
            "parts",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap();
        let latin_1: Vec<u8> = text
            .replace("'Part'", "'Pi\u{e8}ce'")
            .chars()
            .map(|letter| u8::try_from(u32::from(letter)).unwrap())
            .collect();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("piece.step");
        std::fs::write(&path, &latin_1).unwrap();

        let import = read_step_file(&path, &CancelToken::never()).unwrap();

        assert_eq!(import.bodies.len(), 1);
        assert_eq!(import.bodies[0].name, "Pièce");
        assert_eq!(import.notes.len(), 1, "{:?}", import.notes);
        assert!(import.notes[0].contains("Latin-1"));
    }

    fn gzipped(text: &str, flags: u8, after_header: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 255];
        bytes.extend_from_slice(after_header);
        bytes.extend(miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 6));
        bytes.extend(crc32fast::hash(text.as_bytes()).to_le_bytes());
        bytes.extend((text.len() as u32).to_le_bytes());
        bytes
    }

    fn written_block() -> String {
        let solid = block();
        write_step(
            &[StepBody {
                name: "Part",
                solid: &solid,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
            }],
            "Part",
            SystemTime::UNIX_EPOCH,
        )
        .unwrap()
    }

    #[test]
    fn a_gzip_compressed_step_file_imports_like_a_plain_one() {
        let text = written_block();
        let dir = tempfile::TempDir::new().unwrap();
        let named = [b'p', b'a', b'r', b't', b'.', b's', b't', b'p', 0];
        let commented = [b'n', b'o', b't', b'e', 0];
        let mut every_field = vec![3, 0, 7, 7, 7];
        every_field.extend(named);
        every_field.extend(commented);
        every_field.extend([0, 0]);
        let variants = [
            ("plain.stpz", gzipped(&text, 0, &[])),
            ("named.stpz", gzipped(&text, 8, &named)),
            ("every.stpz", gzipped(&text, 2 | 4 | 8 | 16, &every_field)),
        ];
        let plain = dir.path().join("plain.step");
        std::fs::write(&plain, &text).unwrap();
        let expected = read_step_file(&plain, &CancelToken::never()).unwrap();

        for (name, bytes) in variants {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();

            let import = read_step_file(&path, &CancelToken::never()).unwrap();

            assert_eq!(import.bodies.len(), 1, "{name}");
            assert_eq!(
                import.bodies[0].import.step, expected.bodies[0].import.step,
                "{name}"
            );
            assert_eq!(import.notes, expected.notes, "{name}");
        }
    }

    #[test]
    fn a_damaged_or_cut_short_compressed_file_is_refused_in_words() {
        let text = written_block();
        let dir = tempfile::TempDir::new().unwrap();
        let intact = gzipped(&text, 0, &[]);
        let mut flipped = intact.clone();
        let middle = flipped.len() / 2;
        flipped[middle] ^= 0x55;
        let mut wrong_size = intact.clone();
        let last = wrong_size.len() - 1;
        wrong_size[last] ^= 1;
        let cases = [
            ("flipped.stpz", flipped),
            ("cut.stpz", intact[..intact.len() / 2].to_vec()),
            ("tiny.stpz", vec![0x1f, 0x8b, 8]),
            ("wrong_size.stpz", wrong_size),
            (
                "stored_elsewhere.stpz",
                vec![0x1f, 0x8b, 9, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
            ),
        ];

        for (name, bytes) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();

            assert_eq!(
                read_step_file(&path, &CancelToken::never()),
                Err(ImportError::DamagedArchive),
                "{name}"
            );
        }
    }

    #[test]
    fn several_bodies_get_distinct_names_and_other_files_are_refused() {
        let solid = block();
        let text = write_step(
            &[
                StepBody {
                    name: "Part",
                    solid: &solid,
                    colour: None,
                    opacity: None,
                    layer: None,
                    threads: &[],
                },
                StepBody {
                    name: "Part",
                    solid: &solid,
                    colour: None,
                    opacity: None,
                    layer: None,
                    threads: &[],
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
            Err(ImportError::Step(caditor_step::ReadError::NoSolids(_)))
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

fn zigzag(vertices: usize) -> Pairs {
    let mut polyline = vec![
        pair(0, "LWPOLYLINE"),
        pair(8, "0"),
        pair(90, vertices),
        pair(70, 0),
    ];
    for index in 0..vertices {
        polyline.extend([pair(10, index as f64), pair(20, (index % 2) as f64)]);
    }
    polyline
}

#[test]
fn a_long_polyline_in_a_huge_array_runs_out_of_its_budget_quickly() {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![block("Zigzag", (0.0, 0.0), vec![zigzag(20_000)])],
        ),
        section(
            "ENTITIES",
            vec![insert(
                "Zigzag",
                "0",
                &[
                    (10, 0.0),
                    (20, 0.0),
                    (70, 700.0),
                    (71, 700.0),
                    (44, 1.0),
                    (45, 1.0),
                ],
            )],
        ),
    ]);

    let started = std::time::Instant::now();
    assert_eq!(parse_dxf(&bytes), Err(ImportError::TooManyObjects));
    assert!(started.elapsed().as_secs() < 10);
}

#[test]
fn curves_past_the_limit_are_counted_whole_items_at_a_time() {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![block("Zigzag", (0.0, 0.0), vec![zigzag(301)])],
        ),
        section(
            "ENTITIES",
            vec![insert(
                "Zigzag",
                "0",
                &[
                    (10, 0.0),
                    (20, 0.0),
                    (70, 20.0),
                    (71, 20.0),
                    (44, 400.0),
                    (45, 2.0),
                ],
            )],
        ),
    ]);

    let drawing = parse_dxf(&bytes).unwrap();

    assert_eq!(drawing.curves.len(), MAX_READ_CURVES);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.contains("20000 more were left out")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn a_heavy_spline_repeated_in_a_large_array_is_refused_by_its_points() {
    const POINTS: usize = 1000;
    let mut spline = vec![pair(0, "SPLINE"), pair(8, "0"), pair(70, 8), pair(71, 3)];
    let knots = (0..POINTS + 4).map(|index| index.saturating_sub(3).min(POINTS - 3));
    spline.extend(knots.map(|knot| pair(40, knot)));
    for index in 0..POINTS {
        let x = index as f64;
        spline.extend([pair(10, x), pair(20, (x * 0.1).sin()), pair(30, 0.0)]);
    }
    let bytes = text(vec![
        header(Some(4)),
        section("BLOCKS", vec![block("Wave", (0.0, 0.0), vec![spline])]),
        section(
            "ENTITIES",
            vec![insert(
                "Wave",
                "0",
                &[(10, 0.0), (20, 0.0), (70, 100.0), (71, 100.0), (45, 10.0)],
            )],
        ),
    ]);

    assert_eq!(parse_dxf(&bytes), Err(ImportError::TooDetailed));
}

#[test]
fn a_large_array_of_a_block_that_draws_nothing_is_counted_without_repeating_it() {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![block(
                "Label",
                (0.0, 0.0),
                vec![vec![pair(0, "TEXT"), pair(8, "0"), pair(1, "note")]],
            )],
        ),
        section(
            "ENTITIES",
            vec![
                line((0.0, 0.0), (1.0, 0.0)),
                insert(
                    "Label",
                    "0",
                    &[(10, 0.0), (20, 0.0), (70, 1000.0), (71, 100.0)],
                ),
            ],
        ),
    ]);

    let drawing = parse_dxf(&bytes).unwrap();

    assert_eq!(drawing.curves.len(), 1);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("100000 texts were left out")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn a_spline_with_bad_control_data_uses_its_fit_points_or_is_left_out() {
    let mut mismatched = vec![pair(0, "SPLINE"), pair(8, "0"), pair(70, 12), pair(71, 2)];
    for knot in [0.0, 0.0, 0.0, 1.0, 1.0, 1.0] {
        mismatched.push(pair(40, knot));
    }
    for weight in [1.0, 0.5] {
        mismatched.push(pair(41, weight));
    }
    for (x, y) in [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        mismatched.extend([pair(10, x), pair(20, y)]);
    }
    let mut with_fit = mismatched.clone();
    for (x, y) in [(0.0, 20.0), (5.0, 25.0), (10.0, 20.0)] {
        with_fit.extend([pair(11, x), pair(21, y)]);
    }

    let bytes = text(vec![
        header(Some(4)),
        section(
            "ENTITIES",
            vec![mismatched, with_fit, line((0.0, 0.0), (1.0, 0.0))],
        ),
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

    assert_eq!(splines.len(), 1);
    let through = BSpline::clamped(splines[0].clone()).unwrap();
    assert!(near(through.point_at(0.0), Point2::new(0.0, 20.0)));
    assert!(
        drawing.notes.join(" ").contains("could not be read"),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn splines_of_a_degree_above_the_kernel_limit_are_left_out() {
    let degree = MAX_SPLINE_DEGREE + 1;
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

fn end_of_first_line(drawing: &Drawing) -> Point2 {
    lines(drawing).first().unwrap().1
}

#[test]
fn arranging_with_the_defaults_changes_nothing() {
    let drawing = drawing(Some(1), vec![line((0.0, 0.0), (1.0, 0.0))]);

    let arranged = drawing.arranged(&DrawingOptions::default());

    assert_eq!(arranged, drawing);
}

#[test]
fn a_chosen_unit_replaces_the_one_the_file_names() {
    let inches = drawing(Some(1), vec![line((0.0, 0.0), (1.0, 0.0))]);
    let unnamed = drawing(None, vec![line((0.0, 0.0), (1.0, 0.0))]);
    let metres = DrawingOptions {
        unit: DrawingUnit::Metres,
        ..DrawingOptions::default()
    };
    let micrometres = DrawingOptions {
        unit: DrawingUnit::Micrometres,
        ..DrawingOptions::default()
    };

    let from_inches = inches.arranged(&metres);
    let from_unnamed = unnamed.arranged(&metres);
    let tiny = unnamed.arranged(&micrometres);

    assert!(near(end_of_first_line(&inches), Point2::new(25.4, 0.0)));
    assert!(near(
        end_of_first_line(&from_inches),
        Point2::new(1_000.0, 0.0)
    ));
    assert!(near(
        end_of_first_line(&from_unnamed),
        Point2::new(1_000.0, 0.0)
    ));
    assert!(near(end_of_first_line(&tiny), Point2::new(1e-3, 0.0)));
    assert!(
        from_inches
            .notes
            .iter()
            .any(|note| note.contains("read the drawing's numbers as metres"))
    );
}

#[test]
fn a_scale_multiplies_every_length_and_composes_with_the_unit() {
    let drawing = drawing(
        None,
        vec![
            line((1.0, 2.0), (3.0, 4.0)),
            entity("CIRCLE", "0", &[(10, 5.0), (20, 6.0), (40, 2.0)]),
        ],
    );
    let options = DrawingOptions {
        unit: DrawingUnit::Centimetres,
        scale: 2.0,
        ..DrawingOptions::default()
    };

    let arranged = drawing.arranged(&options);

    assert!(near(end_of_first_line(&arranged), Point2::new(60.0, 80.0)));
    assert_eq!(
        arranged.curves.last(),
        Some(&DrawingCurve::Circle {
            center: Point2::new(100.0, 120.0),
            radius: 40.0,
        })
    );
    assert!((arranged.unit_scale - 20.0).abs() < 1e-12);
    assert!(
        arranged
            .notes
            .iter()
            .any(|note| note == "You scaled the drawing by 2.")
    );
}

#[test]
fn recentring_moves_the_middle_of_the_outline_to_the_origin() {
    let drawing = Drawing {
        curves: vec![
            DrawingCurve::Arc {
                center: Point2::new(100.0, 200.0),
                start: Point2::new(110.0, 200.0),
                end: Point2::new(100.0, 210.0),
            },
            DrawingCurve::Line {
                start: Point2::new(100.0, 200.0),
                end: Point2::new(110.0, 200.0),
            },
        ],
        ..Drawing::default()
    };
    let options = DrawingOptions {
        recentre: true,
        ..DrawingOptions::default()
    };

    let arranged = drawing.arranged(&options);

    let (low, high) = arranged.bounds().unwrap();
    assert!(near(low, Point2::new(-5.0, -5.0)));
    assert!(near(high, Point2::new(5.0, 5.0)));
    assert!(
        arranged
            .notes
            .iter()
            .any(|note| note.contains("sits on the origin"))
    );
}

#[test]
fn only_a_finite_scale_within_the_limits_is_valid() {
    let valid = |scale| DrawingOptions {
        scale,
        ..DrawingOptions::default()
    };

    assert!(valid(25.4).is_valid());
    assert!(valid(MIN_SCALE).is_valid());
    assert!(!valid(0.0).is_valid());
    assert!(!valid(-1.0).is_valid());
    assert!(!valid(f64::NAN).is_valid());
    assert!(!valid(MAX_SCALE * 2.0).is_valid());
}

fn line_on(layer: &str, start: (f64, f64), end: (f64, f64)) -> Pairs {
    let mut pairs = line(start, end);
    pairs[1] = pair(8, layer);
    pairs
}

fn layered_drawing() -> Drawing {
    let bytes = text(vec![
        header(Some(4)),
        section(
            "BLOCKS",
            vec![block(
                "Tick",
                (0.0, 0.0),
                vec![line((0.0, 0.0), (1.0, 0.0))],
            )],
        ),
        section(
            "ENTITIES",
            vec![
                line_on("Walls", (0.0, 0.0), (10.0, 0.0)),
                line_on("Notes", (0.0, 1.0), (10.0, 1.0)),
                insert("Tick", "Doors", &[(10, 5.0), (20, 5.0), (30, 0.0)]),
                line_on("WALLS", (0.0, 2.0), (10.0, 2.0)),
            ],
        ),
    ]);
    parse_dxf(&bytes).unwrap()
}

#[test]
fn curves_remember_their_layer_and_a_block_takes_the_layer_it_is_inserted_on() {
    let drawing = layered_drawing();

    assert_eq!(drawing.layers, ["Walls", "Notes", "Doors"]);
    assert_eq!(drawing.curve_layers, [0, 1, 2, 0]);
    assert_eq!(drawing.layer_curve_count(0), 2);
    assert_eq!(drawing.layer_curve_count(2), 1);
}

#[test]
fn leaving_layers_out_drops_their_curves_and_keeps_the_rest_in_order() {
    let mut drawing = layered_drawing();
    drawing.construction.insert(3);
    let options = DrawingOptions {
        left_out_layers: [1].into(),
        ..DrawingOptions::default()
    };

    let arranged = drawing.arranged(&options);

    assert_eq!(arranged.curves.len(), 3);
    assert_eq!(arranged.curve_layers, [0, 2, 0]);
    assert_eq!(arranged.construction, [2].into());
    assert_eq!(arranged.layers, drawing.layers);
    assert!(
        arranged
            .notes
            .iter()
            .any(|note| note == "1 curve on a layer you left out was not imported.")
    );
}

#[test]
fn the_sketch_limit_applies_to_the_layers_chosen_so_leaving_layers_out_brings_later_curves_in() {
    let mut entities: Vec<Pairs> = (0..MAX_DRAWING_CURVES)
        .map(|index| line_on("Hatching", (index as f64, 0.0), (index as f64, 1.0)))
        .collect();
    entities
        .extend((0..3).map(|index| line_on("Outline", (index as f64, 5.0), (index as f64, 6.0))));
    let drawing = parse_dxf(&text(vec![section("ENTITIES", entities)])).unwrap();
    let hatching = drawing
        .layers
        .iter()
        .position(|name| name == "Hatching")
        .unwrap();
    let without_hatching = DrawingOptions {
        left_out_layers: [hatching].into(),
        ..DrawingOptions::default()
    };

    let everything = drawing.arranged(&DrawingOptions::default());
    let outline = drawing.arranged(&without_hatching);

    assert_eq!(everything.curves.len(), MAX_DRAWING_CURVES);
    assert!(
        everything
            .notes
            .iter()
            .any(|note| note.contains("3 more were left out"))
    );
    assert_eq!(outline.curves.len(), 3);
    assert!(
        !outline
            .notes
            .iter()
            .any(|note| note.contains("more were left out"))
    );
    assert_eq!(
        drawing.chosen_curve_count(&without_hatching.left_out_layers),
        3
    );
    assert_eq!(
        drawing.chosen_curve_count(&BTreeSet::new()),
        MAX_DRAWING_CURVES + 3
    );
}
