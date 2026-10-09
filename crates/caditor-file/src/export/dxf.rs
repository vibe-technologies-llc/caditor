use std::{f64::consts::TAU, fmt::Write};

use caditor_geometry::Point2;

use super::{
    annotation::{DEGREE_SIGN, DIAMETER_SIGN},
    figure::{Anchor, Dimension, Ellipse, Figure, Layer, Measure, Shape, Spline, Text},
};

const VERSION: &str = "AC1015";
const MILLIMETRES: u32 = 4;
const METRIC: u32 = 1;
const PLANAR_SPLINE: u32 = 8;
const RATIONAL_SPLINE: u32 = 4;
const OPEN_POLYLINE: u32 = 0;
const DASHED: &str = "DASHED";
const CONTINUOUS: &str = "CONTINUOUS";
const DASH: f64 = 1.0;
const GAP: f64 = 0.5;
const ALIGNED_PATTERN: u32 = 65;
const GREY: u32 = 8;
const ANONYMOUS_BLOCK: u32 = 1;
const ROTATED_DIMENSION: u32 = 0;
const DIAMETER_DIMENSION: u32 = 3;
const RADIUS_DIMENSION: u32 = 4;
const ANGULAR_DIMENSION: u32 = 5;
const OWN_BLOCK: u32 = 32;
const PLACED_TEXT: u32 = 128;
const CENTRED: u32 = 1;
const MIDDLE: u32 = 2;
const DIMENSION_STYLE: &str = "STANDARD";
const LARGEST_ESCAPED: u32 = 0xFFFF;

pub(super) fn encode(figure: &Figure) -> String {
    let mut writer = Writer::default();
    writer.header();
    if figure.layers().contains(&Layer::Construction) {
        writer.dashed_tables();
    }
    let dimensions: Vec<&Dimension> = figure
        .shapes
        .iter()
        .filter_map(|(_, shape)| match shape {
            Shape::Dimension(dimension) => Some(&**dimension),
            _ => None,
        })
        .collect();
    if !dimensions.is_empty() {
        writer.blocks(&dimensions);
    }
    writer.entities();
    for (layer, shape) in &figure.shapes {
        writer.shape(*layer, shape);
    }
    writer.finish();
    writer.text
}

#[derive(Default)]
struct Writer {
    text: String,
    dimensions: usize,
}

impl Writer {
    fn pair(&mut self, code: u32, value: impl std::fmt::Display) {
        let _ = writeln!(self.text, "{code:>3}\n{value}");
    }

    fn real(&mut self, code: u32, value: f64) {
        self.pair(code, Real(value));
    }

    fn location(&mut self, first_code: u32, point: Point2) {
        self.real(first_code, point.x);
        self.real(first_code + 10, point.y);
        self.real(first_code + 20, 0.0);
    }

