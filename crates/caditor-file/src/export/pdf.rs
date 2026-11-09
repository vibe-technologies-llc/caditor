use std::{
    f64::consts::{FRAC_PI_2, TAU},
    fmt::Write,
};

use caditor_expression::format_decimals;
use caditor_geometry::{Point2, Vector2};
use caditor_lettering::{Alignment, Segment, Style, lettered};

use super::{
    APPLICATION, ExportError,
    figure::{Anchor, Figure, Layer, Shape, Text},
};

const POINTS_PER_MILLIMETRE: f64 = 72.0 / 25.4;
const DECIMALS: usize = 4;
const HAIRLINE: f64 = 0.1;
const VISIBLE_WIDTH: f64 = 0.5;
const HIDDEN_WIDTH: f64 = 0.25;
const FRAME_WIDTH: f64 = 0.35;
const HIDDEN_DASH: f64 = 3.0;
const HIDDEN_GAP: f64 = 1.5;
const CONSTRUCTION_DASH: f64 = 1.0;
const CONSTRUCTION_GAP: f64 = 0.5;
const POINT_RADIUS: f64 = 0.25;
const BEZIER_STEPS: usize = 8;
const HEADER: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";

struct Number(f64);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&format_decimals(self.0, DECIMALS))
    }
}

fn point(text: &mut String, operator: &str, at: Point2) -> std::fmt::Result {
    writeln!(text, "{} {} {operator}", Number(at.x), Number(at.y))
}

fn elliptic(
    text: &mut String,
    center: Point2,
    major: Vector2,
    minor: Vector2,
    start: f64,
    end: f64,
) -> std::fmt::Result {
    let at = |angle: f64| center + major * angle.cos() + minor * angle.sin();
    let along = |angle: f64| minor * angle.cos() - major * angle.sin();
    let pieces = ((end - start).abs() / FRAC_PI_2).ceil().clamp(1.0, 64.0) as usize;
    let step = (end - start) / pieces as f64;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    point(text, "m", at(start))?;
    for piece in 0..pieces {
        let from = start + step * piece as f64;
        let to = from + step;
        let first = at(from) + along(from) * handle;
        let second = at(to) - along(to) * handle;
        let last = at(to);
        writeln!(
            text,
            "{} {} {} {} {} {} c",
            Number(first.x),
            Number(first.y),
            Number(second.x),
            Number(second.y),
            Number(last.x),
            Number(last.y)
        )?;
    }
    Ok(())
}

fn polyline(text: &mut String, points: &[Point2]) -> std::fmt::Result {
    for (index, at) in points.iter().enumerate() {
        point(text, if index == 0 { "m" } else { "l" }, *at)?;
    }
    Ok(())
}

fn letters(text: &mut String, label: &Text) -> Result<(), ExportError> {
    let laid = lettered(&label.content, Style::default(), Alignment::Left)
        .map_err(|_| ExportError::Encoding)?;
    let along = Vector2::from_angle(label.angle);
    let up = along.perp();
    let start = match label.anchor {
        Anchor::Start => label.at,
        Anchor::Middle => {
            label.at - along * (laid.width * label.height / 2.0) - up * (label.height / 2.0)
        }
    };
    let place =
        |local: Point2| start + along * (local.x * label.height) + up * (local.y * label.height);
    let mut pen: Option<Point2> = None;
    let mut written = || -> std::fmt::Result {
        for segment in laid.segments() {
            let points = match segment {
                Segment::Line(from, to) => vec![*from, *to],
                Segment::Bezier(_) => segment.sampled(BEZIER_STEPS),
            };
            let Some((first, rest)) = points.split_first() else {
                continue;
            };
            let first = place(*first);
            if pen.is_none_or(|pen| pen.distance(first) > 1e-9) {
                point(text, "m", first)?;
            }
            for next in rest {
                point(text, "l", place(*next))?;
            }
            pen = rest.last().map(|last| place(*last)).or(Some(first));
        }
        if pen.is_some() {
            writeln!(text, "f")?;
        }
        Ok(())
    };
    written().map_err(|_| ExportError::Encoding)
}

