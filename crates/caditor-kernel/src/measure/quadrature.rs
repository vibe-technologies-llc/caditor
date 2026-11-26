use std::{cell::Cell, f64::consts::PI};

use caditor_geometry::{Maths, Vector3};

use crate::{interval::Interval, tally};

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
pub(crate) const KRONROD_EVALUATIONS: usize = 15;
const MAX_STEPS: usize = 4096;
const SLIVER: f64 = 1e-9;
const NEWTON_STEPS: usize = 64;
const NODE_STEP: f64 = 1e-16;
pub(crate) const QUARTIC_RULE: [(f64, f64); 3] = [
    (-0.774_596_669_241_483_4, 0.555_555_555_555_555_6),
    (0.0, 0.888_888_888_888_888_8),
    (0.774_596_669_241_483_4, 0.555_555_555_555_555_6),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tolerance {
    pub(crate) relative: f64,
    pub(crate) scales: Values,
    pub(crate) floor: Values,
    pub(crate) noise: f64,
}

impl Tolerance {
    pub(crate) fn shared(&self, share: f64) -> Self {
        Self {
            floor: scaled(&self.floor, share),
            ..*self
        }
    }

    fn accepts(&self, error: &Values, magnitude: &Values, share: f64, noise: &Values) -> bool {
        let reach = magnitude.first().copied().unwrap_or(0.0);
        error
            .iter()
            .zip(magnitude)
            .zip(self.scales.iter().zip(&self.floor))
            .zip(noise)
            .all(|(((error, magnitude), (scale, floor)), noise)| {
                let allowed = (self.relative * magnitude.max(reach * scale))
                    .max(floor * share)
                    .max(self.noise * noise);
                *error <= allowed
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

        tally::add(evaluations);

        self.0.set(left.saturating_sub(evaluations));
        left >= evaluations
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
    let mut peak_density: Values = [0.0; COMPONENTS];

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
        for (peak, magnitude) in peak_density.iter_mut().zip(&found.magnitude) {
            *peak = peak.max(magnitude / piece.length());
        }

        let noise = peak_density.map(|density| density * piece.length());
        let accepted = tolerance.accepts(
            &found.error,
            &found.magnitude,
            piece.length() / width,
            &noise,
        );
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

pub(crate) fn integrate_pieces(
    pieces: impl IntoIterator<Item = Interval>,
    evaluations: usize,
    budget: &Budget,
    mut piece_integral: impl FnMut(Interval) -> Quadrature,
) -> Quadrature {
    let mut values = [0.0; COMPONENTS];
    let mut converged = true;

    for piece in pieces {
        if !budget.spend(evaluations) {
            return Quadrature {
                values,
                converged: false,
            };
        }

        let found = piece_integral(piece);

        converged &= found.converged;
        add(&mut values, &found.values, 1.0);
    }
    Quadrature {
        converged: converged && values.iter().all(|value| value.is_finite()),
        values,
    }
}

pub(crate) fn gauss(
    piece: Interval,
    rule: &[(f64, f64)],
    mut integrand: impl FnMut(f64) -> Values,
) -> Values {
    let half = 0.5 * piece.length();
    let mut values = [0.0; COMPONENTS];

    for (node, weight) in rule {
        add(
            &mut values,
            &integrand(piece.middle() + half * node),
            weight * half,
        );
    }
    values
}

pub(crate) fn gauss_legendre(count: usize) -> Vec<(f64, f64)> {
    let order = count as f64;

    (0..count)
        .rev()
        .map(|index| {
            let mut node = (PI * (index as f64 + 0.75) / (order + 0.5)).cos_libm();

            for _ in 0..NEWTON_STEPS {
                let (value, slope) = legendre(count, node);

                let step = value / slope;

                node -= step;
                if step.abs() <= NODE_STEP {
                    break;
                }
            }

            let (_, slope) = legendre(count, node);
            (node, 2.0 / ((1.0 - node * node) * slope * slope))
        })
        .collect()
}

fn legendre(count: usize, at: f64) -> (f64, f64) {
    let (mut previous, mut current) = (1.0, at);

    for degree in 2..=count {
        let degree = degree as f64;
        let next = ((2.0 * degree - 1.0) * at * current - (degree - 1.0) * previous) / degree;

        previous = current;
        current = next;
    }
    (
        current,
        count as f64 * (at * current - previous) / (at * at - 1.0),
    )
}

pub(crate) fn mean_length(start: Vector3, end: Vector3) -> f64 {
    let (first, last) = (start.length(), end.length());

    let sum = first + last;

    let Some(direction) = (end - start).try_normalize() else {
        return first;
    };

    if sum == 0.0 {
        return 0.0;
    }

    let (from, to) = (start.dot(direction), end.dot(direction));
    let height = start.cross(direction).length();

    let along = from + to;

    let root = if height == 0.0 {
        0.0
    } else if from * to >= 0.0 {
        let spread = along / (to * first + from * last);
        let ratio = asinh_ratio((to - from) * spread);
        0.5 * height * height * spread * ratio
    } else {
        let change = (to / height).asinh_libm() - (from / height).asinh_libm();
        0.5 * height * height * change / (to - from)
    };
    0.25 * sum + 0.25 * along * along / sum + root
}

fn asinh_ratio(value: f64) -> f64 {
    if value == 0.0 {
        1.0
    } else {
        value.asinh_libm() / value
    }
}

pub(crate) fn pieces(range: Interval, breaks: impl IntoIterator<Item = f64>) -> Vec<Interval> {
    let sliver = SLIVER * range.length();
    let mut ends: Vec<f64> = breaks
        .into_iter()
        .filter(|parameter| {
            *parameter > range.start() + sliver && *parameter < range.end() - sliver
        })
        .collect();

    ends.sort_by(f64::total_cmp);
    ends.dedup_by(|later, kept| *later - *kept <= sliver);
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