    fn header(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "HEADER");
        self.pair(9, "$ACADVER");
        self.pair(1, VERSION);
        self.pair(9, "$INSUNITS");
        self.pair(70, MILLIMETRES);
        self.pair(9, "$MEASUREMENT");
        self.pair(70, METRIC);
        self.pair(0, "ENDSEC");
    }

    fn dashed_tables(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "TABLES");
        self.pair(0, "TABLE");
        self.pair(2, "LTYPE");
        self.pair(70, 2);
        self.pair(0, "LTYPE");
        self.pair(2, CONTINUOUS);
        self.pair(70, 0);
        self.pair(3, "Solid line");
        self.pair(72, ALIGNED_PATTERN);
        self.pair(73, 0);
        self.real(40, 0.0);
        self.pair(0, "LTYPE");
        self.pair(2, DASHED);
        self.pair(70, 0);
        self.pair(3, "Dashed __ __ __");
        self.pair(72, ALIGNED_PATTERN);
        self.pair(73, 2);
        self.real(40, DASH + GAP);
        self.real(49, DASH);
        self.real(49, -GAP);
        self.pair(0, "ENDTAB");
        self.pair(0, "TABLE");
        self.pair(2, "LAYER");
        self.pair(70, 1);
        self.pair(0, "LAYER");
        self.pair(2, Layer::Construction.name());
        self.pair(70, 0);
        self.pair(62, GREY);
        self.pair(6, DASHED);
        self.pair(0, "ENDTAB");
        self.pair(0, "ENDSEC");
    }

    fn entities(&mut self) {
        self.pair(0, "SECTION");
        self.pair(2, "ENTITIES");
    }

    fn entity(&mut self, kind: &str, layer: Layer) {
        self.pair(0, kind);
        self.pair(8, layer.name());
        if layer == Layer::Construction {
            self.pair(6, DASHED);
        }
    }

    fn shape(&mut self, layer: Layer, shape: &Shape) {
        match shape {
            Shape::Point(position) => {
                self.entity("POINT", layer);
                self.location(10, *position);
            }
            Shape::Line(start, end) => {
                self.entity("LINE", layer);
                self.location(10, *start);
                self.location(11, *end);
            }
            Shape::Circle { center, radius } => {
                self.entity("CIRCLE", layer);
                self.location(10, *center);
                self.real(40, *radius);
            }
            Shape::Arc {
                center,
                radius,
                start,
                end,
            } => {
                self.entity("ARC", layer);
                self.location(10, *center);
                self.real(40, *radius);
                self.real(50, start.to_degrees().rem_euclid(360.0));
                self.real(51, end.to_degrees().rem_euclid(360.0));
            }
            Shape::Ellipse(ellipse) => self.ellipse(layer, ellipse),
            Shape::Spline(spline) => self.spline(layer, spline),
            Shape::Polyline(points) => self.polyline(layer, points),
            Shape::Text(text) => self.label(layer, text),
            Shape::Dimension(dimension) => self.dimension(layer, dimension),
        }
    }

    fn label(&mut self, layer: Layer, text: &Text) {
        self.entity("TEXT", layer);
        self.location(10, text.at);
        self.real(40, text.height);
        self.pair(1, escaped(&text.content));
        self.real(50, text.upright_angle().to_degrees());
        if text.anchor == Anchor::Middle {
            self.pair(72, CENTRED);
            self.location(11, text.at);
            self.pair(73, MIDDLE);
        }
    }

    fn blocks(&mut self, dimensions: &[&Dimension]) {
        self.pair(0, "SECTION");
        self.pair(2, "BLOCKS");
        for (index, dimension) in dimensions.iter().enumerate() {
            let name = block_name(index + 1);
            self.pair(0, "BLOCK");
            self.pair(8, Layer::Sketch.name());
            self.pair(2, &name);
            self.pair(70, ANONYMOUS_BLOCK);
            self.location(10, Point2::ZERO);
            self.pair(3, &name);
            for mark in &dimension.marks {
                self.shape(Layer::Sketch, mark);
            }
            self.label(Layer::Sketch, &dimension.text);
            self.pair(0, "ENDBLK");
            self.pair(8, Layer::Sketch.name());
        }
        self.pair(0, "ENDSEC");
    }

    fn dimension(&mut self, layer: Layer, dimension: &Dimension) {
        self.dimensions += 1;
        let kind = match dimension.measure {
            Measure::Linear { .. } => ROTATED_DIMENSION,
            Measure::Angular { .. } => ANGULAR_DIMENSION,
            Measure::Radius { .. } => RADIUS_DIMENSION,
            Measure::Diameter { .. } => DIAMETER_DIMENSION,
        };
        let definition = match dimension.measure {
            Measure::Linear { line, .. } => line,
            Measure::Angular { arc, .. } => arc,
            Measure::Radius { center, .. } => center,
            Measure::Diameter { far, .. } => far,
        };
        self.pair(0, "DIMENSION");
        self.pair(100, "AcDbEntity");
        self.pair(8, layer.name());
        self.pair(100, "AcDbDimension");
        self.pair(2, block_name(self.dimensions));
        self.location(10, definition);
        self.location(11, dimension.text.at);
        self.pair(70, kind | OWN_BLOCK | PLACED_TEXT);
        self.pair(1, escaped(&dimension.text.content));
        self.pair(3, DIMENSION_STYLE);
        match dimension.measure {
            Measure::Linear {
                first,
                second,
                angle,
                ..
            } => {
                self.pair(100, "AcDbAlignedDimension");
                self.location(13, first);
                self.location(14, second);
                self.real(50, angle.to_degrees());
                self.pair(100, "AcDbRotatedDimension");
            }
            Measure::Angular {
                vertex,
                first,
                second,
                ..
            } => {
                self.pair(100, "AcDb3PointAngularDimension");
                self.location(13, first);
                self.location(14, second);
                self.location(15, vertex);
            }
            Measure::Radius { on_curve, .. } => {
                self.pair(100, "AcDbRadialDimension");
                self.location(15, on_curve);
                self.real(40, 0.0);
            }
            Measure::Diameter { near, .. } => {
                self.pair(100, "AcDbDiametricDimension");
                self.location(15, near);
                self.real(40, 0.0);
            }
        }
    }

    fn ellipse(&mut self, layer: Layer, ellipse: &Ellipse) {
        let (start, end) = if ellipse.is_full() {
            (0.0, TAU)
        } else {
            (ellipse.start.rem_euclid(TAU), ellipse.end.rem_euclid(TAU))
        };
        self.entity("ELLIPSE", layer);
        self.location(10, ellipse.center);
        self.location(11, ellipse.major);
        self.real(40, ellipse.ratio);
        self.real(41, start);
        self.real(42, end);
    }

    fn spline(&mut self, layer: Layer, spline: &Spline) {
        let flags = match spline.weights {
            Some(_) => PLANAR_SPLINE | RATIONAL_SPLINE,
            None => PLANAR_SPLINE,
        };
        self.entity("SPLINE", layer);
        self.pair(70, flags);
        self.pair(71, spline.degree);
        self.pair(72, spline.knots.len());
        self.pair(73, spline.control_points.len());
        self.pair(74, 0);
        for knot in &spline.knots {
            self.real(40, *knot);
        }
        for weight in spline.weights.iter().flatten() {
            self.real(41, *weight);
        }
        for point in &spline.control_points {
            self.location(10, *point);
        }
    }

    fn polyline(&mut self, layer: Layer, points: &[Point2]) {
        self.entity("LWPOLYLINE", layer);
        self.pair(90, points.len());
        self.pair(70, OPEN_POLYLINE);
        for point in points {
            self.real(10, point.x);
            self.real(20, point.y);
        }
    }

    fn finish(&mut self) {
        self.pair(0, "ENDSEC");
        self.pair(0, "EOF");
    }
}

fn block_name(number: usize) -> String {
    format!("*D{number}")
}

fn escaped(content: &str) -> String {
    let mut escaped = String::with_capacity(content.len());
    for character in content.chars() {
        match character {
            '%' => escaped.push_str("%%%"),
            DIAMETER_SIGN => escaped.push_str("%%c"),
            DEGREE_SIGN => escaped.push_str("%%d"),
            character if character.is_control() => escaped.push(' '),
            character if character.is_ascii() => escaped.push(character),
            character if u32::from(character) <= LARGEST_ESCAPED => {
                let _ = write!(escaped, "\\U+{:04X}", u32::from(character));
            }
            _ => escaped.push('?'),
        }
    }
    escaped
}

struct Real(f64);

impl std::fmt::Display for Real {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 + 0.0 {
            0.0 => formatter.write_str("0"),
            value => write!(formatter, "{value}"),
        }
    }
}
