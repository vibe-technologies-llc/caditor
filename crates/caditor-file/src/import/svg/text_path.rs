use std::f64::consts::TAU;

use caditor_geometry::{Point2, Vector2};

use crate::import::{
    dxf::geometry::Shape,
    svg::{path::Outline, syntax::Matrix},
};

const STEPS_PER_SPAN: usize = 32;
const STEPS_PER_TURN: f64 = 128.0;
const LEAST_ARC_STEPS: usize = 4;
const MAX_TRACK_POINTS: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Track {
    points: Vec<Point2>,
    distances: Vec<f64>,
    pub start: f64,
}

impl Track {
    pub fn of(outline: &Outline, matrix: &Matrix, side: Side) -> Self {
        let mut pieces: Vec<Vec<Point2>> = Vec::new();
        let mut budget = MAX_TRACK_POINTS;
        for shape in &outline.shapes {
            let piece: Vec<Point2> = samples(shape)
                .into_iter()
                .take(budget)
                .map(|point| matrix.apply(point))
                .collect();
            budget = budget.saturating_sub(piece.len());
            pieces.push(piece);
        }
        if side == Side::Right {
            pieces.reverse();
            for piece in &mut pieces {
                piece.reverse();
            }
        }
        let mut track = Self::default();
        for piece in pieces {
            let mut points = piece.into_iter();
            if let Some(first) = points.next() {
                track.jump(first);
            }
            for point in points {
                track.extend(point);
            }
        }
        track
    }

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

    pub fn length(&self) -> f64 {
        self.distances.last().copied().unwrap_or(0.0)
    }

    pub fn at(&self, distance: f64) -> Option<(Point2, Vector2)> {
        if !(0.0..=self.length()).contains(&distance) {
            return None;
        }
        let mut index = self
            .distances
            .partition_point(|reached| *reached < distance)
            .max(1);
        loop {
            let (from, to) = (self.distances.get(index - 1)?, self.distances.get(index)?);
            if to > from {
                break;
            }
            index += 1;
        }
        let (from, to) = (self.points.get(index - 1)?, self.points.get(index)?);
        let (near, far) = (self.distances.get(index - 1)?, self.distances.get(index)?);
        let share = (distance - near) / (far - near);
        let tangent = (*to - *from).try_normalize()?;
        Some((*from + (*to - *from) * share, tangent))
    }
}

fn samples(shape: &Shape) -> Vec<Point2> {
    match shape {
        Shape::Line(from, to) => vec![from.truncate(), to.truncate()],
        Shape::Conic {
            center,
            major,
            minor,
            start,
            sweep,
        } => {
            let steps = arc_steps(*sweep);
            (0..=steps)
                .map(|step| {
                    let angle = start + sweep * step as f64 / steps as f64;
                    (*center + *major * angle.cos() + *minor * angle.sin()).truncate()
                })
                .collect()
        }
        Shape::Spline(nurbs) => {
            let Some((low, high)) = nurbs.domain() else {
                return Vec::new();
            };
            let steps = nurbs.spans().max(1).saturating_mul(STEPS_PER_SPAN);
            (0..=steps)
                .filter_map(|step| {
                    let parameter = low + (high - low) * step as f64 / steps as f64;
                    nurbs.point(parameter).map(|point| point.truncate())
                })
                .collect()
        }
        Shape::Point(_) | Shape::Interpolated(_) => Vec::new(),
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
