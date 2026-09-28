pub(crate) type Homogeneous = [f64; 4];

const KNOT_MATCH: f64 = 1e-12;

pub(crate) fn expand_knots(
    multiplicities: &[i64],
    knots: &[f64],
    expected: usize,
) -> Option<Vec<f64>> {
    if multiplicities.len() != knots.len() {
        return None;
    }
    let counts = multiplicities
        .iter()
        .map(|count| usize::try_from(*count).ok().filter(|count| *count > 0))
        .collect::<Option<Vec<usize>>>()?;
    let total = counts
        .iter()
        .try_fold(0_usize, |total, count| total.checked_add(*count))?;
    if total != expected {
        return None;
    }
    let mut expanded = Vec::with_capacity(total);
    for (count, knot) in counts.into_iter().zip(knots) {
        expanded.extend(std::iter::repeat_n(*knot, count));
    }
    Some(expanded)
}

pub(crate) fn knot_count(points: usize, degree: usize) -> Option<usize> {
    points.checked_add(degree)?.checked_add(1)
}

pub(crate) fn bezier_knots(count: usize, degree: usize) -> Vec<f64> {
    let whole_segments = degree > 0 && count > degree && (count - 1).is_multiple_of(degree);
    if !whole_segments {
        return uniform_knots(count, degree, true);
    }
    let segments = (count - 1) / degree;
    std::iter::repeat_n(0.0, degree + 1)
        .chain((1..segments).flat_map(|segment| std::iter::repeat_n(segment as f64, degree)))
        .chain(std::iter::repeat_n(segments as f64, degree + 1))
        .collect()
}

pub(crate) fn uniform_knots(count: usize, degree: usize, clamped: bool) -> Vec<f64> {
    if clamped {
        let spans = count.saturating_sub(degree).max(1);
        std::iter::repeat_n(0.0, degree)
            .chain((0..=spans).map(|index| index as f64))
            .chain(std::iter::repeat_n(spans as f64, degree))
            .collect()
    } else {
        (0..count + degree + 1)
            .map(|index| index as f64 - degree as f64)
            .collect()
    }
}

pub(crate) fn clamp(
    degree: usize,
    knots: &[f64],
    points: &[Homogeneous],
) -> Option<(Vec<f64>, Vec<Homogeneous>)> {
    let count = points.len();
    if knots.len() != count + degree + 1 || count <= degree {
        return None;
    }
    let start = *knots.get(degree)?;
    let end = *knots.get(count)?;
    if end <= start {
        return None;
    }
    let mut knots = knots.to_vec();
    let mut points = points.to_vec();
    while copies(&knots, start) < degree {
        insert(degree, &mut knots, &mut points, start)?;
    }
    let last_start = knots.iter().rposition(|knot| same(*knot, start))?;
    let dropped = last_start.checked_sub(degree)?;
    knots.drain(..dropped);
    points.drain(..dropped);
    knots
        .iter_mut()
        .take(degree + 1)
        .for_each(|knot| *knot = start);
    while copies(&knots, end) < degree {
        insert(degree, &mut knots, &mut points, end)?;
    }
    let first_end = knots.iter().position(|knot| same(*knot, end))?;
    points.truncate(first_end);
    knots.truncate(first_end + degree + 1);
    knots
        .iter_mut()
        .skip(first_end)
        .for_each(|knot| *knot = end);
    (knots.len() == points.len() + degree + 1 && points.len() > degree).then_some((knots, points))
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= KNOT_MATCH * (1.0 + a.abs().max(b.abs()))
}

fn copies(knots: &[f64], value: f64) -> usize {
    knots.iter().filter(|knot| same(**knot, value)).count()
}

