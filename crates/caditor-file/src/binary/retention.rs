use std::collections::BTreeSet;

const ALWAYS_KEPT: usize = 10;
const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;

struct Tier {
    younger_than: u64,
    spacing: u64,
}

const TIERS: [Tier; 4] = [
    Tier {
        younger_than: DAY,
        spacing: HOUR,
    },
    Tier {
        younger_than: MONTH,
        spacing: DAY,
    },
    Tier {
        younger_than: YEAR,
        spacing: WEEK,
    },
    Tier {
        younger_than: u64::MAX,
        spacing: MONTH,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Candidate {
    pub saved_at: Option<u64>,
    pub kept: bool,
}

pub(super) fn retained(candidates: &[Candidate], now: u64) -> Vec<bool> {
    let mut occupied = BTreeSet::new();
    let mut listed = 0_usize;
    candidates
        .iter()
        .map(|candidate| {
            let Some(saved) = candidate.saved_at else {
                return true;
            };
            listed += 1;
            let age = now.saturating_sub(saved);
            let slot = TIERS
                .iter()
                .enumerate()
                .find(|(_, tier)| age < tier.younger_than)
                .map(|(index, tier)| (index, saved / tier.spacing));
            let first_in_slot = slot.is_none_or(|slot| occupied.insert(slot));
            listed <= ALWAYS_KEPT || candidate.kept || first_in_slot
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000 * YEAR;

    fn plain(saved_at: &[Option<u64>]) -> Vec<Candidate> {
        saved_at
            .iter()
            .map(|saved_at| Candidate {
                saved_at: *saved_at,
                kept: false,
            })
            .collect()
    }

    fn kept(saved_at: &[Option<u64>]) -> usize {
        retained(&plain(saved_at), NOW)
            .into_iter()
            .filter(|kept| *kept)
            .count()
    }

    #[test]
    fn the_newest_versions_are_always_kept() {
        let within_a_minute: Vec<Option<u64>> = (0..10).map(|second| Some(NOW - second)).collect();
        assert_eq!(kept(&within_a_minute), 10);

        let mut more = within_a_minute.clone();
        more.extend((10..30).map(|second| Some(NOW - second)));
        assert_eq!(retained(&plain(&more), NOW)[..10], [true; 10]);
        assert_eq!(kept(&more), 10);
    }

    #[test]
    fn older_versions_thin_out_to_one_per_hour_day_week_and_month() {
        let every_ten_minutes: Vec<Option<u64>> =
            (0..6 * 48).map(|step| Some(NOW - step * 600)).collect();
        let kept_count = kept(&every_ten_minutes);
        assert!(
            (10 + 24..=10 + 24 + 2 + 2).contains(&kept_count),
            "{kept_count}"
        );

        let hourly_for_two_years: Vec<Option<u64>> = (0..2 * 365 * 24)
            .map(|step| Some(NOW - step * HOUR))
            .collect();
        let kept_count = kept(&hourly_for_two_years);
        assert!(
            (24 + 29 + 47 + 12..=24 + 30 + 49 + 14).contains(&kept_count),
            "{kept_count}"
        );

        let decisions = retained(&plain(&hourly_for_two_years), NOW);
        let oldest_kept = decisions.iter().rposition(|kept| *kept).unwrap();
        assert!(oldest_kept + 31 * 24 >= hourly_for_two_years.len());
    }

    #[test]
    fn versions_in_a_slot_the_newest_ones_already_hold_are_dropped() {
        let saved_at: Vec<Option<u64>> = (0..12)
            .map(|step| Some(NOW - 2 * HOUR - step * 60))
            .collect();
        let decisions = retained(&plain(&saved_at), NOW);
        assert_eq!(decisions.iter().filter(|kept| **kept).count(), 10);
        assert!(decisions[..10].iter().all(|kept| *kept));
    }

    #[test]
    fn unlisted_versions_and_times_in_the_future_are_kept() {
        let mut saved_at: Vec<Option<u64>> = (0..10).map(|step| Some(NOW - step)).collect();
        saved_at.extend([None, Some(NOW + YEAR), None, Some(NOW + YEAR + 1)]);
        assert_eq!(
            retained(&plain(&saved_at), NOW)[10..],
            [true, true, true, false]
        );
    }

    #[test]
    fn a_version_marked_kept_survives_whatever_its_age_or_slot() {
        let hourly_for_two_years: Vec<Option<u64>> = (0..2 * 365 * 24)
            .map(|step| Some(NOW - step * HOUR))
            .collect();
        let marked = [40, 41, 500, 5_000, 17_000];
        let candidates: Vec<Candidate> = hourly_for_two_years
            .iter()
            .enumerate()
            .map(|(position, saved_at)| Candidate {
                saved_at: *saved_at,
                kept: marked.contains(&position),
            })
            .collect();

        let decisions = retained(&candidates, NOW);

        assert!(marked.iter().all(|position| decisions[*position]));
        let unmarked = retained(&plain(&hourly_for_two_years), NOW);
        let extra = decisions.iter().filter(|kept| **kept).count()
            - unmarked.iter().filter(|kept| **kept).count();
        assert!(extra <= marked.len(), "{extra}");
    }

    #[test]
    fn a_kept_version_in_a_slot_holds_it_so_older_ones_in_it_still_thin_out() {
        let newest: Vec<Option<u64>> = (0..10).map(|second| Some(NOW - second)).collect();
        let in_one_day = [Some(NOW - 3 * DAY - HOUR), Some(NOW - 3 * DAY - 2 * HOUR)];
        let saved_at: Vec<Option<u64>> = newest.into_iter().chain(in_one_day).collect();

        let unmarked = retained(&plain(&saved_at), NOW);
        assert_eq!(unmarked[10..], [true, false]);

        let mut older_marked = plain(&saved_at);
        older_marked[11].kept = true;
        assert_eq!(retained(&older_marked, NOW)[10..], [true, true]);

        let mut newer_marked = plain(&saved_at);
        newer_marked[10].kept = true;
        assert_eq!(retained(&newer_marked, NOW)[10..], [true, false]);
    }
}
