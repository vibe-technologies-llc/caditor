use std::{collections::BTreeSet, io::Write};

use caditor_document::CancelToken;
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{ArcGeometry, BSpline, Sketch};
use tempfile::TempDir;

use crate::{
    export::{Construction, SketchFormat, export_sketch},
    import::{Drawing, DrawingCurve, DrawingOptions, ImportError, parse_svg, read_drawing},
};

const SVG_OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">"#;

fn svg(body: &str) -> Vec<u8> {
    format!("{SVG_OPEN}{body}</svg>").into_bytes()
}

fn read(body: &str) -> Drawing {
    parse_svg(&svg(body)).unwrap()
}

fn close(a: Point2, b: Point2) -> bool {
    a.distance(b) < 1e-9
}

fn line(start: (f64, f64), end: (f64, f64)) -> DrawingCurve {
    DrawingCurve::Line {
        start: Point2::new(start.0, start.1),
        end: Point2::new(end.0, end.1),
    }
}

fn assert_curves_close(actual: &[DrawingCurve], expected: &[DrawingCurve]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (actual, expected) in actual.iter().zip(expected) {
        let matches = match (actual, expected) {
            (
                DrawingCurve::Line { start, end },
                DrawingCurve::Line {
                    start: other_start,
                    end: other_end,
                },
            ) => close(*start, *other_start) && close(*end, *other_end),
            (
                DrawingCurve::Circle { center, radius },
                DrawingCurve::Circle {
                    center: other_center,
                    radius: other_radius,
                },
            ) => close(*center, *other_center) && (radius - other_radius).abs() < 1e-9,
            (
                DrawingCurve::Arc { center, start, end },
                DrawingCurve::Arc {
                    center: other_center,
                    start: other_start,
                    end: other_end,
                },
            ) => {
                close(*center, *other_center)
                    && close(*start, *other_start)
                    && close(*end, *other_end)
            }
            (
                DrawingCurve::Spline { control_points },
                DrawingCurve::Spline {
                    control_points: other,
                },
            ) => {
                control_points.len() == other.len()
                    && control_points.iter().zip(other).all(|(a, b)| close(*a, *b))
            }
            _ => false,
        };
        assert!(matches, "{actual:?} is not {expected:?}");
    }
}

#[test]
fn path_lines_read_absolute_and_relative_commands_with_y_pointing_up() {
    let drawing = read(r#"<path d="M0 0 L10 0 H20 V10 l-5 5 h-5 v-5 Z m 30 0 l 10,0 20 0"/>"#);

    assert_curves_close(
        &drawing.curves,
        &[
            line((0.0, 0.0), (10.0, 0.0)),
            line((10.0, 0.0), (20.0, 0.0)),
            line((20.0, 0.0), (20.0, -10.0)),
            line((20.0, -10.0), (15.0, -15.0)),
            line((15.0, -15.0), (10.0, -15.0)),
            line((10.0, -15.0), (10.0, -10.0)),
            line((10.0, -10.0), (0.0, 0.0)),
            line((30.0, 0.0), (40.0, 0.0)),
            line((40.0, 0.0), (60.0, 0.0)),
        ],
    );
    assert_eq!(drawing.unit_scale, 1.0);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
}

#[test]
fn beziers_become_exact_splines_with_reflected_controls() {
    let drawing = read(
        r#"<path d="M0 0 C10 0 10 10 20 10 S30 20 40 20 Q50 0 60 0 T80 0 c5 5 10 5 15 0 q5 -5 10 0"/>"#,
    );

    let flipped = |points: &[(f64, f64)]| DrawingCurve::Spline {
        control_points: points.iter().map(|(x, y)| Point2::new(*x, -*y)).collect(),
    };
    assert_eq!(
        drawing.curves,
        vec![
            flipped(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (20.0, 10.0)]),
            flipped(&[(20.0, 10.0), (30.0, 10.0), (30.0, 20.0), (40.0, 20.0)]),
            flipped(&[(40.0, 20.0), (50.0, 0.0), (60.0, 0.0)]),
            line((60.0, 0.0), (80.0, 0.0)),
            flipped(&[(80.0, 0.0), (85.0, 5.0), (90.0, 5.0), (95.0, 0.0)]),
            flipped(&[(95.0, 0.0), (100.0, -5.0), (105.0, 0.0)]),
        ]
    );
    let DrawingCurve::Spline { control_points } = &drawing.curves[0] else {
        panic!("not a spline");
    };
    let spline = BSpline::clamped(control_points.clone()).unwrap();
    let bezier = |t: f64| {
        let u = 1.0 - t;
        control_points[0] * u.powi(3)
            + control_points[1] * (3.0 * u * u * t)
            + control_points[2] * (3.0 * u * t * t)
            + control_points[3] * t.powi(3)
    };
    for step in 0..=10 {
        let t = f64::from(step) / 10.0;
        assert!(spline.point_at(t).distance(bezier(t)) < 1e-12);
    }
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
}

