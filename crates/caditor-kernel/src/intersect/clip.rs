use crate::{
    interrupt::{self, Interrupted},
    interval::Interval,
};

const BISECTIONS: usize = 60;
const MAX_REFINEMENT_DEPTH: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathBound {
    Misses,
    Monotone,
    TurnsAtBound,
}

impl PathBound {
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Misses, _) | (_, Self::Misses) => Self::Misses,
            (Self::TurnsAtBound, _) | (_, Self::TurnsAtBound) => Self::TurnsAtBound,
            (Self::Monotone, Self::Monotone) => Self::Monotone,
        }
    }
}

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

fn runs(range: Interval, flags: &[(f64, bool)], test: impl Fn(f64) -> bool) -> Vec<Interval> {
    let mut intervals = Vec::new();
    let mut open: Option<f64> = None;
    let mut previous: Option<(f64, bool)> = None;
    for &(parameter, inside) in flags {
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

pub(crate) fn inside_intervals(
    range: Interval,
    samples: usize,
    seeds: &[f64],
    test: impl Fn(f64) -> bool,
) -> Vec<Interval> {
    let mut parameters: Vec<f64> = range.split(samples.max(1)).collect();
    parameters.extend(seeds.iter().copied().filter(|seed| range.contains(*seed)));
    parameters.sort_by(f64::total_cmp);
    parameters.dedup();
    let flags: Vec<(f64, bool)> = parameters
        .into_iter()
        .map(|parameter| (parameter, test(parameter)))
        .collect();
    runs(range, &flags, test)
}

pub(crate) fn guided_intervals<S: Copy>(
    range: Interval,
    samples: usize,
    probe: impl Fn(f64) -> (bool, S),
    guide: impl Fn([S; 3]) -> PathBound,
) -> Result<Vec<Interval>, Interrupted> {
    let mut probed: Vec<Probed<S>> = Vec::with_capacity(samples.max(1) + 1);
    for parameter in range.split(samples.max(1)) {
        interrupt::check()?;
        let (inside, sample) = probe(parameter);
        probed.push(Probed {
            parameter,
            inside,
            sample,
        });
    }
    let mut flags: Vec<(f64, bool)> = Vec::with_capacity(probed.len());
    for (index, &here) in probed.iter().enumerate() {
        if let Some(&before) = index.checked_sub(1).and_then(|at| probed.get(at)) {
            refine(before, here, 0, &probe, &guide, &mut flags)?;
        }
        flags.push((here.parameter, here.inside));
    }
    interrupt::check()?;
    Ok(runs(range, &flags, |parameter| probe(parameter).0))
}

#[derive(Debug, Clone, Copy)]
struct Probed<S> {
    parameter: f64,
    inside: bool,
    sample: S,
}

fn refine<S: Copy>(
    low: Probed<S>,
    high: Probed<S>,
    depth: usize,
    probe: &impl Fn(f64) -> (bool, S),
    guide: &impl Fn([S; 3]) -> PathBound,
    flags: &mut Vec<(f64, bool)>,
) -> Result<(), Interrupted> {
    if depth >= MAX_REFINEMENT_DEPTH || (low.inside && high.inside) {
        return Ok(());
    }
    let parameter = 0.5 * (low.parameter + high.parameter);
    if parameter <= low.parameter || parameter >= high.parameter {
        return Ok(());
    }
    interrupt::check()?;
    let (inside, sample) = probe(parameter);
    let middle = Probed {
        parameter,
        inside,
        sample,
    };
    let bound = guide([low.sample, sample, high.sample]);
    let settled = if low.inside || high.inside {
        bound != PathBound::TurnsAtBound
    } else {
        !inside && bound == PathBound::Misses
    };
    if !settled {
        refine(low, middle, depth + 1, probe, guide, flags)?;
    }
    flags.push((parameter, inside));
    if !settled {
        refine(middle, high, depth + 1, probe, guide, flags)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_runs_are_found_with_exact_ends() {
        let found = inside_intervals(Interval::new(0.0, 10.0).unwrap(), 20, &[], |t| {
            (1.2..=3.7).contains(&t) || t >= 9.1
        });
        assert_eq!(found.len(), 2);
        assert!((found[0].start() - 1.2).abs() < 1e-12);
        assert!((found[0].end() - 3.7).abs() < 1e-12);
        assert!((found[1].start() - 9.1).abs() < 1e-12);
        assert_eq!(found[1].end(), 10.0);
    }

    #[test]
    fn a_seed_finds_a_gap_between_samples() {
        let range = Interval::new(0.0, 10.0).unwrap();
        let outside_gap = |t: f64| !(5.01..=5.02).contains(&t);

        let unseeded = inside_intervals(range, 4, &[], outside_gap);
        let seeded = inside_intervals(range, 4, &[5.015], outside_gap);

        assert_eq!(unseeded.len(), 1);
        assert_eq!(seeded.len(), 2);
        assert!((seeded[0].end() - 5.01).abs() < 1e-12);
        assert!((seeded[1].start() - 5.02).abs() < 1e-12);
    }

    #[test]
    fn a_short_run_between_samples_is_found_where_the_guide_cannot_rule_it_out() {
        let range = Interval::new(0.0, 200.0).unwrap();
        let within = |t: f64| (33.0..=34.0).contains(&t);
        let probe = |t: f64| (within(t), t);
        let guide = |[low, _, high]: [f64; 3]| {
            if high < 33.0 || low > 34.0 {
                PathBound::Misses
            } else {
                PathBound::Monotone
            }
        };

        let found = guided_intervals(range, 32, probe, guide).unwrap();

        assert_eq!(found.len(), 1);
        assert!((found[0].start() - 33.0).abs() < 1e-9);
        assert!((found[0].end() - 34.0).abs() < 1e-9);
    }

    #[test]
    fn a_run_hidden_between_an_outside_and_an_inside_sample_is_found_where_the_path_turns() {
        let range = Interval::new(0.0, 8.0).unwrap();
        let within = |t: f64| (0.5..=0.9).contains(&t) || t >= 1.5;
        let probe = |t: f64| (within(t), t);
        let guide = |[low, _, _]: [f64; 3]| {
            if low < 1.5 {
                PathBound::TurnsAtBound
            } else {
                PathBound::Monotone
            }
        };

        let found = guided_intervals(range, 4, probe, guide).unwrap();

        assert_eq!(found.len(), 2, "{found:?}");
        assert!((found[0].start() - 0.5).abs() < 1e-9);
        assert!((found[0].end() - 0.9).abs() < 1e-9);
        assert!((found[1].start() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_guide_ruling_out_every_span_probes_only_the_samples() {
        let range = Interval::new(0.0, 1.0).unwrap();
        let probes = std::cell::Cell::new(0);
        let probe = |t: f64| {
            probes.set(probes.get() + 1);
            (false, t)
        };

        let found = guided_intervals(range, 8, probe, |_| PathBound::Misses).unwrap();

        assert!(found.is_empty());
        assert_eq!(probes.get(), 9 + 8);
    }
}
