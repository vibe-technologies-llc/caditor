const PIVOT_TOLERANCE: f64 = 1e-12;

pub(crate) struct Banded {
    size: usize,
    reach: usize,
    entries: Vec<f64>,
}

impl Banded {
    pub fn zeros(size: usize, reach: usize) -> Self {
        Self {
            size,
            reach,
            entries: vec![0.0; size * (2 * reach + 1)],
        }
    }

    fn slot(&self, row: usize, column: usize) -> Option<usize> {
        let offset = (column + self.reach).checked_sub(row)?;
        (offset <= 2 * self.reach && row < self.size && column < self.size)
            .then(|| row * (2 * self.reach + 1) + offset)
    }

    pub fn get(&self, row: usize, column: usize) -> f64 {
        self.slot(row, column)
            .and_then(|slot| self.entries.get(slot))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn add(&mut self, row: usize, column: usize, value: f64) -> Option<()> {
        let slot = self.slot(row, column)?;
        *self.entries.get_mut(slot)? += value;
        Some(())
    }

    fn set(&mut self, row: usize, column: usize, value: f64) {
        if let Some(entry) = self
            .slot(row, column)
            .and_then(|slot| self.entries.get_mut(slot))
        {
            *entry = value;
        }
    }

    pub fn solve<const WIDTH: usize>(
        mut self,
        mut right: Vec<[f64; WIDTH]>,
    ) -> Option<Vec<[f64; WIDTH]>> {
        if right.len() != self.size {
            return None;
        }
        let largest = self
            .entries
            .iter()
            .fold(0.0_f64, |largest, entry| largest.max(entry.abs()));
        for pivot_row in 0..self.size {
            let pivot = self.get(pivot_row, pivot_row);
            if !pivot.is_finite() || pivot.abs() <= PIVOT_TOLERANCE * largest {
                return None;
            }
            let last = (pivot_row + self.reach).min(self.size - 1);
            for row in pivot_row + 1..=last {
                let factor = self.get(row, pivot_row) / pivot;
                if factor == 0.0 {
                    continue;
                }
                for column in pivot_row..=last {
                    let updated = self.get(row, column) - factor * self.get(pivot_row, column);
                    self.set(row, column, updated);
                }
                let above = *right.get(pivot_row)?;
                let target = right.get_mut(row)?;
                for (value, pivot_value) in target.iter_mut().zip(above) {
                    *value -= factor * pivot_value;
                }
            }
        }
        for row in (0..self.size).rev() {
            let last = (row + self.reach).min(self.size - 1);
            let mut value = *right.get(row)?;
            for column in row + 1..=last {
                let known = *right.get(column)?;
                let coefficient = self.get(row, column);
                for (entry, known) in value.iter_mut().zip(known) {
                    *entry -= coefficient * known;
                }
            }
            let pivot = self.get(row, row);
            for entry in &mut value {
                *entry /= pivot;
            }
            *right.get_mut(row)? = value;
        }
        right
            .iter()
            .all(|row| row.iter().all(|entry| entry.is_finite()))
            .then_some(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_banded_system_is_solved_like_a_dense_one() {
        let size = 7;
        let mut matrix = Banded::zeros(size, 2);
        let entry = |row: usize, column: usize| {
            if row == column {
                6.0 + row as f64
            } else if row.abs_diff(column) <= 2 {
                1.0 / (1.0 + row as f64 + 2.0 * column as f64)
            } else {
                0.0
            }
        };
        for row in 0..size {
            for column in 0..size {
                if row.abs_diff(column) <= 2 {
                    matrix.add(row, column, entry(row, column)).unwrap();
                }
            }
        }
        let expected: Vec<[f64; 2]> = (0..size)
            .map(|index| [index as f64 - 3.0, (index * index) as f64])
            .collect();
        let right: Vec<[f64; 2]> = (0..size)
            .map(|row| {
                let mut sum = [0.0; 2];
                for (column, known) in expected.iter().enumerate() {
                    for axis in 0..2 {
                        sum[axis] += entry(row, column) * known[axis];
                    }
                }
                sum
            })
            .collect();
        let solved = matrix.solve(right).unwrap();
        for (found, wanted) in solved.iter().zip(&expected) {
            for axis in 0..2 {
                assert!((found[axis] - wanted[axis]).abs() < 1e-10);
            }
        }
        assert!(Banded::zeros(3, 1).solve(vec![[1.0]; 3]).is_none());
    }
}
