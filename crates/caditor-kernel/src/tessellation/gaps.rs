use std::f64::consts::SQRT_2;

use spade::Point2 as PlanePoint;

use crate::{
    interrupt::{self, Interrupted},
    tessellation::{POLL_EVERY, constrained::Outline},
};

const CELLS_PER_POINT: f64 = 1.0;
const MARKS_PER_CELL: f64 = 2.0;
const CELL_ERROR: f64 = SQRT_2;

struct Grid {
    low: PlanePoint<f64>,
    side: f64,
    per_side: f64,
    columns: usize,
    rows: usize,
}

impl Grid {
    fn over(mapped: &[PlanePoint<f64>]) -> Option<Self> {
        let (low, high) = mapped.iter().fold(
            (
                PlanePoint::new(f64::INFINITY, f64::INFINITY),
                PlanePoint::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
            ),
            |(low, high), at| {
                (
                    PlanePoint::new(low.x.min(at.x), low.y.min(at.y)),
                    PlanePoint::new(high.x.max(at.x), high.y.max(at.y)),
                )
            },
        );
        let (width, height) = (high.x - low.x, high.y - low.y);
        let wanted = CELLS_PER_POINT * mapped.len() as f64;
        let side = (width * height / wanted)
            .sqrt()
            .max(width.max(height) / wanted);
        (side.is_finite() && side > 0.0).then(|| Self {
            low,
            side,
            per_side: side.recip(),
            columns: (width / side) as usize + 1,
            rows: (height / side) as usize + 1,
        })
    }

    fn column(&self, x: f64) -> usize {
        (((x - self.low.x) * self.per_side) as usize).min(self.columns.saturating_sub(1))
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.low.y) * self.per_side) as usize).min(self.rows.saturating_sub(1))
    }

    fn width(&self) -> usize {
        self.columns + 2
    }

    fn cells(&self) -> usize {
        self.width() * (self.rows + 2)
    }

    fn cell(&self, column: usize, row: usize) -> usize {
        (row + 1) * self.width() + column + 1
    }

    fn centre_x(&self, column: usize) -> f64 {
        self.low.x + (column as f64 + 0.5) * self.side
    }

    fn centre_y(&self, row: usize) -> f64 {
        self.low.y + (row as f64 + 0.5) * self.side
    }

    fn centres_below(&self, y: f64) -> usize {
        let mut count = (((y - self.low.y) * self.per_side + 0.5) as usize).min(self.rows);
        if count > 0 && self.centre_y(count - 1) >= y {
            count -= 1;
        } else if count < self.rows && self.centre_y(count) < y {
            count += 1;
        }
        count
    }

    fn mark(&self, distance: &mut [f64], at: PlanePoint<f64>) {
        if let Some(slot) = distance.get_mut(self.cell(self.column(at.x), self.row(at.y))) {
            *slot = 0.0;
        }
    }
}

pub(super) fn widest(
    mapped: &[PlanePoint<f64>],
    outline: &Outline,
) -> Result<Option<f64>, Interrupted> {
    let Some(grid) = Grid::over(mapped) else {
        return Ok(None);
    };
    let mut distance = vec![f64::INFINITY; grid.cells()];
    let mut below = Vec::with_capacity(mapped.len());
    for (step, at) in mapped.iter().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        grid.mark(&mut distance, *at);
        below.push(grid.centres_below(at.y));
    }
    let crossings = boundary(&grid, mapped, &below, outline, &mut distance)?;
    let inside = inside(&grid, crossings);
    chamfer(&grid, &mut distance)?;
    let widest = distance
        .iter()
        .zip(&inside)
        .filter(|(_, inside)| **inside)
        .map(|(cells, _)| *cells)
        .fold(0.0, f64::max);
    let gap = (widest + CELL_ERROR) * grid.side;
    Ok(gap.is_finite().then_some(gap))
}

fn boundary(
    grid: &Grid,
    mapped: &[PlanePoint<f64>],
    below: &[usize],
    outline: &Outline,
    distance: &mut [f64],
) -> Result<Vec<(usize, f64)>, Interrupted> {
    let mut crossings = Vec::new();
    for (step, (from, to)) in outline.segments.iter().enumerate() {
        if step.is_multiple_of(POLL_EVERY) {
            interrupt::check()?;
        }
        let (Some(a), Some(b), Some(under_a), Some(under_b)) = (
            mapped.get(*from),
            mapped.get(*to),
            below.get(*from),
            below.get(*to),
        ) else {
            continue;
        };
        let (across, along) = ((b.x - a.x).abs(), (b.y - a.y).abs());
        if across.max(along) * MARKS_PER_CELL > grid.side {
            let marks = ((across + along) * MARKS_PER_CELL * grid.per_side).ceil() as usize;
            for mark in 1..marks {
                let share = mark as f64 / marks as f64;
                grid.mark(
                    distance,
                    PlanePoint::new(a.x + (b.x - a.x) * share, a.y + (b.y - a.y) * share),
                );
            }
        }
        for row in *under_a.min(under_b)..*under_a.max(under_b) {
            let y = grid.centre_y(row);
            crossings.push((row, a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y)));
        }
    }
    Ok(crossings)
}