#[test]
fn circular_arcs_stay_arcs_and_elliptical_ones_stay_elliptical() {
    let drawing = read(
        r#"<path d="M0 0 A10 10 0 0 1 20 0"/>
           <path d="M0 0 A10 10 0 0 0 20 0"/>
           <path d="M0 0 a1 1 0 0 1 20 0"/>
           <path d="M0 0 A10 10 30 1 1 10 10"/>
           <path d="M0 0 A20 10 0 0 1 40 0"/>
           <path d="M0 0 A0 10 0 0 1 40 0"/>"#,
    );

    let [upper, lower, scaled, large, elliptical, flat] = drawing.curves.as_slice() else {
        panic!("{:?}", drawing.curves);
    };
    assert_curves_close(
        &[upper.clone(), lower.clone(), scaled.clone(), flat.clone()],
        &[
            DrawingCurve::Arc {
                center: Point2::new(10.0, 0.0),
                start: Point2::new(20.0, 0.0),
                end: Point2::new(0.0, 0.0),
            },
            DrawingCurve::Arc {
                center: Point2::new(10.0, 0.0),
                start: Point2::new(0.0, 0.0),
                end: Point2::new(20.0, 0.0),
            },
            DrawingCurve::Arc {
                center: Point2::new(10.0, 0.0),
                start: Point2::new(20.0, 0.0),
                end: Point2::new(0.0, 0.0),
            },
            line((0.0, 0.0), (40.0, 0.0)),
        ],
    );
    let DrawingCurve::Arc { center, start, end } = large else {
        panic!("{large:?}");
    };
    let geometry = ArcGeometry::from_points(*center, *start, *end);
    assert!((geometry.radius - 10.0).abs() < 1e-9);
    assert!(geometry.sweep > std::f64::consts::PI);
    let DrawingCurve::Ellipse {
        center,
        major,
        minor_radius,
        ends: Some((start, end)),
    } = elliptical
    else {
        panic!("{elliptical:?}");
    };
    assert!(center.distance(Point2::new(20.0, 0.0)) < 1e-9);
    assert!((major.length() - 20.0).abs() < 1e-9 && major.y.abs() < 1e-9);
    assert!((minor_radius - 10.0).abs() < 1e-9);
    assert!(start.distance(Point2::new(40.0, 0.0)) < 1e-9);
    assert!(end.distance(Point2::new(0.0, 0.0)) < 1e-9);
    assert!(
        drawing
            .notes
            .iter()
            .all(|note| !note.contains("converted to sketch splines")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn basic_shapes_become_lines_arcs_circles_and_ellipses() {
    let drawing = read(
        r#"<rect x="10" y="10" width="20" height="10"/>
           <rect x="0" y="50" width="20" height="10" rx="2"/>
           <circle cx="50" cy="50" r="5"/>
           <ellipse cx="80" cy="50" rx="10" ry="5"/>
           <ellipse cx="80" cy="80" rx="4"/>
           <line x1="0" y1="90" x2="10" y2="95"/>
           <polyline points="0,0 10,0 10,10"/>
           <polygon points="20 0 30 0 30 10"/>
           <rect width="0" height="10"/>
           <circle r="-1"/>"#,
    );

    let curves = &drawing.curves;
    assert_eq!(curves.len(), 4 + 8 + 1 + 1 + 1 + 1 + 2 + 3, "{curves:?}");
    assert_curves_close(
        &curves[..4],
        &[
            line((10.0, -10.0), (30.0, -10.0)),
            line((30.0, -10.0), (30.0, -20.0)),
            line((30.0, -20.0), (10.0, -20.0)),
            line((10.0, -20.0), (10.0, -10.0)),
        ],
    );
    let rounded = &curves[4..12];
    assert_eq!(
        rounded
            .iter()
            .filter(|curve| matches!(curve, DrawingCurve::Arc { .. }))
            .count(),
        4
    );
    assert_curves_close(
        &rounded[..2],
        &[
            line((2.0, -50.0), (18.0, -50.0)),
            DrawingCurve::Arc {
                center: Point2::new(18.0, -52.0),
                start: Point2::new(20.0, -52.0),
                end: Point2::new(18.0, -50.0),
            },
        ],
    );
    assert_curves_close(
        &curves[12..13],
        &[DrawingCurve::Circle {
            center: Point2::new(50.0, -50.0),
            radius: 5.0,
        }],
    );
    assert_eq!(
        curves[13],
        DrawingCurve::Ellipse {
            center: Point2::new(80.0, -50.0),
            major: Vector2::new(10.0, 0.0),
            minor_radius: 5.0,
            ends: None,
        }
    );
    assert_curves_close(
        &curves[14..],
        &[
            DrawingCurve::Circle {
                center: Point2::new(80.0, -80.0),
                radius: 4.0,
            },
            line((0.0, -90.0), (10.0, -95.0)),
            line((0.0, 0.0), (10.0, 0.0)),
            line((10.0, 0.0), (10.0, -10.0)),
            line((20.0, 0.0), (30.0, 0.0)),
            line((30.0, 0.0), (30.0, -10.0)),
            line((30.0, -10.0), (20.0, 0.0)),
        ],
    );
}

#[test]
fn nested_group_transforms_compose() {
    let drawing = read(
        r#"<g transform="translate(10 20)">
             <g transform="rotate(90)"><line x1="0" y1="0" x2="10" y2="0"/></g>
             <line transform="scale(2, 3)" x1="1" y1="1" x2="2" y2="1"/>
             <line transform="matrix(1 0 0 1 5 5)" x1="0" y1="0" x2="1" y2="0"/>
             <line transform="skewX(45)" x1="0" y1="10" x2="0" y2="0"/>
             <line transform="skewY(45)" x1="10" y1="0" x2="0" y2="0"/>
             <line transform="rotate(90 5 5)" x1="5" y1="5" x2="6" y2="5"/>
           </g>"#,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((10.0, -20.0), (10.0, -30.0)),
            line((12.0, -23.0), (14.0, -23.0)),
            line((15.0, -25.0), (16.0, -25.0)),
            line((20.0, -30.0), (10.0, -20.0)),
            line((20.0, -30.0), (10.0, -20.0)),
            line((15.0, -25.0), (15.0, -26.0)),
        ],
    );
}

#[test]
fn circles_stay_circles_under_similarity_and_become_ellipses_when_squashed() {
    let drawing = read(
        r#"<circle transform="translate(50 50) rotate(30) scale(-2 2)" r="5"/>
           <circle transform="scale(2 1)" cx="10" cy="10" r="5"/>
           <path transform="scale(1 -1)" d="M0 0 A10 10 0 0 1 20 0"/>"#,
    );

    assert_curves_close(
        &drawing.curves[..1],
        &[DrawingCurve::Circle {
            center: Point2::new(50.0, -50.0),
            radius: 10.0,
        }],
    );
    let DrawingCurve::Ellipse {
        center,
        major,
        minor_radius,
        ends: None,
    } = drawing.curves[1]
    else {
        panic!("{:?}", drawing.curves[1]);
    };
    assert!(center.distance(Point2::new(20.0, -10.0)) < 1e-9);
    assert!((major.length() - 10.0).abs() < 1e-9 && major.y.abs() < 1e-9);
    assert!((minor_radius - 5.0).abs() < 1e-9);
    assert_curves_close(
        &drawing.curves[2..],
        &[DrawingCurve::Arc {
            center: Point2::new(10.0, 0.0),
            start: Point2::new(0.0, 0.0),
            end: Point2::new(20.0, 0.0),
        }],
    );
}

fn first_line_length(svg: &str) -> (f64, Drawing) {
    let drawing = parse_svg(svg.as_bytes()).unwrap();
    let Some((start, end)) = drawing.curves[0].ends() else {
        panic!("{:?}", drawing.curves);
    };
    (start.distance(end), drawing)
}

#[test]
fn the_size_and_view_box_set_the_scale_in_millimetres() {
    let ns = r#"xmlns="http://www.w3.org/2000/svg""#;
    let segment = r#"<line x1="0" y1="0" x2="1" y2="0"/>"#;
    let cases = [
        (
            r#"width="10mm" height="10mm" viewBox="0 0 10 10""#,
            1.0,
            None,
        ),
        (
            r#"width="10cm" height="5cm" viewBox="0 0 10 5""#,
            10.0,
            Some("centimetres"),
        ),
        (
            r#"width="1in" height="1in" viewBox="0 0 1 1""#,
            25.4,
            Some("inches"),
        ),
        (
            r#"width="72pt" height="72pt" viewBox="0 0 72 72""#,
            25.4 / 72.0,
            Some("points"),
        ),
        (
            r#"width="6pc" viewBox="0 0 6 3""#,
            25.4 / 6.0,
            Some("picas"),
        ),
        (
            r#"width="96px" height="96px" viewBox="0 0 96 96""#,
            25.4 / 96.0,
            Some("96 to the inch"),
        ),
        (
            r#"width="96" height="96" viewBox="0 0 48 48""#,
            25.4 / 48.0,
            Some("96 to the inch"),
        ),
        (
            r#"viewBox="0 0 10 10""#,
            25.4 / 96.0,
            Some("does not name a unit"),
        ),
        (
            r#"width="210mm" height="297mm""#,
            25.4 / 96.0,
            Some("without a viewBox"),
        ),
        ("", 25.4 / 96.0, Some("does not name a unit")),
        (
            r#"width="20mm" height="10mm" viewBox="0 0 10 10""#,
            1.0,
            None,
        ),
        (
            r#"width="20mm" height="10mm" viewBox="0 0 10 10" preserveAspectRatio="xMidYMid slice""#,
            2.0,
            None,
        ),
    ];
    for (attributes, millimetres, note) in cases {
        let text = format!("<svg {ns} {attributes}>{segment}</svg>");
        let (length, drawing) = first_line_length(&text);
        assert!(
            (length - millimetres).abs() < 1e-12,
            "{attributes}: {length}"
        );
        assert!(
            (drawing.unit_scale - millimetres).abs() < 1e-12,
            "{attributes}"
        );
        match note {
            Some(words) => assert!(
                drawing.notes.iter().any(|note| note.contains(words)),
                "{attributes}: {:?}",
                drawing.notes
            ),
            None => assert!(
                drawing.notes.is_empty(),
                "{attributes}: {:?}",
                drawing.notes
            ),
        }
    }
    let stretched = format!(
        r#"<svg {ns} width="20mm" height="10mm" viewBox="0 0 10 10" preserveAspectRatio="none"><line x1="0" y1="0" x2="1" y2="1"/></svg>"#
    );
    let drawing = parse_svg(stretched.as_bytes()).unwrap();
    assert_curves_close(&drawing.curves, &[line((0.0, 0.0), (2.0, -1.0))]);
    let pixels =
        format!(r#"<svg {ns} width="96px" height="96px" viewBox="0 0 96 96">{segment}</svg>"#);
    let drawing = parse_svg(pixels.as_bytes()).unwrap();
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.contains("a pixel is 0.2646 mm")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn lengths_in_shapes_take_units_and_percentages() {
    let drawing = read(
        r#"<line x1="0" y1="0" x2="1in" y2="0"/>
           <line x1="0" y1="0" x2="50%" y2="0"/>
           <circle r="1cm"/>"#,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((0.0, 0.0), (96.0, 0.0)),
            line((0.0, 0.0), (50.0, 0.0)),
            DrawingCurve::Circle {
                center: Point2::ZERO,
                radius: 960.0 / 25.4,
            },
        ],
    );
}

#[test]
fn nested_svg_elements_place_their_view_box() {
    let drawing = read(
        r#"<svg x="10" y="10" width="20" height="10" viewBox="0 0 10 10">
             <line x1="0" y1="0" x2="10" y2="0"/>
           </svg>"#,
    );

    assert_curves_close(&drawing.curves, &[line((15.0, -10.0), (25.0, -10.0))]);
}

#[test]
fn top_groups_and_inkscape_layers_are_offered_as_layers() {
    let drawing = parse_svg(
        br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="10mm" height="10mm" viewBox="0 0 10 10">
               <g id="wrapper">
                 <g inkscape:groupmode="layer" inkscape:label="Outline" id="layer1">
                   <line x1="0" y1="0" x2="1" y2="0"/>
                   <g><line x1="0" y1="1" x2="1" y2="1"/></g>
                 </g>
                 <g id="holes"><circle r="1"/></g>
                 <g><line x1="0" y1="2" x2="1" y2="2"/></g>
                 <g style="display:none" id="hidden"><line x1="0" y1="3" x2="1" y2="3"/></g>
                 <line x1="0" y1="4" x2="1" y2="4"/>
               </g>
             </svg>"##,
    )
    .unwrap();

    assert_eq!(
        drawing.layers,
        vec![
            "Outline".to_owned(),
            "holes".to_owned(),
            "Group 1".to_owned(),
            "Ungrouped".to_owned()
        ]
    );
    assert_eq!(drawing.curve_layers, vec![0, 0, 1, 2, 3]);
    assert!(
        drawing
            .notes
            .contains(&"1 hidden element was left out.".to_owned()),
        "{:?}",
        drawing.notes
    );
    let options = DrawingOptions {
        left_out_layers: BTreeSet::from([0, 2]),
        ..DrawingOptions::default()
    };
    assert_eq!(drawing.arranged(&options).curves.len(), 2);
}

#[test]
fn what_cannot_be_imported_is_reported() {
    let drawing = parse_svg(
        br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd" viewBox="0 0 10 10" width="10mm">
               <sodipodi:namedview/>
               <defs><clipPath id="clip"><rect width="5" height="5"/></clipPath></defs>
               <title>Plate</title>
               <text x="0" y="0">Label</text>
               <text x="0" y="0">Other</text>
               <image href="photo.png" width="1" height="1"/>
               <video/>
               <line clip-path="url(#clip)" x1="0" y1="0" x2="1" y2="0"/>
               <line visibility="hidden" x1="0" y1="1" x2="1" y2="1"/>
               <line transform="wobble(3)" x1="0" y1="2" x2="1" y2="2"/>
               <path d="M0 3 L1 3 L2 x 4"/>
               <use href="#nowhere"/>
             </svg>"##,
    )
    .unwrap();

    assert_eq!(drawing.curves.len(), 3);
    for expected in [
        "1 image and 2 texts were left out, because sketches hold only points, lines, arcs, circles and splines.",
        "1 element that caditor does not read was left out: video.",
        "1 hidden element was left out.",
        "1 element with a clip path or mask was imported whole, without the clipping.",
        "1 path or point list with damaged data was imported up to the damage.",
        "1 transform that could not be read was ignored.",
        "1 reference to an element the drawing does not contain was left out.",
    ] {
        assert!(
            drawing.notes.contains(&expected.to_owned()),
            "{expected}: {:?}",
            drawing.notes
        );
    }
}

#[test]
fn reused_elements_are_placed_and_symbols_drawn() {
    let drawing = read(
        r##"<defs>
              <line id="tick" x1="0" y1="0" x2="1" y2="0"/>
              <symbol id="mark"><circle r="2"/></symbol>
            </defs>
            <use href="#tick" x="10" y="10"/>
            <use xlink:href="#tick" xmlns:xlink="http://www.w3.org/1999/xlink" transform="translate(0 20)"/>
            <use href="#mark" x="50" y="50"/>
            <g id="loop"><use href="#loop"/></g>"##,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((10.0, -10.0), (11.0, -10.0)),
            line((0.0, -20.0), (1.0, -20.0)),
            DrawingCurve::Circle {
                center: Point2::new(50.0, -50.0),
                radius: 2.0,
            },
        ],
    );
    assert!(
        drawing
            .notes
            .contains(&"1 element nested too deeply or reusing itself was left out.".to_owned()),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn dashed_strokes_import_as_construction() {
    let drawing = read(
        r#"<g stroke-dasharray="4 2">
             <line x1="0" y1="0" x2="1" y2="0"/>
             <line style="stroke-dasharray: none" x1="0" y1="1" x2="1" y2="1"/>
           </g>
           <line x1="0" y1="2" x2="1" y2="2"/>"#,
    );

    assert_eq!(drawing.construction, BTreeSet::from([0]));
}

#[test]
fn a_switch_draws_only_its_first_unconditional_child() {
    let drawing = read(
        r#"<switch>
             <foreignObject requiredExtensions="http://example.org"/>
             <line x1="0" y1="0" x2="1" y2="0"/>
             <line x1="0" y1="1" x2="1" y2="1"/>
           </switch>"#,
    );

    assert_eq!(drawing.curves.len(), 1);
}

#[test]
fn a_sketch_exported_as_svg_reads_back_in_place() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("plate.svg");
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0));
    sketch.add_circle(Point2::new(20.0, 10.0), 4.0);
    sketch.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 10.0),
    );
    let guide = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(40.0, -10.0));
    sketch.set_construction(guide, true).unwrap();

    export_sketch(
        &path,
        &sketch,
        SketchFormat::Svg,
        Construction::OnLayer,
        &CancelToken::never(),
    )
    .unwrap();
    let drawing = read_drawing(&path, &CancelToken::never()).unwrap();

    assert_curves_close(
        &drawing.curves,
        &[
            line((0.0, 0.0), (40.0, 0.0)),
            DrawingCurve::Circle {
                center: Point2::new(20.0, 10.0),
                radius: 4.0,
            },
            DrawingCurve::Arc {
                center: Point2::ZERO,
                start: Point2::new(10.0, 0.0),
                end: Point2::new(0.0, 10.0),
            },
            line((0.0, -10.0), (40.0, -10.0)),
        ],
    );
    assert_eq!(drawing.construction, BTreeSet::from([3]));
    assert!(drawing.notes.iter().all(|note| !note.contains("unit")));
}

