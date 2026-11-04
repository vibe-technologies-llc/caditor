use std::f64::consts::TAU;

use crate::tolerance::LINEAR_RESOLUTION;

pub const END_CHANGE_TURNS: f64 = 0.5;
const BREAK_SLACK: f64 = 1e-9;
const ROUNDING_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchStop {
    pub turns: f64,
    pub pitch: f64,
}

impl PitchStop {
    fn pitch_towards(self, to: Self, turns: f64) -> f64 {
        let length = to.turns - self.turns;
        if length <= 0.0 {
            return to.pitch;
        }
        self.pitch + (to.pitch - self.pitch) * (turns - self.turns) / length
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum TableFault {
    NonFinite,
    NoPitch,
    Order { turns: f64 },
    Beyond { turns: f64 },
}

pub(super) fn checked_table(table: &[PitchStop], turns: f64) -> Result<(), TableFault> {
    let mut before = 0.0;
    for stop in table {
        if !stop.turns.is_finite() || !stop.pitch.is_finite() {
            return Err(TableFault::NonFinite);
        }
        if stop.pitch <= LINEAR_RESOLUTION {
            return Err(TableFault::NoPitch);
        }
        if stop.turns <= before + ROUNDING_SLACK {
            return Err(TableFault::Order { turns: stop.turns });
        }
        if stop.turns > turns + ROUNDING_SLACK {
            return Err(TableFault::Beyond { turns: stop.turns });
        }
        before = stop.turns;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ClosedEndLaw {
    pub pitch: f64,
    pub turns: f64,
    pub change: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PitchLaw {
    stops: Vec<PitchStop>,
}

impl PitchLaw {
    pub(super) fn even(pitch: f64) -> Self {
        Self {
            stops: vec![PitchStop { turns: 0.0, pitch }],
        }
    }

    pub(super) fn progressive(first: f64, last: f64, turns: f64) -> Self {
        Self {
            stops: vec![
                PitchStop {
                    turns: 0.0,
                    pitch: first,
                },
                PitchStop { turns, pitch: last },
            ],
        }
    }

    pub(super) fn tabled(first: f64, table: &[PitchStop]) -> Self {
        let start = PitchStop {
            turns: 0.0,
            pitch: first,
        };
        Self {
            stops: std::iter::once(start)
                .chain(table.iter().copied())
                .collect(),
        }
    }

    pub(super) fn closed_ends((first, last): (f64, f64), ends: ClosedEndLaw, turns: f64) -> Self {
        let stop = |turns: f64, pitch: f64| PitchStop { turns, pitch };
        let ClosedEndLaw {
            pitch: end,
            turns: end_turns,
            change,
        } = ends;
        Self {
            stops: vec![
                stop(0.0, end),
                stop(end_turns, end),
                stop(end_turns + change, first),
                stop(turns - end_turns - change, last),
                stop(turns - end_turns, end),
                stop(turns, end),
            ],
        }
    }

    pub(super) fn closed_ends_tabled(
        first: f64,
        table: &[PitchStop],
        ends: ClosedEndLaw,
        turns: f64,
    ) -> Self {
        let stop = |turns: f64, pitch: f64| PitchStop { turns, pitch };
        let ClosedEndLaw {
            pitch: end,
            turns: end_turns,
            change,
        } = ends;
        let body_start = end_turns + change;
        let body_end = turns - body_start;
        let last = table.last().map_or(first, |row| row.pitch);
        let mut stops = vec![
            stop(0.0, end),
            stop(end_turns, end),
            stop(body_start, first),
        ];
        stops.extend(
            table
                .iter()
                .map(|row| stop(body_start + row.turns, row.pitch)),
        );
        let held_to = stops.last().map_or(body_start, |last| last.turns);
        if body_end > held_to + BREAK_SLACK {
            stops.push(stop(body_end, last));
        }
        stops.extend([stop(turns - end_turns, end), stop(turns, end)]);
        Self { stops }
    }

    pub(super) fn narrowest(&self, turns: f64) -> f64 {
        self.stops
            .iter()
            .filter(|stop| stop.turns < turns)
            .map(|stop| stop.pitch)
            .chain([self.pitch_at(TAU * turns)])
            .fold(f64::INFINITY, f64::min)
    }

    pub(super) fn widest(&self, turns: f64) -> f64 {
        self.stops
            .iter()
            .filter(|stop| stop.turns < turns)
            .map(|stop| stop.pitch)
            .chain([self.pitch_at(TAU * turns)])
            .fold(0.0, f64::max)
    }

    fn bounds(&self) -> (PitchStop, PitchStop) {
        let first = self.stops.first().copied().unwrap_or(PitchStop {
            turns: 0.0,
            pitch: 0.0,
        });
        (first, self.stops.last().copied().unwrap_or(first))
    }

    fn pairs(&self) -> impl Iterator<Item = (PitchStop, PitchStop)> + '_ {
        self.stops
            .iter()
            .copied()
            .zip(self.stops.iter().copied().skip(1))
    }

    pub(super) fn pitch_at(&self, angle: f64) -> f64 {
        let turns = angle / TAU;
        let (first, last) = self.bounds();
        if turns <= first.turns {
            return first.pitch;
        }
        if turns >= last.turns {
            return last.pitch;
        }
        self.pairs()
            .find(|(_, to)| turns <= to.turns)
            .map_or(last.pitch, |(from, to)| from.pitch_towards(to, turns))
    }

    pub(super) fn rise(&self, angle: f64) -> f64 {
        let turns = angle / TAU;
        let (first, last) = self.bounds();
        if turns <= first.turns {
            return first.pitch * turns;
        }
        let mut rise = first.pitch * first.turns;
        for (from, to) in self.pairs() {
            if turns <= from.turns {
                break;
            }
            let until = turns.min(to.turns);
            rise += 0.5 * (from.pitch + from.pitch_towards(to, until)) * (until - from.turns);
        }
        if turns > last.turns {
            rise += last.pitch * (turns - last.turns);
        }
        rise
    }

    pub(super) fn turns_for(&self, height: f64) -> f64 {
        let (first, last) = self.bounds();
        let mut rise = first.pitch * first.turns;
        for (from, to) in self.pairs() {
            let span = to.turns - from.turns;
            let spanned = 0.5 * (from.pitch + to.pitch) * span;
            if span > 0.0 && rise + spanned >= height {
                let slope = (to.pitch - from.pitch) / span;
                let needed = height - rise;
                let root = (from.pitch * from.pitch + 2.0 * slope * needed).max(0.0);
                let into = 2.0 * needed / (from.pitch + root.sqrt());
                return from.turns + into;
            }
            rise += spanned;
        }
        last.turns + (height - rise) / last.pitch
    }

    pub(super) fn middle_rise(&self, start: f64, half: f64) -> f64 {
        self.rise(start) + half * self.pitch_at(start) / TAU
    }

    pub(super) fn breaks(&self, turns: f64) -> Vec<f64> {
        let mut breaks = vec![0.0];
        breaks.extend(
            self.stops
                .iter()
                .map(|stop| stop.turns)
                .filter(|at| *at > 0.0 && *at < turns),
        );
        breaks.push(turns);
        breaks.dedup_by(|later, earlier| *later - *earlier <= BREAK_SLACK);
        breaks
    }
}
