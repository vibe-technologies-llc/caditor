const PIVOT_FLOOR: f64 = 1e-300;
const DAMPING: f64 = 1e-13;

fn entry(matrix: &[f64], size: usize, row: usize, column: usize) -> f64 {
    matrix.get(row * size + column).copied().unwrap_or(0.0)
}

fn swap_rows(matrix: &mut [f64], size: usize, first: usize, second: usize) {
    for column in 0..size {
        let a = entry(matrix, size, first, column);
        let b = entry(matrix, size, second, column);
        if let Some(slot) = matrix.get_mut(first * size + column) {
            *slot = b;
        }
        if let Some(slot) = matrix.get_mut(second * size + column) {
            *slot = a;
        }
    }
}

pub(crate) fn solve_dense(
    size: usize,
    mut matrix: Vec<f64>,
    mut rhs: Vec<f64>,
) -> Option<Vec<f64>> {
    if matrix.len() != size * size || rhs.len() != size {
        return None;
    }
    for pivot in 0..size {
        let (best, magnitude) = (pivot..size)
            .map(|row| (row, entry(&matrix, size, row, pivot).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if magnitude.is_nan() || magnitude <= PIVOT_FLOOR {
            return None;
        }
        if best != pivot {
            swap_rows(&mut matrix, size, best, pivot);
            let (a, b) = (
                rhs.get(best).copied().unwrap_or(0.0),
                rhs.get(pivot).copied().unwrap_or(0.0),
            );
            if let Some(slot) = rhs.get_mut(best) {
                *slot = b;
            }
            if let Some(slot) = rhs.get_mut(pivot) {
                *slot = a;
            }
        }
        let diagonal = entry(&matrix, size, pivot, pivot);
        let pivot_value = rhs.get(pivot).copied().unwrap_or(0.0);
        for row in pivot + 1..size {
            let factor = entry(&matrix, size, row, pivot) / diagonal;
            if factor == 0.0 {
                continue;
            }
            for column in pivot..size {
                let value = entry(&matrix, size, row, column)
                    - factor * entry(&matrix, size, pivot, column);
                if let Some(slot) = matrix.get_mut(row * size + column) {
                    *slot = value;
                }
            }
            if let Some(slot) = rhs.get_mut(row) {
                *slot -= factor * pivot_value;
            }
        }
    }
    let mut solution = vec![0.0; size];
    for row in (0..size).rev() {
        let known: f64 = (row + 1..size)
            .map(|column| {
                entry(&matrix, size, row, column) * solution.get(column).copied().unwrap_or(0.0)
            })
            .sum();
        let value = (rhs.get(row).copied().unwrap_or(0.0) - known) / entry(&matrix, size, row, row);
        if let Some(slot) = solution.get_mut(row) {
            *slot = value;
        }
    }
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
}

pub(crate) fn damped_least_squares(rows: &[Vec<f64>], values: &[f64]) -> Option<Vec<f64>> {
    let width = rows.first().map(Vec::len)?;
    let scales: Vec<f64> = (0..width)
        .map(|column| {
            let norm = rows
                .iter()
                .map(|row| row.get(column).copied().unwrap_or(0.0).powi(2))
                .sum::<f64>()
                .sqrt();
            if norm.is_finite() && norm > PIVOT_FLOOR {
                norm
            } else {
                1.0
            }
        })
        .collect();
    let scaled = |row: &Vec<f64>, column: usize| {
        row.get(column).copied().unwrap_or(0.0) / scales.get(column).copied().unwrap_or(1.0)
    };
    let mut normal = vec![0.0; width * width];
    let mut gradient = vec![0.0; width];
    for (row, value) in rows.iter().zip(values) {
        for i in 0..width {
            let a = scaled(row, i);
            if let Some(slot) = gradient.get_mut(i) {
                *slot -= a * value;
            }
            for j in 0..width {
                if let Some(slot) = normal.get_mut(i * width + j) {
                    *slot += a * scaled(row, j);
                }
            }
        }
    }
    let largest = (0..width)
        .map(|i| entry(&normal, width, i, i))
        .fold(0.0, f64::max);
    for i in 0..width {
        if let Some(slot) = normal.get_mut(i * width + i) {
            *slot += DAMPING * largest + PIVOT_FLOOR;
        }
    }
    let solution = solve_dense(width, normal, gradient)?;
    Some(
        solution
            .iter()
            .zip(&scales)
            .map(|(value, scale)| value / scale)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_systems_solve_with_pivoting() {
        let solution = solve_dense(
            3,
            vec![0.0, 2.0, 1.0, 1.0, 1.0, 0.0, 3.0, 0.0, 1.0],
            vec![5.0, 3.0, 6.0],
        )
        .unwrap();
        for (value, expected) in solution.iter().zip([1.4, 1.6, 1.8]) {
            assert!((value - expected).abs() < 1e-12);
        }
        assert!(solve_dense(2, vec![1.0, 2.0, 2.0, 4.0], vec![1.0, 2.0]).is_none());
    }

    #[test]
    fn damped_least_squares_gives_the_minimal_norm_step() {
        let step = damped_least_squares(&[vec![1.0, 1.0]], &[-2.0]).unwrap();
        assert!((step[0] - 1.0).abs() < 1e-9 && (step[1] - 1.0).abs() < 1e-9);
        let fitted = damped_least_squares(
            &[vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]],
            &[-1.0, -2.0, -3.0],
        )
        .unwrap();
        assert!((fitted[0] - 1.0).abs() < 1e-9 && (fitted[1] - 2.0).abs() < 1e-9);
    }
}
