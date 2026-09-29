use std::collections::BTreeSet;

use crate::profile::ProfileCurve;

pub(super) const PROBE_BUDGET: usize = 64;
pub(super) const NAMED_AT_MOST: usize = 6;

pub(super) fn culprits(
    curves: &[ProfileCurve],
    still_fails: impl Fn(&[ProfileCurve]) -> bool,
) -> Vec<u64> {
    let mut kept = curves.to_vec();
    let mut probes = PROBE_BUDGET;
    let mut chunk = kept.len() / 2;
    while chunk > 0 && probes > 0 {
        let mut start = 0;
        let mut shrunk = false;
        while start < kept.len() && probes > 0 {
            let removed = start..start + chunk;
            let candidate: Vec<ProfileCurve> = kept
                .iter()
                .enumerate()
                .filter(|(index, _)| !removed.contains(index))
                .map(|(_, curve)| curve.clone())
                .collect();
            probes -= 1;
            if !candidate.is_empty() && still_fails(&candidate) {
                kept = candidate;
                shrunk = true;
            } else {
                start += chunk;
            }
        }
        if !shrunk {
            chunk /= 2;
        } else {
            chunk = chunk.min(kept.len() / 2).max(1);
        }
    }
    if kept.len() > NAMED_AT_MOST {
        return Vec::new();
    }
    kept.iter()
        .map(|curve| curve.entity)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