fn stroked(text: &mut String, shape: &Shape) -> Result<(), ExportError> {
    let drawn = |text: &mut String| -> std::fmt::Result {
        match shape {
            Shape::Line(from, to) => {
                point(text, "m", *from)?;
                point(text, "l", *to)?;
            }
            Shape::Circle { center, radius } => {
                elliptic(
                    text,
                    *center,
                    Vector2::new(*radius, 0.0),
                    Vector2::new(0.0, *radius),
                    0.0,
                    TAU,
                )?;
            }
            Shape::Arc {
                center,
                radius,
                start,
                end,
            } => elliptic(
                text,
                *center,
                Vector2::new(*radius, 0.0),
                Vector2::new(0.0, *radius),
                *start,
                *end,
            )?,
            Shape::Ellipse(ellipse) => elliptic(
                text,
                ellipse.center,
                ellipse.major,
                ellipse.major.perp() * ellipse.ratio,
                ellipse.start,
                ellipse.end,
            )?,
            Shape::Spline(spline) => polyline(text, &spline.polyline)?,
            Shape::Polyline(points) => polyline(text, points)?,
            Shape::Point(_) | Shape::Text(_) | Shape::Dimension(_) => return Ok(()),
        }
        writeln!(text, "S")
    };
    drawn(text).map_err(|_| ExportError::Encoding)?;
    match shape {
        Shape::Point(at) => {
            let filled = |text: &mut String| -> std::fmt::Result {
                elliptic(
                    text,
                    *at,
                    Vector2::new(POINT_RADIUS, 0.0),
                    Vector2::new(0.0, POINT_RADIUS),
                    0.0,
                    TAU,
                )?;
                writeln!(text, "f")
            };
            filled(text).map_err(|_| ExportError::Encoding)
        }
        Shape::Text(label) => letters(text, label),
        Shape::Dimension(dimension) => {
            for mark in &dimension.marks {
                stroked(text, mark)?;
            }
            letters(text, &dimension.text)
        }
        _ => Ok(()),
    }
}

fn layer_style(layer: Layer) -> (f64, Option<(f64, f64)>) {
    match layer {
        Layer::Visible => (VISIBLE_WIDTH, None),
        Layer::Hidden => (HIDDEN_WIDTH, Some((HIDDEN_DASH, HIDDEN_GAP))),
        Layer::Frame => (FRAME_WIDTH, None),
        Layer::Construction => (HAIRLINE, Some((CONSTRUCTION_DASH, CONSTRUCTION_GAP))),
        Layer::Sketch | Layer::Outline | Layer::Holes | Layer::Dimensions | Layer::Labels => {
            (HAIRLINE, None)
        }
    }
}

fn content(figure: &Figure) -> Result<String, ExportError> {
    let mut text = String::new();
    let opening = |text: &mut String| -> std::fmt::Result {
        writeln!(
            text,
            "{0} 0 0 {0} 0 0 cm 0 0 0 RG 0 0 0 rg 1 J 1 j",
            Number(POINTS_PER_MILLIMETRE)
        )
    };
    opening(&mut text).map_err(|_| ExportError::Encoding)?;
    for layer in figure.layers() {
        let (width, dash) = layer_style(layer);
        let styled = |text: &mut String| -> std::fmt::Result {
            match dash {
                Some((on, off)) => writeln!(
                    text,
                    "{} w [{} {}] 0 d",
                    Number(width),
                    Number(on),
                    Number(off)
                ),
                None => writeln!(text, "{} w [] 0 d", Number(width)),
            }
        };
        styled(&mut text).map_err(|_| ExportError::Encoding)?;
        for (_, shape) in figure.shapes.iter().filter(|(on, _)| *on == layer) {
            stroked(&mut text, shape)?;
        }
    }
    Ok(text)
}

fn literal(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii)
        .map(|character| match character {
            '(' | ')' | '\\' => format!("\\{character}"),
            character if character.is_ascii_control() => " ".to_owned(),
            character => character.to_string(),
        })
        .collect()
}

pub(super) fn encode(figure: &Figure, width: f64, height: f64) -> Result<Vec<u8>, ExportError> {
    let stream = content(figure)?;
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents 4 0 R /Resources << >> >>",
            Number(width * POINTS_PER_MILLIMETRE),
            Number(height * POINTS_PER_MILLIMETRE)
        ),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        format!("<< /Producer ({}) >>", literal(APPLICATION)),
    ];
    let mut bytes = HEADER.to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let table = bytes.len();
    let mut trailer = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        trailer.push_str(&format!("{offset:010} 00000 n \n"));
    }
    trailer.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{table}\n%%EOF\n",
        objects.len() + 1,
        objects.len()
    ));
    bytes.extend_from_slice(trailer.as_bytes());
    Ok(bytes)
}