fn insert(
    degree: usize,
    knots: &mut Vec<f64>,
    points: &mut Vec<Homogeneous>,
    value: f64,
) -> Option<()> {
    let span = knots
        .iter()
        .rposition(|knot| *knot <= value || same(*knot, value))?
        .min(points.len().checked_sub(1)?);
    let mut inserted = Vec::with_capacity(points.len() + 1);
    for index in 0..=points.len() {
        let point = if index + degree <= span {
            *points.get(index)?
        } else if index > span {
            *points.get(index - 1)?
        } else {
            let low = *knots.get(index)?;
            let high = *knots.get(index + degree)?;
            let alpha = if high > low {
                (value - low) / (high - low)
            } else {
                0.0
            };
            let (current, previous) = (*points.get(index)?, *points.get(index.checked_sub(1)?)?);
            std::array::from_fn(|axis| {
                alpha * current.get(axis).copied().unwrap_or(0.0)
                    + (1.0 - alpha) * previous.get(axis).copied().unwrap_or(0.0)
            })
        };
        inserted.push(point);
    }
    knots.insert(span + 1, value);
    *points = inserted;
    Some(())
}

#[cfg(test)]
pub(crate) fn evaluate(
    degree: usize,
    knots: &[f64],
    points: &[Homogeneous],
    value: f64,
) -> Option<Homogeneous> {
    let span = (degree..points.len()).rev().find(|span| {
        match (knots.get(*span), knots.get(span + 1)) {
            (Some(low), Some(high)) => *low <= value && low < high,
            _ => false,
        }
    })?;
    let first = span.checked_sub(degree)?;
    let mut local: Vec<Homogeneous> = points.get(first..=span)?.to_vec();
    for level in 1..=degree {
        for index in (level..=degree).rev() {
            let low = *knots.get(first + index)?;
            let high = *knots.get(first + index + degree + 1 - level)?;
            let alpha = if high > low {
                (value - low) / (high - low)
            } else {
                0.0
            };
            let previous = *local.get(index - 1)?;
            let current = local.get_mut(index)?;
            for (slot, before) in current.iter_mut().zip(previous) {
                *slot = before + (*slot - before) * alpha;
            }
        }
    }
    local.get(degree).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Homogeneous {
        [x, y, 0.0, 1.0]
    }

    #[test]
    fn clamping_keeps_the_curve() {
        let degree = 3;
        let points = vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 3.0),
            point(5.0, 1.0),
            point(6.0, -1.0),
            point(8.0, 0.0),
        ];
        let knots = uniform_knots(points.len(), degree, false);
        let (clamped_knots, clamped_points) = clamp(degree, &knots, &points).unwrap();
        let (start, end) = (knots[degree], knots[points.len()]);
        assert_eq!(&clamped_knots[..=degree], &[start; 4]);
        assert_eq!(
            &clamped_knots[clamped_knots.len() - degree - 1..],
            &[end; 4]
        );
        for step in 0..=20 {
            let value = start + (end - start) * step as f64 / 20.0;
            let before = evaluate(degree, &knots, &points, value).unwrap();
            let after = evaluate(degree, &clamped_knots, &clamped_points, value).unwrap();
            for axis in 0..4 {
                assert!(
                    (before[axis] - after[axis]).abs() < 1e-12,
                    "{before:?} {after:?}"
                );
            }
        }
        let already = uniform_knots(points.len(), degree, true);
        let (same_knots, same_points) = clamp(degree, &already, &points).unwrap();
        assert_eq!(same_knots, already);
        assert_eq!(same_points, points);
    }

    #[test]
    fn knots_expand_from_runs() {
        assert_eq!(
            expand_knots(&[4, 1, 4], &[0.0, 0.5, 1.0], 9),
            Some(vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(expand_knots(&[4, 1, 4], &[0.0, 0.5, 1.0], 8), None);
        assert_eq!(expand_knots(&[1], &[0.0, 1.0], 1), None);
        assert_eq!(expand_knots(&[0], &[0.0], 0), None);
        assert_eq!(
            expand_knots(&[4_000_000_000_000_000_000, 4], &[0.0, 1.0], 8),
            None
        );
        assert_eq!(
            expand_knots(&[i64::MAX, i64::MAX, i64::MAX], &[0.0, 0.5, 1.0], 8),
            None
        );
    }
}
