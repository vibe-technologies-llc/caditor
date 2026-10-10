use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use caditor_document::CancelToken;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use tempfile::TempDir;

use super::{
    figure::{Figure, Layer, Motion, Shape},
    nest::{Item, nest},
    *,
};

fn rectangle(low: Point2, high: Point2) -> (Sketch, Vec<EntityId>) {
    let mut sketch = Sketch::new(Plane::XY);
    let corners = [
        low,
        Point2::new(high.x, low.y),
        high,
        Point2::new(low.x, high.y),
    ];
    let lines = (0..4)
        .map(|index| sketch.add_line(corners[index], corners[(index + 1) % 4]))
        .collect();
    (sketch, lines)
}

fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
    match sketch.entity(line) {
        Some(Entity::Line { start, end }) => (*start, *end),
        other => panic!("not a line: {other:?}"),
    }
}

fn dimensioned() -> Sketch {
    let (mut sketch, lines) = rectangle(Point2::ZERO, Point2::new(60.0, 30.0));
    let (start, end) = ends(&sketch, lines[0]);
    let hole = sketch.add_circle(Point2::new(20.0, 15.0), 5.0);
    let slot = sketch.add_circle(Point2::new(45.0, 15.0), 4.0);
    let slant = sketch.add_line(Point2::new(70.0, 0.0), Point2::new(80.0, 10.0));
    sketch
        .add_constraint(Constraint::HorizontalDistance {
            from: start,
            to: end,
            value: Expression::Measure(60.0, Unit::Millimetre),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Distance {
            from: lines[1],
            to: lines[3],
            value: Expression::Measure(60.0, Unit::Millimetre),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Radius {
            entity: hole,
            value: Expression::Measure(5.0, Unit::Millimetre),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Diameter {
            entity: slot,
            value: Expression::Measure(8.0, Unit::Millimetre),
        })
        .unwrap();
    sketch
        .add_constraint(Constraint::Angle {
            from: lines[0],
            to: slant,
            reversed: false,
            value: Expression::Measure(45.0, Unit::Degree),
        })
        .unwrap();
    sketch
}

fn annotated() -> DrawingSheet {
    DrawingSheet {
        annotations: Annotations::Included,
        ..DrawingSheet::default()
    }
}

fn line_bounds(drawing: &crate::Drawing, range: std::ops::Range<usize>) -> (Point2, Point2) {
    drawing.curves[range]
        .iter()
        .flat_map(|curve| match curve {
            crate::DrawingCurve::Line { start, end } => vec![*start, *end],
            other => panic!("not a line: {other:?}"),
        })
        .fold(
            (
                Point2::splat(f64::INFINITY),
                Point2::splat(f64::NEG_INFINITY),
            ),
            |(low, high), point| (low.min(point), high.max(point)),
        )
}

fn outline(corners: &[(f64, f64)]) -> Figure {
    let mut figure = Figure::default();
    for (index, (x, y)) in corners.iter().enumerate() {
        let (next_x, next_y) = corners[(index + 1) % corners.len()];
        figure.push(
            Layer::Outline,
            Shape::Line(Point2::new(*x, *y), Point2::new(next_x, next_y)),
        );
    }
    figure
}

fn box_of(width: f64, height: f64) -> Figure {
    outline(&[(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)])
}

fn item(figure: &Figure) -> Item<'_> {
    Item {
        figure,
        label: 0.0,
        band: 0.0,
    }
}

fn placed(figure: &Figure, motion: Motion) -> Figure {
    let mut moved = Figure::default();
    moved.append_moved(figure.clone(), motion);
    moved
}

fn segments(figure: &Figure) -> Vec<(Point2, Point2)> {
    figure
        .shapes
        .iter()
        .flat_map(|(_, shape)| shape.traced())
        .flat_map(|line| {
            line.windows(2)
                .map(|pair| (pair[0], pair[1]))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn point_to_segment(point: Point2, (from, to): (Point2, Point2)) -> f64 {
    let along = to - from;
    let t = if along.length_squared() == 0.0 {
        0.0
    } else {
        ((point - from).dot(along) / along.length_squared()).clamp(0.0, 1.0)
    };
    point.distance(from + along * t)
}

fn segment_distance(first: (Point2, Point2), second: (Point2, Point2)) -> f64 {
    let crosses = {
        let side = |(a, b): (Point2, Point2), point: Point2| (b - a).perp_dot(point - a);
        side(first, second.0) * side(first, second.1) < 0.0
            && side(second, first.0) * side(second, first.1) < 0.0
    };
    if crosses {
        return 0.0;
    }
    [
        point_to_segment(first.0, second),
        point_to_segment(first.1, second),
        point_to_segment(second.0, first),
        point_to_segment(second.1, first),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

fn inside(point: Point2, figure: &Figure) -> bool {
    segments(figure)
        .into_iter()
        .filter(|(from, to)| (from.y > point.y) != (to.y > point.y))
        .filter(|(from, to)| {
            let x = from.x + (point.y - from.y) * (to.x - from.x) / (to.y - from.y);
            x > point.x
        })
        .count()
        % 2
        == 1
}

fn assert_apart(figures: &[Figure], spacing: f64, width: f64) {
    for (index, figure) in figures.iter().enumerate() {
        let bounds = figure.bounds().unwrap();
        assert!(
            bounds.min().x >= -1e-9 && bounds.min().y >= -1e-9,
            "{bounds:?}"
        );
        assert!(bounds.max().x <= width + 1e-9, "{bounds:?}");
        for other in &figures[index + 1..] {
            let gap = segments(figure)
                .into_iter()
                .flat_map(|first| {
                    segments(other)
                        .into_iter()
                        .map(move |second| segment_distance(first, second))
                })
                .fold(f64::INFINITY, f64::min);
            assert!(gap >= spacing - 1e-9, "parts {gap} apart");
            let (own, theirs) = (segments(figure)[0].0, segments(other)[0].0);
            assert!(!inside(own, other) && !inside(theirs, figure));
        }
    }
}

#[test]
fn nesting_keeps_every_part_apart_and_on_the_sheet() {
    let figures: Vec<Figure> = (0..40)
        .map(|index| {
            box_of(
                10.0 + f64::from(index * 37 % 50),
                5.0 + f64::from(index * 23 % 30),
            )
        })
        .collect();
    let items: Vec<Item<'_>> = figures.iter().map(item).collect();
    let nesting = Nesting::new(200.0, 3.0, true).unwrap();

    let nested = nest(&items, &nesting, &CancelToken::never()).unwrap();
    let again = nest(&items, &nesting, &CancelToken::never()).unwrap();

    assert_eq!(nested, again);
    assert_eq!(nested.too_wide, 0);
    let moved: Vec<Figure> = figures
        .iter()
        .zip(&nested.motions)
        .map(|(figure, motion)| placed(figure, *motion))
        .collect();
    assert_apart(&moved, 3.0, 200.0);
    let used = moved
        .iter()
        .map(|figure| figure.bounds().unwrap().max().y)
        .fold(0.0, f64::max);
    let area: f64 = figures
        .iter()
        .map(|figure| figure.bounds().unwrap().size())
        .map(|size| size.x * size.y)
        .sum();
    assert!(used * 200.0 < area * 2.0, "the sheet is {used} high");
}

#[test]
fn a_small_part_nests_inside_a_ring_and_the_crook_of_an_l() {
    let mut ring = Figure::default();
    for radius in [50.0, 40.0] {
        ring.push(
            Layer::Outline,
            Shape::Circle {
                center: Point2::new(50.0, 50.0),
                radius,
            },
        );
    }
    let square = box_of(20.0, 20.0);
    let nesting = Nesting::new(110.0, 2.0, false).unwrap();

    let nested = nest(
        &[item(&ring), item(&square)],
        &nesting,
        &CancelToken::never(),
    )
    .unwrap();

    assert_eq!(nested.too_wide, 0);
    let ring_placed = placed(&ring, nested.motions[0]);
    let square_placed = placed(&square, nested.motions[1]);
    let centre = ring_placed.bounds().unwrap().center();
    for (corner, _) in segments(&square_placed) {
        assert!(
            corner.distance(centre) < 40.0,
            "{corner:?} is outside the hollow"
        );
    }
    assert_apart(&[ring_placed, square_placed], 2.0, 110.0);

    let l = outline(&[
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 30.0),
        (30.0, 30.0),
        (30.0, 100.0),
        (0.0, 100.0),
    ]);
    let block = box_of(40.0, 40.0);
    let nesting = Nesting::new(105.0, 5.0, false).unwrap();

    let nested = nest(&[item(&l), item(&block)], &nesting, &CancelToken::never()).unwrap();

    let l_placed = placed(&l, nested.motions[0]);
    let block_placed = placed(&block, nested.motions[1]);
    assert!(block_placed.bounds().unwrap().min().y < 100.0);
    assert_apart(&[l_placed, block_placed], 5.0, 105.0);
}

#[test]
fn parts_turn_in_small_steps_to_fit_a_narrow_sheet() {
    let long = box_of(150.0, 20.0);
    let turning = Nesting::new(100.0, 5.0, true).unwrap();
    let kept = Nesting::new(100.0, 5.0, false).unwrap();

    let turned = nest(&[item(&long)], &turning, &CancelToken::never()).unwrap();
    let too_wide = nest(&[item(&long)], &kept, &CancelToken::never()).unwrap();

    let quarters = turned.motions[0].turn / FRAC_PI_2;
    assert_eq!(quarters, quarters.round());
    assert_eq!(quarters.rem_euclid(2.0), 1.0);
    assert_eq!(turned.too_wide, 0);
    assert_eq!(too_wide.motions[0].turn, 0.0);
    assert_eq!(too_wide.too_wide, 1);
    assert!(Nesting::new(0.0, 5.0, true).is_none());
    assert!(Nesting::new(100.0, -1.0, true).is_none());
    assert!(Nesting::new(f64::NAN, 5.0, true).is_none());

    let diagonal = placed(&box_of(150.0, 10.0), Motion::turn(FRAC_PI_4));

    let nested = nest(&[item(&diagonal)], &turning, &CancelToken::never()).unwrap();

    assert_eq!(nested.too_wide, 0);
    let fitted = placed(&diagonal, nested.motions[0]);
    let bounds = fitted.bounds().unwrap();
    assert!(bounds.size().x < 15.0, "{bounds:?}");
    assert_apart(&[fitted], 5.0, 100.0);
}

#[test]
fn a_turned_arc_keeps_its_points() {
    let arc = Shape::Arc {
        center: Point2::new(3.0, 4.0),
        radius: 2.0,
        start: 0.2,
        end: 1.4,
    };
    for turn in [FRAC_PI_2, 0.7] {
        let motion = Motion {
            turn,
            offset: Vector2::new(10.0, -5.0),
        };

        let moved = arc.clone().moved(motion);

        let points = arc.outline_points();
        let moved_points = moved.outline_points();
        assert_eq!(points.len(), moved_points.len());
        for (point, moved) in points.iter().zip(&moved_points) {
            assert!(motion.point(*point).distance(*moved) < 1e-9);
        }
    }
    let quarter = Motion::turn(FRAC_PI_2);
    assert_eq!(quarter.vector(Vector2::X), Vector2::Y);
}

#[test]
fn several_sketches_nest_into_one_dxf_that_reads_back_without_overlaps() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested.dxf");
    let sketches = [
        rectangle(Point2::new(500.0, 500.0), Point2::new(560.0, 520.0)).0,
        rectangle(Point2::new(-40.0, 0.0), Point2::new(0.0, 90.0)).0,
        rectangle(Point2::ZERO, Point2::new(30.0, 30.0)).0,
    ];
    let named: Vec<NamedSketch<'_>> = sketches
        .iter()
        .zip(["Lid", "Side", "Foot"])
        .map(|(sketch, name)| NamedSketch { name, sketch })
        .collect();
    let sheet = DrawingSheet {
        layout: SheetLayout::Nested(Nesting::new(120.0, 5.0, true).unwrap()),
        ..DrawingSheet::default()
    };

    let exported = export_sketches(
        &path,
        &named,
        SketchFormat::Dxf,
        &sheet,
        &CancelToken::never(),
    )
    .unwrap();
    let drawing = crate::parse_dxf(&std::fs::read(&path).unwrap()).unwrap();

    assert_eq!(exported.sketches, 3);
    assert_eq!(exported.curves, 12);
    assert_eq!(exported.too_wide, 0);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
    assert_eq!(drawing.curves.len(), 12);
    let parts: Vec<(Point2, Point2)> = (0..3)
        .map(|part| line_bounds(&drawing, part * 4..part * 4 + 4))
        .collect();
    for (index, (low, high)) in parts.iter().enumerate() {
        assert!(low.x >= -1e-9 && low.y >= -1e-9, "{low:?}");
        assert!(high.x <= 120.0 + 1e-9, "{high:?}");
        for (other_low, other_high) in &parts[index + 1..] {
            let apart = high.x + 5.0 <= other_low.x + 1e-9
                || other_high.x + 5.0 <= low.x + 1e-9
                || high.y + 5.0 <= other_low.y + 1e-9
                || other_high.y + 5.0 <= low.y + 1e-9;
            assert!(apart, "{low:?}-{high:?} meets {other_low:?}-{other_high:?}");
        }
    }
    let tallest = parts.iter().map(|(_, high)| high.y).fold(0.0, f64::max);
    assert!(tallest < 90.0 + 30.0 + 5.0 + 1e-9, "{tallest}");
}

#[test]
fn several_sketches_side_by_side_leave_the_first_where_it_is() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("row.dxf");
    let first = rectangle(Point2::new(5.0, 5.0), Point2::new(25.0, 15.0)).0;
    let second = rectangle(Point2::ZERO, Point2::new(10.0, 10.0)).0;

    export_sketches(
        &path,
        &[
            NamedSketch {
                name: "A",
                sketch: &first,
            },
            NamedSketch {
                name: "B",
                sketch: &second,
            },
        ],
        SketchFormat::Dxf,
        &DrawingSheet::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let drawing = crate::parse_dxf(&std::fs::read(&path).unwrap()).unwrap();

    assert_eq!(
        line_bounds(&drawing, 0..4),
        (Point2::new(5.0, 5.0), Point2::new(25.0, 15.0))
    );
    let (low, high) = line_bounds(&drawing, 4..8);
    assert!(
        (low.x - 35.0).abs() < 1e-9 && (low.y - 5.0).abs() < 1e-9,
        "{low:?}"
    );
    assert!((high - low - Vector2::splat(10.0)).length() < 1e-9);
}

#[test]
fn dimensions_and_labels_are_written_as_dxf_annotations_the_importer_leaves_out() {
    let sketch = dimensioned();
    let dir = TempDir::new().unwrap();
    let plain = dir.path().join("plain.dxf");
    let path = dir.path().join("dimensioned.dxf");
    let named = [NamedSketch {
        name: "Bracket ⌀ 50% é",
        sketch: &sketch,
    }];

    let without = export_sketches(
        &plain,
        &named,
        SketchFormat::Dxf,
        &DrawingSheet::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let exported = export_sketches(
        &path,
        &named,
        SketchFormat::Dxf,
        &annotated(),
        &CancelToken::never(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let drawing = crate::parse_dxf(text.as_bytes()).unwrap();
    let bare = crate::parse_dxf(&std::fs::read(&plain).unwrap()).unwrap();

    assert_eq!(without.dimensions, 0);
    assert_eq!(exported.dimensions, 5);
    assert_eq!(text.matches("\nDIMENSION\n").count(), 5);
    assert_eq!(text.matches("\nBLOCK\n").count(), 5);
    assert!(text.contains("  2\n*D5\n"));
    assert!(text.contains("\nAcDbRotatedDimension\n"));
    assert!(text.contains("\nAcDbRadialDimension\n"));
    assert!(text.contains("\nAcDbDiametricDimension\n"));
    assert!(text.contains("\nAcDb3PointAngularDimension\n"));
    assert!(text.contains("  1\n60\n"));
    assert!(text.contains("  1\nR5\n"));
    assert!(text.contains("  1\n%%c8\n"));
    assert!(text.contains("  1\n45%%d\n"));
    assert!(text.contains("  8\nLabels\n"));
    assert!(text.contains("  1\nBracket %%c 50%%% \\U+00E9\n"));
    assert_eq!(drawing.curves, bare.curves);
    assert!(
        drawing.notes.iter().any(|note| note.contains("dimension")),
        "{:?}",
        drawing.notes
    );
    assert!(
        drawing.notes.iter().any(|note| note.contains("text")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn dimensions_and_labels_are_drawn_as_svg_text_in_groups_of_their_own() {
    let sketch = dimensioned();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dimensioned.svg");

    export_sketches(
        &path,
        &[NamedSketch {
            name: "Plate <A & B>",
            sketch: &sketch,
        }],
        SketchFormat::Svg,
        &annotated(),
        &CancelToken::never(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();

    assert!(text.contains(r#"<g id="Dimensions">"#));
    assert!(text.contains(r#"<g id="Labels">"#));
    assert!(text.contains(">R5</text>"));
    assert!(text.contains(">⌀8</text>"));
    assert!(text.contains(">45°</text>"));
    assert!(text.contains(">Plate &lt;A &amp; B&gt;</text>"));
    assert_eq!(text.matches("<text ").count(), 6);
}

#[test]
fn a_part_wider_than_the_sheet_is_placed_alone_with_its_name_below() {
    let (sketch, _) = rectangle(Point2::ZERO, Point2::new(40.0, 20.0));
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("labelled.svg");
    let sheet = DrawingSheet {
        layout: SheetLayout::Nested(Nesting::new(30.0, 2.0, false).unwrap()),
        ..annotated()
    };

    let exported = export_sketches(
        &path,
        &[NamedSketch {
            name: "Wide",
            sketch: &sketch,
        }],
        SketchFormat::Svg,
        &sheet,
        &CancelToken::never(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();

    assert_eq!(exported.too_wide, 1);
    assert!(
        text.contains(r#"<text x="0" y="-1.25" font-size="2.5""#),
        "{text}"
    );
    assert!(
        text.contains(r#"<line x1="0" y1="-5" x2="40" y2="-5"/>"#),
        "{text}"
    );
}

#[test]
fn an_ellipse_writes_its_radii_as_leaders_along_its_axes() {
    let mut sketch = Sketch::new(Plane::XY);
    let ellipse = sketch.add_ellipse(Point2::new(10.0, 5.0), Point2::new(18.0, 5.0), 3.0);
    for constraint in [
        Constraint::MajorRadius {
            ellipse,
            value: Expression::Measure(8.0, Unit::Millimetre),
        },
        Constraint::MinorRadius {
            ellipse,
            value: Expression::Measure(3.0, Unit::Millimetre),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ellipse.dxf");

    let exported = export_sketches(
        &path,
        &[NamedSketch {
            name: "Ellipse",
            sketch: &sketch,
        }],
        SketchFormat::Dxf,
        &annotated(),
        &CancelToken::never(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();

    assert_eq!(exported.dimensions, 2);
    assert_eq!(text.matches("\nAcDbRadialDimension\n").count(), 2);
    assert!(text.contains("  1\nR8\n"));
    assert!(text.contains("  1\nR3\n"));
}
