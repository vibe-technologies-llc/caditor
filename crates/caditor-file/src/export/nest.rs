use std::f64::consts::FRAC_PI_2;

use caditor_document::CancelToken;
use caditor_geometry::{Point2, Vector2};

use super::{
    ExportError,
    figure::{Figure, Motion},
    sheet::Nesting,
};

const TURN_STEPS: usize = 24;
const STEPS_PER_QUARTER: usize = TURN_STEPS / 4;
const CELLS_ACROSS: f64 = 400.0;
const MAX_CELLS_ALONG: f64 = 1000.0;
const MAX_SPACING_CELLS: f64 = 64.0;

type Run = (i64, i64);

#[derive(Debug, Clone, Copy)]
pub(super) struct Item<'a> {
    pub(super) figure: &'a Figure,
    pub(super) label: f64,
    pub(super) band: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Nested {
    pub(super) motions: Vec<Motion>,
    pub(super) too_wide: usize,
}

#[derive(Debug, Clone)]
struct Mask {
    turn: f64,
    low: Point2,
    width: i64,
    rows: Vec<Vec<Run>>,
    grown: Vec<Vec<Run>>,
    longest: Vec<i64>,
    margin: i64,
}

impl Mask {
    fn height(&self) -> i64 {
        i64::try_from(self.rows.len()).unwrap_or(i64::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Spot {
    x: i64,
    y: i64,
}

pub(super) fn nest(
    items: &[Item<'_>],
    nesting: &Nesting,
    cancel: &CancelToken,
) -> Result<Nested, ExportError> {
    let cell = cell_size(items, nesting);
    let margin = (nesting.spacing() / cell).ceil() as i64;
    let across = (nesting.width() / cell).floor() as i64;
    let mut masks = Vec::with_capacity(items.len());
    for item in items {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        masks.push(orientations(item, nesting.turns(), cell, margin));
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    let area = |index: &usize| {
        masks
            .get(*index)
            .and_then(|masks: &Vec<Mask>| masks.first())
            .map_or(0, |mask| mask.width.saturating_mul(mask.height()))
    };
    order.sort_by_key(|index| std::cmp::Reverse(area(index)));
    let mut sheet = Sheet {
        rows: Vec::new(),
        widest: Vec::new(),
        left: -margin,
        right: across.saturating_add(margin).saturating_add(1),
    };
    let mut motions = vec![Motion::shift(Vector2::ZERO); items.len()];
    let mut too_wide = 0;
    for index in order {
        let Some(choices) = masks.get(index) else {
            continue;
        };
        let mut best: Option<(Spot, &Mask)> = None;
        for mask in choices {
            if cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let Some(spot) = sheet.lowest_spot(mask, across) else {
                continue;
            };
            let better = best.is_none_or(|(chosen, chosen_mask)| {
                (spot.y, spot.x, spot.y + mask.height())
                    < (chosen.y, chosen.x, chosen.y + chosen_mask.height())
            });
            if better {
                best = Some((spot, mask));
            }
        }
        let placed = match best {
            Some(found) => Some(found),
            None => {
                too_wide += 1;
                choices
                    .iter()
                    .min_by_key(|mask| mask.width)
                    .map(|mask| (sheet.above(mask), mask))
            }
        };
        let Some((spot, mask)) = placed else {
            continue;
        };
        sheet.occupy(mask, spot);
        if let Some(motion) = motions.get_mut(index) {
            let corner = Vector2::new(spot.x as f64, spot.y as f64) * cell;
            *motion = Motion {
                turn: mask.turn,
                offset: corner - mask.low,
            };
        }
    }
    Ok(Nested { motions, too_wide })
}

fn cell_size(items: &[Item<'_>], nesting: &Nesting) -> f64 {
    let sizes: Vec<Vector2> = items
        .iter()
        .filter_map(|item| item.figure.bounds())
        .map(|bounds| bounds.size())
        .collect();
    let largest = sizes
        .iter()
        .map(|size| size.max_element())
        .fold(0.0, f64::max);
    let area: f64 = sizes.iter().map(|size| size.x * size.y).sum();
    let reference = nesting.width().min(largest.max(area.sqrt()));
    let cell = (reference / CELLS_ACROSS)
        .max(largest / MAX_CELLS_ALONG)
        .max(nesting.spacing() / MAX_SPACING_CELLS);
    if cell.is_finite() && cell > 0.0 {
        cell
    } else {
        nesting.width() / CELLS_ACROSS
    }
}

fn orientations(item: &Item<'_>, turns: bool, cell: f64, margin: i64) -> Vec<Mask> {
    let mut steps = vec![0];
    if turns {
        let boxed = (0..TURN_STEPS)
            .filter_map(|step| {
                let bounds = turned(item.figure, step).bounds()?;
                let size = bounds.size();
                Some((step, size.x * size.y))
            })
            .min_by(|first, second| first.1.total_cmp(&second.1))
            .map_or(0, |(step, _)| step % STEPS_PER_QUARTER);
        for start in [0, boxed] {
            for quarter in 0..4 {
                let step = start + quarter * STEPS_PER_QUARTER;
                if !steps.contains(&step) {
                    steps.push(step);
                }
            }
        }
    }
    steps
        .into_iter()
        .filter_map(|step| mask(item, step, cell, margin))
        .collect()
}

fn turn_of(step: usize) -> f64 {
    let quarters = (step / STEPS_PER_QUARTER) as f64;
    let rest = (step % STEPS_PER_QUARTER) as f64;
    quarters * FRAC_PI_2 + rest * FRAC_PI_2 / STEPS_PER_QUARTER as f64
}

fn turned(figure: &Figure, step: usize) -> Figure {
    let mut turned = Figure::default();
    turned.append_moved(figure.clone(), Motion::turn(turn_of(step)));
    turned
}

fn mask(item: &Item<'_>, step: usize, cell: f64, margin: i64) -> Option<Mask> {
    let figure = turned(item.figure, step);
    let bounds = figure.bounds()?;
    let low = Point2::new(bounds.min().x, bounds.min().y - item.band);
    let mut lines: Vec<Vec<Point2>> = figure
        .shapes
        .iter()
        .flat_map(|(_, shape)| shape.traced())
        .collect();
    if item.band > 0.0 {
        let (left, top) = (bounds.min().x, bounds.min().y);
        let right = left + item.label.max(0.0);
        lines.push(vec![
            low,
            Point2::new(right, low.y),
            Point2::new(right, top),
            Point2::new(left, top),
            low,
        ]);
    }
    let high = Point2::new(bounds.max().x.max(low.x + item.label), bounds.max().y);
    let cells = ((high - low) / cell).ceil();
    let width = (cells.x as i64).max(1);
    let height = usize::try_from((cells.y as i64).max(1)).ok()?;
    let rows = raster(&lines, low, cell, height);
    let grown = grown(&rows, margin);
    let longest = grown
        .iter()
        .map(|runs| {
            runs.iter()
                .map(|(start, end)| end - start)
                .max()
                .unwrap_or(0)
        })
        .collect();
    Some(Mask {
        turn: turn_of(step),
        low,
        width,
        rows,
        grown,
        longest,
        margin,
    })
}

fn raster(lines: &[Vec<Point2>], low: Point2, cell: f64, height: usize) -> Vec<Vec<Run>> {
    let mut covered: Vec<Vec<Run>> = vec![Vec::new(); height];
    let mut crossings: Vec<Vec<f64>> = vec![Vec::new(); height];
    let last_row = height as f64 - 1.0;
    let local = |point: &Point2| (*point - low) / cell;
    for line in lines {
        let points: Vec<Vector2> = line.iter().map(local).collect();
        if let [point] = points.as_slice() {
            let row = point.y.floor().clamp(0.0, last_row) as usize;
            if let Some(runs) = covered.get_mut(row) {
                let column = point.x.floor() as i64;
                runs.push((column, column + 1));
            }
        }
        for pair in points.windows(2) {
            let &[from, to] = pair else {
                continue;
            };
            cover(&mut covered, from, to, last_row);
            cross(&mut crossings, from, to, last_row);
        }
    }
    covered
        .into_iter()
        .zip(crossings)
        .map(|(mut runs, mut crossings)| {
            crossings.sort_by(f64::total_cmp);
            for [enter, leave] in crossings.as_chunks::<2>().0 {
                runs.push((enter.floor() as i64, leave.floor() as i64 + 1));
            }
            merged(runs)
        })
        .collect()
}

fn cover(covered: &mut [Vec<Run>], from: Vector2, to: Vector2, last_row: f64) {
    let first = from.y.min(to.y).floor().clamp(0.0, last_row) as usize;
    let last = from.y.max(to.y).floor().clamp(0.0, last_row) as usize;
    let rise = to.y - from.y;
    for row in first..=last {
        let (bottom, top) = (row as f64, row as f64 + 1.0);
        let (start, end) = if rise == 0.0 {
            (0.0, 1.0)
        } else {
            let at = |y: f64| ((y - from.y) / rise).clamp(0.0, 1.0);
            let (a, b) = (at(bottom), at(top));
            (a.min(b), a.max(b))
        };
        let x = |t: f64| from.x + (to.x - from.x) * t;
        let (left, right) = (x(start).min(x(end)), x(start).max(x(end)));
        if let Some(runs) = covered.get_mut(row) {
            runs.push((left.floor() as i64, right.floor() as i64 + 1));
        }
    }
}

fn cross(crossings: &mut [Vec<f64>], from: Vector2, to: Vector2, last_row: f64) {
    let (lower, upper) = if from.y <= to.y {
        (from, to)
    } else {
        (to, from)
    };
    let first = (lower.y - 0.5).ceil().clamp(0.0, last_row + 1.0) as usize;
    let last = (upper.y - 0.5).ceil().clamp(0.0, last_row + 1.0) as usize;
    for row in first..last {
        let middle = row as f64 + 0.5;
        let x = lower.x + (upper.x - lower.x) * (middle - lower.y) / (upper.y - lower.y);
        if let Some(row) = crossings.get_mut(row) {
            row.push(x);
        }
    }
}

fn merged(mut runs: Vec<Run>) -> Vec<Run> {
    runs.sort_unstable();
    let mut joined: Vec<Run> = Vec::with_capacity(runs.len());
    for (start, end) in runs {
        match joined.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => joined.push((start, end)),
        }
    }
    joined
}

fn grown(rows: &[Vec<Run>], margin: i64) -> Vec<Vec<Run>> {
    let padding = usize::try_from(margin).unwrap_or(0);
    let empty = Vec::new();
    let mut grown: Vec<Vec<Run>> = (0..rows.len() + 2 * padding)
        .map(|index| {
            let row = index
                .checked_sub(padding)
                .and_then(|index| rows.get(index))
                .unwrap_or(&empty);
            row.iter()
                .map(|(start, end)| (start - margin, end + margin))
                .collect()
        })
        .collect();
    let mut left = padding;
    let mut reach = 1;
    while left > 0 {
        let step = reach.min(left);
        grown = (0..grown.len())
            .map(|index| {
                let mut runs: Vec<Run> = Vec::new();
                for neighbour in [
                    index.checked_sub(step),
                    Some(index),
                    index.checked_add(step),
                ]
                .into_iter()
                .flatten()
                {
                    runs.extend(grown.get(neighbour).into_iter().flatten());
                }
                merged(runs)
            })
            .collect();
        left -= step;
        reach *= 2;
    }
    grown
}

#[derive(Debug)]
struct Sheet {
    rows: Vec<Vec<Run>>,
    widest: Vec<i64>,
    left: i64,
    right: i64,
}

impl Sheet {
    fn top(&self) -> i64 {
        i64::try_from(self.rows.len()).unwrap_or(i64::MAX)
    }

    fn lowest_spot(&self, mask: &Mask, across: i64) -> Option<Spot> {
        let last_x = across.checked_sub(mask.width).filter(|last| *last >= 0)?;
        let mut blocked = Vec::new();
        (0..=self.top().saturating_add(mask.margin)).find_map(|y| {
            self.leftmost(mask, y, last_x, &mut blocked)
                .map(|x| Spot { x, y })
        })
    }

    fn leftmost(&self, mask: &Mask, y: i64, last_x: i64, blocked: &mut Vec<Run>) -> Option<i64> {
        blocked.clear();
        let first_row = y - mask.margin;
        let room = mask.grown.iter().zip(&mask.longest).enumerate();
        for (index, (_, longest)) in room {
            let row = first_row + i64::try_from(index).ok()?;
            let Ok(row) = usize::try_from(row) else {
                continue;
            };
            match self.widest.get(row) {
                Some(widest) if widest < longest => return None,
                Some(_) => {}
                None => break,
            }
        }
        for (index, runs) in mask.grown.iter().enumerate() {
            let row = first_row + i64::try_from(index).ok()?;
            let Ok(row) = usize::try_from(row) else {
                continue;
            };
            let Some(taken) = self.rows.get(row) else {
                break;
            };
            for (start, end) in taken {
                for (from, to) in runs {
                    blocked.push((start - to + 1, end - from));
                }
            }
        }
        blocked.sort_unstable();
        let mut x = 0;
        for (start, end) in blocked.iter() {
            if *start > x {
                break;
            }
            x = x.max(*end);
        }
        (x <= last_x).then_some(x)
    }

    fn above(&self, mask: &Mask) -> Spot {
        let y = if self.rows.is_empty() {
            0
        } else {
            self.top().saturating_add(mask.margin)
        };
        Spot { x: 0, y }
    }

    fn occupy(&mut self, mask: &Mask, spot: Spot) {
        for (index, runs) in mask.rows.iter().enumerate() {
            let Some(row) = usize::try_from(spot.y)
                .ok()
                .and_then(|y| y.checked_add(index))
            else {
                continue;
            };
            if self.rows.len() <= row {
                self.rows.resize(row + 1, Vec::new());
            }
            if let Some(taken) = self.rows.get_mut(row) {
                taken.extend(
                    runs.iter()
                        .map(|(start, end)| (start + spot.x, end + spot.x)),
                );
                *taken = merged(std::mem::take(taken));
                let widest = widest_gap(taken, self.left, self.right);
                if self.widest.len() <= row {
                    self.widest.resize(row + 1, self.right - self.left);
                }
                if let Some(slot) = self.widest.get_mut(row) {
                    *slot = widest;
                }
            }
        }
    }
}

fn widest_gap(runs: &[Run], left: i64, right: i64) -> i64 {
    let mut widest = 0;
    let mut free_from = left;
    for (start, end) in runs {
        widest = widest.max(start.min(&right) - free_from);
        free_from = free_from.max(*end);
    }
    widest.max(right - free_from)
}