#[test]
fn drawings_are_told_apart_by_extension_and_content() {
    let dir = TempDir::new().unwrap();
    let named = dir.path().join("logo.SVG");
    let unnamed = dir.path().join("logo.drawing");
    let packed = dir.path().join("logo.svgz");
    let wrong = dir.path().join("photo.svg");
    let text = svg(r#"<line x1="0" y1="0" x2="1" y2="0"/>"#);
    std::fs::write(&named, &text).unwrap();
    std::fs::write(&unnamed, &text).unwrap();
    std::fs::write(&packed, gzip(&text)).unwrap();
    std::fs::write(&wrong, b"\x89PNG\r\n\x1a\n").unwrap();

    for path in [&named, &unnamed, &packed] {
        let drawing = read_drawing(path, &CancelToken::never()).unwrap();
        assert_eq!(drawing.curves.len(), 1, "{}", path.display());
    }
    assert_eq!(
        read_drawing(&wrong, &CancelToken::never()),
        Err(ImportError::NotSvg)
    );
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut packed = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    packed.extend(miniz_oxide::deflate::compress_to_vec(bytes, 6));
    packed
        .write_all(&crc32fast::hash(bytes).to_le_bytes())
        .unwrap();
    packed
        .write_all(&u32::try_from(bytes.len()).unwrap().to_le_bytes())
        .unwrap();
    packed
}

#[test]
fn other_documents_are_not_svg_drawings() {
    assert_eq!(parse_svg(b"\x89PNG\r\n\x1a\n"), Err(ImportError::NotSvg));
    assert_eq!(parse_svg(b"<html><body/></html>"), Err(ImportError::NotSvg));
    assert!(matches!(
        parse_svg(br#"<svg xmlns="http://www.w3.org/2000/svg"><text>Hi</text></svg>"#),
        Err(ImportError::Empty { left_out }) if left_out.iter().any(|note| note.contains("1 text was left out"))
    ));
    assert_eq!(parse_svg(b"<svg"), Err(ImportError::DamagedAt(1)));
    assert_eq!(parse_svg(b""), Err(ImportError::NotSvg));
}

#[test]
fn a_truncated_drawing_keeps_what_comes_before_the_damage() {
    let whole = String::from_utf8(svg(r#"<g id="a"><line x1="0" y1="0" x2="1" y2="0"/>
<line x1="0" y1="1" x2="1" y2="1"/></g>
<g id="b"><line x1="0" y1="2" x2="1" y2="2"/><line x1="0" y1="3" x2"#))
    .unwrap();
    let truncated = whole.trim_end_matches("</svg>");

    let drawing = parse_svg(truncated.as_bytes()).unwrap();

    assert_eq!(drawing.curves.len(), 3);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("The drawing is damaged near line 3")),
        "{:?}",
        drawing.notes
    );
    let damaged_attribute = svg(r#"<line x1="0" y1="0" x2="1" y2="0"/><line x1="0 y1="1"/>"#);
    assert_eq!(parse_svg(&damaged_attribute).unwrap().curves.len(), 1);
}

#[test]
fn hostile_drawings_are_refused_or_cut_short_without_panicking() {
    let depth = 100_000;
    let deep = format!(
        "{SVG_OPEN}{}<line x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"/>{}<line x1=\"0\" y1=\"1\" x2=\"1\" y2=\"1\"/></svg>",
        "<g>".repeat(depth),
        "</g>".repeat(depth)
    );
    let drawing = parse_svg(deep.as_bytes()).unwrap();
    assert_eq!(drawing.curves.len(), 1);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.contains("nested too deeply")),
        "{:?}",
        drawing.notes
    );

    let huge = read(
        r#"<line x1="1e308" y1="0" x2="-1e308" y2="0" transform="scale(1e10)"/>
           <circle r="1e300" transform="matrix(1e300 0 0 1e300 0 0)"/>
           <path d="M0 0 A1e200 1e200 0 0 1 1e-200 0 L 1e400 0"/>
           <line x1="0" y1="0" x2="1" y2="0"/>"#,
    );
    assert!(huge.curves.iter().all(|curve| match curve {
        DrawingCurve::Line { start, end } => start.is_finite() && end.is_finite(),
        _ => true,
    }));
    assert!(
        huge.notes
            .iter()
            .any(|note| note.contains("numbers too large to draw")),
        "{:?}",
        huge.notes
    );

    let mut copies = String::from(r#"<defs><g id="g0"><line x1="0" y1="0" x2="1" y2="0"/></g>"#);
    for level in 1..15 {
        copies.push_str(&format!(
            r##"<g id="g{level}">{}</g>"##,
            format!(r##"<use href="#g{}"/>"##, level - 1).repeat(4)
        ));
    }
    copies.push_str(r##"</defs><use href="#g14"/>"##);
    assert_eq!(parse_svg(&svg(&copies)), Err(ImportError::TooManyCopies));

    let laughs = br#"<?xml version="1.0"?>
<!DOCTYPE svg [
<!ENTITY a "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">
<!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
<!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
<!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
<!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;">
]>
<svg xmlns="http://www.w3.org/2000/svg"><text>&e;&e;&e;&e;&e;</text></svg>"#;
    assert!(parse_svg(laughs).is_err());
    let mut entities = String::from(r#"<!ENTITY a "1111111111111111111111111111111111111111">"#);
    for (name, inner) in "bcdefghij".chars().zip("abcdefghi".chars()) {
        entities.push_str(&format!(
            r#"<!ENTITY {name} "{}">"#,
            format!("&{inner};").repeat(16)
        ));
    }
    let bomb = format!(
        r#"<!DOCTYPE svg [{entities}]><svg xmlns="http://www.w3.org/2000/svg"><line x1="0" y1="0" x2="1" y2="0"/><line x2="&j;"/></svg>"#
    );
    let drawing = parse_svg(bomb.as_bytes()).unwrap();
    assert_eq!(drawing.curves.len(), 1);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("The drawing is damaged near line 1")),
        "{:?}",
        drawing.notes
    );

    let long_path = format!(
        r#"<path d="M0 0{}"/>"#,
        " l1 0 l0 1".repeat(crate::import::MAX_READ_CURVES)
    );
    let drawing = read(&long_path);
    assert_eq!(drawing.curves.len(), crate::import::MAX_READ_CURVES);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("Only the first")),
        "{:?}",
        drawing.notes
    );

    let marked = format!(
        r##"<marker id="m"><line x1="0" y1="0" x2="1" y2="0"/><line x1="0" y1="1" x2="1" y2="1"/></marker>
            <path d="M0 0{}" marker-mid="url(#m)"/>"##,
        " l1 0".repeat(250_000)
    );
    assert_eq!(parse_svg(&svg(&marked)), Err(ImportError::TooManyCopies));

    let unclosed = read(
        r#"<style>.a { display: none } .b { stroke: none; fill: none; {{{{ </style>
           <line class="a" x1="0" y1="0" x2="1" y2="0"/>
           <line class="b" x1="0" y1="1" x2="1" y2="1"/>
           <line x1="0" y1="2" x2="1" y2="2"/>"#,
    );
    assert_eq!(unclosed.curves.len(), 1);

    for cut in (0..deep.len().min(4_000)).step_by(37) {
        let _ = parse_svg(deep.as_bytes().get(..cut).unwrap());
    }
    let _ = parse_svg(&[0xff, 0xfe, b'<', 0, b's', 0]);
    let _ = parse_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M 0 0 A\"/></svg>");
}

