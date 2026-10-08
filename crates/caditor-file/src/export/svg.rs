use std::{
    f64::consts::{PI, TAU},
    fmt::Write,
};

use caditor_geometry::Point2;

use super::{
    ExportError,
    figure::{Ellipse, Figure, Layer, Shape},
};

const MARGIN: f64 = 1.0;
const STROKE_WIDTH: f64 = 0.1;
const POINT_RADIUS: f64 = 0.25;
const DECIMALS: usize = 6;
const DASH: f64 = 1.0;
const GAP: f64 = 0.5;

pub(super) fn encode(figure: &Figure) -> Result<String, ExportError> {
    let Some((low, high)) = extent(figure) else {
        return Err(ExportError::NoCurves);
    };
    let mut text = String::new();
    write_document(&mut text, figure, low, high).map_err(|_| ExportError::Encoding)?;
    Ok(text)
}

fn extent(figure: &Figure) -> Option<(Point2, Point2)> {
    figure
        .shapes
        .iter()
        .flat_map(|(_, shape)| shape.outline_points())
        .fold(None, |extent, point| {
            Some(match extent {
                Some((low, high)) => (Point2::min(low, point), Point2::max(high, point)),
                None => (point, point),
            })
        })
}

fn write_document(
    text: &mut String,
    figure: &Figure,
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
    for layer in figure.layers() {
        let grouped = layer != Layer::Sketch;
        match layer {
            Layer::Sketch => {}
            Layer::Construction => writeln!(
                text,
                r##"<g id="{}" stroke="#808080" stroke-dasharray="{} {}">"##,
                layer.name(),
                Real(DASH),
                Real(GAP)
            )?,
            Layer::Outline | Layer::Holes => writeln!(text, r#"<g id="{}">"#, layer.name())?,
        }
        for (_, shape) in figure.shapes.iter().filter(|(on, _)| *on == layer) {
            write_shape(text, shape)?;
        }
        if grouped {
            writeln!(text, "</g>")?;
        }
    }
    writeln!(text, "</g>")?;
    writeln!(text, "</svg>")
}

fn write_shape(text: &mut String, shape: &Shape) -> std::fmt::Result {
    match shape {
        Shape::Point(point) => writeln!(
            text,
            r##"<circle cx="{}" cy="{}" r="{}" fill="#000000" stroke="none"/>"##,
            Real(point.x),
            Real(-point.y),
            Real(POINT_RADIUS)
        ),
        Shape::Line(start, end) => writeln!(
            text,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
            Real(start.x),
            Real(-start.y),
            Real(end.x),
            Real(-end.y)
        ),
        Shape::Circle { center, radius } => writeln!(
            text,
            r#"<circle cx="{}" cy="{}" r="{}"/>"#,
            Real(center.x),
            Real(-center.y),
            Real(*radius)
        ),
        Shape::Arc {
            center,
            radius,
            start,
            end,
        } => {
            let from = Shape::arc_point(*center, *radius, *start);
            let to = Shape::arc_point(*center, *radius, *end);
            writeln!(
                text,
                r#"<path d="M {} {} A {} {} 0 {} 0 {} {}"/>"#,
                Real(from.x),
                Real(-from.y),
                Real(*radius),
                Real(*radius),
                u8::from(end - start > PI),
                Real(to.x),
                Real(-to.y)
            )
        }
        Shape::Ellipse(ellipse) => write_ellipse(text, ellipse),
        Shape::Spline(spline) => write_polyline(text, &spline.polyline),
        Shape::Polyline(points) => write_polyline(text, points),
    }
}

fn write_ellipse(text: &mut String, ellipse: &Ellipse) -> std::fmt::Result {
    let major = ellipse.major.length();
    let minor = major * ellipse.ratio;
    let rotation = -ellipse.rotation().to_degrees();
    let (start, end) = if ellipse.is_full() {
        (0.0, TAU)
    } else {
        (ellipse.start, ellipse.end)
    };
    let pieces: &[(f64, f64)] = if ellipse.is_full() {
        &[(start, start + PI), (start + PI, end)]
    } else {
        &[(start, end)]
    };
    let from = ellipse.point_at(start);
    write!(text, r#"<path d="M {} {}"#, Real(from.x), Real(-from.y))?;
    for (piece_start, piece_end) in pieces {
        let to = ellipse.point_at(*piece_end);
        write!(
            text,
            " A {} {} {} {} 0 {} {}",
            Real(major),
            Real(minor),
            Real(rotation),
            u8::from(piece_end - piece_start > PI),
            Real(to.x),
            Real(-to.y)
        )?;
    }
    writeln!(text, r#""/>"#)
}

fn write_polyline(text: &mut String, points: &[Point2]) -> std::fmt::Result {
    write!(text, r#"<polyline points=""#)?;
    for (index, point) in points.iter().enumerate() {
        let separator = if index == 0 { "" } else { " " };
        write!(text, "{separator}{},{}", Real(point.x), Real(-point.y))?;
    }
    writeln!(text, r#""/>"#)
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
