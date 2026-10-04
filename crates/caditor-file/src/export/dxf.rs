use std::{collections::BTreeSet, f64::consts::TAU, fmt::Write};

use caditor_geometry::Point2;
use caditor_sketch::{Entity, Sketch};

use super::{ExportError, SketchExported};

const VERSION: &str = "AC1015";
const MILLIMETRES: u32 = 4;
const METRIC: u32 = 1;
const PLANAR_SPLINE: u32 = 8;

pub(super) fn encode(sketch: &Sketch) -> Result<(String, SketchExported), ExportError> {
    let mut writer = Writer::default();
    let mut exported = SketchExported {
        curves: 0,
        points: 0,
        construction_left_out: 0,
    };
    writer.header();
    let anchors: BTreeSet<_> = sketch
        .entities()
        .flat_map(|(_, entity)| entity.points())
        .collect();
    for (id, entity) in sketch.entities() {
        if sketch.is_construction(id) {
            exported.construction_left_out += 1;
            continue;
        }
        let written = match entity {
            Entity::Point(_) if anchors.contains(&id) => false,
            Entity::Point(position) => {
                writer.point(*position);
                exported.points += 1;
                true
            }
            Entity::Line { .. } => sketch
                .line_endpoints(id)
                .map(|(start, end)| writer.line(start, end))
                .is_some(),
            Entity::Circle { .. } => sketch
                .circle(id)
                .map(|(center, radius)| writer.circle(center, radius))
                .is_some(),
            Entity::Arc { .. } => sketch
                .arc(id)
                .map(|arc| {
                    if arc.sweep >= TAU {
                        writer.circle(arc.center, arc.radius);
                    } else {
                        writer.arc(arc.center, arc.radius, arc.start_angle, arc.end_angle());
                    }
                })
                .is_some(),
            Entity::Spline { .. } => sketch
                .spline(id)
                .map(|spline| {
                    writer.spline(spline.degree(), spline.knots(), spline.control_points());
                })
                .is_some(),
        };
        if written && !matches!(entity, Entity::Point(_)) {
            exported.curves += 1;
        }
    }
    if exported.curves + exported.points == 0 {
        return Err(ExportError::NoCurves);
    }
    writer.finish();
    Ok((writer.text, exported))
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

    fn entity(&mut self, kind: &str) {
        self.pair(0, kind);
        self.pair(8, "0");
    }

    fn point(&mut self, position: Point2) {
        self.entity("POINT");
        self.location(10, position);
    }

    fn line(&mut self, start: Point2, end: Point2) {
        self.entity("LINE");
        self.location(10, start);
        self.location(11, end);
    }

    fn circle(&mut self, center: Point2, radius: f64) {
        self.entity("CIRCLE");
        self.location(10, center);
        self.real(40, radius);
    }

    fn arc(&mut self, center: Point2, radius: f64, start: f64, end: f64) {
        self.entity("ARC");
        self.location(10, center);
        self.real(40, radius);
        self.real(50, start.to_degrees().rem_euclid(360.0));
        self.real(51, end.to_degrees().rem_euclid(360.0));
    }

    fn spline(&mut self, degree: usize, knots: &[f64], control_points: &[Point2]) {
        self.entity("SPLINE");
        self.pair(70, PLANAR_SPLINE);
        self.pair(71, degree);
        self.pair(72, knots.len());
        self.pair(73, control_points.len());
        self.pair(74, 0);
        for knot in knots {
            self.real(40, *knot);
        }
        for point in control_points {
            self.location(10, *point);
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
