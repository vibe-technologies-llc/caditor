use crate::{interval::Interval, tolerance::LINEAR_RESOLUTION};

const GAUSS_LEGENDRE: [(f64, f64); 8] = [
    (-0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
    (-0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (-0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (-0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
];
const MAX_REFINE_ITERATIONS: usize = 64;
const MAX_REFINED_CANDIDATES: usize = 4;
const PARAMETER_EPSILON: f64 = 1e-15;
const HINT_BRACKETS: f64 = 32.0;
const SQUARED_RESOLUTION: f64 = LINEAR_RESOLUTION * LINEAR_RESOLUTION;

pub(crate) fn integrate(breaks: &[f64], integrand: impl Fn(f64) -> f64) -> f64 {
    breaks
        .windows(2)
        .filter_map(|pair| match pair {
            [start, end] => Interval::new(*start, *end),
            _ => None,
        })
        .map(|piece| {
            let half = 0.5 * piece.length();
            let middle = piece.middle();
            GAUSS_LEGENDRE
                .iter()
                .map(|(node, weight)| weight * integrand(middle + half * node))
                .sum::<f64>()
                * half
        })
        .filter(|value| value.is_finite())
        .sum()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Taylor {
    pub value: f64,
    pub slope: f64,
    pub curvature: f64,
}

pub(crate) fn minimize(samples: &[f64], objective: impl Fn(f64) -> Taylor) -> Option<f64> {
    let values: Vec<(f64, f64)> = samples
        .iter()
        .map(|parameter| (*parameter, objective(*parameter).value))
        .filter(|(parameter, value)| parameter.is_finite() && value.is_finite())
        .collect();
    let mut candidates: Vec<usize> = (0..values.len())
        .filter(|index| is_local_minimum(&values, *index))
        .collect();
    candidates.sort_by(|a, b| sample_value(&values, *a).total_cmp(&sample_value(&values, *b)));
    candidates.truncate(MAX_REFINED_CANDIDATES);
    let mut best = values.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1))?;
    for index in candidates {
        let Some(&(start, _)) = values.get(index) else {
            continue;
        };
        let low = index
            .checked_sub(1)
            .and_then(|previous| values.get(previous))
            .map_or(start, |sample| sample.0);
        let high = values.get(index + 1).map_or(start, |sample| sample.0);
        let refined = refine(low, high, start, &objective);
        if refined.1 < best.1 {
            best = refined;
        }
    }
    Some(best.0)
}

pub(crate) fn minimize_near(
    samples: &[f64],
    objective: impl Fn(f64) -> Taylor,
    range: Interval,
    hint: Option<f64>,
) -> Option<f64> {
    let global = minimize(samples, &objective);
    let Some(hint) = hint.filter(|hint| hint.is_finite()) else {
        return global;
    };
    let reach = range.length() / HINT_BRACKETS;
    let around = [
        range.clamp(hint - reach),
        range.clamp(hint),
        range.clamp(hint + reach),
    ];
    let local = minimize(&around, &objective);
    match (global, local) {
        (Some(global), Some(local)) => {
            let as_good = objective(local).value <= objective(global).value + SQUARED_RESOLUTION;
            Some(if as_good { local } else { global })
        }
        (global, local) => local.or(global),
    }
}

fn sample_value(values: &[(f64, f64)], index: usize) -> f64 {
    values.get(index).map_or(f64::INFINITY, |sample| sample.1)
}

fn is_local_minimum(values: &[(f64, f64)], index: usize) -> bool {
    let here = sample_value(values, index);
    let before = index
        .checked_sub(1)
        .map_or(f64::INFINITY, |previous| sample_value(values, previous));
    let after = sample_value(values, index + 1);
    here <= before && here <= after
}

fn refine(
    mut low: f64,
    mut high: f64,
    start: f64,
    objective: &impl Fn(f64) -> Taylor,
) -> (f64, f64) {
    let mut at = start;
    let mut current = objective(at);
    let mut best = (at, current.value);
    for _ in 0..MAX_REFINE_ITERATIONS {
        if !current.slope.is_finite() || current.slope == 0.0 {
            break;
        }
        if current.slope > 0.0 {
            high = at;
        } else {
            low = at;
        }
        let width = high - low;
        let resolution = PARAMETER_EPSILON * (1.0 + at.abs() + low.abs() + high.abs());
        if width <= resolution {
            break;
        }
        let newton = at - current.slope / current.curvature;
        let next = if current.curvature > 0.0 && newton > low && newton < high {
            newton
        } else {
            low + 0.5 * width
        };
        let step = (next - at).abs();
        at = next;
        current = objective(at);
        if current.value.is_finite() && current.value < best.1 {
            best = (at, current.value);
        }
        if step <= resolution {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    #[test]
    fn integrates_polynomials_exactly() {
        let value = integrate(&[0.0, 1.0, 3.0], |x| x.powi(7) - 2.0 * x);
        let exact = 3f64.powi(8) / 8.0 - 9.0;
        assert!((value - exact).abs() < 1e-9 * exact.abs());
        assert!((integrate(&[0.0, PI], f64::sin) - 2.0).abs() < 1e-12);
        assert_eq!(integrate(&[1.0], |x| x), 0.0);
    }

    #[test]
    fn minimizes_to_the_global_minimum_among_sampled_basins() {
        let objective = |x: f64| Taylor {
            value: (x * x - 1.0).powi(2) + 0.1 * x,
            slope: 4.0 * x * (x * x - 1.0) + 0.1,
            curvature: 12.0 * x * x - 4.0,
        };
        let samples: Vec<f64> = (0..=20).map(|index| -2.0 + 0.2 * index as f64).collect();
        let found = minimize(&samples, objective).unwrap();
        assert!(found < 0.0);
        assert!(objective(found).slope.abs() < 1e-9);
    }

    #[test]
    fn stops_at_a_boundary_minimum_and_ignores_non_finite_samples() {
        let objective = |x: f64| Taylor {
            value: x,
            slope: 1.0,
            curvature: 0.0,
        };
        assert_eq!(minimize(&[0.0, 0.5, 1.0], objective), Some(0.0));
        let broken = |_: f64| Taylor {
            value: f64::NAN,
            slope: f64::NAN,
            curvature: f64::NAN,
        };
        assert_eq!(minimize(&[0.0, 1.0], broken), None);
    }
}
