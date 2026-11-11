use std::{
    f64::consts::{PI, TAU},
    fmt::Write,
};

use caditor_expression::format_decimals;
use caditor_geometry::Point2;

use super::{
    ExportError,
    figure::{Anchor, Ellipse, Figure, Layer, Shape, Text},
};

const MARGIN: f64 = 1.0;
const STROKE_WIDTH: f64 = 0.1;
const POINT_RADIUS: f64 = 0.25;
const DECIMALS: usize = 6;
const DASH: f64 = 1.0;
const GAP: f64 = 0.5;
const VISIBLE_WIDTH: f64 = 0.5;
const HIDDEN_WIDTH: f64 = 0.25;
const HIDDEN_DASH: f64 = 3.0;
const HIDDEN_GAP: f64 = 1.5;
const FRAME_WIDTH: f64 = 0.35;
const THIN_WIDTH: f64 = 0.25;
const CHAIN: &str = "12 3 0.5 3";
const CAP_HEIGHT_PER_FONT_SIZE: f64 = 0.727;

#[derive(Debug, Clone, Copy)]
struct Page {
    low: Point2,
    high: Point2,
    margin: f64,
}

pub(super) fn encode(figure: &Figure) -> Result<String, ExportError> {
    let Some((low, high)) = extent(figure) else {
        return Err(ExportError::NoCurves);
    };
    let mut text = String::new();
    let page = Page {
        low,
        high,
        margin: MARGIN,
    };
    write_document(&mut text, figure, page).map_err(|_| ExportError::Encoding)?;
    Ok(text)
}

pub(super) fn encode_page(figure: &Figure, width: f64, height: f64) -> Result<String, ExportError> {
    let mut text = String::new();
    let page = Page {
        low: Point2::ZERO,
        high: Point2::new(width, height),
        margin: 0.0,
    };
    write_document(&mut text, figure, page).map_err(|_| ExportError::Encoding)?;
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

fn write_document(text: &mut String, figure: &Figure, page: Page) -> std::fmt::Result {
    let Page { low, high, margin } = page;
    let width = high.x - low.x + 2.0 * margin;
    let height = high.y - low.y + 2.0 * margin;
    writeln!(
        text,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="{} {} {} {}">"#,
        Real(width),
        Real(height),
        Real(low.x - margin),
        Real(-high.y - margin),
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
            Layer::Outline | Layer::Holes | Layer::Dimensions | Layer::Labels => {
                writeln!(text, r#"<g id="{}">"#, layer.name())?;
            }
            Layer::Visible => writeln!(
                text,
                r#"<g id="{}" stroke-width="{}">"#,
                layer.name(),
                Real(VISIBLE_WIDTH)
            )?,
            Layer::Hidden => writeln!(
                text,
                r#"<g id="{}" stroke-width="{}" stroke-dasharray="{} {}" stroke-linecap="butt">"#,
                layer.name(),
                Real(HIDDEN_WIDTH),
                Real(HIDDEN_DASH),
                Real(HIDDEN_GAP)
            )?,
            Layer::Hatching => writeln!(
                text,
                r#"<g id="{}" stroke-width="{}">"#,
                layer.name(),
                Real(THIN_WIDTH)
            )?,
            Layer::Cutting => writeln!(
                text,
                r#"<g id="{}" stroke-width="{}" stroke-dasharray="{CHAIN}" stroke-linecap="butt">"#,
                layer.name(),
                Real(THIN_WIDTH)
            )?,
            Layer::Frame => writeln!(
                text,
                r#"<g id="{}" stroke-width="{}">"#,
                layer.name(),
                Real(FRAME_WIDTH)
            )?,
        }
        let font_scale = if layer.is_on_sheet() {
            1.0 / CAP_HEIGHT_PER_FONT_SIZE
        } else {
            1.0
        };
        for (_, shape) in figure.shapes.iter().filter(|(on, _)| *on == layer) {
            write_shape(text, shape, font_scale)?;
        }
        if grouped {
            writeln!(text, "</g>")?;
        }
    }
    writeln!(text, "</g>")?;
    writeln!(text, "</svg>")
}

fn write_shape(text: &mut String, shape: &Shape, font_scale: f64) -> std::fmt::Result {
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
        Shape::Text(label) => write_text(text, label, font_scale),
        Shape::Dimension(dimension) => {
            for mark in &dimension.marks {
                write_shape(text, mark, font_scale)?;
            }
            write_text(text, &dimension.text, font_scale)
        }
    }
}

fn write_text(text: &mut String, label: &Text, font_scale: f64) -> std::fmt::Result {
    let (anchor, baseline) = match label.anchor {
        Anchor::Start => ("start", "auto"),
        Anchor::Middle => ("middle", "central"),
    };
    let rotation = -label.upright_angle().to_degrees();
    write!(
        text,
        r##"<text x="{x}" y="{y}" font-size="{}" font-family="sans-serif" text-anchor="{anchor}" dominant-baseline="{baseline}" fill="#000000" stroke="none""##,
        Real(label.height * font_scale),
        x = Real(label.at.x),
        y = Real(-label.at.y),
    )?;
    if rotation.abs() > 0.0 {
        write!(
            text,
            r#" transform="rotate({} {} {})""#,
            Real(rotation),
            Real(label.at.x),
            Real(-label.at.y)
        )?;
    }
    write!(text, ">")?;
    for character in label.content.chars() {
        match character {
            '&' => text.push_str("&amp;"),
            '<' => text.push_str("&lt;"),
            '>' => text.push_str("&gt;"),
            character if character.is_control() => text.push(' '),
            character => text.push(character),
        }
    }
    writeln!(text, "</text>")
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
        formatter.write_str(&format_decimals(self.0, DECIMALS))
    }
}
