use std::{f64::consts::TAU, fmt::Write};

use caditor_geometry::Point2;

use super::figure::{Ellipse, Figure, Layer, Shape, Spline};

const VERSION: &str = "AC1015";
const MILLIMETRES: u32 = 4;
const METRIC: u32 = 1;
const PLANAR_SPLINE: u32 = 8;
const RATIONAL_SPLINE: u32 = 4;
const OPEN_POLYLINE: u32 = 0;

pub(super) fn encode(figure: &Figure) -> String {
    let mut writer = Writer::default();
    writer.header();
    for (layer, shape) in &figure.shapes {
        writer.shape(*layer, shape);
    }
    writer.finish();
    writer.text
}

#[derive(Default)]
struct Writer {
    text: String,
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
        self.pair(0, "SECTION");
        self.pair(2, "ENTITIES");
    }

    fn entity(&mut self, kind: &str, layer: Layer) {
        self.pair(0, kind);
        self.pair(8, layer.name());
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

struct Real(f64);

impl std::fmt::Display for Real {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 + 0.0 {
            0.0 => formatter.write_str("0"),
            value => write!(formatter, "{value}"),
        }
    }
}