#[test]
fn entities_declared_in_the_doctype_name_namespaces_as_illustrator_writes_them() {
    let drawing = parse_svg(
        br#"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator -->
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
	<!ENTITY ns_xlink "http://www.w3.org/1999/xlink">
	<!ENTITY width "10mm">
]>
<svg version="1.1" xmlns="&ns_svg;" xmlns:xlink="&ns_xlink;" width="&width;" height="10mm" viewBox="0 0 10 10">
<![CDATA[ <line/> ]]>
<line x1="0" y1="0" x2="1&#46;5" y2="0"/>
</svg>"#,
    )
    .unwrap();

    assert_curves_close(&drawing.curves, &[line((0.0, 0.0), (1.5, 0.0))]);
}

#[test]
fn latin_1_names_are_read_with_a_note() {
    let mut bytes =
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1mm" viewBox="0 0 1 1"><g id="Ma"#
            .to_vec();
    bytes.push(0xdf);
    bytes.extend(
        br#"e"><line x1="0" y1="0" x2="1" y2="0"/></g><line x1="0" y1="0" x2="0" y2="1"/></svg>"#,
    );

    let drawing = parse_svg(&bytes).unwrap();

    assert_eq!(drawing.layers[0], "Maße");
    assert!(drawing.notes[0].contains("Latin-1"), "{:?}", drawing.notes);
}

