use std::f64::consts::{FRAC_PI_2, TAU};

use caditor_geometry::Point2;
use caditor_sketch::ArcGeometry;

use crate::import::{Drawing, DrawingCurve};

pub const MIN_SCALE: f64 = 1e-6;
pub const MAX_SCALE: f64 = 1e6;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrawingUnit {
    #[default]
    AsInFile,
    Micrometres,
    Millimetres,
    Centimetres,
    Metres,
}

impl DrawingUnit {
    pub const ALL: [Self; 5] = [
        Self::AsInFile,
        Self::Micrometres,
        Self::Millimetres,
        Self::Centimetres,
        Self::Metres,
    ];

    pub fn millimetres(self) -> Option<f64> {
        match self {
            Self::AsInFile => None,
            Self::Micrometres => Some(1e-3),
            Self::Millimetres => Some(1.0),
            Self::Centimetres => Some(10.0),
            Self::Metres => Some(1_000.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::AsInFile => "as the file says",
            Self::Micrometres => "micrometres",
            Self::Millimetres => "millimetres",
            Self::Centimetres => "centimetres",
            Self::Metres => "metres",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawingOptions {
    pub unit: DrawingUnit,
    pub scale: f64,
    pub recentre: bool,
}

impl Default for DrawingOptions {
    fn default() -> Self {
        Self {
            unit: DrawingUnit::AsInFile,
            scale: 1.0,
            recentre: false,
        }
    }
}

impl DrawingOptions {
    pub fn is_valid(&self) -> bool {
        self.scale.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&self.scale)
    }
}

impl Drawing {
    pub fn factor(&self, options: &DrawingOptions) -> f64 {
        let unit = options
            .unit
            .millimetres()
            .map_or(1.0, |target| target / self.unit_scale);
        unit * options.scale
    }

    pub fn bounds(&self) -> Option<(Point2, Point2)> {
        self.curves
            .iter()
            .flat_map(curve_extremes)
            .fold(None, |bounds, point| {
                Some(match bounds {
                    Some((low, high)) => (low.min(point), high.max(point)),
                    None => (point, point),
                })
            })
    }

    pub fn arranged(&self, options: &DrawingOptions) -> Self {
        let factor = self.factor(options);
        let mut arranged = self.clone();
        let mut notes = Vec::new();
        if let Some(unit) = options.unit.millimetres().map(|_| options.unit.name()) {
            notes.push(format!(
                "You chose to read the drawing's numbers as {unit}, so its lengths were \
                 converted from that unit to millimetres."
            ));
        }
        if options.scale != 1.0 {
            notes.push(format!("You scaled the drawing by {}.", options.scale));
        }
        if factor != 1.0 {
            for curve in &mut arranged.curves {
                curve.map_points(|point| point * factor);
            }
        }
        if options.recentre
            && let Some((low, high)) = arranged.bounds()
        {
            let centre = (low + high) / 2.0;
            for curve in &mut arranged.curves {
                curve.map_points(|point| point - centre);
            }
            notes.push(
                "The drawing was moved so that the centre of its outline sits on the origin."
                    .to_owned(),
            );
        }
        arranged.unit_scale = self.unit_scale * factor;
        arranged.notes.extend(notes);
        arranged
    }
}

impl DrawingCurve {
    fn map_points(&mut self, change: impl Fn(Point2) -> Point2) {
        match self {
            Self::Point(point) => *point = change(*point),
            Self::Line { start, end } => {
                *start = change(*start);
                *end = change(*end);
            }
            Self::Circle { center, radius } => {
                let moved = change(*center + Point2::X * *radius) - change(*center);
                *center = change(*center);
                *radius = moved.length();
            }
            Self::Arc { center, start, end } => {
                *center = change(*center);
                *start = change(*start);
                *end = change(*end);
            }
            Self::Spline { control_points } => {
                for point in control_points {
                    *point = change(*point);
                }
            }
        }
    }
}

fn curve_extremes(curve: &DrawingCurve) -> Vec<Point2> {
    match curve {
        DrawingCurve::Arc { center, start, end } => {
            let arc = ArcGeometry::from_points(*center, *start, *end);
            let mut points = vec![*start, *end];
            points.extend(
                (0..4)
                    .map(|quarter| FRAC_PI_2 * f64::from(quarter))
                    .filter(|angle| (angle - arc.start_angle).rem_euclid(TAU) <= arc.sweep)
                    .map(|angle| arc.point_at(angle)),
            );
            points
        }
        _ => curve.points(),
    }
}
