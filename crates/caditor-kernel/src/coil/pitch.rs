use std::f64::consts::TAU;

pub const END_CHANGE_TURNS: f64 = 0.5;
const STOPS: usize = 6;
const BREAK_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stop {
    turns: f64,
    pitch: f64,
}

impl Stop {
    fn pitch_towards(self, to: Self, turns: f64) -> f64 {
        let length = to.turns - self.turns;
        if length <= 0.0 {
            return to.pitch;
        }
        self.pitch + (to.pitch - self.pitch) * (turns - self.turns) / length
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PitchLaw {
    stops: [Stop; STOPS],
}

impl PitchLaw {
    pub(super) fn even(pitch: f64) -> Self {
        Self {
            stops: [Stop { turns: 0.0, pitch }; STOPS],
        }
    }

    pub(super) fn closed_ends(body: f64, end: f64, end_turns: f64, turns: f64) -> Self {
        let stop = |turns: f64, pitch: f64| Stop { turns, pitch };
        Self {
            stops: [
                stop(0.0, end),
                stop(end_turns, end),
                stop(end_turns + END_CHANGE_TURNS, body),
                stop(turns - end_turns - END_CHANGE_TURNS, body),
                stop(turns - end_turns, end),
                stop(turns, end),
            ],
        }
    }

    fn pairs(&self) -> impl Iterator<Item = (Stop, Stop)> + '_ {
        self.stops
            .iter()
            .copied()
            .zip(self.stops.iter().copied().skip(1))
    }

    pub(super) fn pitch_at(&self, angle: f64) -> f64 {
        let turns = angle / TAU;
        let [first, .., last] = self.stops;
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
        let [first, .., last] = self.stops;
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
