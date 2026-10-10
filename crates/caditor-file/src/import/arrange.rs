use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, TAU},
};

use caditor_geometry::Point2;
use caditor_sketch::ArcGeometry;

use crate::import::{Drawing, DrawingCurve, MAX_DRAWING_CURVES};

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

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingOptions {
    pub unit: DrawingUnit,
    pub scale: f64,
    pub recentre: bool,
    pub left_out_layers: BTreeSet<usize>,
}

impl Default for DrawingOptions {
    fn default() -> Self {
        Self {
            unit: DrawingUnit::AsInFile,
            scale: 1.0,
            recentre: false,
            left_out_layers: BTreeSet::new(),
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

    pub fn layer_curve_count(&self, layer: usize) -> usize {
        self.curves
            .iter()
            .zip(&self.curve_layers)
            .filter(|(curve, curve_layer)| {
                **curve_layer == layer && !matches!(curve, DrawingCurve::Point(_))
            })
            .count()
    }

    fn is_chosen(&self, index: usize, left_out: &BTreeSet<usize>) -> bool {
        !self
            .curve_layers
            .get(index)
            .is_some_and(|layer| left_out.contains(layer))
    }

    pub fn chosen_curve_count(&self, left_out: &BTreeSet<usize>) -> usize {
        (0..self.curves.len())
            .filter(|index| self.is_chosen(*index, left_out))
            .count()
    }

    fn on_chosen_layers(&self, left_out: &BTreeSet<usize>) -> Chosen {
        let mut chosen = Self {
            curves: Vec::new(),
            construction: BTreeSet::new(),
            notes: self.notes.clone(),
            unit_scale: self.unit_scale,
            layers: self.layers.clone(),
            curve_layers: Vec::new(),
        };
        let mut on_left_out_layers = 0;
        let mut past_the_limit = 0;
        for (index, curve) in self.curves.iter().enumerate() {
            if !self.is_chosen(index, left_out) {
                on_left_out_layers += usize::from(!matches!(curve, DrawingCurve::Point(_)));
                continue;
            }
            if chosen.curves.len() >= MAX_DRAWING_CURVES {
                past_the_limit += 1;
                continue;
            }
            if self.construction.contains(&index) {
                chosen.construction.insert(chosen.curves.len());
            }
            chosen.curves.push(curve.clone());
            if let Some(layer) = self.curve_layers.get(index) {
                chosen.curve_layers.push(*layer);
            }
        }
        Chosen {
            drawing: chosen,
            on_left_out_layers,
            past_the_limit,
        }
    }

    pub fn arranged(&self, options: &DrawingOptions) -> Self {
        let factor = self.factor(options);
        let Chosen {
            drawing: mut arranged,
            on_left_out_layers,
            past_the_limit,
        } = self.on_chosen_layers(&options.left_out_layers);
        let mut notes = Vec::new();
        match on_left_out_layers {
            0 => {}
            1 => notes.push("1 curve on a layer you left out was not imported.".to_owned()),
            many => notes.push(format!(
                "{many} curves on layers you left out were not imported."
            )),
        }
        if past_the_limit > 0 {
            notes.push(past_the_limit_note(past_the_limit));
        }
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

struct Chosen {
    drawing: Drawing,
    on_left_out_layers: usize,
    past_the_limit: usize,
}

fn past_the_limit_note(past_the_limit: usize) -> String {
    let were = if past_the_limit == 1 { "was" } else { "were" };
    format!(
        "Only the first {MAX_DRAWING_CURVES} curves were imported, because a sketch holds at most \
         that many; {past_the_limit} more {were} left out. Leave out layers or split the drawing \
         to import the rest."
    )
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
            Self::Spline {
                control_points: points,
            }
            | Self::FitSpline {
                fit_points: points, ..
            } => {
                for point in points {
                    *point = change(*point);
                }
            }
            Self::Ellipse {
                center,
                major,
                minor_radius,
                ends,
            } => {
                let scale = (change(*center + Point2::X) - change(*center)).length();
                *major = change(*center + *major) - change(*center);
                *minor_radius *= scale;
                *center = change(*center);
                if let Some((start, end)) = ends {
                    *start = change(*start);
                    *end = change(*end);
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
