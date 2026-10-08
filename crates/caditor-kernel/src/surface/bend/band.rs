use caditor_geometry::Vector3;

use crate::error::GeometryError;

pub(super) struct Band {
    count: usize,
    reach: usize,
    entries: Vec<f64>,
}

impl Band {
    pub(super) fn new(count: usize, reach: usize) -> Self {
        Self {
            count,
            reach,
            entries: vec![0.0; count * (reach + 1)],
        }
    }

    fn slot(&self, row: usize, column: usize) -> Option<usize> {
        let offset = row.checked_sub(column)?;
        (offset <= self.reach && row < self.count).then_some(row * (self.reach + 1) + offset)
    }

    pub(super) fn add(&mut self, row: usize, column: usize, value: f64) {
        if let Some(entry) = self
            .slot(row, column)
            .and_then(|slot| self.entries.get_mut(slot))
        {
            *entry += value;
        }
    }

    fn at(&self, row: usize, column: usize) -> f64 {
        self.slot(row, column)
            .and_then(|slot| self.entries.get(slot))
            .copied()
            .unwrap_or(0.0)
    }

    pub(super) fn largest_diagonal(&self) -> f64 {
        (0..self.count)
            .map(|index| self.at(index, index))
            .fold(0.0, f64::max)
    }

    pub(super) fn factored(mut self) -> Result<Self, GeometryError> {
        for row in 0..self.count {
            let first = row.saturating_sub(self.reach);
            for column in first..=row {
                let mut sum = self.at(row, column);
                for k in first.max(column.saturating_sub(self.reach))..column {
                    sum -= self.at(row, k) * self.at(column, k);
                }
                let value = if row == column {
                    if !(sum > 0.0 && sum.is_finite()) {
                        return Err(GeometryError::NonFinite);
                    }
                    sum.sqrt()
                } else {
                    sum / self.at(column, column)
                };
                if let Some(entry) = self
                    .slot(row, column)
                    .and_then(|slot| self.entries.get_mut(slot))
                {
                    *entry = value;
                }
            }
        }
        Ok(self)
    }

    pub(super) fn solve(&self, right: Vec<Vector3>) -> Vec<Vector3> {
        let mut values = right;
        for row in 0..self.count {
            let mut sum = values.get(row).copied().unwrap_or(Vector3::ZERO);
            for k in row.saturating_sub(self.reach)..row {
                sum -= values.get(k).copied().unwrap_or(Vector3::ZERO) * self.at(row, k);
            }
            if let Some(entry) = values.get_mut(row) {
                *entry = sum / self.at(row, row);
            }
        }
        for row in (0..self.count).rev() {
            let mut sum = values.get(row).copied().unwrap_or(Vector3::ZERO);
            for k in row + 1..(row + self.reach + 1).min(self.count) {
                sum -= values.get(k).copied().unwrap_or(Vector3::ZERO) * self.at(k, row);
            }
            if let Some(entry) = values.get_mut(row) {
                *entry = sum / self.at(row, row);
            }
        }
        values
    }
}
