use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};

use super::{LEAST_RISE, LoftError, LoftProfile, MAX_SPANS};
use crate::{
    bspline::BSpline,
    build::revolve::{arc_spline, ellipse_spline},
    curve::{BSplineCurve, Curve},
    curve2::{BSplineCurve2, Curve2},
    error::GeometryError,
    interval::Interval,
    profile::Piece,
};

const LENGTH_SAMPLES: usize = 8;
const AIM_SAMPLES: usize = 96;
const AIM_REFINEMENTS: usize = 48;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Span<'a> {
    pub(super) piece: &'a Piece,
    pub(super) range: Interval,
    pub(super) reversed: bool,
    pub(super) part: u32,
}

impl Span<'_> {
    fn start_parameter(&self) -> f64 {
        if self.reversed {
            self.range.end()
        } else {
            self.range.start()
        }
    }

    fn length(&self) -> f64 {
        let curve = self.piece.curve();
        let points: Vec<Point2> = self
            .range
            .split(LENGTH_SAMPLES)
            .map(|parameter| curve.point(parameter))
            .collect();
        points
            .windows(2)
            .map(|pair| match pair {
                [from, to] => from.distance(*to),
                _ => 0.0,
            })
            .sum()
    }

    fn halves(&self) -> Option<[Self; 2]> {
        let middle = self.range.middle();
        let low = Self {
            range: Interval::new(self.range.start(), middle)?,
            ..self.clone()
        };
        let high = Self {
            range: Interval::new(middle, self.range.end())?,
            ..self.clone()
        };
        Some(if self.reversed {
            [high, low]
        } else {
            [low, high]
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Ring<'a> {
    pub(super) sketch: Plane,
    pub(super) facing: f64,
    pub(super) centre: Point3,
    pub(super) spans: Vec<Span<'a>>,
    round: Option<&'a Piece>,
}

impl<'a> Ring<'a> {
    fn of(profile: &'a LoftProfile, index: usize) -> Result<Self, LoftError> {
        let region = &profile.region;
        if !region.holes().is_empty() {
            return Err(LoftError::Holes { profile: index });
        }
        let pieces = region.outer().pieces();
        if pieces.len() > MAX_SPANS {
            return Err(LoftError::TooIntricate);
        }
        let bounds = region.bounds().ok_or(LoftError::TooFewProfiles)?;
        let spans: Vec<Span<'a>> = pieces
            .iter()
            .map(|piece| Span {
                piece,
                range: piece.range(),
                reversed: piece.is_reversed(),
                part: 0,
            })
            .collect();
        let round = match pieces {
            [piece] if piece.curve().period().is_some() => Some(piece),
            _ => None,
        };
        Ok(Self {
            sketch: profile.plane,
            facing: 1.0,
            centre: profile.plane.to_world(bounds.center()),
            spans,
            round,
        })
    }

    pub(super) fn oriented_normal(&self) -> Vector3 {
        self.sketch.normal() * self.facing
    }

    fn turn_round(&mut self) {
        self.facing = -self.facing;
        self.spans.reverse();
        for span in &mut self.spans {
            span.reversed = !span.reversed;
        }
    }

    fn split_to(&mut self, count: usize) -> Result<(), LoftError> {
        while self.spans.len() < count {
            let longest = self
                .spans
                .iter()
                .enumerate()
                .map(|(index, span)| (index, span.length()))
                .fold(
                    None,
                    |best: Option<(usize, f64)>, (index, length)| match best {
                        Some((_, most)) if most >= length => best,
                        _ => Some((index, length)),
                    },
                )
                .map(|(index, _)| index)
                .ok_or(LoftError::TooFewProfiles)?;
            let halves = self
                .spans
                .get(longest)
                .and_then(Span::halves)
                .ok_or(LoftError::TooIntricate)?;
            self.spans.splice(longest..=longest, halves);
        }
        Ok(())
    }

    fn started_at(&mut self, start: f64, count: usize) {
        let Some(piece) = self.round else {
            return;
        };
        let reversed = self.spans.first().is_some_and(|span| span.reversed);
        let period = piece.range().length();
        let step = period / count as f64;
        self.spans = (0..count)
            .filter_map(|index| {
                let (low, high) = if reversed {
                    (
                        start - step * (index + 1) as f64,
                        start - step * index as f64,
                    )
                } else {
                    (
                        start + step * index as f64,
                        start + step * (index + 1) as f64,
                    )
                };
                Some(Span {
                    piece,
                    range: Interval::new(low, high)?,
                    reversed,
                    part: 0,
                })
            })
            .collect();
    }

    pub(super) fn vertex(&self, index: usize) -> Option<Point3> {
        let span = self.spans.get(index % self.spans.len().max(1))?;
        Some(
            self.sketch
                .to_world(span.piece.curve().point(span.start_parameter())),
        )
    }

    fn vertices(&self) -> Vec<Point3> {
        (0..self.spans.len())
            .filter_map(|index| self.vertex(index))
            .collect()
    }

    fn aim_at(&mut self, direction: Vector3, count: usize) {
        let Some(piece) = self.round else {
            return;
        };
        let aim = Vector2::new(
            direction.dot(self.sketch.x_axis()),
            direction.dot(self.sketch.y_axis()),
        );
        let centre = self.sketch.to_local(self.centre);
        let start = match aim.try_normalize() {
            Some(aim) => {
                let curve = piece.curve();
                let facing = |parameter: f64| {
                    (curve.point(parameter) - centre)
                        .try_normalize()
                        .map_or(-2.0, |toward| toward.dot(aim))
                };
                let range = piece.range();
                let step = range.length() / AIM_SAMPLES as f64;
                let coarse = range
                    .split(AIM_SAMPLES)
                    .fold((range.start(), f64::NEG_INFINITY), |best, parameter| {
                        let score = facing(parameter);
                        if score > best.1 {
                            (parameter, score)
                        } else {
                            best
                        }
                    })
                    .0;
                let (mut low, mut high) = (coarse - step, coarse + step);
                for _ in 0..AIM_REFINEMENTS {
                    let first = low + (high - low) / 3.0;
                    let second = high - (high - low) / 3.0;
                    if facing(first) < facing(second) {
                        low = first;
                    } else {
                        high = second;
                    }
                }
                0.5 * (low + high)
            }
            None => piece.range().start(),
        };
        self.started_at(start, count);
    }

    fn shifted_like(&mut self, neighbour: &Ring<'_>) {
        let count = self.spans.len();
        let own = self.vertices();
        let theirs = neighbour.vertices();
        if own.len() != count || theirs.len() != count || count < 2 {
            return;
        }
        let misfit = |shift: usize| -> f64 {
            theirs
                .iter()
                .enumerate()
                .filter_map(|(index, their)| {
                    let mine = own.get((index + shift) % count)?;
                    Some(((*mine - self.centre) - (*their - neighbour.centre)).length_squared())
                })
                .sum()
        };
        let best = (0..count).fold((0, f64::INFINITY), |best, shift| {
            let score = misfit(shift);
            if score < best.1 { (shift, score) } else { best }
        });
        self.spans.rotate_left(best.0);
    }

    fn aligned_to(&mut self, neighbour: &Ring<'_>, count: usize) {
        if self.round.is_some() {
            let toward = neighbour
                .vertex(0)
                .map_or(Vector3::ZERO, |vertex| vertex - neighbour.centre);
            let across = toward - self.sketch.normal() * toward.dot(self.sketch.normal());
            self.aim_at(across, count);
        } else {
            self.shifted_like(neighbour);
        }
    }

    fn number_parts(&mut self) {
        let starts: Vec<(&'a Piece, f64)> = self
            .spans
            .iter()
            .map(|span| (span.piece, span.range.start()))
            .collect();
        for span in &mut self.spans {
            let earlier = starts
                .iter()
                .filter(|(piece, start)| {
                    piece.id() == span.piece.id() && *start < span.range.start()
                })
                .count();
            span.part = u32::try_from(earlier).unwrap_or(u32::MAX);
        }
    }

    pub(super) fn span_spline(&self, index: usize) -> Result<BSplineCurve, GeometryError> {
        let span = self
            .spans
            .get(index)
            .ok_or(GeometryError::DegenerateSurface)?;
        let range = span.range;
        let flat: BSplineCurve2 = match span.piece.curve() {
            Curve2::Line(line) => BSpline::new(
                1,
                vec![range.start(), range.start(), range.end(), range.end()],
                vec![line.point(range.start()), line.point(range.end())],
            )?,
            Curve2::Circle(circle) => arc_spline(circle, range)?,
            Curve2::Ellipse(ellipse) => ellipse_spline(ellipse, range)?,
            Curve2::BSpline(spline) => spline.restricted(range).ok_or(GeometryError::Knots)?,
        };
        let placed = flat.map_points(|point| self.sketch.to_world(point))?;
        Ok(if span.reversed {
            placed.reversed()
        } else {
            placed
        })
    }

    pub(super) fn span_edge(&self, index: usize) -> Result<(Curve, Interval), GeometryError> {
        let span = self
            .spans
            .get(index)
            .ok_or(GeometryError::DegenerateSurface)?;
        Ok((span.piece.curve().on_plane(&self.sketch)?, span.range))
    }

    pub(super) fn traversal_tangent(&self, index: usize) -> Option<(Point3, Vector3)> {
        let span = self.spans.get(index)?;
        let derivatives = span.piece.curve().evaluate(span.range.middle());
        let along =
            self.sketch.x_axis() * derivatives.first.x + self.sketch.y_axis() * derivatives.first.y;
        let along = if span.reversed { -along } else { along };
        Some((self.sketch.to_world(derivatives.point), along))
    }
}

fn closed_travel(centres: &[Point3], index: usize, tolerance: f64) -> Result<Vector3, LoftError> {
    let count = centres.len();
    let at = |offset: usize| {
        centres
            .get((index + offset) % count.max(1))
            .copied()
            .ok_or(LoftError::TooFewProfiles)
    };
    let (before, here, after) = (at(count - 1)?, at(0)?, at(1)?);
    let gap = after.distance(here);
    if !gap.is_finite() || gap <= tolerance {
        return Err(LoftError::Coincident {
            profiles: [index, (index + 1) % count],
        });
    }
    Ok(after - before)
}

fn open_travel(centres: &[Point3], index: usize, tolerance: f64) -> Result<Vector3, LoftError> {
    let last = centres.len().saturating_sub(1);
    let pair = if index < last {
        [index, index + 1]
    } else {
        [index.saturating_sub(1), index]
    };
    let [from, to] = pair.map(|at| centres.get(at).copied());
    let (Some(from), Some(to)) = (from, to) else {
        return Err(LoftError::TooFewProfiles);
    };
    let travel = to - from;
    let reach = travel.length();
    if !reach.is_finite() || reach <= tolerance {
        return Err(LoftError::Coincident { profiles: pair });
    }
    Ok(travel)
}

pub(super) fn rings<'a>(
    profiles: &'a [LoftProfile],
    apex: Option<Point3>,
    closed: bool,
    tolerance: f64,
) -> Result<Vec<Ring<'a>>, LoftError> {
    let mut rings = profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| Ring::of(profile, index))
        .collect::<Result<Vec<_>, _>>()?;
    let centres: Vec<Point3> = rings.iter().map(|ring| ring.centre).chain(apex).collect();
    for (index, ring) in rings.iter_mut().enumerate() {
        let travel = if closed {
            closed_travel(&centres, index, tolerance)?
        } else {
            open_travel(&centres, index, tolerance)?
        };
        let reach = travel.length();
        if !reach.is_finite() || reach <= tolerance {
            return Err(LoftError::AlongProfile { profile: index });
        }
        let rise = ring.sketch.normal().dot(travel) / reach;
        if rise.abs() < LEAST_RISE {
            return Err(LoftError::AlongProfile { profile: index });
        }
        if rise < 0.0 {
            ring.turn_round();
        }
    }
    let count = rings
        .iter()
        .map(|ring| ring.spans.len())
        .max()
        .ok_or(LoftError::TooFewProfiles)?;
    for ring in &mut rings {
        if ring.round.is_none() {
            ring.split_to(count)?;
        }
    }
    let reference = rings
        .iter()
        .position(|ring| ring.round.is_none() && ring.spans.len() == count)
        .unwrap_or(0);
    if let Some(ring) = rings.get_mut(reference)
        && let Some(piece) = ring.round
    {
        let start = piece.range().start();
        ring.started_at(start, count);
    }
    for index in reference + 1..rings.len() {
        align(&mut rings, index, index - 1, count);
    }
    for index in (0..reference).rev() {
        align(&mut rings, index, index + 1, count);
    }
    for ring in &mut rings {
        ring.number_parts();
    }
    Ok(rings)
}

fn align(rings: &mut [Ring<'_>], index: usize, neighbour: usize, count: usize) {
    let Some(guide) = rings.get(neighbour).cloned() else {
        return;
    };
    if let Some(ring) = rings.get_mut(index) {
        ring.aligned_to(&guide, count);
    }
}
