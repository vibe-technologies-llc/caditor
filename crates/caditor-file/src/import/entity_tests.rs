use std::f64::consts::{FRAC_PI_2, PI};

use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeExtent, FeatureKind, RegionChoice, SolidFeature,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::SamplingTolerance;
use caditor_sketch::BSpline;

use crate::import::{
    Drawing, DrawingCurve, SketchTarget, drawing_transaction, parse_dxf,
    tests::{
        Pairs, arcs, entity, evaluate, header, layer, line, lines, millimetre_drawing, near, pair,
        section, tables, text,
    },
};

fn hatch(paths: Vec<Pairs>) -> Pairs {
    let mut pairs = vec![
        pair(0, "HATCH"),
        pair(8, "0"),
        pair(10, 0.0),
        pair(20, 0.0),
        pair(30, 0.0),
        pair(210, 0.0),
        pair(220, 0.0),
        pair(230, 1.0),
        pair(2, "SOLID"),
        pair(70, 1),
        pair(71, 0),
        pair(91, paths.len()),
    ];
    pairs.extend(paths.into_iter().flatten());
    pairs.extend([
        pair(75, 1),
        pair(76, 1),
        pair(98, 1),
        pair(10, 500.0),
        pair(20, 500.0),
    ]);
    pairs
}

fn polyline_path(flags: i64, vertices: &[(f64, f64, f64)], sources: &[&str]) -> Pairs {
    let mut pairs = vec![
        pair(92, flags),
        pair(72, 1),
        pair(73, 1),
        pair(93, vertices.len()),
    ];
    for (x, y, bulge) in vertices {
        pairs.extend([pair(10, x), pair(20, y), pair(42, bulge)]);
    }
    pairs.push(pair(97, sources.len()));
    pairs.extend(sources.iter().map(|handle| pair(330, handle)));
    pairs
}

fn edge_path(edges: Vec<Pairs>) -> Pairs {
    let mut pairs = vec![pair(92, 1), pair(93, edges.len())];
    pairs.extend(edges.into_iter().flatten());
    pairs.push(pair(97, 0));
    pairs
}

fn line_edge(start: (f64, f64), end: (f64, f64)) -> Pairs {
    vec![
        pair(72, 1),
        pair(10, start.0),
        pair(20, start.1),
        pair(11, end.0),
        pair(21, end.1),
    ]
}

fn arc_edge(center: (f64, f64), radius: f64, angles: (f64, f64), ccw: bool) -> Pairs {
    vec![
        pair(72, 2),
        pair(10, center.0),
        pair(20, center.1),
        pair(40, radius),
        pair(50, angles.0),
        pair(51, angles.1),
        pair(73, i64::from(ccw)),
    ]
}

