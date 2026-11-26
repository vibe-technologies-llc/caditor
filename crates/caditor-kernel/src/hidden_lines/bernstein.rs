use caditor_geometry::Vector3;

use crate::tally;

const RESTRICTION_WORK: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bernstein {
    width: usize,
    coefficients: Vec<f64>,
    magnitude: f64,
}

fn binomials(degree: usize) -> Vec<f64> {
    let mut row = Vec::with_capacity(degree + 1);
    let mut value = 1.0;

    for index in 0..=degree {
        row.push(value);
        value = value * (degree - index) as f64 / (index + 1) as f64;
    }
    row
}

fn keep_below(points: &mut [f64], at: f64) {
    for level in 1..points.len() {
        for index in (level..points.len()).rev() {
            let (Some(before), Some(here)) = (points.get(index - 1), points.get(index)) else {
                continue;
            };

            let blended = before + (here - before) * at;
            if let Some(slot) = points.get_mut(index) {
                *slot = blended;
            }
        }
    }
}

fn keep_above(points: &mut [f64], at: f64) {
    for level in 1..points.len() {
        for index in 0..points.len() - level {
            let (Some(here), Some(after)) = (points.get(index), points.get(index + 1)) else {
                continue;
            };

            let blended = here + (after - here) * at;
            if let Some(slot) = points.get_mut(index) {
                *slot = blended;
            }
        }
    }
}

fn restrict_line(points: &mut [f64], low: f64, high: f64) {
    if high < 1.0 {
        keep_below(points, high);
    }
    if low > 0.0 && high > 0.0 {
        keep_above(points, low / high);
    }
}

impl Bernstein {
    pub(super) fn new(width: usize, coefficients: Vec<f64>) -> Option<Self> {
        if width == 0 || coefficients.is_empty() || !coefficients.len().is_multiple_of(width) {
            return None;
        }

        let magnitude = coefficients
            .iter()
            .fold(0.0_f64, |most, value| most.max(value.abs()));
        magnitude.is_finite().then_some(Self {
            width,
            coefficients,
            magnitude,
        })
    }

    pub(super) fn line(coefficients: Vec<f64>) -> Option<Self> {
        Self::new(coefficients.len(), coefficients)
    }

    fn height(&self) -> usize {
        self.coefficients.len() / self.width
    }

    fn at(&self, column: usize, row: usize) -> f64 {
        self.coefficients
            .get(row * self.width + column)
            .copied()
            .unwrap_or(0.0)
    }

    fn rows(&self) -> impl Iterator<Item = &[f64]> {
        self.coefficients.chunks(self.width)
    }

    pub(super) fn along_u(&self) -> Option<Self> {
        let width = self.width.checked_sub(1).filter(|width| *width > 0)?;
        let coefficients = self
            .rows()
            .flat_map(|row| {
                row.windows(2).map(|pair| match pair {
                    [from, to] => to - from,
                    _ => 0.0,
                })
            })
            .collect();
        Self::new(width, coefficients)
    }

    pub(super) fn along_v(&self) -> Option<Self> {
        let rows = self.height().checked_sub(1).filter(|rows| *rows > 0)?;
        let coefficients = (0..rows)
            .flat_map(|row| {
                (0..self.width).map(move |column| self.at(column, row + 1) - self.at(column, row))
            })
            .collect();
        Self::new(self.width, coefficients)
    }

    fn weighted(&self) -> Vec<f64> {
        let (across, along) = (binomials(self.width - 1), binomials(self.height() - 1));
        self.rows()
            .zip(&along)
            .flat_map(|(line, along)| {
                line.iter()
                    .zip(&across)
                    .map(move |(value, across)| value * across * along)
            })
            .collect()
    }

    pub(super) fn product(&self, other: &Self) -> Self {
        let width = self.width + other.width - 1;
        let height = self.height() + other.height() - 1;
        let (own, theirs) = (self.weighted(), other.weighted());

        let mut coefficients = vec![0.0; width * height];

        for (own_row, own_line) in own.chunks(self.width).enumerate() {
            for (other_row, other_line) in theirs.chunks(other.width).enumerate() {
                let Some(line) = coefficients
                    .get_mut((own_row + other_row) * width..(own_row + other_row + 1) * width)
                else {
                    continue;
                };
                for (own_column, own_value) in own_line.iter().enumerate() {
                    let Some(slots) = line.get_mut(own_column..own_column + other.width) else {
                        continue;
                    };
                    for (slot, other_value) in slots.iter_mut().zip(other_line) {
                        *slot += own_value * other_value;
                    }
                }
            }
        }

        let (across, along) = (binomials(width - 1), binomials(height - 1));

        for (line, along) in coefficients.chunks_mut(width).zip(&along) {
            for (slot, across) in line.iter_mut().zip(&across) {
                *slot /= across * along;
            }
        }
        Self {
            width,
            coefficients,
            magnitude: self.magnitude * other.magnitude,
        }
    }

