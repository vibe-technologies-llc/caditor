const MAX_NODES: usize = 8192;
const NARROWEST: f64 = 1e-9;

pub(crate) fn graded(
    (from, to): (f64, f64),
    junctions: &[f64],
    spans: usize,
    deviations: impl Fn(&[f64]) -> Option<Vec<f64>>,
    tolerance: f64,
) -> Vec<f64> {
    let mut nodes: Vec<f64> = (0..=spans)
        .map(|index| from + (to - from) * index as f64 / spans as f64)
        .chain(
            junctions
                .iter()
                .copied()
                .filter(|junction| from < *junction && *junction < to),
        )
        .collect();
    nodes.sort_by(f64::total_cmp);
    nodes.dedup_by(|later, earlier| *later - *earlier < NARROWEST);
    loop {
        let Some(deviation) = deviations(&nodes) else {
            return nodes;
        };
        let wide: Vec<f64> = nodes
            .windows(2)
            .zip(&deviation)
            .filter_map(|(pair, deviation)| match pair {
                [low, high] if *deviation > tolerance && high - low > 2.0 * NARROWEST => {
                    Some(0.5 * (low + high))
                }
                _ => None,
            })
            .collect();
        if wide.is_empty() || nodes.len() + wide.len() > MAX_NODES {
            return nodes;
        }
        nodes.extend(wide);
        nodes.sort_by(f64::total_cmp);
    }
}
