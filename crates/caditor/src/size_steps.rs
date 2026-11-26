const FINE_POINTS: f64 = 1.5;
const ROUND_PULL_POINTS: f64 = 3.0;
const MANTISSAS: [f64; 3] = [1.0, 2.0, 5.0];
const MOST_DECADES: i32 = 300;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    mantissa: f64,
    decade: i32,
}

impl Step {
    fn at_least(length: f64) -> Option<Self> {
        if !(length.is_finite() && length > 0.0) {
            return None;
        }

        let decade = length.log10().floor();

        if decade.abs() > f64::from(MOST_DECADES) {
            return None;
        }

        let decade = decade as i32;
        Some(
            MANTISSAS
                .into_iter()
                .map(|mantissa| Self { mantissa, decade })
                .find(|step| step.length() >= length)
                .unwrap_or(Self {
                    mantissa: 1.0,
                    decade: decade + 1,
                }),
        )
    }

    fn rounder(self) -> Self {
        if self.mantissa < 1.5 {
            Self {
                mantissa: 5.0,
                decade: self.decade,
            }
        } else if self.mantissa < 3.5 {
            Self {
                mantissa: 1.0,
                decade: self.decade + 1,
            }
        } else {
            Self {
                mantissa: 5.0,
                decade: self.decade + 1,
            }
        }
    }

    fn length(self) -> f64 {
        in_decade(self.mantissa, self.decade)
    }

    fn nearest(self, size: f64) -> f64 {
        in_decade((size / self.length()).round() * self.mantissa, self.decade)
    }
}

fn in_decade(value: f64, decade: i32) -> f64 {
    if decade < 0 {
        value / 10.0_f64.powi(-decade)
    } else {
        value * 10.0_f64.powi(decade)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeSteps {
    fine: Step,
    round: Step,
    pull: f64,
}

impl SizeSteps {
    pub fn at(per_point: f64) -> Option<Self> {
        let fine = Step::at_least(per_point * FINE_POINTS)?;
        Some(Self {
            fine,
            round: fine.rounder(),
            pull: per_point * ROUND_PULL_POINTS,
        })
    }

    pub fn rounded(&self, size: f64) -> f64 {
        if !(size.is_finite() && size > 0.0) {
            return size;
        }

        let round = self.round.nearest(size);

        if round > 0.0 && (round - size).abs() <= self.pull {
            return round;
        }

        let fine = self.fine.nearest(size);
        if fine > 0.0 { fine } else { size }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_move_in_fine_steps_and_are_pulled_onto_rounder_ones() {
        let steps = SizeSteps::at(0.054).unwrap();

        assert_eq!(steps.fine.length(), 0.1);
        assert_eq!(steps.round.length(), 0.5);
        assert_eq!(steps.rounded(24.87), 25.0);
        assert_eq!(steps.rounded(25.13), 25.0);
        assert_eq!(steps.rounded(24.76), 24.8);
        assert_eq!(steps.rounded(24.31), 24.3);
        assert_eq!(steps.rounded(0.3), 0.3);
    }

    #[test]
    fn every_step_is_one_two_or_five_in_its_decade_and_the_rounder_one_five_or_ten_times_it() {
        for (per_point, fine, round) in [
            (0.0054, 0.01, 0.05),
            (0.012, 0.02, 0.1),
            (0.03, 0.05, 0.5),
            (0.34, 1.0, 5.0),
            (1.0, 2.0, 10.0),
            (3.0, 5.0, 50.0),
            (40.0, 100.0, 500.0),
        ] {
            let steps = SizeSteps::at(per_point).unwrap();

            assert_eq!(steps.fine.length(), fine, "{per_point}");
            assert_eq!(steps.round.length(), round, "{per_point}");
        }
    }

    #[test]
    fn a_rounded_size_reads_as_typed() {
        let steps = SizeSteps::at(0.054).unwrap();

        for tenth in 1..2_000 {
            let typed = f64::from(tenth) / 10.0;
            let rounded = steps.rounded(typed + 0.004);

            assert_eq!(rounded, (rounded * 10.0).round() / 10.0, "{typed}");
            assert!((rounded - typed).abs() <= 0.2, "{typed} became {rounded}");
        }
    }

    #[test]
    fn a_size_is_never_rounded_away_and_no_scale_means_no_steps() {
        let steps = SizeSteps::at(1.0).unwrap();

        assert_eq!(steps.rounded(0.4), 0.4);
        assert_eq!(steps.rounded(0.0), 0.0);
        assert!(steps.rounded(f64::NAN).is_nan());
        assert_eq!(SizeSteps::at(0.0), None);
        assert_eq!(SizeSteps::at(f64::INFINITY), None);
        assert_eq!(SizeSteps::at(1e-320), None);
    }
}
