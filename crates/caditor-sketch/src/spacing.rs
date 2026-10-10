use caditor_geometry::Point2;

use crate::{
    banded::Banded,
    curve::{
        BSpline, MAX_SPLINE_DEGREE, MIN_CLOSED_POINTS, basis_values, clamped_knots, periodic_knots,
        periodic_through,
    },
    entity::{Entity, FitSpacing, SplineKind},
    fit::{averaged_knots, collocate},
    id::EntityId,
    sketch::Sketch,
};

const SHORTEST_GAP_SHARE: f64 = 1e-3;
const BORDER: usize = 2;
const SINGULAR_SHARE: f64 = 1e-14;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FitLayout {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub parameters: Vec<f64>,
}

impl FitLayout {
    pub fn of(points: &[Point2], closed: bool, spacing: FitSpacing) -> Option<Self> {
        if closed {
            Self::closed(points, spacing)
        } else {
            Self::open(points, spacing)
        }
    }

    pub fn open(points: &[Point2], spacing: FitSpacing) -> Option<Self> {
        let count = points.len();
        if count < 2 {
            return None;
        }
        let (degree, uniform) = clamped_knots(count);
        match spacing {
            FitSpacing::Even => {
                let last = (count - 1) as f64;
                Some(Self {
                    degree,
                    knots: uniform,
                    parameters: (0..count).map(|index| index as f64 / last).collect(),
                })
            }
            FitSpacing::Centripetal => {
                let parameters = centripetal_parameters(points, false)?;
                Some(Self {
                    degree,
                    knots: averaged_knots(&parameters, degree),
                    parameters,
                })
            }
        }
    }

    pub fn closed(points: &[Point2], spacing: FitSpacing) -> Option<Self> {
        let count = points.len();
        if count < MIN_CLOSED_POINTS {
            return None;
        }
        match spacing {
            FitSpacing::Even => {
                let (degree, knots) = periodic_knots(count);
                Some(Self {
                    degree,
                    knots,
                    parameters: (0..count)
                        .map(|index| index as f64 / count as f64)
                        .collect(),
                })
            }
            FitSpacing::Centripetal => {
                let parameters = centripetal_parameters(points, true)?;
                let degree = MAX_SPLINE_DEGREE;
                let knots = (0..count + 2 * degree + 1)
                    .map(|index| looped(&parameters, index as isize - degree as isize))
                    .collect::<Option<Vec<_>>>()?;
                Some(Self {
                    degree,
                    knots,
                    parameters,
                })
            }
        }
    }

    pub fn settled_against(&self, other: &Self, tolerance: f64) -> bool {
        self.knots.len() == other.knots.len()
            && self
                .knots
                .iter()
                .zip(&other.knots)
                .all(|(own, theirs)| (own - theirs).abs() <= tolerance)
    }
}

pub(crate) fn fit_controls(
    points: &[Point2],
    closed: bool,
    spacing: FitSpacing,
    layout: &FitLayout,
) -> Option<Vec<Point2>> {
    match (closed, spacing) {
        (false, _) => collocate(layout.degree, &layout.knots, points, &layout.parameters),
        (true, FitSpacing::Even) => periodic_through(points),
        (true, FitSpacing::Centripetal) => periodic_through_knots(points, layout),
    }
}

impl Sketch {
    pub(crate) fn fit_layout(&self, spline: EntityId) -> Option<FitLayout> {
        let Some(Entity::Spline {
            points,
            kind: SplineKind::Fit { closed, spacing },
        }) = self.entity(spline)
        else {
            return None;
        };
        let positions = points
            .iter()
            .map(|point| self.point(*point))
            .collect::<Option<Vec<_>>>()?;
        FitLayout::of(&positions, *closed, *spacing)
    }
}

impl BSpline {
    pub fn interpolate_centripetal(points: &[Point2]) -> Option<BSpline> {
        let layout = FitLayout::open(points, FitSpacing::Centripetal)?;
        let control_points = fit_controls(points, false, FitSpacing::Centripetal, &layout)?;
        Some(BSpline::from_parts(
            control_points,
            layout.degree,
            layout.knots,
            None,
        ))
    }

    pub fn interpolate_closed_centripetal(points: &[Point2]) -> Option<BSpline> {
        let layout = FitLayout::closed(points, FitSpacing::Centripetal)?;
        let control_points = fit_controls(points, true, FitSpacing::Centripetal, &layout)?;
        BSpline::periodic_on(&control_points, layout.knots)
    }
}

