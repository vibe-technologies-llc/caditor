use std::f64::consts::TAU;

use caditor_document::FeatureId;
use caditor_kernel::{
    Accuracy, EdgeForm, EdgeId, FaceForm, FaceId, Solid, edge_measure, face_form, hole_faces,
};

use crate::{measure, model::Model};

const EXACT_SLACK: f64 = 1e-6;
const MESH_SLACK: f64 = 1e-3;
const FULL_TURN_SLACK: f64 = 1e-9;

fn slack_of(accuracy: Accuracy) -> f64 {
    match accuracy {
        Accuracy::Exact => EXACT_SLACK,
        Accuracy::Approximate => MESH_SLACK,
    }
}

fn near(first: f64, second: f64, slack: f64) -> bool {
    (first - second).abs() <= slack * first.abs().max(second.abs()).max(1.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Plane { area: f64, slack: f64 },
    Cylinder { radius: f64, hole: bool },
    Cone { half_angle: f64 },
    Sphere { radius: f64 },
    Torus { major: f64, minor: f64 },
    Line { length: f64, slack: f64 },
    Circle { radius: f64, sweep: f64 },
    Ellipse { major: f64, minor: f64 },
    Curve { length: f64, slack: f64 },
}

impl Shape {
    pub fn of_face(model: &Model, body: FeatureId, solid: &Solid, face: FaceId) -> Option<Self> {
        Some(match face_form(solid, face).ok()? {
            FaceForm::Plane { .. } => {
                let (area, accuracy) = measure::face_area_of(model, body, face)?;
                Self::Plane {
                    area,
                    slack: slack_of(accuracy),
                }
            }
            FaceForm::Cylinder { radius, .. } => Self::Cylinder {
                radius,
                hole: !hole_faces(solid, &[face]).is_empty(),
            },
            FaceForm::Cone { half_angle, .. } => Self::Cone { half_angle },
            FaceForm::Sphere { radius, .. } => Self::Sphere { radius },
            FaceForm::Torus {
                major_radius,
                minor_radius,
                ..
            } => Self::Torus {
                major: major_radius,
                minor: minor_radius,
            },
            FaceForm::Surface => return None,
        })
    }

    pub fn of_edge(solid: &Solid, edge: EdgeId) -> Option<Self> {
        let measured = edge_measure(solid, edge).ok()?;
        let slack = slack_of(measured.length_accuracy);
        Some(match measured.form {
            EdgeForm::Line { .. } => Self::Line {
                length: measured.length,
                slack,
            },
            EdgeForm::Circle { radius, sweep, .. } => Self::Circle { radius, sweep },
            EdgeForm::Ellipse {
                major_radius,
                minor_radius,
                ..
            } => Self::Ellipse {
                major: major_radius,
                minor: minor_radius,
            },
            EdgeForm::Curve => Self::Curve {
                length: measured.length,
                slack,
            },
        })
    }

    pub fn matches(&self, other: &Self) -> bool {
        match (*self, *other) {
            (
                Self::Plane { area, slack },
                Self::Plane {
                    area: other_area,
                    slack: other_slack,
                },
            ) => near(area, other_area, slack.max(other_slack)),
            (
                Self::Cylinder { radius, hole },
                Self::Cylinder {
                    radius: other_radius,
                    hole: other_hole,
                },
            ) => hole == other_hole && near(radius, other_radius, EXACT_SLACK),
            (
                Self::Cone { half_angle },
                Self::Cone {
                    half_angle: other_angle,
                },
            ) => near(half_angle, other_angle, EXACT_SLACK),
            (
                Self::Sphere { radius },
                Self::Sphere {
                    radius: other_radius,
                },
            ) => near(radius, other_radius, EXACT_SLACK),
            (
                Self::Torus { major, minor },
                Self::Torus {
                    major: other_major,
                    minor: other_minor,
                },
            ) => near(major, other_major, EXACT_SLACK) && near(minor, other_minor, EXACT_SLACK),
            (
                Self::Line { length, slack },
                Self::Line {
                    length: other_length,
                    slack: other_slack,
                },
            )
            | (
                Self::Curve { length, slack },
                Self::Curve {
                    length: other_length,
                    slack: other_slack,
                },
            ) => near(length, other_length, slack.max(other_slack)),
            (
                Self::Circle { radius, sweep },
                Self::Circle {
                    radius: other_radius,
                    sweep: other_sweep,
                },
            ) => near(radius, other_radius, EXACT_SLACK) && near(sweep, other_sweep, EXACT_SLACK),
            (
                Self::Ellipse { major, minor },
                Self::Ellipse {
                    major: other_major,
                    minor: other_minor,
                },
            ) => near(major, other_major, EXACT_SLACK) && near(minor, other_minor, EXACT_SLACK),
            _ => false,
        }
    }

    pub fn words(&self, model: &Model) -> String {
        let unit = model.length_unit();
        let length = |value| unit.measured_length(value);
        let angle = |radians: f64| model.units().angle.readout_text(radians.to_degrees());
        match *self {
            Self::Plane { area, .. } => format!("area {}", unit.measured_area(area)),
            Self::Cylinder { radius, hole: true } => format!("hole radius {}", length(radius)),
            Self::Cylinder {
                radius,
                hole: false,
            }
            | Self::Sphere { radius } => format!("radius {}", length(radius)),
            Self::Cone { half_angle } => format!("half angle {}", angle(half_angle)),
            Self::Torus { major, minor } => format!(
                "ring radius {} and tube radius {}",
                length(major),
                length(minor)
            ),
            Self::Line { length: value, .. } | Self::Curve { length: value, .. } => {
                format!("length {}", length(value))
            }
            Self::Circle { radius, sweep } if sweep >= TAU - FULL_TURN_SLACK => {
                format!("radius {}", length(radius))
            }
            Self::Circle { radius, sweep } => {
                format!("radius {} and sweep {}", length(radius), angle(sweep))
            }
            Self::Ellipse { major, minor } => {
                format!("radii {} × {}", length(major), length(minor))
            }
        }
    }
}

pub fn distinct(shapes: impl IntoIterator<Item = Shape>) -> Vec<Shape> {
    let mut distinct: Vec<Shape> = Vec::new();
    for shape in shapes {
        if !distinct.iter().any(|known| known.matches(&shape)) {
            distinct.push(shape);
        }
    }
    distinct
}

pub fn any_matches(shapes: &[Shape], shape: &Shape) -> bool {
    shapes.iter().any(|known| known.matches(shape))
}
