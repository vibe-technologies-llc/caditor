use std::{f64::consts::TAU, rc::Rc};

use caditor_geometry::{Point2, Vector2};

use crate::{
    import::{
        dxf::geometry::Shape,
        svg::{path::Outline, syntax::Matrix},
    },
    tally,
};

const STEPS_PER_SPAN: usize = 32;
const STEPS_PER_TURN: f64 = 128.0;
const LEAST_ARC_STEPS: usize = 4;
const MAX_TRACK_POINTS: usize = 200_000;
const STRAIGHT: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Method {
    #[default]
    Align,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Spacing {
    #[default]
    Exact,
    Auto,
}

#[derive(Debug, Default, PartialEq)]
struct Line {
    points: Vec<Point2>,
    distances: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Track {
    line: Rc<Line>,
    pub start: f64,
    pub method: Method,
    pub spacing: Spacing,
}

impl Track {
    pub fn of(outline: &Outline, matrix: &Matrix, side: Side) -> Self {
        let mut pieces: Vec<Vec<Point2>> = Vec::new();
        let mut budget = MAX_TRACK_POINTS;

        for shape in &outline.shapes {
            if budget == 0 {
                break;
            }

            let piece: Vec<Point2> = samples(shape)
                .take(budget)
                .map(|point| matrix.apply(point))
                .collect();

            tally::add(piece.len());
            budget = budget.saturating_sub(piece.len());
            pieces.push(piece);
        }
        if side == Side::Right {
            pieces.reverse();
            for piece in &mut pieces {
                piece.reverse();
            }
        }

        let mut line = Line::default();

        for piece in pieces {
            let mut points = piece.into_iter();

            if let Some(first) = points.next() {
                line.jump(first);
            }
            for point in points {
                line.extend(point);
            }
        }
        Self {
            line: Rc::new(line),
            ..Self::default()
        }
    }

    pub fn point_count(&self) -> usize {
        self.line.points.len()
    }

    pub fn length(&self) -> f64 {
        self.line.length()
    }
}

impl Line {
    fn jump(&mut self, point: Point2) {
        if self.points.last() == Some(&point) {
            return;
        }

        let distance = self.length();

        self.points.push(point);
        self.distances.push(distance);
    }

    fn extend(&mut self, point: Point2) {
        let Some(last) = self.points.last().copied() else {
            self.jump(point);
            return;
        };
        let step = (point - last).length();

        if !step.is_finite() || step == 0.0 {
            return;
        }
        self.points.push(point);
        self.distances.push(self.length() + step);
    }

    fn length(&self) -> f64 {
        self.distances.last().copied().unwrap_or(0.0)
    }
}

impl Track {
    pub fn extended(&self, distance: f64) -> Option<(Point2, Vector2)> {
        let length = self.length();
        if distance < 0.0 {
            let (point, tangent) = self.at(0.0)?;
            Some((point + tangent * distance, tangent))
        } else if distance > length {
            let (point, tangent) = self.at(length)?;
            Some((point + tangent * (distance - length), tangent))
        } else {
            self.at(distance)
        }
    }

    pub fn turns_between(&self, from: f64, to: f64) -> bool {
        let Some((_, first)) = self.extended(from) else {
            return false;
        };
        let inside = self
            .line
            .distances
            .partition_point(|reached| *reached <= from)
            ..self.line.distances.partition_point(|reached| *reached < to);
        let corners = self.line.distances.get(inside).unwrap_or_default();
        let ends = [from, to];
        corners
            .iter()
            .copied()
            .chain(ends)
            .filter_map(|distance| self.extended(distance))
            .any(|(_, tangent)| (tangent - first).length() > STRAIGHT)
    }

    pub fn at(&self, distance: f64) -> Option<(Point2, Vector2)> {
        if !(0.0..=self.length()).contains(&distance) {
            return None;
        }

        let mut index = self
            .line
            .distances
            .partition_point(|reached| *reached < distance)
            .max(1);

        loop {
            let (from, to) = (
                self.line.distances.get(index - 1)?,
                self.line.distances.get(index)?,
            );

            if to > from {
                break;
            }
            index += 1;
        }

        let (from, to) = (
            self.line.points.get(index - 1)?,
            self.line.points.get(index)?,
        );
        let (near, far) = (
            self.line.distances.get(index - 1)?,
            self.line.distances.get(index)?,
        );

        let share = (distance - near) / (far - near);
        let tangent = (*to - *from).try_normalize()?;
        Some((*from + (*to - *from) * share, tangent))
    }
}

fn samples(shape: &Shape) -> Box<dyn Iterator<Item = Point2> + '_> {
    match shape {
        Shape::Line(from, to) => Box::new([from.truncate(), to.truncate()].into_iter()),
        Shape::Conic {
            center,
            major,
            minor,
            start,
            sweep,
        } => {
            let steps = arc_steps(*sweep);
            Box::new((0..=steps).map(move |step| {
                let angle = start + sweep * step as f64 / steps as f64;
                (*center + *major * angle.cos() + *minor * angle.sin()).truncate()
            }))
        }
        Shape::Spline(nurbs) => {
            let Some((low, high)) = nurbs.domain() else {
                return Box::new(std::iter::empty());
            };
            let steps = nurbs.spans().max(1).saturating_mul(STEPS_PER_SPAN);
            Box::new((0..=steps).filter_map(move |step| {
                let parameter = low + (high - low) * step as f64 / steps as f64;
                nurbs.point(parameter).map(|point| point.truncate())
            }))
        }
        Shape::Point(_) | Shape::Interpolated(_) => Box::new(std::iter::empty()),
    }
}

fn arc_steps(sweep: f64) -> usize {
    let turns = (sweep.abs() / TAU * STEPS_PER_TURN).ceil();
    if turns.is_finite() && turns > 0.0 {
        (turns as usize).max(LEAST_ARC_STEPS)
    } else {
        LEAST_ARC_STEPS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(data: &str) -> Outline {
        crate::import::svg::path::path_outline(data)
    }

    #[test]
    fn a_track_measures_along_its_pieces_and_skips_the_gaps_between_them() {
        let track = Track::of(
            &outline("M 0 0 L 10 0 M 20 0 L 20 5"),
            &Matrix::IDENTITY,
            Side::Left,
        );

        assert_eq!(track.length(), 15.0);
        assert_eq!(
            track.at(5.0),
            Some((Point2::new(5.0, 0.0), Vector2::new(1.0, 0.0)))
        );
        assert_eq!(
            track.at(12.0),
            Some((Point2::new(20.0, 2.0), Vector2::new(0.0, 1.0)))
        );
        assert_eq!(track.at(15.5), None);
        assert_eq!(track.at(-0.5), None);
    }

    #[test]
    fn the_right_side_walks_the_path_backwards() {
        let track = Track::of(&outline("M 0 0 L 10 0"), &Matrix::IDENTITY, Side::Right);

        assert_eq!(
            track.at(2.0),
            Some((Point2::new(8.0, 0.0), Vector2::new(-1.0, 0.0)))
        );
    }

    #[test]
    fn a_circle_is_as_long_as_its_circumference() {
        let track = Track::of(
            &outline("M 10 0 A 10 10 0 1 1 -10 0 A 10 10 0 1 1 10 0"),
            &Matrix::IDENTITY,
            Side::Left,
        );

        assert!(
            (track.length() - TAU * 10.0).abs() < 0.01,
            "{}",
            track.length()
        );
    }
}
