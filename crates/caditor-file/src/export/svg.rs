use std::{f64::consts::PI, fmt::Write};

use caditor_geometry::Point2;
use caditor_sketch::{Entity, Sketch};

use super::{ExportError, SketchExported};

const MARGIN: f64 = 1.0;
const STROKE_WIDTH: f64 = 0.1;
const POINT_RADIUS: f64 = 0.25;
const SEGMENT_ANGLE: f64 = 5.0 * PI / 180.0;
const DECIMALS: usize = 6;

pub(super) fn encode(sketch: &Sketch) -> Result<(String, SketchExported), ExportError> {
    let anchors: std::collections::BTreeSet<_> = sketch
        .entities()
        .flat_map(|(_, entity)| entity.points())
        .collect();
    let mut shapes = Vec::new();
    let mut extent = Extent::default();
    let mut exported = SketchExported {
        curves: 0,
        points: 0,
        construction_left_out: 0,
    };
    for (id, entity) in sketch.entities() {
        if sketch.is_construction(id) {
            exported.construction_left_out += 1;
            continue;
        }
        let shape = match entity {
            Entity::Point(_) if anchors.contains(&id) => None,
            Entity::Point(position) => {
                exported.points += 1;
                extent.include(*position);
                Some(Shape::Point(*position))
            }
            Entity::Line { .. } => sketch.line_endpoints(id).map(|(start, end)| {
                extent.include(start);
                extent.include(end);
                Shape::Line(start, end)
            }),
            Entity::Circle { .. } => sketch.circle(id).map(|(center, radius)| {
                extent.include(center - Point2::splat(radius));
                extent.include(center + Point2::splat(radius));
                Shape::Circle(center, radius)
            }),
            Entity::Arc { .. } => sketch.arc(id).map(|arc| {
                for point in arc.polyline(SEGMENT_ANGLE) {
                    extent.include(point);
                }
                if arc.sweep >= 2.0 * PI {
                    Shape::Circle(arc.center, arc.radius)
                } else {
                    Shape::Arc {
                        radius: arc.radius,
                        start: arc.point_at(arc.start_angle),
                        end: arc.point_at(arc.end_angle()),
                        large: arc.sweep > PI,
                    }
                }
            }),
            Entity::Spline { .. } => sketch.spline(id).map(|spline| {
                let points = spline.polyline(SEGMENT_ANGLE);
                for point in &points {
                    extent.include(*point);
                }
                Shape::Polyline(points)
            }),
        };
        if let Some(shape) = shape {
            if !matches!(shape, Shape::Point(_)) {
                exported.curves += 1;
            }
            shapes.push(shape);
        }
    }
    let Some((low, high)) = extent.corners() else {
        return Err(ExportError::NoCurves);
    };
    let mut text = String::new();
    write_document(&mut text, &shapes, low, high).map_err(|_| ExportError::Encoding)?;
    Ok((text, exported))
}

#[derive(Debug, Clone, PartialEq)]
enum Shape {
    Point(Point2),
    Line(Point2, Point2),
    Circle(Point2, f64),
    Arc {
        radius: f64,
        start: Point2,
        end: Point2,
        large: bool,
    },
    Polyline(Vec<Point2>),
}

#[derive(Default)]
struct Extent(Option<(Point2, Point2)>);

impl Extent {
    fn include(&mut self, point: Point2) {
        self.0 = Some(match self.0 {
            Some((low, high)) => (low.min(point), high.max(point)),
            None => (point, point),
        });
    }

    fn corners(&self) -> Option<(Point2, Point2)> {
        self.0
    }
}

fn write_document(
    text: &mut String,
    shapes: &[Shape],
    low: Point2,
    high: Point2,
) -> std::fmt::Result {
    let width = high.x - low.x + 2.0 * MARGIN;
    let height = high.y - low.y + 2.0 * MARGIN;
    writeln!(
        text,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="{} {} {} {}">"#,
        Real(width),
        Real(height),
        Real(low.x - MARGIN),
        Real(-high.y - MARGIN),
        Real(width),
        Real(height)
    )?;
    writeln!(
        text,
        r##"<g fill="none" stroke="#000000" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round">"##,
        Real(STROKE_WIDTH)
    )?;
    for shape in shapes {
        match shape {
            Shape::Point(point) => writeln!(
                text,
                r##"<circle cx="{}" cy="{}" r="{}" fill="#000000" stroke="none"/>"##,
                Real(point.x),
                Real(-point.y),
                Real(POINT_RADIUS)
            )?,
            Shape::Line(start, end) => writeln!(
                text,
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
                Real(start.x),
                Real(-start.y),
                Real(end.x),
                Real(-end.y)
            )?,
            Shape::Circle(center, radius) => writeln!(
                text,
                r#"<circle cx="{}" cy="{}" r="{}"/>"#,
                Real(center.x),
                Real(-center.y),
                Real(*radius)
            )?,
            Shape::Arc {
                radius,
                start,
                end,
                large,
            } => writeln!(
                text,
                r#"<path d="M {} {} A {} {} 0 {} 0 {} {}"/>"#,
                Real(start.x),
                Real(-start.y),
                Real(*radius),
                Real(*radius),
                u8::from(*large),
                Real(end.x),
                Real(-end.y)
            )?,
            Shape::Polyline(points) => {
                write!(text, r#"<polyline points=""#)?;
                for (index, point) in points.iter().enumerate() {
                    let separator = if index == 0 { "" } else { " " };
                    write!(text, "{separator}{},{}", Real(point.x), Real(-point.y))?;
                }
                writeln!(text, r#""/>"#)?;
            }
        }
    }
    writeln!(text, "</g>")?;
    writeln!(text, "</svg>")
}

struct Real(f64);

impl std::fmt::Display for Real {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fixed = format!("{:.DECIMALS$}", self.0);
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        match trimmed {
            "-0" | "" => formatter.write_str("0"),
            trimmed => formatter.write_str(trimmed),
        }
    }
}
