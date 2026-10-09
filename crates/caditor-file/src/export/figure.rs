use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

use caditor_geometry::{Aabb2, Point2, Vector2};
use caditor_sketch::{Entity, Sketch};

use super::{Construction, SketchExported};

pub(super) const SEGMENT_ANGLE: f64 = 5.0 * PI / 180.0;
const TEXT_WIDTH_PER_HEIGHT: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Layer {
    Sketch,
    Construction,
    Outline,
    Holes,
    Dimensions,
    Labels,
}

impl Layer {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Sketch => "0",
            Self::Construction => "Construction",
            Self::Outline => "Outline",
            Self::Holes => "Holes",
            Self::Dimensions => "Dimensions",
            Self::Labels => "Labels",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Anchor {
    Start,
    Middle,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Text {
    pub(super) at: Point2,
    pub(super) height: f64,
    pub(super) angle: f64,
    pub(super) content: String,
    pub(super) anchor: Anchor,
}

impl Text {
    pub(super) fn width(&self) -> f64 {
        self.height * TEXT_WIDTH_PER_HEIGHT * self.content.chars().count() as f64
    }

    pub(super) fn upright_angle(&self) -> f64 {
        match self.anchor {
            Anchor::Start => self.angle,
            Anchor::Middle => FRAC_PI_2 - (FRAC_PI_2 - self.angle).rem_euclid(PI),
        }
    }

    fn outline(&self) -> Vec<Point2> {
        match self.corners().as_slice() {
            &[low_left, low_right, high_left, high_right] => {
                vec![low_left, low_right, high_right, high_left, low_left]
            }
            corners => corners.to_vec(),
        }
    }

    fn corners(&self) -> Vec<Point2> {
        let along = Vector2::from_angle(self.angle);
        let up = along.perp();
        let width = self.width();
        let (left, bottom) = match self.anchor {
            Anchor::Start => (0.0, 0.0),
            Anchor::Middle => (-width / 2.0, -self.height / 2.0),
        };
        [
            (0.0, 0.0),
            (width, 0.0),
            (0.0, self.height),
            (width, self.height),
        ]
        .into_iter()
        .map(|(x, y)| self.at + along * (left + x) + up * (bottom + y))
        .collect()
    }

    fn moved(self, motion: Motion) -> Self {
        Self {
            at: motion.point(self.at),
            angle: motion.angle(self.angle),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Measure {
    Linear {
        first: Point2,
        second: Point2,
        line: Point2,
        angle: f64,
    },
    Angular {
        vertex: Point2,
        first: Point2,
        second: Point2,
        arc: Point2,
    },
    Radius {
        center: Point2,
        on_curve: Point2,
    },
    Diameter {
        near: Point2,
        far: Point2,
    },
}

impl Measure {
    fn moved(self, motion: Motion) -> Self {
        match self {
            Self::Linear {
                first,
                second,
                line,
                angle,
            } => Self::Linear {
                first: motion.point(first),
                second: motion.point(second),
                line: motion.point(line),
                angle: motion.angle(angle),
            },
            Self::Angular {
                vertex,
                first,
                second,
                arc,
            } => Self::Angular {
                vertex: motion.point(vertex),
                first: motion.point(first),
                second: motion.point(second),
                arc: motion.point(arc),
            },
            Self::Radius { center, on_curve } => Self::Radius {
                center: motion.point(center),
                on_curve: motion.point(on_curve),
            },
            Self::Diameter { near, far } => Self::Diameter {
                near: motion.point(near),
                far: motion.point(far),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Dimension {
    pub(super) measure: Measure,
    pub(super) marks: Vec<Shape>,
    pub(super) text: Text,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Motion {
    pub(super) turn: f64,
    pub(super) offset: Vector2,
}

impl Motion {
    pub(super) fn shift(offset: Vector2) -> Self {
        Self { turn: 0.0, offset }
    }

    pub(super) fn turn(turn: f64) -> Self {
        Self {
            turn,
            offset: Vector2::ZERO,
        }
    }

    pub(super) fn vector(self, vector: Vector2) -> Vector2 {
        let quarters = self.turn / FRAC_PI_2;
        if quarters == quarters.round() {
            match (quarters as i64).rem_euclid(4) {
                0 => vector,
                1 => vector.perp(),
                2 => -vector,
                _ => -vector.perp(),
            }
        } else {
            Vector2::from_angle(self.turn).rotate(vector)
        }
    }

    pub(super) fn point(self, point: Point2) -> Point2 {
        self.vector(point) + self.offset
    }

    fn angle(self, angle: f64) -> f64 {
        angle + self.turn
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
    Text(Text),
    Dimension(Box<Dimension>),
}

impl Shape {
    pub(super) fn arc_point(center: Point2, radius: f64, angle: f64) -> Point2 {
        let (sin, cos) = angle.sin_cos();
        center + Vector2::new(cos, sin) * radius
    }

    pub(super) fn moved(self, motion: Motion) -> Self {
        let points = |points: Vec<Point2>| -> Vec<Point2> {
            points
                .into_iter()
                .map(|point| motion.point(point))
                .collect()
        };
        match self {
            Self::Point(point) => Self::Point(motion.point(point)),
            Self::Line(start, end) => Self::Line(motion.point(start), motion.point(end)),
            Self::Circle { center, radius } => Self::Circle {
                center: motion.point(center),
                radius,
            },
            Self::Arc {
                center,
                radius,
                start,
                end,
            } => Self::Arc {
                center: motion.point(center),
                radius,
                start: motion.angle(start),
                end: motion.angle(end),
            },
            Self::Ellipse(ellipse) => Self::Ellipse(Ellipse {
                center: motion.point(ellipse.center),
                major: motion.vector(ellipse.major),
                ..ellipse
            }),
            Self::Spline(spline) => Self::Spline(Spline {
                control_points: points(spline.control_points),
                polyline: points(spline.polyline),
                ..spline
            }),
            Self::Polyline(line) => Self::Polyline(points(line)),
            Self::Text(text) => Self::Text(text.moved(motion)),
            Self::Dimension(dimension) => {
                let Dimension {
                    measure,
                    marks,
                    text,
                } = *dimension;
                Self::Dimension(Box::new(Dimension {
                    measure: measure.moved(motion),
                    marks: marks.into_iter().map(|mark| mark.moved(motion)).collect(),
                    text: text.moved(motion),
                }))
            }
        }
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
            Self::Text(text) => text.corners(),
            Self::Dimension(dimension) => dimension
                .marks
                .iter()
                .flat_map(Self::outline_points)
                .chain(dimension.text.corners())
                .collect(),
        }
    }
}

impl Shape {
    pub(super) fn traced(&self) -> Vec<Vec<Point2>> {
        match self {
            Self::Circle { center, radius } => vec![
                sweep_angles(0.0, TAU)
                    .map(|angle| Self::arc_point(*center, *radius, angle))
                    .collect(),
            ],
            Self::Text(text) => vec![text.outline()],
            Self::Dimension(dimension) => dimension
                .marks
                .iter()
                .flat_map(Self::traced)
                .chain(std::iter::once(dimension.text.outline()))
                .collect(),
            shape => vec![shape.outline_points()],
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

    pub(super) fn bounds(&self) -> Option<Aabb2> {
        Aabb2::from_points(
            self.shapes
                .iter()
                .flat_map(|(_, shape)| shape.outline_points()),
        )
    }

    pub(super) fn append_moved(&mut self, other: Self, motion: Motion) {
        self.shapes.extend(
            other
                .shapes
                .into_iter()
                .map(|(layer, shape)| (layer, shape.moved(motion))),
        );
    }

    pub(super) fn layers(&self) -> BTreeSet<Layer> {
        self.shapes.iter().map(|(layer, _)| *layer).collect()
    }

    pub(super) fn of_sketch(sketch: &Sketch, construction: Construction) -> (Self, SketchExported) {
        let mut figure = Self::default();
        let mut exported = SketchExported {
            sketches: 1,
            ..SketchExported::default()
        };
        let anchors: BTreeSet<_> = sketch
            .entities()
            .flat_map(|(_, entity)| entity.points())
            .collect();
        for (id, entity) in sketch.entities() {
            let layer = match (sketch.is_construction(id), construction) {
                (false, _) => Layer::Sketch,
                (true, Construction::OnLayer) => Layer::Construction,
                (true, Construction::LeftOut) => {
                    exported.construction_left_out += 1;
                    continue;
                }
            };
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
            match (layer, &shape) {
                (Layer::Construction, _) => exported.construction += 1,
                (_, Shape::Point(_)) => exported.points += 1,
                _ => exported.curves += 1,
            }
            figure.push(layer, shape);
        }
        (figure, exported)
    }
}