fn centripetal_parameters(points: &[Point2], closed: bool) -> Option<Vec<f64>> {
    if points.iter().any(|point| !point.is_finite()) {
        return None;
    }
    let closing = closed.then(|| (points.last().copied(), points.first().copied()));
    let mut gaps: Vec<f64> = points
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some(a.distance(*b).sqrt()),
            _ => None,
        })
        .collect();
    if let Some((Some(last), Some(first))) = closing {
        gaps.push(last.distance(first).sqrt());
    }
    let total: f64 = gaps.iter().sum();
    let spans = gaps.len().max(1) as f64;
    if !(total > 0.0 && total.is_finite()) {
        gaps.iter_mut().for_each(|gap| *gap = 1.0);
    } else {
        let shortest = total * SHORTEST_GAP_SHARE / spans;
        gaps.iter_mut().for_each(|gap| *gap = gap.max(shortest));
    }
    let total: f64 = gaps.iter().sum();
    let mut travelled = 0.0;
    let mut parameters = Vec::with_capacity(points.len());
    parameters.push(0.0);
    for gap in gaps.iter().take(points.len().saturating_sub(1)) {
        travelled += gap;
        parameters.push(travelled / total);
    }
    if !closed && let Some(last) = parameters.last_mut() {
        *last = 1.0;
    }
    Some(parameters)
}

fn looped(parameters: &[f64], index: isize) -> Option<f64> {
    let count = isize::try_from(parameters.len()).ok()?;
    let turns = index.div_euclid(count);
    let within = usize::try_from(index.rem_euclid(count)).ok()?;
    Some(parameters.get(within)? + turns as f64)
}

pub(crate) fn periodic_through_knots(points: &[Point2], layout: &FitLayout) -> Option<Vec<Point2>> {
    let count = points.len();
    if count < MIN_CLOSED_POINTS || layout.parameters.len() != count {
        return None;
    }
    let rows: Vec<Vec<(usize, f64)>> = layout
        .parameters
        .iter()
        .map(|parameter| {
            let (first, weights) = basis_values(
                layout.degree,
                &layout.knots,
                count + layout.degree,
                *parameter,
            );
            let mut row: Vec<(usize, f64)> = Vec::with_capacity(weights.len());
            for (offset, weight) in weights.iter().enumerate() {
                if *weight == 0.0 {
                    continue;
                }
                let column = (first + offset + count - 1) % count;
                match row.iter_mut().find(|(known, _)| *known == column) {
                    Some((_, total)) => *total += weight,
                    None => row.push((column, *weight)),
                }
            }
            row
        })
        .collect();
    let columns = cyclic_solve(&rows, points)?;
    Some(
        (0..count)
            .filter_map(|slot| columns.get((slot + count - 1) % count).copied())
            .collect(),
    )
}

