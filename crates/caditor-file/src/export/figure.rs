use std::{
    collections::BTreeSet,
    f64::consts::{PI, TAU},
};

use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Entity, Sketch};

use super::SketchExported;

pub(super) const SEGMENT_ANGLE: f64 = 5.0 * PI / 180.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Layer {
    Sketch,
    Outline,
    Holes,
}

impl Layer {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Sketch => "0",
            Self::Outline => "Outline",
            Self::Holes => "Holes",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Spline {
    pub(super) degree: usize,
    pub(super) knots: Vec<f64>,
    pub(super) control_points: Vec<Point2>,
    pub(super) weights: Option<Vec<f64>>,
    pub(super) polyline: Vec<Point2>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Ellipse {
    pub(super) center: Point2,
    pub(super) major: Vector2,
    pub(super) ratio: f64,
    pub(super) start: f64,
    pub(super) end: f64,
}

impl Ellipse {
    pub(super) fn is_full(&self) -> bool {
        self.end - self.start >= TAU
    }

    pub(super) fn point_at(&self, parameter: f64) -> Point2 {
        let minor = self.major.perp() * self.ratio;
        let (sin, cos) = parameter.sin_cos();
        self.center + self.major * cos + minor * sin
    }

    pub(super) fn rotation(&self) -> f64 {
        self.major.y.atan2(self.major.x)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Shape {
    Point(Point2),
    Line(Point2, Point2),
    Circle {
        center: Point2,
        radius: f64,
    },
    Arc {
        center: Point2,
        radius: f64,
        start: f64,
        end: f64,
    },
    Ellipse(Ellipse),
    Spline(Spline),
    Polyline(Vec<Point2>),
}

impl Shape {
    pub(super) fn arc_point(center: Point2, radius: f64, angle: f64) -> Point2 {
        let (sin, cos) = angle.sin_cos();
        center + Vector2::new(cos, sin) * radius
    }

    pub(super) fn outline_points(&self) -> Vec<Point2> {
        match self {
            Self::Point(point) => vec![*point],
            Self::Line(start, end) => vec![*start, *end],
            Self::Circle { center, radius } => vec![
                *center - Vector2::splat(*radius),
                *center + Vector2::splat(*radius),
            ],
            Self::Arc {
                center,
                radius,
                start,
                end,
            } => sweep_angles(*start, *end)
                .map(|angle| Self::arc_point(*center, *radius, angle))
                .collect(),
            Self::Ellipse(ellipse) => sweep_angles(ellipse.start, ellipse.end)
                .map(|angle| ellipse.point_at(angle))
                .collect(),
            Self::Spline(spline) => spline.polyline.clone(),
            Self::Polyline(points) => points.clone(),
        }
    }
}

fn sweep_angles(start: f64, end: f64) -> impl Iterator<Item = f64> {
    let steps = ((end - start) / SEGMENT_ANGLE).ceil().clamp(1.0, 1024.0) as usize;
    (0..=steps).map(move |step| start + (end - start) * step as f64 / steps as f64)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Figure {
    pub(super) shapes: Vec<(Layer, Shape)>,
}

impl Figure {
    pub(super) fn push(&mut self, layer: Layer, shape: Shape) {
        self.shapes.push((layer, shape));
    }

    pub(super) fn layers(&self) -> BTreeSet<Layer> {
        self.shapes.iter().map(|(layer, _)| *layer).collect()
    }

    pub(super) fn of_sketch(sketch: &Sketch) -> (Self, SketchExported) {
        let mut figure = Self::default();
        let mut exported = SketchExported {
            curves: 0,
            points: 0,
            construction_left_out: 0,
        };
        let anchors: BTreeSet<_> = sketch
            .entities()
            .flat_map(|(_, entity)| entity.points())
            .collect();
        for (id, entity) in sketch.entities() {
            if sketch.is_construction(id) {
                exported.construction_left_out += 1;
                continue;
            }
            let shape = match entity {
                Entity::Point(_) if anchors.contains(&id) => None,
                Entity::Point(position) => Some(Shape::Point(*position)),
                Entity::Line { .. } => sketch
                    .line_endpoints(id)
                    .map(|(start, end)| Shape::Line(start, end)),
                Entity::Circle { .. } => sketch
                    .circle(id)
                    .map(|(center, radius)| Shape::Circle { center, radius }),
                Entity::Arc { .. } => sketch.arc(id).map(|arc| {
                    if arc.sweep >= TAU {
                        Shape::Circle {
                            center: arc.center,
                            radius: arc.radius,
                        }
                    } else {
                        Shape::Arc {
                            center: arc.center,
                            radius: arc.radius,
                            start: arc.start_angle,
                            end: arc.start_angle + arc.sweep,
                        }
                    }
                }),
                Entity::Spline { .. } => sketch.spline(id).map(|spline| {
                    Shape::Spline(Spline {
                        degree: spline.degree(),
                        knots: spline.knots().to_vec(),
                        control_points: spline.control_points().to_vec(),
                        weights: None,
                        polyline: spline.polyline(SEGMENT_ANGLE),
                    })
                }),
            };
            let Some(shape) = shape else {
                continue;
            };
            match shape {
                Shape::Point(_) => exported.points += 1,
                _ => exported.curves += 1,
            }
            figure.push(Layer::Sketch, shape);
        }
        (figure, exported)
    }
}