#[test]
fn style_sheets_hide_dash_and_unpaint_by_class_id_and_element() {
    let drawing = read(
        r#"<style type="text/css"><![CDATA[
             /* guides */
             .guide, #ghost { display: none }
             line.cut { stroke-dasharray: 3 1 }
             .plain { stroke-dasharray: 2 2 }
             #solid { stroke-dasharray: none }
             .invisible { stroke: none; fill: none }
             g > line { display: none }
             @media print { line { display: none } }
           ]]></style>
           <line class="guide" x1="0" y1="0" x2="1" y2="0"/>
           <line id="ghost" x1="0" y1="1" x2="1" y2="1"/>
           <line class="cut" x1="0" y1="2" x2="1" y2="2"/>
           <line class="plain" id="solid" x1="0" y1="3" x2="1" y2="3"/>
           <line class="plain" style="stroke-dasharray: none" x1="0" y1="4" x2="1" y2="4"/>
           <line class="plain" stroke-dasharray="none" x1="0" y1="5" x2="1" y2="5"/>
           <line class="invisible" x1="0" y1="6" x2="1" y2="6"/>
           <line class="invisible" stroke="black" x1="0" y1="7" x2="1" y2="7"/>
           <line class="invisible" style="stroke: black" x1="0" y1="8" x2="1" y2="8"/>"#,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((0.0, -2.0), (1.0, -2.0)),
            line((0.0, -3.0), (1.0, -3.0)),
            line((0.0, -4.0), (1.0, -4.0)),
            line((0.0, -5.0), (1.0, -5.0)),
            line((0.0, -8.0), (1.0, -8.0)),
        ],
    );
    assert_eq!(drawing.construction, BTreeSet::from([0, 3]));
    for expected in [
        "2 hidden elements were left out.",
        "2 elements drawn with neither stroke nor fill were left out.",
        "1 rule in the drawing's style sheet was ignored, because caditor reads only selectors of an element name, class or id.",
    ] {
        assert!(
            drawing.notes.contains(&expected.to_owned()),
            "{expected}: {:?}",
            drawing.notes
        );
    }
}