fn inside(grid: &Grid, mut crossings: Vec<(usize, f64)>) -> Vec<bool> {
    crossings.sort_unstable_by(|one, other| one.0.cmp(&other.0).then(one.1.total_cmp(&other.1)));
    let mut inside = vec![false; grid.cells()];
    for row in crossings.chunk_by(|one, other| one.0 == other.0) {
        for [(row, from), (_, to)] in row.as_chunks::<2>().0 {
            for column in grid.column(*from)..=grid.column(*to) {
                let x = grid.centre_x(column);
                if *from < x
                    && x < *to
                    && let Some(slot) = inside.get_mut(grid.cell(column, *row))
                {
                    *slot = true;
                }
            }
        }
    }
    inside
}

fn chamfer(grid: &Grid, distance: &mut [f64]) -> Result<(), Interrupted> {
    let width = grid.width();
    for row in 1..=grid.rows {
        interrupt::check()?;
        let (before, after) = distance.split_at_mut(row * width);
        if let (Some(previous), Some(current)) = (
            before.get(before.len().saturating_sub(width)..),
            after.get_mut(1..width.saturating_sub(1)),
        ) {
            relax_across(current.iter_mut(), previous.windows(3));
            relax_along(current.iter_mut());
        }
    }
    for row in (1..=grid.rows).rev() {
        interrupt::check()?;
        let (current, next) = distance.split_at_mut((row + 1) * width);
        if let (Some(next), Some(current)) = (
            next.get(..width),
            current.get_mut(row * width + 1..(row + 1) * width - 1),
        ) {
            relax_across(current.iter_mut(), next.windows(3));
            relax_along(current.iter_mut().rev());
        }
    }
    Ok(())
}

fn relax_across<'a>(
    cells: impl Iterator<Item = &'a mut f64>,
    beside: impl Iterator<Item = &'a [f64]>,
) {
    for (cell, near) in cells.zip(beside) {
        if let [corner, across, other_corner] = near {
            *cell = cell
                .min(across + 1.0)
                .min(corner.min(*other_corner) + SQRT_2);
        }
    }
}

fn relax_along<'a>(cells: impl Iterator<Item = &'a mut f64>) {
    let mut previous = f64::INFINITY;
    for cell in cells {
        *cell = cell.min(previous + 1.0);
        previous = *cell;
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;
    use crate::tessellation::constrained;

    fn ring(centre: PlanePoint<f64>, radius: f64, samples: usize) -> Vec<PlanePoint<f64>> {
        (0..samples)
            .map(|step| {
                let angle = TAU * step as f64 / samples as f64;
                PlanePoint::new(
                    centre.x + radius * angle.cos(),
                    centre.y + radius * angle.sin(),
                )
            })
            .collect()
    }

    fn lattice(within: impl Fn(PlanePoint<f64>) -> bool, extent: i32) -> Vec<PlanePoint<f64>> {
        (-extent..=extent)
            .flat_map(|x| {
                (-extent..=extent).map(move |y| PlanePoint::new(f64::from(x), f64::from(y)))
            })
            .filter(|at| within(*at))
            .collect()
    }

    #[test]
    fn the_widest_gap_is_the_largest_empty_disc_within_the_face_give_or_take_a_cell() {
        let mut mapped = ring(PlanePoint::new(0.0, 0.0), 10.0, 400);
        let outer: Vec<usize> = (0..mapped.len()).collect();
        mapped.extend(lattice(|at| (2.5..9.0).contains(&at.x.hypot(at.y)), 9));
        let outline = constrained::outline(&mapped, &[outer]);
        let side = Grid::over(&mapped).unwrap().side;
        let empty_radius = 2.0_f64.hypot(2.0);

        let gap = widest(&mapped, &outline).unwrap().unwrap();

        assert!(gap >= empty_radius, "{gap}");
        assert!(
            gap <= empty_radius * 1.1 + 2.0 * CELL_ERROR * side,
            "{gap} at cells of {side}"
        );
    }

    #[test]
    fn the_widest_gap_ignores_what_lies_outside_the_face_and_inside_its_holes() {
        let mut notched = vec![
            PlanePoint::new(-20.0, -20.0),
            PlanePoint::new(20.0, -20.0),
            PlanePoint::new(20.0, -16.0),
            PlanePoint::new(-16.0, -16.0),
            PlanePoint::new(-16.0, 20.0),
            PlanePoint::new(-20.0, 20.0),
        ];
        let notched_loop: Vec<usize> = (0..notched.len()).collect();
        notched.extend(lattice(|at| at.x.min(at.y) < -16.0, 20));
        let mut holed = ring(PlanePoint::new(0.0, 0.0), 20.0, 200);
        let outer: Vec<usize> = (0..holed.len()).collect();
        let first = holed.len();
        holed.extend(ring(PlanePoint::new(0.0, 0.0), 12.0, 120).into_iter().rev());
        let inner: Vec<usize> = (first..holed.len()).collect();
        holed.extend(lattice(|at| (12.5..19.5).contains(&at.x.hypot(at.y)), 20));

        let notched_gap = widest(&notched, &constrained::outline(&notched, &[notched_loop]));
        let holed_gap = widest(&holed, &constrained::outline(&holed, &[outer, inner]));

        assert!(notched_gap.unwrap().unwrap() < 4.0, "{notched_gap:?}");
        assert!(holed_gap.unwrap().unwrap() < 4.0, "{holed_gap:?}");
    }
}
