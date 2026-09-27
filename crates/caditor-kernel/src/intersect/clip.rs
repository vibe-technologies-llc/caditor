use crate::interval::Interval;

const BISECTIONS: usize = 60;

fn boundary(outside: f64, inside: f64, test: &impl Fn(f64) -> bool) -> f64 {
    let (mut outside, mut inside) = (outside, inside);
    for _ in 0..BISECTIONS {
        let middle = 0.5 * (outside + inside);
        if middle == outside || middle == inside {
            break;
        }
        if test(middle) {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    inside
}

pub(crate) fn inside_intervals(
    range: Interval,
    samples: usize,
    test: impl Fn(f64) -> bool,
) -> Vec<Interval> {
    let parameters: Vec<f64> = range.split(samples.max(1)).collect();
    let flags: Vec<bool> = parameters
        .iter()
        .map(|parameter| test(*parameter))
        .collect();
    let mut intervals = Vec::new();
    let mut open: Option<f64> = None;
    let mut previous: Option<(f64, bool)> = None;
    for (parameter, inside) in parameters.iter().copied().zip(flags) {
        match (previous, inside) {
            (None, true) => open = Some(parameter),
            (Some((before, false)), true) => open = Some(boundary(before, parameter, &test)),
            (Some((before, true)), false) => {
                let end = boundary(parameter, before, &test);
                if let Some(start) = open.take()
                    && let Some(interval) = Interval::new(start, end)
                {
                    intervals.push(interval);
                }
            }
            _ => {}
        }
        previous = Some((parameter, inside));
    }
    if let Some(start) = open
        && let Some(interval) = Interval::new(start, range.end())
    {
        intervals.push(interval);
    }
    intervals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_runs_are_found_with_exact_ends() {
        let found = inside_intervals(Interval::new(0.0, 10.0).unwrap(), 20, |t| {
            (1.2..=3.7).contains(&t) || t >= 9.1
        });
        assert_eq!(found.len(), 2);
        assert!((found[0].start() - 1.2).abs() < 1e-12);
        assert!((found[0].end() - 3.7).abs() < 1e-12);
        assert!((found[1].start() - 9.1).abs() < 1e-12);
        assert_eq!(found[1].end(), 10.0);
    }
}