#[test]
fn important_declarations_and_later_rules_win_by_specificity() {
    let drawing = read(
        r#"<style>
             .a.b { display: none }
             .a { display: inline }
             line { display: none !important }
             #keep { display: inline !important }
           </style>
           <line class="a b" id="keep" style="display: none" x1="0" y1="0" x2="1" y2="0"/>
           <line class="a" style="display: inline" x1="0" y1="1" x2="1" y2="1"/>
           <rect class="a b" width="1" height="1"/>
           <circle class="a" r="1"/>"#,
    );

    assert_eq!(drawing.curves.len(), 2, "{:?}", drawing.curves);
    assert!(matches!(drawing.curves[0], DrawingCurve::Line { .. }));
    assert!(matches!(drawing.curves[1], DrawingCurve::Circle { .. }));
}

#[test]
fn a_symbol_fits_its_view_box_into_the_use_size() {
    let drawing = read(
        r##"<defs>
              <symbol id="bar" viewBox="0 0 10 10"><line x1="0" y1="0" x2="10" y2="0"/></symbol>
              <symbol id="stretched" viewBox="0 0 10 10" preserveAspectRatio="none">
                <line x1="0" y1="10" x2="10" y2="10"/>
              </symbol>
            </defs>
            <use href="#bar" x="20" y="20" width="20" height="40"/>
            <use href="#stretched" width="20" height="40"/>
            <use href="#bar" width="0" height="10"/>"##,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((20.0, -30.0), (40.0, -30.0)),
            line((0.0, -40.0), (20.0, -40.0)),
        ],
    );
}

