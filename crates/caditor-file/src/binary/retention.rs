use std::collections::BTreeSet;

const ALWAYS_KEPT: usize = 10;
const MORE_ALWAYS_KEPT: usize = 25;
const FEWER_ALWAYS_KEPT: usize = 5;
const QUARTER_HOUR: u64 = 900;
const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;
const MONTH: u64 = 30 * DAY;
const YEAR: u64 = 365 * DAY;

struct Tier {
    younger_than: u64,
    spacing: u64,
}

const MORE_TIERS: [Tier; 4] = [
    Tier {
        younger_than: 2 * DAY,
        spacing: QUARTER_HOUR,
    },
    Tier {
        younger_than: 3 * MONTH,
        spacing: 4 * HOUR,
    },
    Tier {
        younger_than: 2 * YEAR,
        spacing: DAY,
    },
    Tier {
        younger_than: u64::MAX,
        spacing: WEEK,
    },
];

const FEWER_TIERS: [Tier; 4] = [
    Tier {
        younger_than: DAY,
        spacing: 6 * HOUR,
    },
    Tier {
        younger_than: MONTH,
        spacing: WEEK,
    },
    Tier {
        younger_than: YEAR,
        spacing: MONTH,
    },
    Tier {
        younger_than: u64::MAX,
        spacing: YEAR,
    },
];

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Retention {
    More,
    #[default]
    Standard,
    Fewer,
}

impl Retention {
    pub const ALL: [Self; 3] = [Self::More, Self::Standard, Self::Fewer];

    fn always_kept(self) -> usize {
        match self {
            Self::More => MORE_ALWAYS_KEPT,
            Self::Standard => ALWAYS_KEPT,
            Self::Fewer => FEWER_ALWAYS_KEPT,
        }
    }

    fn tiers(self) -> &'static [Tier] {
        match self {
            Self::More => &MORE_TIERS,
            Self::Standard => &TIERS,
            Self::Fewer => &FEWER_TIERS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Candidate {
    pub saved_at: Option<u64>,
    pub kept: bool,
}

pub(super) fn retained(candidates: &[Candidate], now: u64, retention: Retention) -> Vec<bool> {
    let always_kept = retention.always_kept();
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
            let slot = retention
                .tiers()
                .iter()
                .enumerate()
                .find(|(_, tier)| age < tier.younger_than)
                .map(|(index, tier)| (index, saved / tier.spacing));
            let first_in_slot = slot.is_none_or(|slot| occupied.insert(slot));
            listed <= always_kept || candidate.kept || first_in_slot
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
        kept_with(saved_at, Retention::Standard)
    }

    fn kept_with(saved_at: &[Option<u64>], retention: Retention) -> usize {
        retained(&plain(saved_at), NOW, retention)
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
        assert_eq!(
            retained(&plain(&more), NOW, Retention::Standard)[..10],
            [true; 10]
        );
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

        let decisions = retained(&plain(&hourly_for_two_years), NOW, Retention::Standard);
        let oldest_kept = decisions.iter().rposition(|kept| *kept).unwrap();
        assert!(oldest_kept + 31 * 24 >= hourly_for_two_years.len());
    }

    #[test]
    fn versions_in_a_slot_the_newest_ones_already_hold_are_dropped() {
        let saved_at: Vec<Option<u64>> = (0..12)
            .map(|step| Some(NOW - 2 * HOUR - step * 60))
            .collect();
        let decisions = retained(&plain(&saved_at), NOW, Retention::Standard);
        assert_eq!(decisions.iter().filter(|kept| **kept).count(), 10);
        assert!(decisions[..10].iter().all(|kept| *kept));
    }

    #[test]
    fn unlisted_versions_and_times_in_the_future_are_kept() {
        let mut saved_at: Vec<Option<u64>> = (0..10).map(|step| Some(NOW - step)).collect();
        saved_at.extend([None, Some(NOW + YEAR), None, Some(NOW + YEAR + 1)]);
        assert_eq!(
            retained(&plain(&saved_at), NOW, Retention::Standard)[10..],
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

        let decisions = retained(&candidates, NOW, Retention::Standard);

        assert!(marked.iter().all(|position| decisions[*position]));
        let unmarked = retained(&plain(&hourly_for_two_years), NOW, Retention::Standard);
        let extra = decisions.iter().filter(|kept| **kept).count()
            - unmarked.iter().filter(|kept| **kept).count();
        assert!(extra <= marked.len(), "{extra}");
    }

    #[test]
    fn a_kept_version_in_a_slot_holds_it_so_older_ones_in_it_still_thin_out() {
        let newest: Vec<Option<u64>> = (0..10).map(|second| Some(NOW - second)).collect();
        let in_one_day = [Some(NOW - 3 * DAY - HOUR), Some(NOW - 3 * DAY - 2 * HOUR)];
        let saved_at: Vec<Option<u64>> = newest.into_iter().chain(in_one_day).collect();

        let unmarked = retained(&plain(&saved_at), NOW, Retention::Standard);
        assert_eq!(unmarked[10..], [true, false]);

        let mut older_marked = plain(&saved_at);
        older_marked[11].kept = true;
        assert_eq!(
            retained(&older_marked, NOW, Retention::Standard)[10..],
            [true, true]
        );

        let mut newer_marked = plain(&saved_at);
        newer_marked[10].kept = true;
        assert_eq!(
            retained(&newer_marked, NOW, Retention::Standard)[10..],
            [true, false]
        );
    }

    #[test]
    fn more_keeps_more_and_fewer_keeps_fewer_versions_of_the_same_history() {
        let every_ten_minutes_for_a_year: Vec<Option<u64>> = (0..6 * 24 * 365)
            .map(|step| Some(NOW - step * 600))
            .collect();

        let more = kept_with(&every_ten_minutes_for_a_year, Retention::More);
        let standard = kept_with(&every_ten_minutes_for_a_year, Retention::Standard);
        let fewer = kept_with(&every_ten_minutes_for_a_year, Retention::Fewer);

        assert!(more > 2 * standard, "{more} {standard}");
        assert!(fewer * 2 < standard, "{fewer} {standard}");

        let newest: Vec<Option<u64>> = (0..40).map(|second| Some(NOW - second)).collect();
        assert_eq!(kept_with(&newest, Retention::More), MORE_ALWAYS_KEPT);
        assert_eq!(kept_with(&newest, Retention::Standard), ALWAYS_KEPT);
        assert_eq!(kept_with(&newest, Retention::Fewer), FEWER_ALWAYS_KEPT);
    }

    #[test]
    fn every_preset_keeps_marked_and_unlisted_versions() {
        let mut candidates = plain(&[None; 3]);
        candidates.extend((0..40_u64).map(|step| Candidate {
            saved_at: Some(NOW - step),
            kept: step == 39,
        }));
        for retention in Retention::ALL {
            let decisions = retained(&candidates, NOW, retention);
            assert!(decisions[..3].iter().all(|kept| *kept), "{retention:?}");
            assert_eq!(decisions.last(), Some(&true), "{retention:?}");
        }
    }
}
