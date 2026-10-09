use std::cell::Cell;

use crate::interval::Interval;

pub(crate) const COMPONENTS: usize = 11;
pub(crate) type Values = [f64; COMPONENTS];

const KRONROD_NODES: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_5,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_48,
    0.0,
];
const KRONROD_WEIGHTS: [f64; 8] = [
    0.022_935_322_010_529_224,
    0.063_092_092_629_978_56,
    0.104_790_010_322_250_19,
    0.140_653_259_715_525_92,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_42,
    0.204_432_940_075_298_89,
    0.209_482_141_084_727_82,
];
const GAUSS_WEIGHTS: [f64; 8] = [
    0.0,
    0.129_484_966_168_869_7,
    0.0,
    0.279_705_391_489_276_64,
    0.0,
    0.381_830_050_505_118_9,
    0.0,
    0.417_959_183_673_469_4,
];
const MAX_DEPTH: usize = 30;
const KRONROD_EVALUATIONS: usize = 15;
const MAX_STEPS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tolerance {
    pub(crate) relative: f64,
    pub(crate) scales: Values,
    pub(crate) floor: Values,
}

impl Tolerance {
    fn accepts(&self, error: &Values, magnitude: &Values, share: f64) -> bool {
        let reach = magnitude.first().copied().unwrap_or(0.0);
        error
            .iter()
            .zip(magnitude)
            .zip(self.scales.iter().zip(&self.floor))
            .all(|((error, magnitude), (scale, floor))| {
                *error <= (self.relative * magnitude.max(reach * scale)).max(floor * share)
            })
    }
}

#[derive(Debug)]
pub(crate) struct Budget(Cell<usize>);

impl Budget {
    pub(crate) fn new(evaluations: usize) -> Self {
        Self(Cell::new(evaluations))
    }

    fn spend(&self, evaluations: usize) -> bool {
        let left = self.0.get();
        self.0.set(left.saturating_sub(evaluations));
        left >= evaluations
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Quadrature {
    pub(crate) values: Values,
    pub(crate) converged: bool,
}

pub(crate) fn add(sum: &mut Values, values: &Values, weight: f64) {
    for (entry, value) in sum.iter_mut().zip(values) {
        *entry += weight * value;
    }
}

pub(crate) fn scaled(values: &Values, weight: f64) -> Values {
    values.map(|value| value * weight)
}

struct Estimate {
    kronrod: Values,
    error: Values,
    magnitude: Values,
}

fn estimate(piece: Interval, integrand: &mut impl FnMut(f64) -> Values) -> Estimate {
    let half = 0.5 * piece.length();
    let middle = piece.middle();
    let mut kronrod = [0.0; COMPONENTS];
    let mut gauss = [0.0; COMPONENTS];
    let mut magnitude = [0.0; COMPONENTS];
    for ((node, kronrod_weight), gauss_weight) in
        KRONROD_NODES.iter().zip(KRONROD_WEIGHTS).zip(GAUSS_WEIGHTS)
    {
        let mut take = |values: Values| {
            add(&mut kronrod, &values, kronrod_weight);
            add(&mut gauss, &values, gauss_weight);
            add(&mut magnitude, &values.map(f64::abs), kronrod_weight);
        };
        take(integrand(middle + half * node));
        if *node != 0.0 {
            take(integrand(middle - half * node));
        }
    }
    let error = std::array::from_fn(|index| {
        let kronrod = kronrod.get(index).copied().unwrap_or(0.0);
        let gauss = gauss.get(index).copied().unwrap_or(0.0);
        (kronrod - gauss).abs() * half
    });
    Estimate {
        kronrod: scaled(&kronrod, half),
        error,
        magnitude: scaled(&magnitude, half),
    }
}

pub(crate) fn integrate(
    pieces: impl IntoIterator<Item = Interval>,
    tolerance: &Tolerance,
    budget: &Budget,
    mut integrand: impl FnMut(f64) -> Values,
) -> Quadrature {
    let mut values = [0.0; COMPONENTS];
    let mut converged = true;
    let mut pending: Vec<(Interval, usize)> = pieces
        .into_iter()
        .filter(|piece| piece.length() > 0.0)
        .map(|piece| (piece, 0))
        .collect();
    let width: f64 = pending.iter().map(|(piece, _)| piece.length()).sum();
    while let Some((piece, depth)) = pending.pop() {
        if !budget.spend(KRONROD_EVALUATIONS) {
            return Quadrature {
                values,
                converged: false,
            };
        }
        let found = estimate(piece, &mut integrand);
        let finite = found.kronrod.iter().all(|value| value.is_finite());
        if !finite {
            return Quadrature {
                values,
                converged: false,
            };
        }
        let accepted = tolerance.accepts(&found.error, &found.magnitude, piece.length() / width);
        let halves = (
            Interval::new(piece.start(), piece.middle()),
            Interval::new(piece.middle(), piece.end()),
        );
        match halves {
            (Some(low), Some(high))
                if !accepted && depth < MAX_DEPTH && low.length() > 0.0 && high.length() > 0.0 =>
            {
                pending.push((low, depth + 1));
                pending.push((high, depth + 1));
            }
            _ => {
                converged &= accepted;
                add(&mut values, &found.kronrod, 1.0);
            }
        }
    }
    Quadrature { values, converged }
}

pub(crate) fn pieces(range: Interval, breaks: impl IntoIterator<Item = f64>) -> Vec<Interval> {
    let mut ends: Vec<f64> = breaks
        .into_iter()
        .filter(|parameter| *parameter > range.start() && *parameter < range.end())
        .collect();
    ends.sort_by(f64::total_cmp);
    ends.insert(0, range.start());
    ends.push(range.end());
    ends.windows(2)
        .filter_map(|pair| match pair {
            [start, end] => Interval::new(*start, *end),
            _ => None,
        })
        .collect()
}

pub(crate) fn steps(range: Interval, step: f64) -> impl Iterator<Item = f64> {
    let first = (range.start() / step).ceil();
    let last = (range.end() / step).floor();
    let count = if last >= first {
        (last - first) as usize + 1
    } else {
        0
    };
    (0..count.min(MAX_STEPS)).map(move |index| (first + index as f64) * step)
}