#[test]
fn markers_are_drawn_at_the_vertices_of_a_path() {
    let drawing = read(
        r##"<defs>
              <marker id="tick" markerUnits="userSpaceOnUse" orient="auto">
                <line x1="0" y1="0" x2="-1" y2="1"/>
              </marker>
              <marker id="dot" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="2" markerHeight="2">
                <circle cx="5" cy="5" r="5"/>
              </marker>
            </defs>
            <path d="M0 0 L10 0" marker-end="url(#tick)"/>
            <line x1="50" y1="0" x2="50" y2="10" style="marker-start: url(#tick)"/>
            <polyline points="0 50 10 50 10 60" stroke="none" fill="none" stroke-width="3" marker-mid="url(#dot)"/>"##,
    );

    assert_curves_close(
        &drawing.curves,
        &[
            line((0.0, 0.0), (10.0, 0.0)),
            line((10.0, 0.0), (9.0, -1.0)),
            line((50.0, 0.0), (50.0, -10.0)),
            line((50.0, 0.0), (49.0, 1.0)),
            DrawingCurve::Circle {
                center: Point2::new(10.0, -50.0),
                radius: 3.0,
            },
        ],
    );
}

#[test]
fn a_marker_drawing_itself_is_left_out() {
    let drawing = read(
        r##"<marker id="loop" orient="auto">
              <path d="M0 0 L1 0" marker-end="url(#loop)"/>
            </marker>
            <path d="M0 0 L10 0" marker-end="url(#loop)"/>"##,
    );

    assert_eq!(drawing.curves.len(), 2);
    assert!(
        drawing
            .notes
            .contains(&"1 element nested too deeply or reusing itself was left out.".to_owned()),
        "{:?}",
        drawing.notes
    );
}