    pub(super) fn dot_of(
        first: &[Vector3],
        first_width: usize,
        second: &[Vector3],
        second_width: usize,
    ) -> Option<Self> {
        let first_height = first
            .len()
            .checked_div(first_width)
            .filter(|rows| *rows > 0)?;
        let second_height = second
            .len()
            .checked_div(second_width)
            .filter(|rows| *rows > 0)?;
        let weighted = |vectors: &[Vector3], width: usize, height: usize| -> Vec<Vector3> {
            let (across, along) = (binomials(width - 1), binomials(height - 1));
            vectors
                .chunks(width)
                .zip(&along)
                .flat_map(|(line, along)| {
                    line.iter()
                        .zip(&across)
                        .map(move |(vector, across)| *vector * (across * along))
                })
                .collect()
        };

        let own = weighted(first, first_width, first_height);
        let theirs = weighted(second, second_width, second_height);

        let width = first_width + second_width - 1;
        let height = first_height + second_height - 1;

        let mut coefficients = vec![0.0; width * height];

        for (own_row, own_line) in own.chunks(first_width).enumerate() {
            for (other_row, other_line) in theirs.chunks(second_width).enumerate() {
                let row = own_row + other_row;
                let Some(line) = coefficients.get_mut(row * width..(row + 1) * width) else {
                    continue;
                };
                for (own_column, own_vector) in own_line.iter().enumerate() {
                    let Some(slots) = line.get_mut(own_column..own_column + second_width) else {
                        continue;
                    };
                    for (slot, other_vector) in slots.iter_mut().zip(other_line) {
                        *slot += own_vector.dot(*other_vector);
                    }
                }
            }
        }

        let (across, along) = (binomials(width - 1), binomials(height - 1));

        for (line, along) in coefficients.chunks_mut(width).zip(&along) {
            for (slot, across) in line.iter_mut().zip(&across) {
                *slot /= across * along;
            }
        }

        let longest = |vectors: &[Vector3]| {
            vectors
                .iter()
                .fold(0.0_f64, |most, vector| most.max(vector.length()))
        };
        let magnitude = longest(first) * longest(second);
        magnitude.is_finite().then_some(Self {
            width,
            coefficients,
            magnitude,
        })
    }

    pub(super) fn scaled(&self, factor: f64) -> Self {
        Self {
            width: self.width,
            coefficients: self
                .coefficients
                .iter()
                .map(|value| value * factor)
                .collect(),
            magnitude: self.magnitude * factor.abs(),
        }
    }

    pub(super) fn plus(&self, other: &Self) -> Option<Self> {
        (self.width == other.width && self.coefficients.len() == other.coefficients.len()).then(
            || Self {
                width: self.width,
                coefficients: self
                    .coefficients
                    .iter()
                    .zip(&other.coefficients)
                    .map(|(own, theirs)| own + theirs)
                    .collect(),
                magnitude: self.magnitude + other.magnitude,
            },
        )
    }

    pub(super) fn restricted(
        &self,
        (u_low, u_high): (f64, f64),
        (v_low, v_high): (f64, f64),
    ) -> Self {
        let mut coefficients = self.coefficients.clone();

        for row in coefficients.chunks_mut(self.width) {
            restrict_line(row, u_low, u_high);
        }

        let height = self.height();
        let mut line = vec![0.0; height];

        for column in 0..self.width {
            for (row, slot) in line.iter_mut().enumerate() {
                *slot = coefficients
                    .get(row * self.width + column)
                    .copied()
                    .unwrap_or(0.0);
            }
            restrict_line(&mut line, v_low, v_high);
            for (row, value) in line.iter().enumerate() {
                if let Some(slot) = coefficients.get_mut(row * self.width + column) {
                    *slot = *value;
                }
            }
        }
        Self {
            width: self.width,
            coefficients,
            magnitude: self.magnitude,
        }
    }

    pub(super) fn may_hold(&self, positive: bool, slack: f64) -> bool {
        let floor = -slack * self.magnitude;
        if positive {
            self.coefficients.iter().any(|value| *value > floor)
        } else {
            self.coefficients.iter().any(|value| *value < floor)
        }
    }
}