fn cyclic_solve(rows: &[Vec<(usize, f64)>], targets: &[Point2]) -> Option<Vec<Point2>> {
    let count = rows.len();
    let core = count.checked_sub(BORDER).filter(|core| *core > 0)?;
    let mut band = Banded::zeros(core, 1);
    let mut right: Vec<[f64; 2 + BORDER]> = Vec::with_capacity(core);
    let mut lower: Vec<(usize, usize, f64)> = Vec::new();
    let mut corner = [[0.0; BORDER]; BORDER];
    let mut tail = [[0.0; 2]; BORDER];
    for (row, (entries, target)) in rows.iter().zip(targets).enumerate() {
        if row < core {
            let mut values = [target.x, target.y, 0.0, 0.0];
            for (column, weight) in entries {
                if *column < core {
                    if row.abs_diff(*column) > 1 {
                        return None;
                    }
                    band.add(row, *column, *weight)?;
                } else {
                    *values.get_mut(2 + column - core)? += weight;
                }
            }
            right.push(values);
        } else {
            let border_row = row - core;
            *tail.get_mut(border_row)? = [target.x, target.y];
            for (column, weight) in entries {
                if *column < core {
                    lower.push((border_row, *column, *weight));
                } else {
                    *corner.get_mut(border_row)?.get_mut(column - core)? += weight;
                }
            }
        }
    }
    let solved = band.solve(right)?;
    let mut schur = corner;
    let mut reduced = tail;
    for (border_row, column, weight) in &lower {
        let [x, y, first, second] = *solved.get(*column)?;
        let schur_row = schur.get_mut(*border_row)?;
        schur_row[0] -= weight * first;
        schur_row[1] -= weight * second;
        let reduced_row = reduced.get_mut(*border_row)?;
        reduced_row[0] -= weight * x;
        reduced_row[1] -= weight * y;
    }
    let [[a, b], [c, d]] = schur;
    let determinant = a * d - b * c;
    let largest = [a, b, c, d]
        .iter()
        .fold(0.0_f64, |largest, entry| largest.max(entry.abs()));
    let singular =
        !determinant.is_finite() || determinant.abs() <= SINGULAR_SHARE * largest * largest;
    if singular {
        return None;
    }
    let [[first_x, first_y], [second_x, second_y]] = reduced;
    let border = [
        Point2::new(
            (d * first_x - b * second_x) / determinant,
            (d * first_y - b * second_y) / determinant,
        ),
        Point2::new(
            (a * second_x - c * first_x) / determinant,
            (a * second_y - c * first_y) / determinant,
        ),
    ];
    let mut columns: Vec<Point2> = solved
        .iter()
        .map(|[x, y, first, second]| Point2::new(*x, *y) - border[0] * *first - border[1] * *second)
        .collect();
    columns.extend(border);
    columns
        .iter()
        .all(|point| point.is_finite())
        .then_some(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uneven() -> Vec<Point2> {
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.5),
            Point2::new(2.0, 0.0),
            Point2::new(30.0, 4.0),
            Point2::new(31.0, 3.0),
            Point2::new(32.0, 5.0),
        ]
    }

    fn passes(curve: &BSpline, points: &[Point2], layout: &FitLayout) -> bool {
        points
            .iter()
            .zip(&layout.parameters)
            .all(|(point, parameter)| curve.point_at(*parameter).distance(*point) < 1e-9)
    }

    fn straying(curve: &BSpline, points: &[Point2]) -> f64 {
        let off_polyline = |at: Point2| {
            points
                .windows(2)
                .map(|pair| {
                    let (start, end) = (pair[0], pair[1]);
                    let along = end - start;
                    let share = ((at - start).dot(along) / along.length_squared()).clamp(0.0, 1.0);
                    at.distance(start + along * share)
                })
                .fold(f64::INFINITY, f64::min)
        };
        (0..=2_000)
            .map(|index| off_polyline(curve.point_at(f64::from(index) / 2_000.0)))
            .fold(0.0, f64::max)
    }

    fn backtracking(curve: &BSpline) -> f64 {
        let xs: Vec<f64> = (0..=2_000)
            .map(|index| curve.point_at(f64::from(index) / 2_000.0).x)
            .collect();
        xs.windows(2).map(|pair| (pair[0] - pair[1]).max(0.0)).sum()
    }

    #[test]
    fn a_centripetal_spline_passes_unevenly_spaced_points_without_overshooting() {
        let points = uneven();
        let on_line: Vec<Point2> = [0.0, 1.0, 2.0, 30.0, 31.0, 32.0]
            .map(|x| Point2::new(x, 0.0))
            .to_vec();
        let waving: Vec<Point2> = [0.0_f64, 1.0, 2.0, 3.0, 50.0]
            .map(|x| Point2::new(x, (x * 0.3).sin()))
            .to_vec();

        let spaced = BSpline::interpolate_centripetal(&points).unwrap();
        let even = BSpline::interpolate(&points).unwrap();

        assert!(passes(
            &spaced,
            &points,
            &FitLayout::open(&points, FitSpacing::Centripetal).unwrap()
        ));
        assert!(spaced.point_at(0.0).distance(points[0]) < 1e-12);
        assert!(spaced.point_at(1.0).distance(points[5]) < 1e-12);
        assert!(straying(&spaced, &points) < 0.5 * straying(&even, &points));
        for row in [on_line, waving] {
            assert!(backtracking(&BSpline::interpolate_centripetal(&row).unwrap()) < 1e-9);
            assert!(backtracking(&BSpline::interpolate(&row).unwrap()) > 1.0);
        }
    }

    #[test]
    fn a_closed_centripetal_spline_passes_its_points_and_closes_smoothly() {
        let points = uneven();

        let curve = BSpline::interpolate_closed_centripetal(&points).unwrap();
        let [start, start_bend] = curve.derivatives(0.0);
        let [end, end_bend] = curve.derivatives(1.0);

        assert!(passes(
            &curve,
            &points,
            &FitLayout::closed(&points, FitSpacing::Centripetal).unwrap()
        ));
        assert!(curve.point_at(0.0).distance(curve.point_at(1.0)) < 1e-9);
        assert!(start.distance(end) < 1e-7 * start.length());
        assert!(start_bend.distance(end_bend) < 1e-6 * start_bend.length().max(1.0));
    }

    #[test]
    fn the_cyclic_solve_agrees_with_the_even_closed_interpolation() {
        let points = uneven();
        let layout = FitLayout::closed(&points, FitSpacing::Even).unwrap();

        let cyclic = periodic_through_knots(&points, &layout).unwrap();
        let iterated = periodic_through(&points).unwrap();

        for (one, other) in cyclic.iter().zip(&iterated) {
            assert!(one.distance(*other) < 1e-9);
        }
    }

    #[test]
    fn repeated_points_still_give_a_finite_centripetal_curve() {
        let points = vec![
            Point2::new(0.0, 0.0),
            Point2::new(5.0, 5.0),
            Point2::new(5.0, 5.0),
            Point2::new(10.0, 0.0),
        ];

        let open = BSpline::interpolate_centripetal(&points).unwrap();
        let closed = BSpline::interpolate_closed_centripetal(&points).unwrap();

        assert!(open.control_points().iter().all(|point| point.is_finite()));
        assert!(
            closed
                .control_points()
                .iter()
                .all(|point| point.is_finite())
        );
    }
}