fn splines(drawing: &Drawing) -> Vec<BSpline> {
    drawing
        .curves
        .iter()
        .filter_map(|curve| match curve {
            DrawingCurve::Spline { control_points } => BSpline::clamped(control_points.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn hatch_boundaries_become_curves_but_their_seed_points_do_not() {
    let slot = polyline_path(
        2,
        &[
            (0.0, 0.0, 0.0),
            (20.0, 0.0, 1.0),
            (20.0, 10.0, 0.0),
            (0.0, 10.0, 1.0),
        ],
        &[],
    );
    let edges = edge_path(vec![
        line_edge((30.0, 0.0), (40.0, 0.0)),
        arc_edge((40.0, 5.0), 5.0, (270.0, 90.0), true),
        line_edge((40.0, 10.0), (30.0, 10.0)),
        arc_edge((30.0, 5.0), 5.0, (90.0, 270.0), false),
    ]);

    let drawing = millimetre_drawing(vec![hatch(vec![slot, edges])]);
    let arcs = arcs(&drawing);
    let right = arcs
        .iter()
        .find(|arc| near(arc.center, Point2::new(40.0, 5.0)))
        .unwrap();
    let left = arcs
        .iter()
        .find(|arc| near(arc.center, Point2::new(30.0, 5.0)))
        .unwrap();

    assert_eq!(lines(&drawing).len(), 4);
    assert_eq!(arcs.len(), 4);
    assert!((right.sweep - PI).abs() < 1e-12);
    assert!(near(
        right.point_at(right.start_angle),
        Point2::new(40.0, 0.0)
    ));
    assert!((left.sweep - PI).abs() < 1e-12);
    assert!(near(
        left.point_at(left.start_angle),
        Point2::new(30.0, 10.0)
    ));
    assert!(near(
        left.point_at(left.end_angle()),
        Point2::new(30.0, 0.0)
    ));
    assert!(
        drawing
            .curves
            .iter()
            .all(|curve| curve.ends().is_none_or(|(start, _)| start.x < 100.0))
    );
    assert!(
        drawing
            .notes
            .contains(&"The boundary of 1 hatch was imported without its fill.".to_owned()),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn hatch_ellipse_and_spline_edges_follow_their_curves() {
    let ellipse = vec![
        pair(72, 3),
        pair(10, 0.0),
        pair(20, 0.0),
        pair(11, 10.0),
        pair(21, 0.0),
        pair(40, 0.5),
        pair(50, 0.0),
        pair(51, 45.0),
        pair(73, 1),
    ];
    let mut spline = vec![
        pair(72, 4),
        pair(94, 3),
        pair(73, 0),
        pair(74, 0),
        pair(95, 8),
        pair(96, 4),
    ];
    for knot in [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0] {
        spline.push(pair(40, knot));
    }
    for (x, y) in [(20.0, 0.0), (21.0, 3.0), (24.0, 3.0), (25.0, 0.0)] {
        spline.extend([pair(10, x), pair(20, y)]);
    }

    let drawing = millimetre_drawing(vec![hatch(vec![edge_path(vec![ellipse, spline])])]);
    let found = splines(&drawing);
    let quarter = found
        .iter()
        .find(|spline| spline.point_at(0.0).x < 15.0)
        .unwrap();
    let kept = found
        .iter()
        .find(|spline| spline.point_at(0.0).x > 15.0)
        .unwrap();
    let diagonal = 10.0 / 5.0_f64.sqrt();

    assert!(near(quarter.point_at(0.0), Point2::new(10.0, 0.0)));
    assert!(quarter.point_at(1.0).distance(Point2::splat(diagonal)) < 1e-6);
    assert_eq!(
        kept.control_points(),
        &[
            Point2::new(20.0, 0.0),
            Point2::new(21.0, 3.0),
            Point2::new(24.0, 3.0),
            Point2::new(25.0, 0.0)
        ]
    );
}

#[test]
fn an_associative_hatch_is_not_drawn_over_its_own_outline() {
    let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let outline = |hidden_last: bool| -> Vec<Pairs> {
        (0..4)
            .map(|index| {
                let mut edge = line(square[index], square[(index + 1) % 4]);
                edge.push(pair(5, format!("a{index}")));
                if hidden_last && index == 3 {
                    edge[1] = pair(8, "Off");
                }
                edge
            })
            .collect()
    };
    let traced = || {
        hatch(vec![polyline_path(
            2 | 1,
            &square.map(|(x, y)| (x, y, 0.0)),
            &["A0", "A1", "A2", "A3"],
        )])
    };
    let drawn = |hidden_last: bool| {
        let mut entities = outline(hidden_last);
        entities.push(traced());
        parse_dxf(&text(vec![
            header(Some(4)),
            tables(vec![layer("Off", -7, 0)]),
            section("ENTITIES", entities),
        ]))
        .unwrap()
    };

    let shown = drawn(false);
    let partly_hidden = drawn(true);

    assert_eq!(lines(&shown).len(), 4);
    assert!(shown.notes.iter().all(|note| !note.contains("hatch")));
    assert_eq!(lines(&partly_hidden).len(), 7);
}

#[test]
fn text_box_boundaries_of_a_hatch_are_left_out() {
    let text_box = polyline_path(
        8 | 2,
        &[(0.0, 0.0, 0.0), (5.0, 0.0, 0.0), (5.0, 2.0, 0.0)],
        &[],
    );
    let outer = polyline_path(
        2,
        &[(-1.0, -1.0, 0.0), (6.0, -1.0, 0.0), (6.0, 3.0, 0.0)],
        &[],
    );

    let drawing = millimetre_drawing(vec![hatch(vec![outer, text_box])]);

    let corner = Point2::new(5.0, 0.0);
    let found = lines(&drawing);

    assert_eq!(found.len(), 3);
    assert!(
        found
            .iter()
            .all(|(start, end)| !near(*start, corner) && !near(*end, corner))
    );
}

#[test]
fn solids_traces_and_faces_become_their_outlines() {
    let solid = entity(
        "SOLID",
        "0",
        &[
            (10, 0.0),
            (20, 0.0),
            (11, 10.0),
            (21, 0.0),
            (12, 0.0),
            (22, 10.0),
            (13, 10.0),
            (23, 10.0),
            (210, 0.0),
            (220, 0.0),
            (230, -1.0),
        ],
    );
    let triangle = entity(
        "TRACE",
        "0",
        &[
            (10, 20.0),
            (20, 0.0),
            (11, 30.0),
            (21, 0.0),
            (12, 20.0),
            (22, 5.0),
        ],
    );
    let face = entity(
        "3DFACE",
        "0",
        &[
            (10, 40.0),
            (20, 0.0),
            (30, 0.0),
            (11, 50.0),
            (21, 0.0),
            (31, 0.0),
            (12, 50.0),
            (22, 10.0),
            (32, 0.0),
            (13, 40.0),
            (23, 10.0),
            (33, 0.0),
            (70, 1.0),
        ],
    );

    let drawing = millimetre_drawing(vec![solid, triangle, face]);
    let found = lines(&drawing);
    let has = |start: (f64, f64), end: (f64, f64)| {
        let (start, end) = (Point2::new(start.0, start.1), Point2::new(end.0, end.1));
        found
            .iter()
            .any(|(a, b)| (near(*a, start) && near(*b, end)) || (near(*a, end) && near(*b, start)))
    };

    assert_eq!(found.len(), 4 + 3 + 3);
    assert!(has((0.0, 0.0), (-10.0, 0.0)));
    assert!(has((-10.0, 0.0), (-10.0, 10.0)));
    assert!(has((-10.0, 10.0), (0.0, 10.0)));
    assert!(has((0.0, 10.0), (0.0, 0.0)));
    assert!(has((30.0, 0.0), (20.0, 5.0)));
    assert!(!has((40.0, 0.0), (50.0, 0.0)));
    assert!(has((40.0, 10.0), (40.0, 0.0)));
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
}

fn mline_style(handle: &str, flags: i64) -> Pairs {
    vec![
        pair(0, "MLINESTYLE"),
        pair(5, handle),
        pair(2, "Wall"),
        pair(70, flags),
    ]
}

struct Joint {
    at: (f64, f64),
    miter: (f64, f64),
    offset: f64,
}

fn mline(flags: i64, vertices: &[Joint]) -> Pairs {
    let mut pairs = vec![
        pair(0, "MLINE"),
        pair(8, "0"),
        pair(2, "Wall"),
        pair(340, "5a"),
        pair(40, 1.0),
        pair(70, 0),
        pair(71, flags),
        pair(72, vertices.len()),
        pair(73, 2),
        pair(10, 0.0),
        pair(20, 0.0),
        pair(30, 0.0),
    ];
    for Joint { at, miter, offset } in vertices {
        pairs.extend([
            pair(11, at.0),
            pair(21, at.1),
            pair(31, 0.0),
            pair(12, 1.0),
            pair(22, 0.0),
            pair(32, 0.0),
            pair(13, miter.0),
            pair(23, miter.1),
            pair(33, 0.0),
        ]);
        for element_offset in [*offset, -*offset] {
            pairs.extend([
                pair(74, 2),
                pair(41, element_offset),
                pair(41, 0.0),
                pair(75, 0),
            ]);
        }
    }
    pairs
}

fn mline_drawing(style_flags: i64, line: Pairs) -> Drawing {
    parse_dxf(&text(vec![
        header(Some(4)),
        section("ENTITIES", vec![line]),
        section("OBJECTS", vec![mline_style("5A", style_flags)]),
    ]))
    .unwrap()
}

#[test]
fn multilines_become_their_element_lines_with_the_caps_of_their_style() {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let corner = [
        Joint {
            at: (0.0, 0.0),
            miter: (0.0, 1.0),
            offset: 0.5,
        },
        Joint {
            at: (10.0, 0.0),
            miter: (-half, half),
            offset: 0.5 / half,
        },
        Joint {
            at: (10.0, 10.0),
            miter: (-1.0, 0.0),
            offset: 0.5,
        },
    ];

    let square_caps = mline_drawing(16 | 256, mline(1, &corner));
    let round_start = mline_drawing(64, mline(1, &corner));
    let closed = mline_drawing(16 | 256, mline(1 | 2, &corner));
    let found = lines(&square_caps);
    let has = |start: (f64, f64), end: (f64, f64)| {
        let (start, end) = (Point2::new(start.0, start.1), Point2::new(end.0, end.1));
        found.iter().any(|(a, b)| near(*a, start) && near(*b, end))
    };
    let cap = arcs(&round_start);

    assert_eq!(found.len(), 6);
    assert!(has((0.0, 0.5), (9.5, 0.5)));
    assert!(has((0.0, -0.5), (10.5, -0.5)));
    assert!(has((9.5, 0.5), (9.5, 10.0)));
    assert!(has((0.0, 0.5), (0.0, -0.5)));
    assert!(has((9.5, 10.0), (10.5, 10.0)));
    assert_eq!(lines(&round_start).len(), 4);
    assert_eq!(cap.len(), 1);
    assert!(near(cap[0].center, Point2::ZERO));
    assert!((cap[0].sweep - PI).abs() < 1e-12);
    assert!(near(
        cap[0].point_at(cap[0].start_angle + FRAC_PI_2),
        Point2::new(-0.5, 0.0)
    ));
    assert_eq!(lines(&closed).len(), 6);
    assert!(arcs(&closed).is_empty());
}

#[test]
fn a_drawing_whose_only_outline_is_a_hatch_extrudes_into_a_solid() {
    let edges = edge_path(vec![
        line_edge((30.0, 0.0), (40.0, 0.0)),
        arc_edge((40.0, 5.0), 5.0, (270.0, 90.0), true),
        line_edge((40.0, 10.0), (30.0, 10.0)),
        arc_edge((30.0, 5.0), 5.0, (90.0, 270.0), false),
    ]);
    let drawing = millimetre_drawing(vec![hatch(vec![edges])]);
    let mut document = Document::default();

    let import = drawing_transaction(
        &document,
        &drawing,
        SketchTarget::New {
            name: "hatch".to_owned(),
            plane: Plane::XY,
        },
        "Import hatch.dxf",
    );
    document.apply(import.transaction).unwrap();
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
    let volume = evaluation
        .body(body)
        .unwrap()
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume;
    let expected = 2.0 * (10.0 * 10.0 + PI * 25.0);

    assert_eq!(import.joints, 4);
    assert_eq!(evaluation.failed_count(), 0);
    assert!((volume - expected).abs() < 0.05, "{volume} vs {expected}");
}
