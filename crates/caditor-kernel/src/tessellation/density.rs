use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Point2, Point3};

use crate::{surface::Surface, tolerance::SamplingTolerance};

const LATTICE: usize = 8;
const MAX_SEGMENTS: usize = 1024;
const MAX_GRID_POINTS: f64 = (1 << 19) as f64;
const FLAT_CURVATURE: f64 = 1e-12;
const FLAT_SEGMENTS: usize = 2;
const SPAN_SAMPLES: usize = 4;
const MAX_LATTICE: usize = 64;
const REFINEMENTS: usize = 4;
const REFINEMENT_MARGIN: f64 = 1.05;
const GRID_SHARE: f64 = 0.7;
const MIN_GROWTH: f64 = 1.1;
const CHECKED_CELLS: usize = 16;
const PLACED_CHECKS: usize = 8;
const COLLAPSED_EDGE: f64 = 1e-9;
const ROUNDING_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Spacing {
    lines: Vec<f64>,
    speed: f64,
}

impl Spacing {
    fn uniform(low: f64, high: f64, segments: usize, speed: f64) -> Self {
        let segments = segments.max(1);
        let lines = (0..=segments)
            .map(|line| {
                if line == segments {
                    high
                } else {
                    low + (high - low) * line as f64 / segments as f64
                }
            })
            .collect();
        Self { lines, speed }
    }

