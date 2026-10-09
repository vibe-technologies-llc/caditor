use caditor_document::CancelToken;
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Vector2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};
use tempfile::TempDir;

use super::{
    figure::{Motion, Shape},
    sheet::{Item, nest},
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

#[test]
fn nesting_keeps_every_part_apart_and_on_the_sheet() {
    let items: Vec<Item> = (0..40)
        .map(|index| {
            let size = Vector2::new(
                10.0 + f64::from(index * 37 % 50),
                5.0 + f64::from(index * 23 % 30),
            );
            Item::of(size, 0.0, 0.0)
        })
        .collect();
    let nesting = Nesting::new(200.0, 3.0, true).unwrap();

    let nested = nest(&items, &nesting, &CancelToken::never()).unwrap();
    let again = nest(&items, &nesting, &CancelToken::never()).unwrap();

    assert_eq!(nested, again);
    assert_eq!(nested.too_wide, 0);
    let rects: Vec<(Point2, Point2)> = nested
        .placements
        .iter()
        .zip(&items)
        .map(|(placement, item)| {
            let extent = if placement.turned {
                item.turned
            } else {
                item.upright
            };
            (placement.at, placement.at + extent)
        })
        .collect();
    for (index, (low, high)) in rects.iter().enumerate() {
        assert!(low.x >= -1e-9 && low.y >= -1e-9, "{low:?}");
        assert!(high.x <= 200.0 + 1e-9, "{high:?}");
        for (other_low, other_high) in &rects[index + 1..] {
            let apart = high.x + 3.0 <= other_low.x + 1e-9
                || other_high.x + 3.0 <= low.x + 1e-9
                || high.y + 3.0 <= other_low.y + 1e-9
                || other_high.y + 3.0 <= low.y + 1e-9;
            assert!(apart, "{low:?}-{high:?} meets {other_low:?}-{other_high:?}");
        }
    }
    let used = rects.iter().map(|(_, high)| high.y).fold(0.0, f64::max);
    let area: f64 = items
        .iter()
        .map(|item| item.upright.x * item.upright.y)
        .sum();
    assert!(used * 200.0 < area * 2.0, "the sheet is {used} high");
}

#[test]
fn a_quarter_turn_fits_a_long_part_on_a_narrow_sheet() {
    let long = [Item::of(Vector2::new(150.0, 20.0), 0.0, 0.0)];
    let turning = Nesting::new(100.0, 5.0, true).unwrap();
    let kept = Nesting::new(100.0, 5.0, false).unwrap();

    let turned = nest(&long, &turning, &CancelToken::never()).unwrap();
    let too_wide = nest(&long, &kept, &CancelToken::never()).unwrap();

    assert!(turned.placements[0].turned);
    assert_eq!(turned.too_wide, 0);
    assert!(!too_wide.placements[0].turned);
    assert_eq!(too_wide.too_wide, 1);
    assert!(Nesting::new(0.0, 5.0, true).is_none());
    assert!(Nesting::new(100.0, -1.0, true).is_none());
    assert!(Nesting::new(f64::NAN, 5.0, true).is_none());
}

#[test]
fn a_turned_arc_keeps_its_points() {
    let arc = Shape::Arc {
        center: Point2::new(3.0, 4.0),
        radius: 2.0,
        start: 0.2,
        end: 1.4,
    };
    let motion = Motion {
        turned: true,
        offset: Vector2::new(10.0, -5.0),
    };

    let moved = arc.clone().moved(motion);

    let points = arc.outline_points();
    let moved_points = moved.outline_points();
    assert_eq!(points.len(), moved_points.len());
    for (point, moved) in points.iter().zip(&moved_points) {
        assert!(motion.point(*point).distance(*moved) < 1e-9);
    }
    assert!((motion.vector(Vector2::X) - Vector2::Y).length() < 1e-12);
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