type Range = (f64, f64);

#[derive(Clone, Debug, PartialEq)]
struct Quadrant {
    across: Range,
    along: Range,
    polynomial: Bernstein,
    children: Option<[usize; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Quadrants {
    nodes: Vec<Quadrant>,
}

fn overlap((low, high): Range, (from, to): Range) -> Option<Range> {
    let (start, end) = (low.max(from), high.min(to));
    (end > start).then_some((start, end))
}

fn local((start, end): Range, (low, high): Range) -> Range {
    ((start - low) / (high - low), (end - low) / (high - low))
}

impl Quadrants {
    pub(super) fn new(polynomial: Bernstein) -> Self {
        Self {
            nodes: vec![Quadrant {
                across: (0.0, 1.0),
                along: (0.0, 1.0),
                polynomial,
                children: None,
            }],
        }
    }

    fn children(&mut self, index: usize) -> Option<[usize; 4]> {
        let node = self.nodes.get(index)?;

        if let Some(children) = node.children {
            return Some(children);
        }

        let ((u_low, u_high), (v_low, v_high)) = (node.across, node.along);

        let (u_middle, v_middle) = (0.5 * (u_low + u_high), 0.5 * (v_low + v_high));

        let made: Vec<Quadrant> = [
            ((0.0, 0.5), (0.0, 0.5), (u_low, u_middle), (v_low, v_middle)),
            (
                (0.5, 1.0),
                (0.0, 0.5),
                (u_middle, u_high),
                (v_low, v_middle),
            ),
            (
                (0.0, 0.5),
                (0.5, 1.0),
                (u_low, u_middle),
                (v_middle, v_high),
            ),
            (
                (0.5, 1.0),
                (0.5, 1.0),
                (u_middle, u_high),
                (v_middle, v_high),
            ),
        ]
        .into_iter()
        .map(|(within_u, within_v, across, along)| Quadrant {
            across,
            along,
            polynomial: node.polynomial.restricted(within_u, within_v),
            children: None,
        })
        .collect();

        tally::add_searched(RESTRICTION_WORK * made.len());

        let first = self.nodes.len();
        let children = [first, first + 1, first + 2, first + 3];

        self.nodes.extend(made);
        if let Some(node) = self.nodes.get_mut(index) {
            node.children = Some(children);
        }
        Some(children)
    }

    pub(super) fn may_hold(
        &mut self,
        across: Range,
        along: Range,
        positive: bool,
        slack: f64,
    ) -> bool {
        let mut pending = vec![0_usize];

        while let Some(index) = pending.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            let (Some(u_part), Some(v_part)) =
                (overlap(node.across, across), overlap(node.along, along))
            else {
                continue;
            };

            if !node.polynomial.may_hold(positive, slack) {
                continue;
            }
            if u_part == node.across && v_part == node.along {
                return true;
            }

            let width = |(low, high): Range| high - low;
            let small = width(node.across) <= 2.0 * width(across)
                && width(node.along) <= 2.0 * width(along);

            if small {
                tally::add_searched(RESTRICTION_WORK);

                let held = node
                    .polynomial
                    .restricted(local(u_part, node.across), local(v_part, node.along))
                    .may_hold(positive, slack);

                if held {
                    return true;
                }
                continue;
            }
            pending.extend(self.children(index).into_iter().flatten());
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(polynomial: &Bernstein, u: f64, v: f64) -> f64 {
        let (columns, rows) = (polynomial.width - 1, polynomial.height() - 1);

        let (across, along) = (binomials(columns), binomials(rows));
        let mut sum = 0.0;

        for (row, along) in along.iter().enumerate() {
            for (column, across) in across.iter().enumerate() {
                sum += polynomial.at(column, row)
                    * across
                    * u.powi(column as i32)
                    * (1.0 - u).powi((columns - column) as i32)
                    * along
                    * v.powi(row as i32)
                    * (1.0 - v).powi((rows - row) as i32);
            }
        }
        sum
    }

    fn sample() -> Bernstein {
        Bernstein::new(3, vec![1.0, -2.0, 0.5, 3.0, 0.25, -1.5]).unwrap()
    }

    #[test]
    fn products_restrictions_and_sums_keep_the_values_of_the_polynomials() {
        let first = sample();
        let second = Bernstein::new(2, vec![2.0, -1.0, 0.0, 4.0, 1.0, -3.0]).unwrap();

        let product = first.product(&second);
        let restricted = first.restricted((0.2, 0.7), (0.1, 0.9));
        let sum = first.plus(&first.scaled(-3.0)).unwrap();

        for (u, v) in [(0.0, 0.0), (0.3, 0.6), (1.0, 0.25), (0.75, 1.0)] {
            let wanted = value(&first, u, v) * value(&second, u, v);

            assert!((value(&product, u, v) - wanted).abs() <= 1e-12);

            let (inner_u, inner_v) = (0.2 + 0.5 * u, 0.1 + 0.8 * v);

            assert!((value(&restricted, u, v) - value(&first, inner_u, inner_v)).abs() <= 1e-12);
            assert!((value(&sum, u, v) + 2.0 * value(&first, u, v)).abs() <= 1e-12);
        }
    }

    #[test]
    fn derivatives_are_differences_of_coefficients() {
        let polynomial = sample();

        let along_u = polynomial.along_u().unwrap();
        let along_v = polynomial.along_v().unwrap();

        let step = 1e-6;

        for (u, v) in [(0.2, 0.3), (0.8, 0.5)] {
            let slope_u =
                (value(&polynomial, u + step, v) - value(&polynomial, u - step, v)) / (2.0 * step);
            let slope_v =
                (value(&polynomial, u, v + step) - value(&polynomial, u, v - step)) / (2.0 * step);

            assert!((2.0 * value(&along_u, u, v) - slope_u).abs() <= 1e-6);
            assert!((value(&along_v, u, v) - slope_v).abs() <= 1e-6);
        }

        assert!(Bernstein::line(vec![1.0]).unwrap().along_u().is_none());
    }

    #[test]
    fn a_polynomial_of_one_sign_cannot_hold_the_other() {
        let above = Bernstein::new(2, vec![1.0, 2.0, 0.5, 3.0]).unwrap();
        let squared = above.product(&above);
        let cancelled = squared.plus(&squared.scaled(-1.0 - 1e-15)).unwrap();

        assert!(above.may_hold(true, 1e-10));
        assert!(!above.may_hold(false, 1e-10));
        assert!(above.scaled(-1.0).may_hold(false, 1e-10));
        assert!(!cancelled.may_hold(false, 1e-10));
        assert!(cancelled.may_hold(false, 0.0));
    }

    #[test]
    fn a_dot_product_of_vector_polynomials_is_the_sum_of_its_component_products() {
        let first = [
            Vector3::new(1.0, -2.0, 0.5),
            Vector3::new(0.0, 3.0, 1.0),
            Vector3::new(-1.0, 0.25, 2.0),
            Vector3::new(4.0, 1.0, -0.5),
        ];
        let second = [
            Vector3::new(2.0, 0.0, 1.0),
            Vector3::new(-1.0, 1.0, 0.0),
            Vector3::new(0.5, -3.0, 2.0),
        ];
        let component = |vectors: &[Vector3], width: usize, axis: usize| {
            Bernstein::new(width, vectors.iter().map(|vector| vector[axis]).collect()).unwrap()
        };

        let dotted = Bernstein::dot_of(&first, 2, &second, 3).unwrap();
        let summed = (0..3)
            .map(|axis| component(&first, 2, axis).product(&component(&second, 3, axis)))
            .reduce(|sum, term| sum.plus(&term).unwrap())
            .unwrap();

        for (found, wanted) in dotted.coefficients.iter().zip(&summed.coefficients) {
            assert!((found - wanted).abs() <= 1e-12);
        }
    }

    #[test]
    fn quadrants_tell_where_a_polynomial_may_change_sign() {
        let rising = Bernstein::new(2, vec![-1.0, 1.0, -1.0, 1.0]).unwrap();
        let mut quadrants = Quadrants::new(rising.clone());

        assert!(!quadrants.may_hold((0.0, 0.2), (0.3, 0.6), true, 0.0));
        assert!(quadrants.may_hold((0.0, 0.2), (0.3, 0.6), false, 0.0));
        assert!(!quadrants.may_hold((0.6, 0.9), (0.0, 0.1), false, 0.0));
        assert!(quadrants.may_hold((0.45, 0.55), (0.7, 0.8), true, 0.0));
        assert!(quadrants.may_hold((0.45, 0.55), (0.7, 0.8), false, 0.0));

        for step in 0..20 {
            let low = step as f64 / 20.0;
            let range = (low, low + 0.05);

            assert_eq!(
                quadrants.may_hold(range, (0.2, 0.25), true, 0.0),
                rising.restricted(range, (0.2, 0.25)).may_hold(true, 0.0),
                "{range:?}"
            );
        }
    }
}