    pub(crate) fn segments(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    pub(crate) fn line(&self, index: usize) -> Option<f64> {
        self.lines.get(index).copied()
    }

    pub(crate) fn speed(&self) -> f64 {
        self.speed
    }

    fn has_extent(&self) -> bool {
        matches!((self.lines.first(), self.lines.last()), (Some(low), Some(high)) if high > low)
    }

    fn cell(&self, index: usize) -> Option<(f64, f64)> {
        Some((self.line(index)?, self.line(index + 1)?))
    }

    pub(crate) fn index(&self, value: f64) -> f64 {
        let last = self.segments();
        if last == 0 {
            return 0.0;
        }
        let span = self
            .lines
            .partition_point(|line| *line <= value)
            .clamp(1, last)
            - 1;
        let Some((low, high)) = self.cell(span) else {
            return 0.0;
        };
        let width = high - low;
        if width > 0.0 {
            span as f64 + (value - low) / width
        } else {
            span as f64
        }
    }

    fn at(&self, index: f64) -> f64 {
        let last = self.segments().max(1);
        let span = index.floor().clamp(0.0, (last - 1) as f64) as usize;
        match self.cell(span) {
            Some((low, high)) => low + (high - low) * (index - span as f64),
            None => self.line(0).unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Density {
    pub u: Spacing,
    pub v: Spacing,
}

impl Density {
    pub(crate) fn index(&self, uv: Point2) -> Point2 {
        Point2::new(self.u.index(uv.x), self.v.index(uv.y))
    }

    pub(crate) fn at(&self, cell: Point2) -> Point2 {
        Point2::new(self.u.at(cell.x), self.v.at(cell.y))
    }

    pub(crate) fn gridded(&self) -> bool {
        self.u.segments() >= 2
            && self.v.segments() >= 2
            && self.u.has_extent()
            && self.v.has_extent()
    }

    pub(crate) fn heap_size(&self) -> usize {
        size_of_val(self.u.lines.as_slice()) + size_of_val(self.v.lines.as_slice())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Grading {
    lattice: Vec<f64>,
    needs: Vec<f64>,
    speed: f64,
}

impl Grading {
    fn new(lattice: Vec<f64>, needs_at_points: &[f64], speed: f64) -> Self {
        let needs = needs_at_points
            .windows(2)
            .map(|pair| match pair {
                [low, high] => low.max(*high),
                _ => 0.0,
            })
            .collect();
        let mut grading = Self {
            lattice,
            needs,
            speed,
        };
        grading.bound_needs();
        grading
    }

    fn bound_needs(&mut self) {
        let (low, high) = self.bounds();
        let most = MAX_SEGMENTS as f64 / (high - low);
        for need in &mut self.needs {
            *need = if need.is_nan() { most } else { need.min(most) };
        }
    }

    fn spans(&self) -> impl Iterator<Item = ((f64, f64), f64)> + '_ {
        self.lattice
            .windows(2)
            .filter_map(|pair| match pair {
                [low, high] => Some((*low, *high)),
                _ => None,
            })
            .zip(self.needs.iter().copied())
    }

    fn needed(&self) -> f64 {
        self.spans()
            .map(|((low, high), need)| need * (high - low))
            .sum()
    }

    fn is_flat(&self) -> bool {
        self.needed() <= 0.0
    }

    fn bounds(&self) -> (f64, f64) {
        let low = self.lattice.first().copied().unwrap_or(0.0);
        (low, self.lattice.last().copied().unwrap_or(low))
    }

    fn spacing(&self, segments: usize) -> Spacing {
        let (low, high) = self.bounds();
        let total = self.needed();
        if !total.is_finite() || total <= 0.0 || segments < 2 {
            return Spacing::uniform(low, high, segments, self.speed);
        }
        let mut lines = Vec::with_capacity(segments + 1);
        lines.push(low);
        let mut spans = self.spans().peekable();
        let mut reached = 0.0;
        for line in 1..segments {
            let target = total * line as f64 / segments as f64;
            while let Some(((start, end), need)) = spans.peek().copied() {
                let measure = need * (end - start);
                if reached + measure >= target && need > 0.0 {
                    let at = start + (target - reached) / need;
                    lines.push(at.clamp(start, end));
                    break;
                }
                reached += measure;
                spans.next();
            }
        }
        lines.push(high);
        lines.dedup();
        if lines.len() < 2 {
            return Spacing::uniform(low, high, 1, self.speed);
        }
        Spacing {
            lines,
            speed: self.speed,
        }
    }

    fn refine(&mut self, current: &Spacing, factor: f64) {
        let spans: Vec<(f64, f64)> = self.spans().map(|(span, _)| span).collect();
        for ((low, high), need) in spans.into_iter().zip(self.needs.iter_mut()) {
            let width = high - low;
            if width > 0.0 {
                let present = (current.index(high) - current.index(low)) / width;
                *need = need.max(present) * factor;
            }
        }
        self.bound_needs();
    }
}

fn lattice(low: f64, high: f64, knots: &[f64]) -> Vec<f64> {
    let mut breaks: Vec<f64> = std::iter::once(low)
        .chain(
            knots
                .iter()
                .copied()
                .filter(|knot| *knot > low && *knot < high),
        )
        .chain(std::iter::once(high))
        .collect();
    breaks.dedup();
    let stride = breaks.len().saturating_sub(1).div_ceil(MAX_LATTICE).max(1);
    let mut kept: Vec<f64> = breaks.iter().copied().step_by(stride).collect();
    kept.push(high);
    kept.dedup();
    let spans = kept.len().saturating_sub(1).max(1);
    let per_span = (MAX_LATTICE / spans).clamp(1, SPAN_SAMPLES);
    let mut values: Vec<f64> = (0..=LATTICE)
        .map(|index| low + (high - low) * index as f64 / LATTICE as f64)
        .collect();
    for pair in kept.windows(2) {
        if let [start, end] = pair {
            values.extend(
                (0..per_span).map(|index| start + (end - start) * index as f64 / per_span as f64),
            );
        }
    }
    values.push(high);
    values.sort_by(f64::total_cmp);
    values.dedup();
    values
}

pub(crate) fn density(surface: &Surface, bounds: Aabb2, tolerance: &SamplingTolerance) -> Density {
    let (u_knots, v_knots): (&[f64], &[f64]) = match surface {
        Surface::BSpline(spline) => (spline.u_knots(), spline.v_knots()),
        _ => (&[], &[]),
    };
    let us = lattice(bounds.min().x, bounds.max().x, u_knots);
    let vs = lattice(bounds.min().y, bounds.max().y, v_knots);
    let mut u_needs = vec![0.0_f64; us.len()];
    let mut v_needs = vec![0.0_f64; vs.len()];
    let (mut u_speeds, mut v_speeds, mut count) = (0.0, 0.0, 0.0);
    for (v, v_need) in vs.iter().zip(v_needs.iter_mut()) {
        for (u, u_need) in us.iter().zip(u_needs.iter_mut()) {
            let derivatives = surface.evaluate(*u, *v);
            let (u_speed, v_speed) = (derivatives.du.length(), derivatives.dv.length());
            if !u_speed.is_finite() || !v_speed.is_finite() {
                continue;
            }
            u_speeds += u_speed;
            v_speeds += v_speed;
            count += 1.0;
            let Some(normal) = derivatives.normal().or_else(|| surface.normal(*u, *v)) else {
                continue;
            };
            let twist = if u_speed > 0.0 && v_speed > 0.0 {
                derivatives.duv.dot(normal).abs() / (u_speed * v_speed)
            } else {
                0.0
            };
            if u_speed > 0.0 {
                let curvature = derivatives.duu.dot(normal).abs() / (u_speed * u_speed);
                *u_need = u_need.max(u_speed / step_for(curvature.max(twist), tolerance));
            }
            if v_speed > 0.0 {
                let curvature = derivatives.dvv.dot(normal).abs() / (v_speed * v_speed);
                *v_need = v_need.max(v_speed / step_for(curvature.max(twist), tolerance));
            }
        }
    }
    let mut u = Grading::new(us, &u_needs, scale(u_speeds, count));
    let mut v = Grading::new(vs, &v_needs, scale(v_speeds, count));
    let mut u_segments = segments(u.needed());
    let mut v_segments = segments(v.needed());
    if u.is_flat() && v_segments > 1 {
        u_segments = FLAT_SEGMENTS;
    }
    if v.is_flat() && u_segments > 1 {
        v_segments = FLAT_SEGMENTS;
    }
    let mut found = Density {
        u: u.spacing(u_segments),
        v: v.spacing(v_segments),
    };
    let measured = matches!(
        surface,
        Surface::BSpline(_)
            | Surface::Revolution(_)
            | Surface::Extrusion(_)
            | Surface::Cone(_)
            | Surface::Sphere(_)
            | Surface::Torus(_)
    );
    let allowed = tolerance.chord() * GRID_SHARE;
    for _ in 0..if measured { REFINEMENTS } else { 0 } {
        let growth = growth(surface, &found, allowed);
        if growth.u <= 1.0 && growth.v <= 1.0 {
            break;
        }
        u.refine(&found.u, growth.u);
        v.refine(&found.v, growth.v);
        found = Density {
            u: u.spacing(segments(u.needed())),
            v: v.spacing(segments(v.needed())),
        };
    }
    let total = found.u.segments() as f64 * found.v.segments() as f64;
    if total > MAX_GRID_POINTS {
        let shrink = (MAX_GRID_POINTS / total).sqrt();
        found = Density {
            u: u.spacing(segments(found.u.segments() as f64 * shrink)),
            v: v.spacing(segments(found.v.segments() as f64 * shrink)),
        };
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Growth {
    u: f64,
    v: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CellDeviation {
    along_u: f64,
    along_v: f64,
    across: f64,
}

impl CellDeviation {
    fn worst(&self) -> f64 {
        self.along_u.max(self.along_v).max(self.across)
    }
}

fn checked_cells(spacing: &Spacing) -> BTreeSet<usize> {
    let count = spacing.segments();
    let stride = (count / CHECKED_CELLS).max(1);
    let mut cells: BTreeSet<usize> = (0..count).step_by(stride).collect();
    if let (Some(low), Some(high)) = (spacing.line(0), spacing.line(count)) {
        for place in 0..PLACED_CHECKS {
            let at = low + (high - low) * (place as f64 + 0.5) / PLACED_CHECKS as f64;
            let index = spacing.index(at).floor();
            if index.is_finite() && index >= 0.0 {
                cells.insert((index as usize).min(count.saturating_sub(1)));
            }
        }
    }
    cells
}

fn cells_of(density: &Density) -> impl Iterator<Item = ((f64, f64), (f64, f64))> + '_ {
    let columns = checked_cells(&density.u);
    let rows = checked_cells(&density.v);
    rows.into_iter()
        .filter_map(|row| density.v.cell(row))
        .flat_map(move |v| {
            columns
                .clone()
                .into_iter()
                .filter_map(|column| density.u.cell(column))
                .map(move |u| (u, v))
        })
}

fn growth(surface: &Surface, density: &Density, allowed: f64) -> Growth {
    let mut found = Growth { u: 1.0, v: 1.0 };
    let factor =
        |deviation: f64| ((deviation / allowed).sqrt() * REFINEMENT_MARGIN).max(MIN_GROWTH);
    for ((u_low, u_high), (v_low, v_high)) in cells_of(density) {
        let deviation = cell_deviation(
            surface,
            Point2::new(u_low, v_low),
            Point2::new(u_high, v_high),
        );
        if !deviation.worst().is_finite() || deviation.worst() <= allowed {
            continue;
        }
        let twisted = deviation.across > allowed
            && deviation.across > REFINEMENT_MARGIN * deviation.along_u.max(deviation.along_v);
        let across = if twisted { deviation.across } else { 0.0 };
        let (along_u, along_v) = (deviation.along_u.max(across), deviation.along_v.max(across));
        if along_u > allowed {
            found.u = found.u.max(factor(along_u));
        }
        if along_v > allowed {
            found.v = found.v.max(factor(along_v));
        }
    }
    found
}

fn cell_deviation(surface: &Surface, low: Point2, high: Point2) -> CellDeviation {
    let uv = |u: f64, v: f64| low + (high - low) * Point2::new(u, v);
    let at = |u: f64, v: f64| surface.point_at(uv(u, v));
    let [a, b, c, d] = [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)];
    let span = a.distance(d).max(b.distance(c));
    let collapsed = |p: Point3, q: Point3| p.distance(q) <= COLLAPSED_EDGE * span;
    let bow = |u: f64, v: f64, chords: &[(Point3, Point3)]| {
        let place = uv(u, v);
        let middle = surface.point_at(place);
        let normal = surface.normal(place.x, place.y);
        chords
            .iter()
            .map(|(from, to)| {
                let offset = middle - (*from + *to) * 0.5;
                normal.map_or(offset.length(), |normal| offset.dot(normal).abs())
            })
            .fold(0.0, f64::max)
    };
    let (low_collapsed, high_collapsed) = (collapsed(a, b), collapsed(c, d));
    let along_u = [
        (!low_collapsed).then(|| bow(0.5, 0.0, &[(a, b)])),
        (!high_collapsed).then(|| bow(0.5, 1.0, &[(c, d)])),
    ]
    .into_iter()
    .flatten()
    .fold(0.0, f64::max);
    let along_v = bow(0.0, 0.5, &[(a, c)]).max(bow(1.0, 0.5, &[(b, d)]));
    let across = if low_collapsed || high_collapsed {
        0.0
    } else {
        bow(0.5, 0.5, &[(a, d), (b, c)])
    };
    CellDeviation {
        along_u,
        along_v,
        across,
    }
}

fn scale(sum: f64, count: f64) -> f64 {
    let mean = if count > 0.0 { sum / count } else { 0.0 };
    if mean.is_finite() && mean > 0.0 {
        mean
    } else {
        1.0
    }
}

fn step_for(curvature: f64, tolerance: &SamplingTolerance) -> f64 {
    if curvature.is_finite() && curvature > FLAT_CURVATURE {
        tolerance.step_on_radius(1.0 / curvature)
    } else {
        f64::INFINITY
    }
}

fn segments(count: f64) -> usize {
    let count = (count * (1.0 - ROUNDING_SLACK)).ceil();
    if count.is_finite() && count >= 1.0 {
        (count as usize).min(MAX_SEGMENTS)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;

    use super::*;
    use crate::{
        fixtures,
        surface::{BSplineSurface, Cylinder},
    };

    fn unit_square() -> Aabb2 {
        Aabb2::from_points([Point2::ZERO, Point2::new(1.0, 1.0)]).unwrap()
    }

    fn worst_cell(surface: &Surface, density: &Density) -> f64 {
        let mut worst: f64 = 0.0;
        for row in 0..density.v.segments() {
            for column in 0..density.u.segments() {
                let (u_low, u_high) = density.u.cell(column).unwrap();
                let (v_low, v_high) = density.v.cell(row).unwrap();
                let deviation = cell_deviation(
                    surface,
                    Point2::new(u_low, v_low),
                    Point2::new(u_high, v_high),
                );
                worst = worst.max(deviation.worst());
            }
        }
        worst
    }

    #[test]
    fn a_twisted_patch_is_divided_until_its_triangles_follow_it() {
        let saddle = BSplineSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, 0.0, 0.0),
                Point3::new(0.0, 10.0, 0.0),
                Point3::new(10.0, 10.0, 10.0),
            ],
            None,
        )
        .unwrap();
        let surface = Surface::BSpline(saddle);
        let tolerance = SamplingTolerance::new(0.01, 0.2).unwrap();

        let found = density(&surface, unit_square(), &tolerance);

        assert!(
            found.u.segments() > 4 && found.v.segments() > 4,
            "{found:?}"
        );
        assert!(worst_cell(&surface, &found) <= tolerance.chord());
    }

    #[test]
    fn a_small_bump_divides_only_the_rows_and_columns_through_it() {
        let surface = Surface::BSpline(fixtures::bumped_sheet(100.0, 0.0, 41, 2.0));
        let tolerance = SamplingTolerance::new(0.01, 0.2).unwrap();

        let found = density(&surface, unit_square(), &tolerance);
        let width = |spacing: &Spacing, index: usize| {
            let (low, high) = spacing.cell(index).unwrap();
            high - low
        };
        let narrowest = (0..found.u.segments())
            .map(|index| width(&found.u, index))
            .fold(f64::INFINITY, f64::min);
        let widest = (0..found.u.segments())
            .map(|index| width(&found.u, index))
            .fold(0.0, f64::max);

        assert!(worst_cell(&surface, &found) <= tolerance.chord());
        assert!(widest > 10.0 * narrowest, "{found:?}");
        assert!(
            found.u.segments() < 150 && found.v.segments() < 150,
            "{} by {}",
            found.u.segments(),
            found.v.segments()
        );
    }

    #[test]
    fn a_direction_without_curvature_gets_two_cells_whatever_its_length() {
        let cylinder = Surface::Cylinder(Cylinder::new(Plane::XY, 0.5).unwrap());
        let bounds =
            Aabb2::from_points([Point2::ZERO, Point2::new(std::f64::consts::TAU, 1000.0)]).unwrap();
        let tolerance = SamplingTolerance::new(0.25, 0.1).unwrap();

        let found = density(&cylinder, bounds, &tolerance);

        assert_eq!(found.v.segments(), FLAT_SEGMENTS);
        assert!(found.u.segments() >= 60, "{found:?}");
        assert!(found.gridded());
    }

    #[test]
    fn a_degenerate_need_at_a_pole_leaves_every_other_span_its_cells() {
        let grading = Grading::new(vec![0.0, 1.0, 2.0, 3.0], &[2e16, 10.0, 10.0, 10.0], 1.0);

        let spacing = grading.spacing(segments(grading.needed()));
        let cells_in = |low: f64, high: f64| spacing.index(high) - spacing.index(low);

        assert!(spacing.segments() <= MAX_SEGMENTS);
        assert!(cells_in(1.0, 2.0) >= 10.0 && cells_in(2.0, 3.0) >= 10.0);
    }

    #[test]
    fn grid_lines_map_to_whole_cell_indices_in_order() {
        let grading = Grading::new(vec![0.0, 1.0, 2.0, 10.0], &[0.0, 8.0, 0.0, 0.0], 1.0);

        let spacing = grading.spacing(segments(grading.needed()));
        let indices: Vec<f64> = (0..=spacing.segments())
            .map(|line| spacing.index(spacing.line(line).unwrap()))
            .collect();

        assert_eq!(spacing.segments(), 16);
        assert_eq!(
            indices,
            (0..=16).map(|line| line as f64).collect::<Vec<_>>()
        );
        assert!(spacing.cell(15).unwrap().1 - spacing.cell(15).unwrap().0 > 4.0);
        assert!(spacing.index(-1.0) < 0.0 && spacing.index(11.0) > 16.0);
        for value in [-1.0, 0.3, 1.5, 1.9, 7.0, 11.0] {
            assert!((spacing.at(spacing.index(value)) - value).abs() < 1e-12);
        }
    }
}
